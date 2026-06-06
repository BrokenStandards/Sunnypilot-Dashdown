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

/// Implemented by the native layer to receive download progress. Exported as a
/// UniFFI foreign trait (M8) so Swift/Kotlin can implement it; foreign-trait
/// methods take owned `String` (not `&str`).
#[uniffi::export(with_foreign)]
pub trait ProgressSink: Send + Sync {
    fn on_progress(&self, p: DownloadProgress);
    fn on_completed(&self, drive_key: String);
    fn on_failed(&self, drive_key: String, error: String);
}

/// A progress snapshot for one drive download (per-file granularity in M4).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
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
/// **Byte-range resume:** the attempt derives a resume offset from any existing
/// `.part` and sends `Range: bytes=N-`. On `206` it appends the tail to the
/// `.part`; on `200` (server ignored Range) or a stale/oversized `.part` it
/// restarts from byte 0. Cancellation races the attempt against the token in a
/// `biased` `tokio::select!`; on cancel the future (which owns the `PartFile`) is
/// dropped, leaving the partial `.part` for the *next* resume.
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
                // Resume from the `.part` size, unless it's stale (>= expected,
                // e.g. the remote shrank) — then restart from 0.
                let existing = mirror.part_size(rel).unwrap_or(0);
                let resume_from = if existing > 0 && existing < expected_size { existing } else { 0 };

                let fetch = client.fetch(rel, (resume_from > 0).then_some(resume_from)).await?;
                let (mut pf, base) = if resume_from > 0 && fetch.partial() {
                    (mirror.open_part_append(rel).await?, resume_from) // 206: append the tail
                } else {
                    (mirror.create_part(rel).await?, 0) // fresh, or 200 (Range ignored): restart
                };
                let written = fetch.stream_to(pf.writer()).await?;
                Ok::<_, CoreError>((pf, base + written))
            } => r,
        };

        match attempt_result {
            Ok((pf, total)) if total == expected_size => {
                pf.commit().await?;
                return Ok(FileOutcome::Complete);
            }
            Ok((_pf, total)) => {
                tracing::warn!(
                    rel,
                    attempt,
                    total,
                    expected_size,
                    "size mismatch; re-fetching"
                );
                // `_pf` drops here, leaving the `.part`; the next attempt resumes/truncates it.
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
