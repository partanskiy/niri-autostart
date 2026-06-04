use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use niri_ipc::Window;
use serde::{Deserialize, Serialize};

use crate::config::WindowSpec;
use crate::error::{NiriAutostartError, Result};
use crate::state::ActualState;

const STATE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowKey {
    pub workspace: String,
    pub column: usize,
    pub row: usize,
}

impl WindowKey {
    pub fn new(workspace: &str, column: usize, row: usize) -> Self {
        Self {
            workspace: workspace.to_string(),
            column,
            row,
        }
    }
}

impl std::fmt::Display for WindowKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}:{}", self.workspace, self.column, self.row)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeState {
    version: u32,
    #[serde(default)]
    windows: Vec<StoredWindow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct StoredWindow {
    workspace: String,
    column: usize,
    row: usize,
    window_id: u64,
    app_id: String,
    pid: Option<i32>,
    command: Vec<String>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            windows: Vec::new(),
        }
    }
}

impl RuntimeState {
    pub fn default_path() -> PathBuf {
        env::temp_dir().join("niri-autostart").join("windows.json")
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => {
                return Err(NiriAutostartError::RuntimeStateRead {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };

        let mut state: Self =
            serde_json::from_str(&text).map_err(|err| NiriAutostartError::RuntimeStateParse {
                path: path.to_path_buf(),
                message: err.to_string(),
            })?;
        state.version = STATE_VERSION;
        Ok(state)
    }

    pub fn persist(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| NiriAutostartError::RuntimeStateWrite {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let text = serde_json::to_string_pretty(self)
            .map_err(|err| NiriAutostartError::RuntimeStateSerialize(err.to_string()))?;
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, text).map_err(|source| NiriAutostartError::RuntimeStateWrite {
            path: tmp.clone(),
            source,
        })?;
        fs::rename(&tmp, path).map_err(|source| NiriAutostartError::RuntimeStateWrite {
            path: path.to_path_buf(),
            source,
        })?;

        Ok(())
    }

    pub fn window_id(
        &self,
        key: &WindowKey,
        spec: &WindowSpec,
        state: &ActualState,
    ) -> Option<u64> {
        let stored = self.windows.iter().find(|stored| stored.matches_key(key))?;

        if stored.app_id != spec.app_id || stored.command != spec.command {
            return None;
        }

        let window = state.windows.get(&stored.window_id)?;
        if window.app_id.as_deref() != Some(spec.app_id.as_str()) || window.pid != stored.pid {
            return None;
        }

        Some(stored.window_id)
    }

    pub fn record(&mut self, key: &WindowKey, spec: &WindowSpec, window: &Window) {
        let stored = StoredWindow {
            workspace: key.workspace.clone(),
            column: key.column,
            row: key.row,
            window_id: window.id,
            app_id: spec.app_id.clone(),
            pid: window.pid,
            command: spec.command.clone(),
        };

        if let Some(existing) = self
            .windows
            .iter_mut()
            .find(|existing| existing.matches_key(key))
        {
            *existing = stored;
        } else {
            self.windows.push(stored);
        }

        self.windows
            .sort_by_key(|stored| (stored.workspace.clone(), stored.column, stored.row));
    }
}

impl StoredWindow {
    fn matches_key(&self, key: &WindowKey) -> bool {
        self.workspace == key.workspace && self.column == key.column && self.row == key.row
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use niri_ipc::{Timestamp, WindowLayout};

    fn spec(command: &[&str]) -> WindowSpec {
        WindowSpec {
            app_id: "kitty".into(),
            command: command.iter().map(|arg| (*arg).to_string()).collect(),
            height: crate::config::SizeSpec::Proportion(1.0),
            floating: false,
        }
    }

    fn window(id: u64, app_id: &str, pid: Option<i32>) -> Window {
        Window {
            id,
            title: None,
            app_id: Some(app_id.to_string()),
            pid,
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
    fn resolves_recorded_live_window() {
        let key = WindowKey::new("code", 1, 1);
        let spec = spec(&["terminal", "-e", "nvim"]);
        let window = window(42, "kitty", Some(1000));

        let mut runtime = RuntimeState::default();
        runtime.record(&key, &spec, &window);

        let mut state = ActualState::default();
        state.replace_windows(vec![window]);

        assert_eq!(runtime.window_id(&key, &spec, &state), Some(42));
    }

    #[test]
    fn ignores_stale_or_shifted_entries() {
        let key = WindowKey::new("code", 1, 1);
        let window_spec = spec(&["terminal", "-e", "nvim"]);
        let changed_spec = spec(&["terminal", "-e", "fish"]);
        let recorded_window = window(42, "kitty", Some(1000));

        let mut runtime = RuntimeState::default();
        runtime.record(&key, &window_spec, &recorded_window);

        let mut state = ActualState::default();
        state.replace_windows(vec![window(42, "kitty", Some(1001))]);

        assert_eq!(runtime.window_id(&key, &window_spec, &state), None);
        assert_eq!(runtime.window_id(&key, &changed_spec, &state), None);
    }
}
