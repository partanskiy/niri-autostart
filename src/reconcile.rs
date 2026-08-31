use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
#[cfg(test)]
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;

#[cfg(test)]
use niri_ipc::Event;
use niri_ipc::{Action, SizeChange, WorkspaceReferenceArg};

use crate::config::{ColumnSpec, Config, SizeSpec, WindowSpec, WorkspaceSpec};
use crate::error::{NiriAutostartError, Result};
use crate::event_adapter::EventAdapter;
use crate::ipc::CommandClient;
#[cfg(test)]
use crate::ipc::EventMessage;
use crate::predicate;
#[cfg(test)]
use crate::reducer::apply_event;
use crate::runtime_state::{RuntimeState, WindowKey};
use crate::state::ActualState;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const STABLE_FOCUS_QUIET: Duration = Duration::from_millis(150);

#[cfg(test)]
pub fn bootstrap_initial_state(
    rx: &Receiver<EventMessage>,
    timeout: Duration,
) -> Result<ActualState> {
    let start = Instant::now();
    let mut state = ActualState::default();
    let mut saw_workspaces = false;
    let mut saw_windows = false;

    while !(saw_workspaces && saw_windows) {
        let remaining =
            timeout
                .checked_sub(start.elapsed())
                .ok_or_else(|| NiriAutostartError::Timeout {
                    what: "initial niri event-stream state".to_string(),
                    timeout,
                })?;

        match rx.recv_timeout(remaining) {
            Ok(EventMessage::Event(event)) => {
                if matches!(event, Event::WorkspacesChanged { .. }) {
                    saw_workspaces = true;
                }
                if matches!(event, Event::WindowsChanged { .. }) {
                    saw_windows = true;
                }
                apply_event(&mut state, event);
            }
            Ok(EventMessage::Closed(message)) => {
                return Err(NiriAutostartError::EventStreamClosed(message));
            }
            Err(RecvTimeoutError::Timeout) => {
                return Err(NiriAutostartError::Timeout {
                    what: "initial niri event-stream state".to_string(),
                    timeout,
                });
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(NiriAutostartError::EventStreamClosed(
                    "event thread disconnected".to_string(),
                ));
            }
        }
    }

    Ok(state)
}

pub struct Reconciler {
    commands: CommandClient,
    events: EventAdapter,
    state: ActualState,
    runtime_state: RuntimeState,
    runtime_state_path: PathBuf,
    app_id_counts: HashMap<String, usize>,
}

impl Reconciler {
    pub fn new(
        commands: CommandClient,
        events: EventAdapter,
        runtime_state: RuntimeState,
        runtime_state_path: PathBuf,
    ) -> Self {
        let state = events.state();
        Self {
            commands,
            events,
            state,
            runtime_state,
            runtime_state_path,
            app_id_counts: HashMap::new(),
        }
    }

    fn window_id_by_spec(
        &mut self,
        key: &WindowKey,
        spec: &WindowSpec,
        preferred_workspace: Option<&str>,
    ) -> Result<Option<u64>> {
        self.sync_state();

        if let Some(window_id) = self.runtime_state.window_id(key, spec, &self.state) {
            return Ok(Some(window_id));
        }

        if self.app_id_counts.get(&spec.app_id).copied() != Some(1) {
            return Ok(None);
        }

        let Some(window_id) = self
            .state
            .preferred_window_id_by_app_id(&spec.app_id, preferred_workspace)
            .or_else(|| self.state.preferred_window_id_by_app_id(&spec.app_id, None))
        else {
            return Ok(None);
        };

        self.record_runtime_window(key, spec, window_id)?;
        Ok(Some(window_id))
    }

    pub fn run(&mut self, config: &Config) -> Result<()> {
        self.app_id_counts = app_id_counts(config);

        for workspace in &config.workspaces {
            self.reconcile_workspace(workspace)?;
        }

        self.wait_until_quiet(
            DEFAULT_TIMEOUT,
            STABLE_FOCUS_QUIET,
            "event stream to settle before final focus".to_string(),
        )?;
        self.finalize_focus(config)?;

        Ok(())
    }

