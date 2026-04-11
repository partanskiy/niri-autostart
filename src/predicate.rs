use crate::state::ActualState;

pub fn workspace_known(state: &ActualState, workspace_name: &str) -> bool {
    state.workspace_by_name(workspace_name).is_some()
}

pub fn workspace_active(state: &ActualState, workspace_name: &str) -> bool {
    state
        .workspace_by_name(workspace_name)
        .is_some_and(|workspace| workspace.is_active)
}

pub fn window_exists_by_app_id(state: &ActualState, app_id: &str) -> bool {
    state.window_by_app_id(app_id).is_some()
}

pub fn window_on_workspace(state: &ActualState, app_id: &str, workspace_name: &str) -> bool {
    state
        .window_id_by_app_id_on_workspace(app_id, workspace_name)
        .is_some()
}

pub fn window_at_position(
    state: &ActualState,
    app_id: &str,
    workspace_name: &str,
    column: usize,
    row: usize,
) -> bool {
    let workspace_id = match state.workspace_id_by_name(workspace_name) {
        Some(id) => id,
        None => return false,
    };

    state
        .window_id_by_app_id_on_workspace(app_id, workspace_name)
        .and_then(|id| state.windows.get(&id))
        .is_some_and(|window| {
            window.workspace_id == Some(workspace_id)
                && window.layout.pos_in_scrolling_layout == Some((column, row))
        })
}

pub fn column_has_window_count(
    state: &ActualState,
    workspace_name: &str,
    column: usize,
    count: usize,
) -> bool {
    state
        .workspace_column_counts(workspace_name)
        .and_then(|counts| counts.get(column.saturating_sub(1)).copied())
        == Some(count)
}

#[cfg(test)]
pub fn workspace_column_counts(
    state: &ActualState,
    workspace_name: &str,
    expected: &[usize],
) -> bool {
    state
        .workspace_column_counts(workspace_name)
        .is_some_and(|counts| counts == expected)
}

#[cfg(test)]
pub fn window_tile_size(
    state: &ActualState,
    app_id: &str,
    expected_width: f64,
    expected_height: f64,
) -> bool {
    const SIZE_TOLERANCE: f64 = 1.0;

    state.window_by_app_id(app_id).is_some_and(|window| {
        (window.layout.tile_size.0 - expected_width).abs() <= SIZE_TOLERANCE
            && (window.layout.tile_size.1 - expected_height).abs() <= SIZE_TOLERANCE
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ActualState;
    use niri_ipc::{Timestamp, Window, WindowLayout, Workspace};

    fn state() -> ActualState {
        let mut state = ActualState::default();
        state.replace_workspaces(vec![Workspace {
            id: 1,
            idx: 1,
            name: Some("firework".into()),
            output: Some("HDMI-A-1".into()),
            is_urgent: false,
            is_active: true,
            is_focused: true,
            active_window_id: Some(1),
        }]);
        state.replace_windows(vec![Window {
            id: 1,
            title: Some("fastfetch".into()),
            app_id: Some("fw-fastfetch".into()),
            pid: Some(1),
            workspace_id: Some(1),
            is_focused: true,
            is_floating: false,
            is_urgent: false,
            layout: WindowLayout {
                pos_in_scrolling_layout: Some((1, 1)),
                tile_size: (633.0, 284.0),
                window_size: (633, 284),
                tile_pos_in_workspace_view: None,
                window_offset_in_tile: (0.0, 0.0),
            },
            focus_timestamp: Some(Timestamp { secs: 0, nanos: 0 }),
        }]);
        state
    }

    #[test]
    fn matches_exact_app_id() {
        let state = state();
        assert!(window_exists_by_app_id(&state, "fw-fastfetch"));
        assert!(!window_exists_by_app_id(&state, "kitty"));
    }

    #[test]
    fn matches_position() {
        let state = state();
        assert!(window_at_position(&state, "fw-fastfetch", "firework", 1, 1));
        assert!(!window_at_position(&state, "fw-fastfetch", "firework", 2, 1));
    }

    #[test]
    fn matches_tile_size() {
        let state = state();
        assert!(window_tile_size(&state, "fw-fastfetch", 633.0, 284.0));
        assert!(!window_tile_size(&state, "fw-fastfetch", 600.0, 284.0));
    }

    #[test]
    fn matches_column_counts() {
        let state = state();
        assert!(workspace_column_counts(&state, "firework", &[1]));
        assert!(column_has_window_count(&state, "firework", 1, 1));
    }
}
