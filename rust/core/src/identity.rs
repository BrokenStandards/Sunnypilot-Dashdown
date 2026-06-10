//! Device identity: prove a reachable endpoint is the **same comma** across its
//! several IPs (hotspot / home Wi-Fi) and over time.
//!
//! The stable anchor is copyparty's **server hostname** (e.g. `comma-e0e384a`),
//! the device's persistent system hostname, which copyparty renders into the
//! HTML listing's `srv_info` element and the page `<title>`. The self-signed TLS
//! cert is captured for transport security but is *not* a stable id (it may be
//! the shared bundled cert, or be regenerated on network change), so a cert
//! change is **tolerated when the hostname still matches**. A different hostname
//! means a different device → reject.

/// A device's identity as stored (a pin) or just observed on a connect.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceIdentity {
    /// copyparty server hostname (the stable anchor); `None` if not readable.
    pub hostname: Option<String>,
    /// Hex SHA-256 of the leaf TLS cert; `None` over plain HTTP.
    pub cert_sha256: Option<String>,
}

/// Outcome of comparing a stored pin against a fresh observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityVerdict {
    /// Same device (or first contact). `repin` is `Some` when the stored pin
    /// should be updated (first pin, learned a hostname, or tolerated cert
    /// rotation); `None` when nothing changed.
    Ok { repin: Option<DeviceIdentity> },
    /// The hostname differs from the pin — not the same device.
    Mismatch { pinned: String, seen: String },
}

/// Decide whether `observed` is the pinned device. Lenient when a value is
/// unknown (can't compare ⇒ don't reject); strict only on a hostname conflict.
pub fn decide(stored: Option<&DeviceIdentity>, observed: &DeviceIdentity) -> IdentityVerdict {
    if let (Some(pinned), Some(seen)) = (
        stored.and_then(|s| s.hostname.as_deref()),
        observed.hostname.as_deref(),
    ) {
        if pinned != seen {
            return IdentityVerdict::Mismatch {
                pinned: pinned.to_string(),
                seen: seen.to_string(),
            };
        }
    }
    // Compatible: build the pin to persist, preferring fresh values but keeping
    // stored ones where the observation is blank.
    let merged = DeviceIdentity {
        hostname: observed
            .hostname
            .clone()
            .or_else(|| stored.and_then(|s| s.hostname.clone())),
        cert_sha256: observed
            .cert_sha256
            .clone()
            .or_else(|| stored.and_then(|s| s.cert_sha256.clone())),
    };
    let repin = if stored != Some(&merged) {
        Some(merged)
    } else {
        None
    };
    IdentityVerdict::Ok { repin }
}

/// Extract the copyparty server hostname from a listing HTML page: the first
/// `<span>` text inside the `srv_info` element (before the ` // ` free-space
/// span), falling back to the hostname prefix of `<title>`.
pub fn parse_hostname(html: &str) -> Option<String> {
    if let Some(i) = html.find("id=\"srv_info\"") {
        let rest = &html[i..];
        if let Some(s) = rest.find("<span>") {
            let after = &rest[s + "<span>".len()..];
            if let Some(e) = after.find("</span>") {
                let name = clean(&after[..e]);
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }
    }
    if let Some(i) = html.find("<title>") {
        let after = &html[i + "<title>".len()..];
        if let Some(e) = after.find("</title>") {
            // copyparty titles are "<hostname> - <path>".
            let head = after[..e].split(" - ").next().unwrap_or("");
            let name = clean(head);
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

/// Trim and minimally HTML-unescape an extracted token.
fn clean(s: &str) -> String {
    s.trim()
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(h: Option<&str>, c: Option<&str>) -> DeviceIdentity {
        DeviceIdentity {
            hostname: h.map(str::to_string),
            cert_sha256: c.map(str::to_string),
        }
    }

    #[test]
    fn parses_srv_info_hostname() {
        let html = r#"<html><head><title>comma-e0e384a - /routes/</title></head>
            <body><p><span id="srv_info"><span>comma-e0e384a</span> // <span>8.87 GiB free of 88.1 GiB</span></span></p></body></html>"#;
        assert_eq!(parse_hostname(html).as_deref(), Some("comma-e0e384a"));
    }

    #[test]
    fn falls_back_to_title() {
        let html = "<html><head><title>comma-abc123 - /</title></head><body>no srv_info here</body></html>";
        assert_eq!(parse_hostname(html).as_deref(), Some("comma-abc123"));
    }

    #[test]
    fn no_hostname_when_absent() {
        assert_eq!(parse_hostname("<html><body>nothing</body></html>"), None);
    }

    #[test]
    fn first_contact_pins() {
        let v = decide(None, &id(Some("comma-x"), Some("aa")));
        assert_eq!(
            v,
            IdentityVerdict::Ok {
                repin: Some(id(Some("comma-x"), Some("aa")))
            }
        );
    }

    #[test]
    fn same_name_changed_cert_repins() {
        let stored = id(Some("comma-x"), Some("old"));
        let v = decide(Some(&stored), &id(Some("comma-x"), Some("new")));
        assert_eq!(
            v,
            IdentityVerdict::Ok {
                repin: Some(id(Some("comma-x"), Some("new")))
            }
        );
    }

    #[test]
    fn same_identity_no_repin() {
        let stored = id(Some("comma-x"), Some("aa"));
        assert_eq!(
            decide(Some(&stored), &id(Some("comma-x"), Some("aa"))),
            IdentityVerdict::Ok { repin: None }
        );
    }

    #[test]
    fn different_hostname_rejected() {
        let stored = id(Some("comma-x"), Some("aa"));
        assert_eq!(
            decide(Some(&stored), &id(Some("comma-y"), Some("aa"))),
            IdentityVerdict::Mismatch {
                pinned: "comma-x".into(),
                seen: "comma-y".into()
            }
        );
    }

    #[test]
    fn unknown_observed_hostname_is_lenient() {
        let stored = id(Some("comma-x"), Some("aa"));
        // Couldn't read a hostname this time, cert rotated → tolerate, keep name.
        assert_eq!(
            decide(Some(&stored), &id(None, Some("bb"))),
            IdentityVerdict::Ok {
                repin: Some(id(Some("comma-x"), Some("bb")))
            }
        );
    }
}