    fn finalize_focus(&mut self, config: &Config) -> Result<()> {
        for workspace in &config.workspaces {
            self.focus_workspace_first_window(workspace)?;
        }

        if let Some(workspace) = config.workspaces.first() {
            self.focus_workspace_first_window(workspace)?;
        }

        Ok(())
    }

    fn reconcile_workspace(&mut self, workspace: &WorkspaceSpec) -> Result<()> {
        if !predicate::workspace_known(&self.state, &workspace.name) {
            return Err(NiriAutostartError::MissingWorkspace(workspace.name.clone()));
        }

        if let Some(output) = &workspace.output {
            self.ensure_workspace_output(&workspace.name, output)?;
        }
        self.ensure_workspace_active(&workspace.name)?;

        for (column_idx, column) in workspace.columns.iter().enumerate() {
            self.reconcile_column(workspace, column_idx + 1, column)?;
        }

        self.focus_workspace_first_window(workspace)?;

        Ok(())
    }

    fn focus_workspace_first_window(&mut self, workspace: &WorkspaceSpec) -> Result<()> {
        let Some(first_window) = workspace
            .columns
            .first()
            .and_then(|column| column.windows.first())
        else {
            return Ok(());
        };
        let key = WindowKey::new(&workspace.name, 1, 1);

        let window_id = self
            .window_id_by_spec(&key, first_window, Some(&workspace.name))?
            .ok_or_else(|| NiriAutostartError::MissingWindow(window_label(&key, first_window)))?;

        self.ensure_workspace_active(&workspace.name)?;
        self.wait_until_quiet(
            DEFAULT_TIMEOUT,
            STABLE_FOCUS_QUIET,
            format!(
                "workspace {:?} to settle before final focus",
                workspace.name
            ),
        )?;
        self.commands
            .action(Action::FocusWindow { id: window_id })?;
        self.wait_for_stable(
            DEFAULT_TIMEOUT,
            STABLE_FOCUS_QUIET,
            format!(
                "workspace {:?} first window {:?} to become focused",
                workspace.name,
                window_label(&key, first_window)
            ),
            |state| {
                let window_focused = state
                    .windows
                    .get(&window_id)
                    .is_some_and(|window| window.is_focused);
                let workspace_active_window = state
                    .workspace_by_name(&workspace.name)
                    .is_some_and(|workspace| workspace.active_window_id == Some(window_id));

                window_focused && workspace_active_window
            },
        )
    }

    fn reconcile_column(
        &mut self,
        workspace: &WorkspaceSpec,
        column_index: usize,
        column: &ColumnSpec,
    ) -> Result<()> {
        let first = column
            .windows
            .first()
            .ok_or_else(|| NiriAutostartError::Validation("column without windows".to_string()))?;

        let first_key = WindowKey::new(&workspace.name, column_index, 1);
        let first_id = self.ensure_window_present(workspace, &first_key, first)?;
        self.ensure_primary_window_position(workspace, &first_key, first, first_id, column_index)?;
        let mut positioned = vec![(first_key, first, first_id)];

        for (row_index, window) in column.windows.iter().enumerate().skip(1) {
            let target_row = row_index + 1;
            let key = WindowKey::new(&workspace.name, column_index, target_row);
            let window_id = self.ensure_window_present(workspace, &key, window)?;
            self.ensure_stacked_window_position(
                workspace,
                &key,
                window,
                window_id,
                column_index,
                target_row,
            )?;
            positioned.push((key, window, window_id));
        }

        if positioned
            .iter()
            .any(|(_, window, _)| scrolling_layout_target(window, column_index, 1).is_some())
        {
            self.ensure_workspace_active(&workspace.name)?;
            self.commands.action(Action::FocusColumn {
                index: column_index,
            })?;
            self.commands.action(Action::SetColumnWidth {
                change: column.width.to_size_change(),
            })?;
        }

        for (key, window, window_id) in positioned {
            let label = window_label(&key, window);
            self.apply_window_floating(window_id, window.floating, &label)?;
            if window.floating {
                self.apply_window_width(window_id, &label, column.width)?;
            }
            self.apply_window_height(window_id, &label, window.height)?;
        }

        Ok(())
    }

