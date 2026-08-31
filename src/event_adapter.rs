use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use niri_ipc::{Event, Window, Workspace};

use crate::error::{NiriAutostartError, Result};
use crate::ipc::{EventMessage, EventStream};
use crate::reducer::apply_event;
use crate::state::ActualState;

const HISTORY_LIMIT: usize = 512;

#[derive(Debug, Default)]
struct EventAdapterState {
    state: ActualState,
    seq: u64,
    saw_workspaces: bool,
    saw_windows: bool,
    closed: Option<String>,
    history: VecDeque<(u64, Event)>,
}

pub struct EventAdapter {
    inner: Arc<(Mutex<EventAdapterState>, Condvar)>,
    _applier: thread::JoinHandle<()>,
}

impl EventAdapter {
    pub fn connect(initial_timeout: Duration) -> Result<Self> {
        let stream = EventStream::connect()?;
        let rx = stream.into_receiver();
        let inner = Arc::new((Mutex::new(EventAdapterState::default()), Condvar::new()));
        let applier_inner = Arc::clone(&inner);

        let applier = thread::spawn(move || {
            while let Ok(message) = rx.recv() {
                let (lock, cvar) = &*applier_inner;
                let mut guard = lock.lock().expect("event adapter lock poisoned");

                let should_notify = match message {
                    EventMessage::Event(event) => {
                        let saw_initial_state = match &event {
                            Event::WorkspacesChanged { .. } => !guard.saw_workspaces,
                            Event::WindowsChanged { .. } => !guard.saw_windows,
                            _ => false,
                        };
                        let affects_reconciliation =
                            event_affects_reconciliation(&guard.state, &event);
                        if affects_reconciliation {
                            guard.seq += 1;
                        }
                        let seq = guard.seq;

                        if matches!(event, Event::WorkspacesChanged { .. }) {
                            guard.saw_workspaces = true;
                        }
                        if matches!(event, Event::WindowsChanged { .. }) {
                            guard.saw_windows = true;
                        }

                        apply_event(&mut guard.state, event.clone());
                        guard.history.push_back((seq, event));
                        while guard.history.len() > HISTORY_LIMIT {
                            guard.history.pop_front();
                        }
                        affects_reconciliation || saw_initial_state
                    }
                    EventMessage::Closed(message) => {
                        guard.closed = Some(message);
                        true
                    }
                };

                if should_notify {
                    cvar.notify_all();
                }
            }
        });

        let adapter = Self {
            inner,
            _applier: applier,
        };
        adapter.wait_for_initial_state(initial_timeout)?;
        Ok(adapter)
    }

    pub fn state(&self) -> ActualState {
        self.inner
            .0
            .lock()
            .expect("event adapter lock poisoned")
            .state
            .clone()
    }

    pub fn wait_for<F>(&self, timeout: Duration, what: String, predicate: F) -> Result<()>
    where
        F: Fn(&ActualState) -> bool,
    {
        self.wait_loop(timeout, what, |guard| predicate(&guard.state))
    }

    pub fn wait_for_stable<F>(
        &self,
        timeout: Duration,
        quiet_for: Duration,
        what: String,
        predicate: F,
    ) -> Result<()>
    where
        F: Fn(&ActualState) -> bool,
    {
        let start = Instant::now();
        let (lock, cvar) = &*self.inner;
        let mut guard = lock.lock().expect("event adapter lock poisoned");

        loop {
            if let Some(message) = &guard.closed {
                return Err(NiriAutostartError::EventStreamClosed(message.clone()));
            }

            let remaining = timeout.checked_sub(start.elapsed()).ok_or_else(|| {
                NiriAutostartError::Timeout {
                    what: what.clone(),
                    timeout,
                }
            })?;

            if predicate(&guard.state) {
                let wait_for = quiet_for.min(remaining);
                let stable_seq = guard.seq;
                let (next_guard, wait_result) = cvar
                    .wait_timeout(guard, wait_for)
                    .expect("event adapter lock poisoned");
                guard = next_guard;

                if wait_result.timed_out() && guard.seq == stable_seq && predicate(&guard.state) {
                    if wait_for == quiet_for {
                        return Ok(());
                    }

                    return Err(NiriAutostartError::Timeout {
                        what: what.clone(),
                        timeout,
                    });
                }

                continue;
            }

            let (next_guard, wait_result) = cvar
                .wait_timeout(guard, remaining)
                .expect("event adapter lock poisoned");
            guard = next_guard;

            if wait_result.timed_out() && !predicate(&guard.state) {
                return Err(NiriAutostartError::Timeout { what, timeout });
            }
        }
    }

    pub fn wait_until_quiet(
        &self,
        timeout: Duration,
        quiet_for: Duration,
        what: String,
    ) -> Result<()> {
        let start = Instant::now();
        let (lock, cvar) = &*self.inner;
        let mut guard = lock.lock().expect("event adapter lock poisoned");

        loop {
            if let Some(message) = &guard.closed {
                return Err(NiriAutostartError::EventStreamClosed(message.clone()));
            }

            let remaining = timeout.checked_sub(start.elapsed()).ok_or_else(|| {
                NiriAutostartError::Timeout {
                    what: what.clone(),
                    timeout,
                }
            })?;
            let wait_for = quiet_for.min(remaining);
            let stable_seq = guard.seq;
            let (next_guard, wait_result) = cvar
                .wait_timeout(guard, wait_for)
                .expect("event adapter lock poisoned");
            guard = next_guard;

            if wait_result.timed_out() && guard.seq == stable_seq {
                if wait_for == quiet_for {
                    return Ok(());
                }

                return Err(NiriAutostartError::Timeout { what, timeout });
            }
        }
    }

