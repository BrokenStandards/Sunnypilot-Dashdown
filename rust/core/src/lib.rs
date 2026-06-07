//! Sunnypilot Dashdown — Rust core.
//!
//! M0 scaffolding: this crate currently exposes only **smoke/health** exports
//! (`ping`, `version`, `ping_async`) used to validate the UniFFI build + binding
//! generation on every target platform. They are replaced by the real `AppCore`
//! surface in M8 — nothing here is product functionality.

pub mod connectivity;
pub mod copyparty_client;
pub mod db;
pub mod drive_grouping;
pub mod error;
pub mod ffi;
pub mod logging;
pub mod model;
pub mod settings;
pub mod storage;
pub mod sync_engine;
pub mod tls;
pub mod video;

uniffi::setup_scaffolding!();

/// Sync smoke export — proves the basic FFI path.
#[uniffi::export]
pub fn ping() -> String {
    "pong".to_string()
}

/// Build/version smoke export.
#[uniffi::export]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Async smoke export — proves the async + tokio FFI path generates on all targets.
#[uniffi::export(async_runtime = "tokio")]
pub async fn ping_async() -> String {
    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    "pong".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_works() {
        assert_eq!(ping(), "pong");
    }

    #[test]
    fn version_is_package_version() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn ping_async_works() {
        assert_eq!(ping_async().await, "pong");
    }
}