    fn ensure_workspace_active(&mut self, workspace: &str) -> Result<()> {
        self.commands.action(Action::FocusWorkspace {
            reference: WorkspaceReferenceArg::Name(workspace.to_string()),
        })?;
        self.wait_for(
            DEFAULT_TIMEOUT,
            format!("workspace {workspace:?} to become active"),
            |state| predicate::workspace_active(state, workspace),
        )
    }

    fn ensure_workspace_output(&mut self, workspace: &str, output: &str) -> Result<()> {
        if predicate::workspace_on_output(&self.state, workspace, output) {
            return Ok(());
        }

        self.commands.action(Action::MoveWorkspaceToMonitor {
            output: output.to_string(),
            reference: Some(WorkspaceReferenceArg::Name(workspace.to_string())),
        })?;
        self.wait_for(
            DEFAULT_TIMEOUT,
            format!("workspace {workspace:?} to move to output {output:?}"),
            |state| predicate::workspace_on_output(state, workspace, output),
        )
    }

    fn ensure_window_present(
        &mut self,
        workspace: &WorkspaceSpec,
        key: &WindowKey,
        spec: &WindowSpec,
    ) -> Result<u64> {
        let label = window_label(key, spec);
        let mut window_id = self.window_id_by_spec(key, spec, Some(&workspace.name))?;

        if window_id.is_none() {
            let before = self.state.windows.keys().copied().collect::<HashSet<_>>();
            self.commands.action(Action::Spawn {
                command: spec.command.clone(),
            })?;
            let spawned_id =
                self.wait_for_spawned_window(DEFAULT_TIMEOUT, &label, &before, spec)?;
            self.record_runtime_window(key, spec, spawned_id)?;
            window_id = Some(spawned_id);
        }

        let window_id =
            window_id.ok_or_else(|| NiriAutostartError::MissingWindow(label.clone()))?;

        self.ensure_workspace_active(&workspace.name)?;

        if !predicate::window_id_on_workspace(&self.state, window_id, &workspace.name) {
            self.commands.action(Action::MoveWindowToWorkspace {
                window_id: Some(window_id),
                reference: WorkspaceReferenceArg::Name(workspace.name.clone()),
                focus: false,
            })?;
            self.wait_for(
                DEFAULT_TIMEOUT,
                format!("window {label:?} to move to workspace {:?}", workspace.name),
                |state| predicate::window_id_on_workspace(state, window_id, &workspace.name),
            )?;
        }

        Ok(window_id)
    }

    fn ensure_primary_window_position(
        &mut self,
        workspace: &WorkspaceSpec,
        key: &WindowKey,
        spec: &WindowSpec,
        window_id: u64,
        target_column: usize,
    ) -> Result<()> {
        let label = window_label(key, spec);
        self.apply_window_floating(window_id, spec.floating, &label)?;
        let Some((target_column, target_row)) = scrolling_layout_target(spec, target_column, 1)
        else {
            return Ok(());
        };
        self.ensure_window_row(window_id, &label, target_row)?;

        if predicate::window_id_at_position(
            &self.state,
            window_id,
            &workspace.name,
            target_column,
            target_row,
        ) {
            return Ok(());
        }

        self.ensure_workspace_active(&workspace.name)?;
        self.commands
            .action(Action::FocusWindow { id: window_id })?;
        self.wait_for(
            DEFAULT_TIMEOUT,
            format!("window {label:?} to become focused"),
            |state| {
                state
                    .windows
                    .get(&window_id)
                    .is_some_and(|window| window.is_focused)
            },
        )?;
        self.commands.action(Action::MoveColumnToIndex {
            index: target_column,
        })?;
        self.wait_for(
            DEFAULT_TIMEOUT,
            format!(
                "window {:?} to reach column {} row {}",
                label, target_column, target_row
            ),
            |state| {
                state.windows.get(&window_id).is_some_and(|window| {
                    window.workspace_id == state.workspace_id_by_name(&workspace.name)
                        && window.layout.pos_in_scrolling_layout
                            == Some((target_column, target_row))
                })
            },
        )
    }