    fn wait_for_initial_state(&self, timeout: Duration) -> Result<()> {
        self.wait_loop(
            timeout,
            "initial niri event-stream state".to_string(),
            |guard| guard.saw_workspaces && guard.saw_windows,
        )
    }

    fn wait_loop<F>(&self, timeout: Duration, what: String, predicate: F) -> Result<()>
    where
        F: Fn(&EventAdapterState) -> bool,
    {
        let start = Instant::now();
        let (lock, cvar) = &*self.inner;
        let mut guard = lock.lock().expect("event adapter lock poisoned");

        loop {
            if predicate(&guard) {
                return Ok(());
            }

            if let Some(message) = &guard.closed {
                return Err(NiriAutostartError::EventStreamClosed(message.clone()));
            }

            let remaining = timeout.checked_sub(start.elapsed()).ok_or_else(|| {
                NiriAutostartError::Timeout {
                    what: what.clone(),
                    timeout,
                }
            })?;
            let (next_guard, wait_result) = cvar
                .wait_timeout(guard, remaining)
                .expect("event adapter lock poisoned");
            guard = next_guard;

            if wait_result.timed_out() && !predicate(&guard) {
                return Err(NiriAutostartError::Timeout { what, timeout });
            }
        }
    }
}

fn event_affects_reconciliation(state: &ActualState, event: &Event) -> bool {
    match event {
        Event::WorkspacesChanged { workspaces } => {
            state.workspaces.len() != workspaces.len()
                || workspaces.iter().any(|workspace| {
                    state
                        .workspaces
                        .get(&workspace.id)
                        .is_none_or(|current| !workspace_fields_equal(current, workspace))
                })
        }
        Event::WorkspaceActivated { .. } | Event::WorkspaceActiveWindowChanged { .. } => true,
        Event::WindowsChanged { windows } => {
            state.windows.len() != windows.len()
                || windows.iter().any(|window| {
                    state
                        .windows
                        .get(&window.id)
                        .is_none_or(|current| !window_fields_equal(current, window))
                })
        }
        Event::WindowOpenedOrChanged { window } => {
            let changes_other_focus = window.is_focused
                && state
                    .windows
                    .values()
                    .any(|current| current.id != window.id && current.is_focused);
            changes_other_focus
                || state
                    .windows
                    .get(&window.id)
                    .is_none_or(|current| !window_fields_equal(current, window))
        }
        Event::WindowClosed { id } => state.windows.contains_key(id),
        Event::WindowFocusChanged { .. } => true,
        Event::WindowLayoutsChanged { changes } => changes.iter().any(|(id, layout)| {
            state.windows.get(id).is_some_and(|window| {
                window.layout.pos_in_scrolling_layout != layout.pos_in_scrolling_layout
                    || window.layout.tile_size != layout.tile_size
            })
        }),
        _ => false,
    }
}

fn workspace_fields_equal(left: &Workspace, right: &Workspace) -> bool {
    left.id == right.id
        && left.idx == right.idx
        && left.name == right.name
        && left.output == right.output
        && left.is_active == right.is_active
        && left.is_focused == right.is_focused
        && left.active_window_id == right.active_window_id
}

fn window_fields_equal(left: &Window, right: &Window) -> bool {
    left.id == right.id
        && left.app_id == right.app_id
        && left.workspace_id == right.workspace_id
        && left.is_focused == right.is_focused
        && left.is_floating == right.is_floating
        && left.layout.pos_in_scrolling_layout == right.layout.pos_in_scrolling_layout
        && left.layout.tile_size == right.layout.tile_size
}

#[cfg(test)]
mod tests {
    use super::*;
    use niri_ipc::{Timestamp, WindowLayout};

    fn window() -> Window {
        Window {
            id: 1,
            title: Some("spinner 1".into()),
            app_id: Some("kitty".into()),
            pid: Some(1),
            workspace_id: Some(1),
            is_focused: true,
            is_floating: false,
            is_urgent: false,
            layout: WindowLayout {
                pos_in_scrolling_layout: Some((1, 1)),
                tile_size: (100.0, 100.0),
                window_size: (100, 100),
                tile_pos_in_workspace_view: None,
                window_offset_in_tile: (0.0, 0.0),
            },
            focus_timestamp: Some(Timestamp { secs: 0, nanos: 0 }),
        }
    }

    fn state() -> ActualState {
        let mut state = ActualState::default();
        state.replace_windows(vec![window()]);
        state
    }

    #[test]
    fn ignores_title_only_window_updates() {
        let state = state();
        let mut changed = window();
        changed.title = Some("spinner 2".into());

        assert!(!event_affects_reconciliation(
            &state,
            &Event::WindowOpenedOrChanged { window: changed }
        ));
    }

    #[test]
    fn observes_window_layout_updates() {
        let state = state();
        let mut layout = window().layout;
        layout.tile_size = (200.0, 100.0);

        assert!(event_affects_reconciliation(
            &state,
            &Event::WindowLayoutsChanged {
                changes: vec![(1, layout)]
            }
        ));
    }
}
