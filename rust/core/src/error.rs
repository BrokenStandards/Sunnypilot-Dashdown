//! Core error type. M8 maps this onto a UniFFI error; for now it is internal.

use thiserror::Error;

/// Errors produced by the core engine. Variants carry `String` payloads (not
/// foreign error types) so they stay FFI-friendly when exposed in M8.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("http error: {0}")]
    Http(String),

    /// 401 — server requires a password and none/anonymous was rejected.
    #[error("authentication required")]
    AuthRequired,

    /// 403 — authenticated but lacking permission.
    #[error("forbidden")]
    Forbidden,

    #[error("not found: {0}")]
    NotFound(String),

    #[error("parse error: {0}")]
    Parse(String),

    #[error("io error: {0}")]
    Io(String),

    #[error("database error: {0}")]
    Db(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;

impl From<reqwest::Error> for CoreError {
    fn from(e: reqwest::Error) -> Self {
        CoreError::Http(e.to_string())
    }
}

impl From<serde_json::Error> for CoreError {
    fn from(e: serde_json::Error) -> Self {
        CoreError::Parse(e.to_string())
    }
}

impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        CoreError::Io(e.to_string())
    }
}

impl From<rusqlite::Error> for CoreError {
    fn from(e: rusqlite::Error) -> Self {
        CoreError::Db(e.to_string())
    }
}

impl From<r2d2::Error> for CoreError {
    fn from(e: r2d2::Error) -> Self {
        CoreError::Db(e.to_string())
    }
}