    fn ensure_stacked_window_position(
        &mut self,
        workspace: &WorkspaceSpec,
        key: &WindowKey,
        spec: &WindowSpec,
        window_id: u64,
        target_column: usize,
        target_row: usize,
    ) -> Result<()> {
        let label = window_label(key, spec);
        self.apply_window_floating(window_id, spec.floating, &label)?;
        let Some((target_column, target_row)) =
            scrolling_layout_target(spec, target_column, target_row)
        else {
            return Ok(());
        };

        if predicate::window_id_at_position(
            &self.state,
            window_id,
            &workspace.name,
            target_column,
            target_row,
        ) {
            return Ok(());
        }

        let (current_column, _) = self
            .state
            .window_position_by_id(window_id)
            .ok_or_else(|| NiriAutostartError::MissingWindow(label.clone()))?;

        if current_column == target_column {
            return self.ensure_window_row(window_id, &label, target_row);
        }

        self.ensure_workspace_active(&workspace.name)?;
        self.commands
            .action(Action::FocusWindow { id: window_id })?;
        self.wait_for(
            DEFAULT_TIMEOUT,
            format!("window {label:?} to become focused"),
            |state| {
                state
                    .windows
                    .get(&window_id)
                    .is_some_and(|window| window.is_focused)
            },
        )?;

        let desired_column = target_column + 1;
        if current_column != desired_column {
            self.commands.action(Action::MoveColumnToIndex {
                index: desired_column,
            })?;
            self.wait_for(
                DEFAULT_TIMEOUT,
                format!(
                    "window {:?} to move to helper column {}",
                    label, desired_column
                ),
                |state| {
                    state
                        .window_position_by_id(window_id)
                        .is_some_and(|(column, _)| column == desired_column)
                },
            )?;
        }

        let helper_column = self
            .state
            .window_position_by_id(window_id)
            .map(|(column, _)| column)
            .ok_or_else(|| NiriAutostartError::MissingWindow(label.clone()))?;
        if helper_column != desired_column {
            return Err(NiriAutostartError::NonAdjacentColumn {
                app_id: label.clone(),
                actual: helper_column,
                expected_left: target_column,
            });
        }

        self.commands.action(Action::FocusColumn {
            index: target_column,
        })?;
        self.commands.action(Action::ConsumeWindowIntoColumn {})?;
        self.wait_for(
            DEFAULT_TIMEOUT,
            format!(
                "window {:?} to reach column {} row {}",
                label, target_column, target_row
            ),
            |state| {
                state.windows.get(&window_id).is_some_and(|window| {
                    window.workspace_id == state.workspace_id_by_name(&workspace.name)
                        && window.layout.pos_in_scrolling_layout
                            == Some((target_column, target_row))
                }) && predicate::column_has_window_count(
                    state,
                    &workspace.name,
                    target_column,
                    target_row,
                )
            },
        )
    }

