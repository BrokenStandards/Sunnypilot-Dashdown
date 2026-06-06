//! Process-wide rustls crypto provider installation.
//!
//! We build rustls with the **ring** provider and *no* compiled-in default
//! (`reqwest`'s `rustls-no-provider` feature) so the whole dependency graph
//! cross-compiles to iOS/Android without aws-lc-rs (CMake + C). The cost is that
//! a `CryptoProvider` must be installed as the process default *before the first*
//! `reqwest::Client` is built, or the build call fails at runtime with
//! "no process-level CryptoProvider available".
//!
//! [`ensure_crypto_provider`] does that exactly once; it is idempotent and safe
//! to call from every client-construction path (and from `AppCore::new`).

use std::sync::OnceLock;

static TLS_INIT: OnceLock<()> = OnceLock::new();

/// Install the ring `CryptoProvider` as the process default, once.
///
/// Idempotent: subsequent calls are no-ops. If a provider is already installed
/// (e.g. the host application set one), `install_default` returns `Err` and we
/// deliberately ignore it — any working provider satisfies the requirement.
pub fn ensure_crypto_provider() {
    TLS_INIT.get_or_init(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}
