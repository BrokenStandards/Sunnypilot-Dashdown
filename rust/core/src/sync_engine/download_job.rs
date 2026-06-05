//! Per-file transfer with size-verify, bounded retry, and external cancellation,
//! plus the progress callback surface the native layer implements.

use tokio_util::sync::CancellationToken;

use crate::copyparty_client::CopypartyClient;
use crate::error::{CoreError, Result};
use crate::storage::MirrorStore;

/// Default download attempts per file (1 try + 1 retry on a transient failure).
pub const MAX_ATTEMPTS: u32 = 2;

/// Outcome of a single file download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOutcome {
    Complete,
    Canceled,
}

/// Outcome of a whole drive download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobOutcome {
    Complete,
    Canceled,
    Failed(String),
}

/// Implemented by the native layer (M8) to receive download progress. Plain
/// Rust trait for now; becomes a UniFFI callback interface at the FFI boundary.
pub trait ProgressSink: Send + Sync {
    fn on_progress(&self, p: DownloadProgress);
    fn on_completed(&self, drive_key: &str);
    fn on_failed(&self, drive_key: &str, error: &str);
}

/// A progress snapshot for one drive download (per-file granularity in M4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadProgress {
    pub drive_key: String,
    pub files_done: u32,
    pub files_total: u32,
    pub bytes_done: u64,
    pub bytes_total: u64,
    /// The file just completed (or about to start).
    pub current_file: Option<String>,
}

/// Download one file into the mirror with size verification, bounded retry, and
/// cancellation.
///
/// - `Ok(Complete)` — bytes matched `expected_size` and the file was committed.
/// - `Ok(Canceled)` — the token fired; the in-flight `.part` is left for M5.
/// - `Err(..)` — exhausted attempts on a size mismatch, or a non-retriable
///   transport error (401/403/404). A leftover `.part` may remain (benign).
///
/// Cancellation works by racing the download future against the token in
/// `tokio::select!`; on cancel the future (which owns the `PartFile`) is dropped,
/// closing the in-flight stream. No change to `download_to` is needed.
pub async fn download_file(
    client: &CopypartyClient,
    mirror: &MirrorStore,
    rel: &str,
    expected_size: u64,
    cancel: &CancellationToken,
    max_attempts: u32,
) -> Result<FileOutcome> {
    for attempt in 1..=max_attempts {
        if cancel.is_cancelled() {
            return Ok(FileOutcome::Canceled);
        }
        let attempt_result = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Ok(FileOutcome::Canceled),
            r = async {
                // `create_part` truncates any stale `.part`, so each attempt
                // restarts from byte 0 (file-granular; byte-range resume is M5).
                let mut pf = mirror.create_part(rel).await?;
                let written = client.download_to(rel, pf.writer()).await?;
                Ok::<_, CoreError>((pf, written))
            } => r,
        };

        match attempt_result {
            Ok((pf, written)) if written == expected_size => {
                pf.commit().await?;
                return Ok(FileOutcome::Complete);
            }
            Ok((_pf, written)) => {
                tracing::warn!(
                    rel,
                    attempt,
                    written,
                    expected_size,
                    "size mismatch; re-fetching"
                );
                // `_pf` drops here, leaving the `.part`; the next attempt truncates it.
            }
            Err(e) if is_retriable(&e) && attempt < max_attempts => {
                tracing::warn!(rel, attempt, error = %e, "download attempt failed; retrying");
            }
            Err(e) => return Err(e),
        }
    }
    Err(CoreError::Http(format!(
        "size mismatch after {max_attempts} attempts: {rel}"
    )))
}

/// Transport/IO errors are worth retrying; auth/permission/not-found are not.
fn is_retriable(e: &CoreError) -> bool {
    matches!(e, CoreError::Http(_) | CoreError::Io(_))
}
