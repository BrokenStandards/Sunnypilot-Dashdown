//! Async copyparty client: `?ls=j` listing, segment enumeration, streamed
//! download, `PW:` auth, and WebDAV `DELETE` (M6 auto-delete-from-comma).
//! Atomic mirror writes live in `storage` (M3).

pub mod auth;
pub mod download;
pub mod listing;

pub use auth::Credentials;
pub use listing::{DirListing, Entry};

use url::Url;

use crate::error::{CoreError, Result};
use crate::model::{FileKind, Segment, SegmentFile, SegmentName};

/// A client bound to one copyparty base URL + credentials.
pub struct CopypartyClient {
    base: Url,
    creds: Credentials,
    http: reqwest::Client,
}

impl CopypartyClient {
    pub fn new(base_url: &str, creds: Credentials) -> Result<Self> {
        // Devices are on the LAN (hotspot/wifi IPs); never route via a proxy.
        let http = reqwest::Client::builder().no_proxy().build()?;
        Self::with_client(base_url, creds, http)
    }

    /// Construct with a caller-provided `reqwest::Client` (e.g. for tests).
    pub fn with_client(base_url: &str, creds: Credentials, http: reqwest::Client) -> Result<Self> {
        Ok(Self {
            base: normalize_base(base_url)?,
            creds,
            http,
        })
    }

    fn url_for(&self, rel: &str) -> Result<Url> {
        self.base
            .join(rel)
            .map_err(|e| CoreError::Parse(format!("bad path {rel}: {e}")))
    }

    async fn list_url(&self, mut url: Url) -> Result<DirListing> {
        url.query_pairs_mut().append_pair("ls", "j");
        let resp = auth::apply_auth(self.http.get(url), &self.creds)
            .send()
            .await?;
        check_status(&resp)?;
        let text = resp.text().await?;
        listing::parse_listing(&text)
    }

    /// List a single directory (relative to the base URL).
    pub async fn list_dir(&self, rel: &str) -> Result<DirListing> {
        self.list_url(self.url_for(rel)?).await
    }

    /// Enumerate every segment under `realdata_rel`: list the directory, keep
    /// entries that parse as segment names, then list each segment for its
    /// files (size + mtime). Non-segment dirs are skipped. Results are sorted
    /// by (route_id, segment_num).
    pub async fn list_segments(&self, realdata_rel: &str) -> Result<Vec<Segment>> {
        let realdata_rel = ensure_trailing_slash(realdata_rel);
        let dir_url = self.url_for(&realdata_rel)?;
        let top = self.list_url(dir_url.clone()).await?;

        let mut segments = Vec::new();
        for d in &top.dirs {
            let Ok(name) = SegmentName::parse(&d.name) else {
                continue;
            };
            let seg_url = dir_url
                .join(&d.href)
                .map_err(|e| CoreError::Parse(format!("bad segment href {}: {e}", d.href)))?;
            let listing = self.list_url(seg_url).await?;

            let mut files = Vec::new();
            let mut recording = false;
            for f in &listing.files {
                let kind = FileKind::from_filename(&f.name);
                if kind == FileKind::LockMarker {
                    recording = true;
                    continue;
                }
                files.push(SegmentFile {
                    kind,
                    name: f.name.clone(),
                    remote_size: f.size,
                    mtime_s: f.mtime_s,
                });
            }
            files.sort_by(|a, b| a.name.cmp(&b.name));
            segments.push(Segment {
                name,
                files,
                recording,
            });
        }
        segments.sort_by(|a, b| {
            a.name
                .route_id
                .cmp(&b.name.route_id)
                .then(a.name.segment_num.cmp(&b.name.segment_num))
        });
        Ok(segments)
    }

    /// Stream a file (relative path) into `writer`; returns bytes written.
    pub async fn download_to<W: tokio::io::AsyncWrite + Unpin>(
        &self,
        rel: &str,
        writer: &mut W,
    ) -> Result<u64> {
        self.fetch(rel, None).await?.stream_to(writer).await
    }

    /// Convenience: download a file fully into memory.
    pub async fn download(&self, rel: &str) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        self.download_to(rel, &mut buf).await?;
        Ok(buf)
    }

    /// Issue a GET for `rel`, optionally resuming from byte `range_from`
    /// (`Range: bytes=N-`). Both `200` (full body) and `206` (partial) are
    /// success — inspect [`Fetch::partial`] to decide append-vs-restart.
    pub async fn fetch(&self, rel: &str, range_from: Option<u64>) -> Result<Fetch> {
        let mut req = auth::apply_auth(self.http.get(self.url_for(rel)?), &self.creds);
        if let Some(start) = range_from {
            req = req.header(reqwest::header::RANGE, format!("bytes={start}-"));
        }
        let resp = req.send().await?;
        check_status(&resp)?;
        Ok(Fetch { resp })
    }

    /// Delete a remote path (file or directory) via WebDAV `DELETE`. copyparty
    /// removes directories recursively and answers `200 OK`; a missing path
    /// (`404`) is treated as idempotent success (already gone, the desired end
    /// state). Requires the volume's `d` permission, else `403 Forbidden`.
    pub async fn delete(&self, rel: &str) -> Result<()> {
        let resp = auth::apply_auth(self.http.delete(self.url_for(rel)?), &self.creds)
            .send()
            .await?;
        match resp.status().as_u16() {
            200 | 204 | 404 => Ok(()),
            401 => Err(CoreError::AuthRequired),
            403 => Err(CoreError::Forbidden),
            code => Err(CoreError::Http(format!("delete status {code}"))),
        }
    }

    /// Re-verify that the server honors HTTP Range (a `bytes=0-0` probe returns
    /// `206`). The downloader is self-correcting (it falls back to a full fetch
    /// on `200`), so this is for verification/diagnostics + a future decision.
    pub async fn probe_range(&self, rel: &str) -> Result<bool> {
        let resp = auth::apply_auth(self.http.get(self.url_for(rel)?), &self.creds)
            .header(reqwest::header::RANGE, "bytes=0-0")
            .send()
            .await?;
        check_status(&resp)?;
        Ok(resp.status() == reqwest::StatusCode::PARTIAL_CONTENT)
    }
}

/// A GET response in flight, possibly a Range partial.
pub struct Fetch {
    resp: reqwest::Response,
}

impl Fetch {
    /// Whether the server answered `206 Partial Content` (honored the Range).
    pub fn partial(&self) -> bool {
        self.resp.status() == reqwest::StatusCode::PARTIAL_CONTENT
    }

    /// Stream the body into `writer`, returning bytes written.
    pub async fn stream_to<W: tokio::io::AsyncWrite + Unpin>(self, writer: &mut W) -> Result<u64> {
        download::stream_to_writer(self.resp, writer).await
    }
}

fn normalize_base(base_url: &str) -> Result<Url> {
    let mut u = Url::parse(base_url).map_err(|e| CoreError::Parse(format!("bad base url: {e}")))?;
    if !u.path().ends_with('/') {
        let p = format!("{}/", u.path());
        u.set_path(&p);
    }
    Ok(u)
}

fn ensure_trailing_slash(rel: &str) -> String {
    if rel.is_empty() || rel.ends_with('/') {
        rel.to_string()
    } else {
        format!("{rel}/")
    }
}

fn check_status(resp: &reqwest::Response) -> Result<()> {
    let s = resp.status();
    if s.is_success() {
        return Ok(());
    }
    Err(match s.as_u16() {
        401 => CoreError::AuthRequired,
        403 => CoreError::Forbidden,
        404 => CoreError::NotFound(resp.url().to_string()),
        code => CoreError::Http(format!("status {code}")),
    })
}
