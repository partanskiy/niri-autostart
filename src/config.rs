use std::collections::HashSet;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser;

use crate::error::NiriAutostartError;

type AppResult<T> = crate::error::Result<T>;

#[derive(Debug, Parser)]
#[command(author, version, about)]
pub struct Cli {
    #[arg(long)]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub workspaces: Vec<WorkspaceSpec>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceSpec {
    pub name: String,
    pub columns: Vec<ColumnSpec>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColumnSpec {
    pub width: SizeSpec,
    pub windows: Vec<WindowSpec>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowSpec {
    pub app_id: String,
    pub command: Vec<String>,
    pub height: SizeSpec,
    pub floating: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SizeSpec {
    Fixed(i32),
    Proportion(f64),
}

#[derive(Debug, Clone, PartialEq, knuffel::Decode)]
struct RawAutostartConfig {
    #[knuffel(children(name = "workspace"))]
    workspaces: Vec<RawWorkspaceSpec>,
}

#[derive(Debug, Clone, PartialEq, knuffel::Decode)]
struct RawWorkspaceSpec {
    #[knuffel(argument)]
    name: String,
    #[knuffel(children(name = "column"))]
    columns: Vec<RawColumnSpec>,
}

#[derive(Debug, Clone, PartialEq, knuffel::Decode)]
struct RawColumnSpec {
    #[knuffel(child)]
    width: RawSizeSpec,
    #[knuffel(children(name = "window"))]
    windows: Vec<RawWindowSpec>,
}

#[derive(Debug, Clone, PartialEq, knuffel::Decode)]
struct RawWindowSpec {
    #[knuffel(property(name = "app-id"))]
    app_id: String,
    #[knuffel(child, unwrap(arguments))]
    command: Option<Vec<String>>,
    #[knuffel(child)]
    height: RawSizeSpec,
    #[knuffel(property(name = "floating"), default = false)]
    floating: bool,
}

#[derive(Debug, Clone, PartialEq, knuffel::Decode)]
struct RawSizeSpec {
    #[knuffel(child, unwrap(argument))]
    fixed: Option<i32>,
    #[knuffel(child, unwrap(argument))]
    proportion: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, knuffel::Decode)]
struct ConfigDocument {
    #[knuffel(children(name = "autostart"))]
    autostart: Vec<RawAutostartConfig>,
}

impl Config {
    pub fn load(path: &Path) -> AppResult<Self> {
        let text = fs::read_to_string(path).map_err(|source| NiriAutostartError::ConfigRead {
            path: path.to_path_buf(),
            source,
        })?;

        Self::parse(path, &text)
    }

    pub fn parse(path: &Path, text: &str) -> AppResult<Self> {
        let filename = path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("config.kdl");

        let parsed = knuffel::parse::<ConfigDocument>(filename, text).map_err(|err| {
            NiriAutostartError::ConfigParse {
                path: path.to_path_buf(),
                message: format!("{err:?}"),
            }
        })?;

        match parsed.autostart.as_slice() {
            [autostart] => Self::from_raw(autostart.clone()),
            [] => Err(NiriAutostartError::Validation(
                "expected a single top-level `autostart` node".to_string(),
            )),
            _ => Err(NiriAutostartError::Validation(
                "duplicate top-level `autostart` nodes are not allowed".to_string(),
            )),
        }
    }

    pub fn default_path() -> AppResult<PathBuf> {
        if let Some(path) = env::var_os("XDG_CONFIG_HOME") {
            return Ok(PathBuf::from(path).join("niri-autostart").join("config.kdl"));
        }

        if let Some(home) = env::var_os("HOME") {
            return Ok(PathBuf::from(home)
                .join(".config")
                .join("niri-autostart")
                .join("config.kdl"));
        }

        Err(NiriAutostartError::MissingDefaultConfigBase)
    }

    fn from_raw(raw: RawAutostartConfig) -> AppResult<Self> {
        let workspaces = raw
            .workspaces
            .into_iter()
            .map(WorkspaceSpec::from_raw)
            .collect::<AppResult<Vec<_>>>()?;

        let config = Self { workspaces };
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> AppResult<()> {
        let mut app_ids = HashSet::new();

        if self.workspaces.is_empty() {
            return Err(NiriAutostartError::Validation(
                "at least one `workspace` node is required".to_string(),
            ));
        }

        for workspace in &self.workspaces {
            if workspace.columns.is_empty() {
                return Err(NiriAutostartError::Validation(format!(
                    "workspace {:?} must contain at least one column",
                    workspace.name
                )));
            }

            for column in &workspace.columns {
                if column.windows.is_empty() {
                    return Err(NiriAutostartError::Validation(format!(
                        "workspace {:?} contains a column without windows",
                        workspace.name
                    )));
                }

                for window in &column.windows {
                    if !app_ids.insert(window.app_id.clone()) {
                        return Err(NiriAutostartError::Validation(format!(
                            "duplicate app-id {:?} in config",
                            window.app_id
                        )));
                    }

                    if window.command.is_empty() {
                        return Err(NiriAutostartError::Validation(format!(
                            "window {:?} must have a non-empty command",
                            window.app_id
                        )));
                    }
                }
            }
        }

        Ok(())
    }
}

impl WorkspaceSpec {
    fn from_raw(raw: RawWorkspaceSpec) -> AppResult<Self> {
        Ok(Self {
            name: raw.name,
            columns: raw
                .columns
                .into_iter()
                .map(ColumnSpec::from_raw)
                .collect::<AppResult<Vec<_>>>()?,
        })
    }
}

impl ColumnSpec {
    fn from_raw(raw: RawColumnSpec) -> AppResult<Self> {
        Ok(Self {
            width: raw.width.into_size_spec("width")?,
            windows: raw
                .windows
                .into_iter()
                .map(WindowSpec::from_raw)
                .collect::<AppResult<Vec<_>>>()?,
        })
    }
}

impl WindowSpec {
    fn from_raw(raw: RawWindowSpec) -> AppResult<Self> {
        Ok(Self {
            app_id: raw.app_id,
            command: raw.command.unwrap_or_default(),
            height: raw.height.into_size_spec("height")?,
            floating: raw.floating,
        })
    }
}

impl RawSizeSpec {
    fn into_size_spec(self, field: &str) -> AppResult<SizeSpec> {
        match (self.fixed, self.proportion) {
            (Some(value), None) => Ok(SizeSpec::Fixed(value)),
            (None, Some(value)) => Ok(SizeSpec::Proportion(value)),
            (Some(_), Some(_)) => Err(NiriAutostartError::Validation(format!(
                "{field} must contain exactly one of `fixed` or `proportion`",
            ))),
            (None, None) => Err(NiriAutostartError::Validation(format!(
                "{field} must contain one of `fixed` or `proportion`",
            ))),
        }
    }
}

pub fn resolve_config_path(cli: &Cli) -> AppResult<PathBuf> {
    if let Some(path) = &cli.config {
        return Ok(path.clone());
    }

    Config::default_path()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> AppResult<Config> {
        Config::parse(Path::new("config.kdl"), text)
    }

    #[test]
    fn parses_minimal_valid_config() {
        let config = parse(
            r#"
            autostart {
                workspace "firework" {
                    column {
                        width {
                            fixed 640
                        }
                        window app-id="fw-fastfetch" {
                            command "terminal" "--class" "fw-fastfetch" "-e" "fastfetch"
                            height {
                                fixed 284
                            }
                        }
                    }
                }
            }
            "#,
        )
        .unwrap();

        assert_eq!(config.workspaces.len(), 1);
        assert_eq!(
            config.workspaces[0].columns[0].windows[0].app_id,
            "fw-fastfetch"
        );
    }

    #[test]
    fn rejects_duplicate_app_ids() {
        let err = parse(
            r#"
            autostart {
                workspace "firework" {
                    column {
                        width {
                            fixed 640
                        }
                        window app-id="dup" {
                            command "a"
                            height {
                                fixed 1
                            }
                        }
                    }
                    column {
                        width {
                            fixed 640
                        }
                        window app-id="dup" {
                            command "b"
                            height {
                                fixed 1
                            }
                        }
                    }
                }
            }
            "#,
        )
        .unwrap_err();

        assert!(err.to_string().contains("duplicate app-id"));
    }

    #[test]
    fn rejects_missing_command() {
        let err = parse(
            r#"
            autostart {
                workspace "firework" {
                    column {
                        width {
                            fixed 640
                        }
                        window app-id="fw-fastfetch" {
                            height {
                                fixed 284
                            }
                        }
                    }
                }
            }
            "#,
        )
        .unwrap_err();

        assert!(err.to_string().contains("non-empty command"));
    }

    #[test]
    fn rejects_invalid_width_or_height_nodes() {
        let err = parse(
            r#"
            autostart {
                workspace "firework" {
                    column {
                        width {
                            fixed 640
                            proportion 0.5
                        }
                        window app-id="fw-fastfetch" {
                            command "terminal"
                            height {
                                fixed 284
                            }
                        }
                    }
                }
            }
            "#,
        )
        .unwrap_err();

        assert!(err.to_string().contains("exactly one"));
    }

    #[test]
    fn rejects_unknown_nodes() {
        let err = parse(
            r#"
            autostart {
                workspace "firework" {
                    mystery {}
                }
            }
            "#,
        )
        .unwrap_err();

        assert!(err.to_string().contains("unexpected"));
    }
}
