//! Streamed download helper. M1 streams the whole body to a sink; M3 adds the
//! atomic `.part` → fsync → rename mirror write on top of this.

use tokio::io::{AsyncWrite, AsyncWriteExt};

use crate::error::Result;

/// Stream a response body chunk-by-chunk into `writer`, returning bytes written.
pub async fn stream_to_writer<W: AsyncWrite + Unpin>(
    mut resp: reqwest::Response,
    writer: &mut W,
) -> Result<u64> {
    let mut total = 0u64;
    while let Some(chunk) = resp.chunk().await? {
        writer.write_all(&chunk).await?;
        total += chunk.len() as u64;
    }
    writer.flush().await?;
    Ok(total)
}
