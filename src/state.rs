use std::collections::{BTreeSet, HashMap};

use niri_ipc::{Window, Workspace};

#[derive(Debug, Default, Clone)]
pub struct ActualState {
    pub workspaces: HashMap<u64, Workspace>,
    pub windows: HashMap<u64, Window>,
    pub last_config_loaded_failed: Option<bool>,
    pub workspace_name_to_id: HashMap<String, u64>,
    pub windows_by_app_id: HashMap<String, BTreeSet<u64>>,
    pub positions: HashMap<(u64, usize, usize), u64>,
}

impl ActualState {
    pub fn replace_workspaces(&mut self, workspaces: Vec<Workspace>) {
        self.workspaces = workspaces.into_iter().map(|ws| (ws.id, ws)).collect();
        self.rebuild_indices();
    }

    pub fn replace_windows(&mut self, windows: Vec<Window>) {
        self.windows = windows.into_iter().map(|win| (win.id, win)).collect();
        self.rebuild_indices();
    }

    pub fn rebuild_indices(&mut self) {
        self.workspace_name_to_id.clear();
        self.windows_by_app_id.clear();
        self.positions.clear();

        for workspace in self.workspaces.values() {
            if let Some(name) = &workspace.name {
                self.workspace_name_to_id.insert(name.clone(), workspace.id);
            }
        }

        for window in self.windows.values() {
            if let Some(app_id) = &window.app_id {
                self.windows_by_app_id
                    .entry(app_id.clone())
                    .or_default()
                    .insert(window.id);
            }

            if let (Some(workspace_id), Some((column, row))) =
                (window.workspace_id, window.layout.pos_in_scrolling_layout)
            {
                self.positions.insert((workspace_id, column, row), window.id);
            }
        }
    }

    pub fn workspace_id_by_name(&self, name: &str) -> Option<u64> {
        self.workspace_name_to_id.get(name).copied()
    }

    pub fn workspace_by_name(&self, name: &str) -> Option<&Workspace> {
        self.workspace_id_by_name(name)
            .and_then(|id| self.workspaces.get(&id))
    }

    pub fn preferred_window_id_by_app_id(
        &self,
        app_id: &str,
        preferred_workspace: Option<&str>,
    ) -> Option<u64> {
        let preferred_workspace_id = preferred_workspace.and_then(|name| self.workspace_id_by_name(name));

        self.windows_by_app_id.get(app_id).and_then(|ids| {
            ids.iter()
                .filter_map(|id| self.windows.get(id).map(|window| (*id, window)))
                .max_by_key(|(id, window)| {
                    (
                        preferred_workspace_id.is_some_and(|workspace_id| {
                            window.workspace_id == Some(workspace_id)
                        }),
                        !window.is_floating,
                        window.is_focused,
                        *id,
                    )
                })
                .map(|(id, _)| id)
        })
    }

    pub fn first_window_id_by_app_id(&self, app_id: &str) -> Option<u64> {
        self.preferred_window_id_by_app_id(app_id, None)
    }

    pub fn window_id_by_app_id_on_workspace(&self, app_id: &str, workspace_name: &str) -> Option<u64> {
        let workspace_id = self.workspace_id_by_name(workspace_name)?;
        self.preferred_window_id_by_app_id(app_id, Some(workspace_name))
            .filter(|id| self.windows.get(id).is_some_and(|window| window.workspace_id == Some(workspace_id)))
    }

    pub fn window_by_app_id(&self, app_id: &str) -> Option<&Window> {
        self.first_window_id_by_app_id(app_id)
            .and_then(|id| self.windows.get(&id))
    }

    pub fn window_position_by_id(&self, window_id: u64) -> Option<(usize, usize)> {
        self.windows
            .get(&window_id)
            .and_then(|window| window.layout.pos_in_scrolling_layout)
    }

    pub fn workspace_column_counts(&self, workspace_name: &str) -> Option<Vec<usize>> {
        let workspace_id = self.workspace_id_by_name(workspace_name)?;
        let mut by_column: HashMap<usize, usize> = HashMap::new();

        for window in self.windows.values() {
            if window.workspace_id != Some(workspace_id) || window.is_floating {
                continue;
            }

            if let Some((column, _)) = window.layout.pos_in_scrolling_layout {
                *by_column.entry(column).or_insert(0) += 1;
            }
        }

        let mut columns = by_column.into_iter().collect::<Vec<_>>();
        columns.sort_by_key(|(column, _)| *column);
        Some(columns.into_iter().map(|(_, count)| count).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use niri_ipc::{Timestamp, WindowLayout};

    fn window(id: u64, app_id: &str, workspace_id: u64, column: usize, row: usize) -> Window {
        Window {
            id,
            title: Some(app_id.to_string()),
            app_id: Some(app_id.to_string()),
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
    fn computes_workspace_column_counts() {
        let mut state = ActualState::default();
        state.replace_workspaces(vec![Workspace {
            id: 10,
            idx: 1,
            name: Some("firework".into()),
            output: Some("HDMI-A-1".into()),
            is_urgent: false,
            is_active: true,
            is_focused: true,
            active_window_id: None,
        }]);
        state.replace_windows(vec![
            window(1, "a", 10, 1, 1),
            window(2, "b", 10, 1, 2),
            window(3, "c", 10, 2, 1),
        ]);

        assert_eq!(state.workspace_column_counts("firework"), Some(vec![2, 1]));
    }

    #[test]
    fn prefers_tiled_window_on_requested_workspace() {
        let mut state = ActualState::default();
        state.replace_workspaces(vec![
            Workspace {
                id: 10,
                idx: 1,
                name: Some("internet".into()),
                output: Some("eDP-1".into()),
                is_urgent: false,
                is_active: true,
                is_focused: true,
                active_window_id: None,
            },
            Workspace {
                id: 11,
                idx: 2,
                name: Some("other".into()),
                output: Some("eDP-1".into()),
                is_urgent: false,
                is_active: false,
                is_focused: false,
                active_window_id: None,
            },
        ]);

        let mut floating_on_internet = window(1, "org.telegram.desktop", 10, 1, 1);
        floating_on_internet.is_floating = true;
        let tiled_elsewhere = window(2, "org.telegram.desktop", 11, 1, 1);
        let tiled_on_internet = window(3, "org.telegram.desktop", 10, 2, 1);

        state.replace_windows(vec![floating_on_internet, tiled_elsewhere, tiled_on_internet]);

        assert_eq!(
            state.window_id_by_app_id_on_workspace("org.telegram.desktop", "internet"),
            Some(3)
        );
    }
}