    fn ensure_window_row(&mut self, window_id: u64, app_id: &str, target_row: usize) -> Result<()> {
        loop {
            let (_, current_row) = self
                .state
                .window_position_by_id(window_id)
                .ok_or_else(|| NiriAutostartError::MissingWindow(app_id.to_string()))?;
            if current_row == target_row {
                return Ok(());
            }

            self.commands
                .action(Action::FocusWindow { id: window_id })?;
            self.wait_for(
                DEFAULT_TIMEOUT,
                format!("window {:?} to become focused", app_id),
                |state| {
                    state
                        .windows
                        .get(&window_id)
                        .is_some_and(|window| window.is_focused)
                },
            )?;

            if current_row < target_row {
                self.commands.action(Action::MoveWindowDown {})?;
                let next_row = current_row + 1;
                self.wait_for(
                    DEFAULT_TIMEOUT,
                    format!("window {:?} to move down to row {}", app_id, next_row),
                    |state| {
                        state
                            .window_position_by_id(window_id)
                            .is_some_and(|(_, row)| row == next_row)
                    },
                )?;
            } else {
                self.commands.action(Action::MoveWindowUp {})?;
                let next_row = current_row - 1;
                self.wait_for(
                    DEFAULT_TIMEOUT,
                    format!("window {:?} to move up to row {}", app_id, next_row),
                    |state| {
                        state
                            .window_position_by_id(window_id)
                            .is_some_and(|(_, row)| row == next_row)
                    },
                )?;
            }
        }
    }

    fn apply_window_floating(
        &mut self,
        window_id: u64,
        floating: bool,
        app_id: &str,
    ) -> Result<()> {
        let is_floating = self
            .state
            .windows
            .get(&window_id)
            .map(|window| window.is_floating)
            .ok_or_else(|| NiriAutostartError::MissingWindow(app_id.to_string()))?;

        if floating == is_floating {
            return Ok(());
        }

        let action = if floating {
            Action::MoveWindowToFloating {
                id: Some(window_id),
            }
        } else {
            Action::MoveWindowToTiling {
                id: Some(window_id),
            }
        };
        self.commands.action(action)?;
        self.wait_for(
            DEFAULT_TIMEOUT,
            format!("window {:?} floating state to become {}", app_id, floating),
            |state| {
                state
                    .windows
                    .get(&window_id)
                    .is_some_and(|window| window.is_floating == floating)
            },
        )
    }

    fn apply_window_height(
        &mut self,
        window_id: u64,
        app_id: &str,
        height: SizeSpec,
    ) -> Result<()> {
        self.commands.action(Action::SetWindowHeight {
            id: Some(window_id),
            change: height.to_size_change(),
        })?;

        if let SizeSpec::Fixed(expected) = height {
            self.wait_for(
                DEFAULT_TIMEOUT,
                format!("window {:?} height to become {}", app_id, expected),
                |state| {
                    let current_width = state
                        .windows
                        .get(&window_id)
                        .map(|window| window.layout.tile_size.0)
                        .unwrap_or_default();
                    state.windows.get(&window_id).is_some_and(|window| {
                        (window.layout.tile_size.0 - current_width).abs() <= 1.0
                            && (window.layout.tile_size.1 - f64::from(expected)).abs() <= 1.0
                    })
                },
            )?;
        }

        Ok(())
    }

    fn apply_window_width(&mut self, window_id: u64, app_id: &str, width: SizeSpec) -> Result<()> {
        self.commands.action(Action::SetWindowWidth {
            id: Some(window_id),
            change: width.to_size_change(),
        })?;

        if let SizeSpec::Fixed(expected) = width {
            self.wait_for(
                DEFAULT_TIMEOUT,
                format!("window {:?} width to become {}", app_id, expected),
                |state| {
                    state.windows.get(&window_id).is_some_and(|window| {
                        (window.layout.tile_size.0 - f64::from(expected)).abs() <= 1.0
                    })
                },
            )?;
        }

        Ok(())
    }

    fn wait_for_spawned_window(
        &mut self,
        timeout: Duration,
        label: &str,
        before: &HashSet<u64>,
        spec: &WindowSpec,
    ) -> Result<u64> {
        self.events
            .wait_for(timeout, format!("window {label:?} to appear"), |state| {
                Self::spawned_window_id_in_state(state, before, spec).is_some()
            })?;
        self.sync_state();

        self.spawned_window_id(before, spec)
            .ok_or_else(|| NiriAutostartError::MissingWindow(label.to_string()))
    }

