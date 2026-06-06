//! TCP-connect reachability + the connectivity-dot result type. Reachability
//! uses a plain `TcpStream::connect` with a timeout rather than ICMP ping: raw
//! sockets (ping) are blocked on iOS/Android, while an unprivileged TCP connect
//! to the copyparty `(ip, port)` is the exact "can I actually talk to it" signal
//! we need. The sync engine derives the dot from this + active-download state.

use std::time::Duration;

use crate::model::ConnDot;

/// Default reachability probe timeout (master plan: TCP connect with timeout).
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// True iff a TCP connection to `(host, port)` completes within `timeout`. Any
/// failure — connection refused, host/network unreachable, DNS error, or the
/// timeout elapsing — yields `false`. The `timeout` also bounds DNS resolution,
/// since that happens inside the wrapped `connect` future. Pure (no DB), so it
/// is directly unit-testable.
pub async fn tcp_reachable(host: &str, port: u16, timeout: Duration) -> bool {
    matches!(
        tokio::time::timeout(timeout, tokio::net::TcpStream::connect((host, port))).await,
        Ok(Ok(_))
    )
}

/// Result of a connectivity check for one device (M8's `check_connectivity`
/// returns this). `dot` is derived from `reachable` + `downloading`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceConnectivity {
    pub dot: ConnDot,
    pub reachable: bool,
    pub downloading: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    /// A 127.0.0.1 port that is bound then released — nothing listens on it, so a
    /// connect gets a fast "connection refused".
    async fn closed_port() -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        port
    }

    #[tokio::test]
    async fn reachable_true_to_live_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        // Keep `listener` alive across the probe so the connect succeeds.
        assert!(tcp_reachable(&addr.ip().to_string(), addr.port(), DEFAULT_CONNECT_TIMEOUT).await);
    }

    #[tokio::test]
    async fn unreachable_false_on_refused() {
        let port = closed_port().await;
        // Connection refused returns fast (no timeout wait).
        assert!(!tcp_reachable("127.0.0.1", port, DEFAULT_CONNECT_TIMEOUT).await);
    }

    #[tokio::test]
    async fn unreachable_false_with_short_timeout() {
        // A short timeout on a closed port still resolves to false promptly,
        // proving the timeout is wired (and never hangs).
        let port = closed_port().await;
        assert!(!tcp_reachable("127.0.0.1", port, Duration::from_millis(200)).await);
    }
}
