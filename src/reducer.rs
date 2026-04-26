use niri_ipc::Event;

use crate::state::ActualState;

pub fn apply_event(state: &mut ActualState, event: Event) {
    match event {
        Event::WorkspacesChanged { workspaces } => state.replace_workspaces(workspaces),
        Event::WorkspaceUrgencyChanged { id, urgent } => {
            if let Some(workspace) = state.workspaces.get_mut(&id) {
                workspace.is_urgent = urgent;
            }
            state.rebuild_indices();
        }
        Event::WorkspaceActivated { id, focused } => {
            let output = state.workspaces.get(&id).and_then(|workspace| workspace.output.clone());

            if let Some(output) = output {
                for workspace in state.workspaces.values_mut() {
                    if workspace.output.as_deref() == Some(output.as_str()) {
                        workspace.is_active = workspace.id == id;
                    }
                }
            }

            if focused {
                for workspace in state.workspaces.values_mut() {
                    workspace.is_focused = workspace.id == id;
                }
            }

            state.rebuild_indices();
        }
        Event::WorkspaceActiveWindowChanged {
            workspace_id,
            active_window_id,
        } => {
            if let Some(workspace) = state.workspaces.get_mut(&workspace_id) {
                workspace.active_window_id = active_window_id;
            }
            state.rebuild_indices();
        }
        Event::WindowsChanged { windows } => state.replace_windows(windows),
        Event::WindowOpenedOrChanged { window } => {
            if window.is_focused {
                for existing in state.windows.values_mut() {
                    existing.is_focused = false;
                }
            }
            state.windows.insert(window.id, window);
            state.rebuild_indices();
        }
        Event::WindowClosed { id } => {
            state.windows.remove(&id);
            state.rebuild_indices();
        }
        Event::WindowFocusChanged { id } => {
            for window in state.windows.values_mut() {
                window.is_focused = Some(window.id) == id;
            }
            state.rebuild_indices();
        }
        Event::WindowFocusTimestampChanged { id, focus_timestamp } => {
            if let Some(window) = state.windows.get_mut(&id) {
                window.focus_timestamp = focus_timestamp;
            }
            state.rebuild_indices();
        }
        Event::WindowUrgencyChanged { id, urgent } => {
            if let Some(window) = state.windows.get_mut(&id) {
                window.is_urgent = urgent;
            }
            state.rebuild_indices();
        }
        Event::WindowLayoutsChanged { changes } => {
            for (id, layout) in changes {
                if let Some(window) = state.windows.get_mut(&id) {
                    window.layout = layout;
                }
            }
            state.rebuild_indices();
        }
        Event::ConfigLoaded { failed } => {
            state.last_config_loaded_failed = Some(failed);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ActualState;
    use niri_ipc::{Timestamp, Window, WindowLayout, Workspace};

    fn workspace() -> Workspace {
        Workspace {
            id: 1,
            idx: 1,
            name: Some("firework".into()),
            output: Some("HDMI-A-1".into()),
            is_urgent: false,
            is_active: true,
            is_focused: true,
            active_window_id: None,
        }
    }

    fn window(id: u64, app_id: &str) -> Window {
        Window {
            id,
            title: Some(app_id.into()),
            app_id: Some(app_id.into()),
            pid: Some(1),
            workspace_id: Some(1),
            is_focused: false,
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

    #[test]
    fn windows_changed_replaces_full_map() {
        let mut state = ActualState::default();
        state.replace_windows(vec![window(1, "old")]);

        apply_event(
            &mut state,
            Event::WindowsChanged {
                windows: vec![window(2, "new")],
            },
        );

        assert!(state.windows.contains_key(&2));
        assert!(!state.windows.contains_key(&1));
    }

    #[test]
    fn window_opened_or_changed_updates_one_window() {
        let mut state = ActualState::default();
        state.replace_windows(vec![window(1, "a")]);

        apply_event(
            &mut state,
            Event::WindowOpenedOrChanged {
                window: window(2, "b"),
            },
        );

        assert!(state.windows.contains_key(&1));
        assert!(state.windows.contains_key(&2));
    }

    #[test]
    fn window_layouts_changed_updates_layout_only() {
        let mut state = ActualState::default();
        state.replace_windows(vec![window(1, "a")]);

        let mut layout = state.windows.get(&1).unwrap().layout.clone();
        layout.pos_in_scrolling_layout = Some((2, 3));

        apply_event(
            &mut state,
            Event::WindowLayoutsChanged {
                changes: vec![(1, layout.clone())],
            },
        );

        assert_eq!(state.windows.get(&1).unwrap().layout, layout);
    }

    #[test]
    fn workspaces_changed_rebuilds_name_index() {
        let mut state = ActualState::default();
        apply_event(
            &mut state,
            Event::WorkspacesChanged {
                workspaces: vec![workspace()],
            },
        );

        assert_eq!(state.workspace_id_by_name("firework"), Some(1));
    }
}