    fn spawned_window_id(&self, before: &HashSet<u64>, spec: &WindowSpec) -> Option<u64> {
        Self::spawned_window_id_in_state(&self.state, before, spec)
    }

    fn spawned_window_id_in_state(
        state: &ActualState,
        before: &HashSet<u64>,
        spec: &WindowSpec,
    ) -> Option<u64> {
        state
            .windows
            .values()
            .filter(|window| {
                !before.contains(&window.id)
                    && window.app_id.as_deref() == Some(spec.app_id.as_str())
            })
            .max_by_key(|window| (!window.is_floating, window.is_focused, window.id))
            .map(|window| window.id)
    }

    fn record_runtime_window(
        &mut self,
        key: &WindowKey,
        spec: &WindowSpec,
        window_id: u64,
    ) -> Result<()> {
        let window = self
            .state
            .windows
            .get(&window_id)
            .cloned()
            .ok_or_else(|| NiriAutostartError::MissingWindow(window_label(key, spec)))?;

        self.runtime_state.record(key, spec, &window);
        self.runtime_state.persist(&self.runtime_state_path)
    }

    fn wait_for<F>(&mut self, timeout: Duration, what: String, predicate: F) -> Result<()>
    where
        F: Fn(&ActualState) -> bool,
    {
        self.events.wait_for(timeout, what, predicate)?;
        self.sync_state();
        Ok(())
    }

    fn wait_for_stable<F>(
        &mut self,
        timeout: Duration,
        quiet_for: Duration,
        what: String,
        predicate: F,
    ) -> Result<()>
    where
        F: Fn(&ActualState) -> bool,
    {
        self.events
            .wait_for_stable(timeout, quiet_for, what, predicate)?;
        self.sync_state();
        Ok(())
    }

    fn wait_until_quiet(
        &mut self,
        timeout: Duration,
        quiet_for: Duration,
        what: String,
    ) -> Result<()> {
        self.events.wait_until_quiet(timeout, quiet_for, what)?;
        self.sync_state();
        Ok(())
    }

    fn sync_state(&mut self) {
        self.state = self.events.state();
    }
}

fn app_id_counts(config: &Config) -> HashMap<String, usize> {
    let mut counts = HashMap::new();

    for workspace in &config.workspaces {
        for column in &workspace.columns {
            for window in &column.windows {
                *counts.entry(window.app_id.clone()).or_insert(0) += 1;
            }
        }
    }

    counts
}

fn window_label(key: &WindowKey, spec: &WindowSpec) -> String {
    format!("{} at {}", spec.app_id, key)
}

fn scrolling_layout_target(spec: &WindowSpec, column: usize, row: usize) -> Option<(usize, usize)> {
    (!spec.floating).then_some((column, row))
}

