//! Remote drive grouping: list a device's segments over copyparty, then group
//! them with the shared [`group_segments`](super::group_segments). The offline
//! mirror scan (`local`, M3) reuses the same grouping core.

use crate::copyparty_client::CopypartyClient;
use crate::error::Result;
use crate::model::Drive;

/// List every segment under `realdata_rel` and group them into drives. This is
/// pure read + group; persisting the result (`Repo::replace_drives`) is composed
/// separately by the M4/M8 orchestrator.
pub async fn group_remote(client: &CopypartyClient, realdata_rel: &str) -> Result<Vec<Drive>> {
    Ok(super::group_segments(
        client.list_segments(realdata_rel).await?,
    ))
}
