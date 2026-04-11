use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use niri_ipc::{Action, Event, SizeChange, WorkspaceReferenceArg};

use crate::config::{ColumnSpec, Config, SizeSpec, WindowSpec, WorkspaceSpec};
use crate::error::{NiriAutostartError, Result};
use crate::ipc::{CommandClient, EventMessage};
use crate::predicate;
use crate::reducer::apply_event;
use crate::state::ActualState;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

pub fn bootstrap_initial_state(
    rx: &Receiver<EventMessage>,
    timeout: Duration,
) -> Result<ActualState> {
    let start = Instant::now();
    let mut state = ActualState::default();
    let mut saw_workspaces = false;
    let mut saw_windows = false;

    while !(saw_workspaces && saw_windows) {
        let remaining = timeout
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
    events: Receiver<EventMessage>,
    state: ActualState,
}

impl Reconciler {
    pub fn new(commands: CommandClient, events: Receiver<EventMessage>, state: ActualState) -> Self {
        Self {
            commands,
            events,
            state,
        }
    }

    pub fn run(&mut self, config: &Config) -> Result<()> {
        for workspace in &config.workspaces {
            self.reconcile_workspace(workspace)?;
        }

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

        self.ensure_workspace_active(&workspace.name)?;

        for (column_idx, column) in workspace.columns.iter().enumerate() {
            self.reconcile_column(workspace, column_idx + 1, column)?;
        }

        self.focus_workspace_first_window(workspace)?;

        Ok(())
    }

    fn focus_workspace_first_window(&mut self, workspace: &WorkspaceSpec) -> Result<()> {
        let Some(first_window) = workspace.columns.first().and_then(|column| column.windows.first()) else {
            return Ok(());
        };

        let window_id = self
            .state
            .window_id_by_app_id_on_workspace(&first_window.app_id, &workspace.name)
            .ok_or_else(|| NiriAutostartError::MissingWindow(first_window.app_id.clone()))?;

        self.ensure_workspace_active(&workspace.name)?;
        self.commands.action(Action::FocusColumn { index: 1 })?;
        self.commands.action(Action::FocusWindow { id: window_id })?;
        self.wait_for(
            DEFAULT_TIMEOUT,
            format!(
                "workspace {:?} first window {:?} to become focused",
                workspace.name, first_window.app_id
            ),
            |state| state.windows.get(&window_id).is_some_and(|window| window.is_focused),
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

        let first_id = self.ensure_window_present(workspace, first)?;
        self.ensure_primary_window_position(workspace, first, first_id, column_index)?;

        for (row_index, window) in column.windows.iter().enumerate().skip(1) {
            let target_row = row_index + 1;
            let window_id = self.ensure_window_present(workspace, window)?;
            self.ensure_stacked_window_position(
                workspace,
                window,
                window_id,
                column_index,
                target_row,
            )?;
        }

        self.ensure_workspace_active(&workspace.name)?;
        self.commands.action(Action::FocusColumn { index: column_index })?;
        self.commands.action(Action::SetColumnWidth {
            change: column.width.to_size_change(),
        })?;

        for window in &column.windows {
            let window_id = self
                .state
                .window_id_by_app_id_on_workspace(&window.app_id, &workspace.name)
                .ok_or_else(|| NiriAutostartError::MissingWindow(window.app_id.clone()))?;

            self.apply_window_floating(window_id, window.floating, &window.app_id)?;
            self.apply_window_height(window_id, &window.app_id, window.height)?;
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

    fn ensure_window_present(
        &mut self,
        workspace: &WorkspaceSpec,
        spec: &WindowSpec,
    ) -> Result<u64> {
        if self
            .state
            .preferred_window_id_by_app_id(&spec.app_id, Some(&workspace.name))
            .is_none()
            && self.state.first_window_id_by_app_id(&spec.app_id).is_none()
        {
            self.commands.action(Action::Spawn {
                command: spec.command.clone(),
            })?;
            self.wait_for(
                DEFAULT_TIMEOUT,
                format!("window {:?} to appear", spec.app_id),
                |state| predicate::window_exists_by_app_id(state, &spec.app_id),
            )?;
        }

        let window_id = self
            .state
            .preferred_window_id_by_app_id(&spec.app_id, Some(&workspace.name))
            .or_else(|| self.state.first_window_id_by_app_id(&spec.app_id))
            .ok_or_else(|| NiriAutostartError::MissingWindow(spec.app_id.clone()))?;

        self.ensure_workspace_active(&workspace.name)?;

        if !predicate::window_on_workspace(&self.state, &spec.app_id, &workspace.name) {
            self.commands.action(Action::MoveWindowToWorkspace {
                window_id: Some(window_id),
                reference: WorkspaceReferenceArg::Name(workspace.name.clone()),
                focus: false,
            })?;
            self.wait_for(
                DEFAULT_TIMEOUT,
                format!("window {:?} to move to workspace {:?}", spec.app_id, workspace.name),
                |state| predicate::window_on_workspace(state, &spec.app_id, &workspace.name),
            )?;
        }

        Ok(self
            .state
            .window_id_by_app_id_on_workspace(&spec.app_id, &workspace.name)
            .or_else(|| {
                self.state
                    .preferred_window_id_by_app_id(&spec.app_id, Some(&workspace.name))
            })
            .ok_or_else(|| NiriAutostartError::MissingWindow(spec.app_id.clone()))?)
    }

    fn ensure_primary_window_position(
        &mut self,
        workspace: &WorkspaceSpec,
        spec: &WindowSpec,
        window_id: u64,
        target_column: usize,
    ) -> Result<()> {
        self.apply_window_floating(window_id, spec.floating, &spec.app_id)?;
        self.ensure_window_row(window_id, &spec.app_id, 1)?;

        if predicate::window_at_position(
            &self.state,
            &spec.app_id,
            &workspace.name,
            target_column,
            1,
        ) {
            return Ok(());
        }

        self.ensure_workspace_active(&workspace.name)?;
        self.commands.action(Action::FocusWindow { id: window_id })?;
        self.wait_for(
            DEFAULT_TIMEOUT,
            format!("window {:?} to become focused", spec.app_id),
            |state| state.windows.get(&window_id).is_some_and(|window| window.is_focused),
        )?;
        self.commands.action(Action::MoveColumnToIndex { index: target_column })?;
        self.wait_for(
            DEFAULT_TIMEOUT,
            format!(
                "window {:?} to reach column {} row 1",
                spec.app_id, target_column
            ),
            |state| state.windows.get(&window_id).is_some_and(|window| {
                window.workspace_id == state.workspace_id_by_name(&workspace.name)
                    && window.layout.pos_in_scrolling_layout == Some((target_column, 1))
            }),
        )
    }

    fn ensure_stacked_window_position(
        &mut self,
        workspace: &WorkspaceSpec,
        spec: &WindowSpec,
        window_id: u64,
        target_column: usize,
        target_row: usize,
    ) -> Result<()> {
        self.apply_window_floating(window_id, spec.floating, &spec.app_id)?;

        if predicate::window_at_position(
            &self.state,
            &spec.app_id,
            &workspace.name,
            target_column,
            target_row,
        ) {
            return Ok(());
        }

        let (current_column, _) = self
            .state
            .window_position_by_id(window_id)
            .ok_or_else(|| NiriAutostartError::MissingWindow(spec.app_id.clone()))?;

        if current_column == target_column {
            return self.ensure_window_row(window_id, &spec.app_id, target_row);
        }

        self.ensure_workspace_active(&workspace.name)?;
        self.commands.action(Action::FocusWindow { id: window_id })?;
        self.wait_for(
            DEFAULT_TIMEOUT,
            format!("window {:?} to become focused", spec.app_id),
            |state| state.windows.get(&window_id).is_some_and(|window| window.is_focused),
        )?;

        let desired_column = target_column + 1;
        if current_column != desired_column {
            self.commands
                .action(Action::MoveColumnToIndex { index: desired_column })?;
            self.wait_for(
                DEFAULT_TIMEOUT,
                format!(
                    "window {:?} to move to helper column {}",
                    spec.app_id, desired_column
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
            .ok_or_else(|| NiriAutostartError::MissingWindow(spec.app_id.clone()))?;
        if helper_column != desired_column {
            return Err(NiriAutostartError::NonAdjacentColumn {
                app_id: spec.app_id.clone(),
                actual: helper_column,
                expected_left: target_column,
            });
        }

        self.commands.action(Action::FocusColumn { index: target_column })?;
        self.commands.action(Action::ConsumeWindowIntoColumn {})?;
        self.wait_for(
            DEFAULT_TIMEOUT,
            format!(
                "window {:?} to reach column {} row {}",
                spec.app_id, target_column, target_row
            ),
            |state| {
                state.windows.get(&window_id).is_some_and(|window| {
                    window.workspace_id == state.workspace_id_by_name(&workspace.name)
                        && window.layout.pos_in_scrolling_layout == Some((target_column, target_row))
                }) && predicate::column_has_window_count(state, &workspace.name, target_column, target_row)
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

            self.commands.action(Action::FocusWindow { id: window_id })?;
            self.wait_for(
                DEFAULT_TIMEOUT,
                format!("window {:?} to become focused", app_id),
                |state| state.windows.get(&window_id).is_some_and(|window| window.is_focused),
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

    fn apply_window_floating(&mut self, window_id: u64, floating: bool, app_id: &str) -> Result<()> {
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
            |state| state.windows.get(&window_id).is_some_and(|window| window.is_floating == floating),
        )
    }

    fn apply_window_height(&mut self, window_id: u64, app_id: &str, height: SizeSpec) -> Result<()> {
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

    fn wait_for<F>(&mut self, timeout: Duration, what: String, predicate: F) -> Result<()>
    where
        F: Fn(&ActualState) -> bool,
    {
        if predicate(&self.state) {
            return Ok(());
        }

        let start = Instant::now();
        loop {
            let remaining = timeout
                .checked_sub(start.elapsed())
                .ok_or_else(|| NiriAutostartError::Timeout {
                    what: what.clone(),
                    timeout,
                })?;

            match self.events.recv_timeout(remaining) {
                Ok(EventMessage::Event(event)) => {
                    apply_event(&mut self.state, event);
                    if predicate(&self.state) {
                        return Ok(());
                    }
                }
                Ok(EventMessage::Closed(message)) => {
                    return Err(NiriAutostartError::EventStreamClosed(message));
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(NiriAutostartError::Timeout { what, timeout });
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(NiriAutostartError::EventStreamClosed(
                        "event thread disconnected".to_string(),
                    ));
                }
            }
        }
    }
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
}