impl SizeSpec {
    pub fn to_size_change(self) -> SizeChange {
        match self {
            SizeSpec::Fixed(value) => SizeChange::SetFixed(value),
            // niri IPC expects proportions in percent units, while the KDL schema uses
            // normalized fractions like 0.33333 and 0.5 to match niri config style.
            SizeSpec::Proportion(value) => SizeChange::SetProportion(value * 100.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ActualState;
    use niri_ipc::{Timestamp, Window, WindowLayout, Workspace};
    use std::sync::mpsc;

    fn window(id: u64, app_id: &str, workspace_id: u64, column: usize, row: usize) -> Window {
        Window {
            id,
            title: Some(app_id.into()),
            app_id: Some(app_id.into()),
            pid: Some(1),
            workspace_id: Some(workspace_id),
            is_focused: false,
            is_floating: false,
            is_urgent: false,
            layout: WindowLayout {
                pos_in_scrolling_layout: Some((column, row)),
                tile_size: (100.0, 100.0),
                window_size: (100, 100),
                tile_pos_in_workspace_view: None,
                window_offset_in_tile: (0.0, 0.0),
            },
            focus_timestamp: Some(Timestamp { secs: 0, nanos: 0 }),
        }
    }

    #[test]
    fn bootstrap_collects_initial_state() {
        let (tx, rx) = mpsc::channel();
        tx.send(EventMessage::Event(Event::WorkspacesChanged {
            workspaces: vec![Workspace {
                id: 1,
                idx: 1,
                name: Some("firework".into()),
                output: Some("HDMI-A-1".into()),
                is_urgent: false,
                is_active: true,
                is_focused: true,
                active_window_id: None,
            }],
        }))
        .unwrap();
        tx.send(EventMessage::Event(Event::WindowsChanged {
            windows: vec![window(1, "fw-fastfetch", 1, 1, 1)],
        }))
        .unwrap();

        let state = bootstrap_initial_state(&rx, Duration::from_secs(1)).unwrap();
        assert_eq!(state.workspace_id_by_name("firework"), Some(1));
        assert_eq!(state.first_window_id_by_app_id("fw-fastfetch"), Some(1));
    }

    #[test]
    fn bootstrap_times_out_when_stream_never_delivers_state() {
        let (_tx, rx) = mpsc::channel();
        let err = bootstrap_initial_state(&rx, Duration::from_millis(10)).unwrap_err();
        assert!(matches!(err, NiriAutostartError::Timeout { .. }));
    }

    #[test]
    fn integration_like_sequence_reaches_final_layout_without_sleep() {
        let mut state = ActualState::default();
        state.replace_workspaces(vec![Workspace {
            id: 1,
            idx: 1,
            name: Some("firework".into()),
            output: Some("HDMI-A-1".into()),
            is_urgent: false,
            is_active: true,
            is_focused: true,
            active_window_id: None,
        }]);

        apply_event(
            &mut state,
            Event::WindowsChanged {
                windows: vec![window(1, "fw-fastfetch", 1, 1, 1)],
            },
        );
        apply_event(
            &mut state,
            Event::WindowOpenedOrChanged {
                window: window(2, "fw-tty-clock", 1, 2, 1),
            },
        );
        apply_event(
            &mut state,
            Event::WindowLayoutsChanged {
                changes: vec![(
                    2,
                    WindowLayout {
                        pos_in_scrolling_layout: Some((1, 2)),
                        tile_size: (633.0, 207.0),
                        window_size: (633, 207),
                        tile_pos_in_workspace_view: None,
                        window_offset_in_tile: (0.0, 0.0),
                    },
                )],
            },
        );

        assert!(predicate::window_at_position(
            &state,
            "fw-tty-clock",
            "firework",
            1,
            2
        ));
    }

    #[test]
    fn converts_fractional_proportions_to_ipc_percent_units() {
        assert_eq!(
            SizeSpec::Proportion(0.5).to_size_change(),
            SizeChange::SetProportion(50.0)
        );
        assert_eq!(
            SizeSpec::Proportion(0.33333).to_size_change(),
            SizeChange::SetProportion(33.333)
        );
    }

    #[test]
    fn floating_windows_have_no_scrolling_layout_target() {
        let mut floating = window(1, "btop", 1, 1, 1);
        floating.is_floating = true;
        floating.layout.pos_in_scrolling_layout = None;

        let spec = WindowSpec {
            app_id: "btop".into(),
            command: vec!["btop".into()],
            height: SizeSpec::Fixed(822),
            floating: true,
        };

        assert!(floating.layout.pos_in_scrolling_layout.is_none());
        assert_eq!(scrolling_layout_target(&spec, 1, 1), None);

        let tiled_spec = WindowSpec {
            floating: false,
            ..spec
        };
        assert_eq!(scrolling_layout_target(&tiled_spec, 1, 1), Some((1, 1)));
    }
}
