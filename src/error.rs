use std::io;
use std::path::PathBuf;
use std::time::Duration;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, NiriAutostartError>;

#[derive(Debug, Error)]
pub enum NiriAutostartError {
    #[error(
        "cannot determine the default config path: XDG_CONFIG_HOME is unset or not absolute, and HOME does not provide an absolute fallback"
    )]
    MissingDefaultConfigBase,

    #[error("refusing to use insecure runtime directory {path}: expected an owned 0700 directory")]
    InsecureRuntimeDirectory { path: PathBuf },

    #[error("failed to read config from {path}: {source}")]
    ConfigRead { path: PathBuf, source: io::Error },

    #[error("failed to parse config from {path}: {message}")]
    ConfigParse { path: PathBuf, message: String },

    #[error("failed to read runtime state from {path}: {source}")]
    RuntimeStateRead { path: PathBuf, source: io::Error },

    #[error("failed to write runtime state to {path}: {source}")]
    RuntimeStateWrite { path: PathBuf, source: io::Error },

    #[error("failed to parse runtime state from {path}: {message}")]
    RuntimeStateParse { path: PathBuf, message: String },

    #[error("failed to serialize runtime state: {0}")]
    RuntimeStateSerialize(String),

    #[error("config validation failed: {0}")]
    Validation(String),

    #[error("ipc communication error: {0}")]
    Ipc(#[from] io::Error),

    #[error("niri returned an error: {0}")]
    Niri(String),

    #[error("unexpected reply while handling {context}")]
    UnexpectedReply { context: &'static str },

    #[error("timed out after {timeout:?} while waiting for {what}")]
    Timeout { what: String, timeout: Duration },

    #[error("event stream closed: {0}")]
    EventStreamClosed(String),

    #[error("workspace {0:?} was not found in niri state")]
    MissingWorkspace(String),

    #[error("window with app-id {0:?} was not found in niri state")]
    MissingWindow(String),

    #[error(
        "managed window {app_id:?} is in column {actual}, cannot consume it into column {expected_left} because it is not immediately to the right"
    )]
    NonAdjacentColumn {
        app_id: String,
        actual: usize,
        expected_left: usize,
    },
}
