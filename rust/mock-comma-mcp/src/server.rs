//! The MCP tool surface + a registry of running mock devices.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::Mutex;

use mock_copyparty::{fixtures, MockServer, ServeOptions};

const HOST: &str = "127.0.0.1";

/// The hermetic server states the UI tests exercise.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Fixture {
    SingleDrive,
    GapSplit,
    Partial,
    SizeMismatch,
}

impl Fixture {
    fn build(self) -> mock_copyparty::Fixture {
        match self {
            Fixture::SingleDrive => fixtures::single_drive(),
            // An index gap models a >1-min recording gap within one route — truer
            // to the plan's "1-min-gap split" than the distinct-routes gap_split.
            Fixture::GapSplit => fixtures::gap_index(),
            Fixture::Partial => fixtures::partial(),
            Fixture::SizeMismatch => fixtures::size_mismatch(),
        }
    }
}

/// One provisioned device: a fixture tree served on a STABLE port. The
/// `MockServer` is dropped/recreated to toggle reachability; the `TempDir`
/// (and thus the tree) survives toggles.
struct RunningDevice {
    fixture: Fixture,
    password: Option<String>,
    port: u16,
    temp: TempDir,
    overrides: HashMap<String, u64>,
    server: Option<MockServer>, // None ⇒ unreachable (listening socket closed)
}

impl RunningDevice {
    fn addr(&self) -> SocketAddr {
        SocketAddr::new(HOST.parse().unwrap(), self.port)
    }
    fn base_url(&self) -> String {
        format!("http://{HOST}:{}/", self.port)
    }
    /// (Re)bind the server on the device's fixed port if it isn't already up.
    async fn bring_up(&mut self) -> std::io::Result<()> {
        if self.server.is_none() {
            self.server = Some(
                MockServer::spawn_with(
                    self.temp.path().to_path_buf(),
                    ServeOptions {
                        addr: Some(self.addr()),
                        password: self.password.clone(),
                        size_overrides: self.overrides.clone(),
                    },
                )
                .await?,
            );
        }
        Ok(())
    }
    /// Close the listening socket and wait for the port to be released, so a
    /// subsequent `bring_up` can re-bind it without racing (→ connect refused
    /// in the gap = the Red state).
    async fn bring_down(&mut self) {
        if let Some(srv) = self.server.take() {
            srv.shutdown().await;
        }
    }
    fn info(&self, device_id: &str) -> serde_json::Value {
        // host is 127.0.0.1; an Android emulator reaches it at 10.0.2.2:<port>.
        json!({
            "device_id": device_id,
            "fixture": self.fixture,
            "host": HOST,
            "port": self.port,
            "base_url": self.base_url(),
            "reachable": self.server.is_some(),
        })
    }
}

/// The MCP server: a shared registry of provisioned devices.
#[derive(Clone, Default)]
pub struct MockComma {
    devices: Arc<Mutex<HashMap<String, RunningDevice>>>,
}

impl MockComma {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ProvisionParams {
    /// Caller-chosen id; re-provisioning the same id replaces it.
    pub device_id: String,
    pub fixture: Fixture,
    /// Optional copyparty password the served device requires.
    pub password: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetStateParams {
    pub device_id: String,
    pub fixture: Fixture,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetReachableParams {
    pub device_id: String,
    pub reachable: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeviceQuery {
    /// Limit to one device; omit to act on all.
    pub device_id: Option<String>,
}

fn not_found(id: &str) -> ErrorData {
    ErrorData::invalid_params(format!("no such device: {id}"), None)
}
fn io_err(e: std::io::Error) -> ErrorData {
    ErrorData::internal_error(format!("mock server error: {e}"), None)
}
fn ok(value: serde_json::Value) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::structured(value))
}

#[tool_router]
impl MockComma {
    #[tool(
        description = "Provision a mock device serving a fixture tree (single_drive, gap_split, partial, size_mismatch). Re-provisioning an existing device_id replaces it. Returns base_url/host/port/reachable."
    )]
    async fn provision_device(
        &self,
        Parameters(p): Parameters<ProvisionParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let fx = p.fixture.build();
        let overrides = fx.size_overrides;
        let temp = fx.dir;
        // Bind ephemeral once to claim a free port, then own it explicitly so it
        // stays stable across reachability toggles.
        let first = MockServer::spawn_with(
            temp.path().to_path_buf(),
            ServeOptions {
                password: p.password.clone(),
                size_overrides: overrides.clone(),
                ..Default::default()
            },
        )
        .await
        .map_err(io_err)?;
        let port = first.addr().port();
        let dev = RunningDevice {
            fixture: p.fixture,
            password: p.password,
            port,
            temp,
            overrides,
            server: Some(first),
        };
        let info = dev.info(&p.device_id);
        self.devices.lock().await.insert(p.device_id, dev);
        ok(info)
    }

    #[tool(
        description = "Rebuild a provisioned device with a different fixture on the SAME port (the app's configured URL stays valid). Device becomes reachable."
    )]
    async fn set_state(
        &self,
        Parameters(p): Parameters<SetStateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut devices = self.devices.lock().await;
        let dev = devices
            .get_mut(&p.device_id)
            .ok_or_else(|| not_found(&p.device_id))?;
        let fx = p.fixture.build();
        dev.bring_down().await;
        dev.fixture = p.fixture;
        dev.overrides = fx.size_overrides;
        dev.temp = fx.dir;
        dev.bring_up().await.map_err(io_err)?;
        ok(dev.info(&p.device_id))
    }

    #[tool(
        description = "Toggle reachability. false closes the listening socket (TCP connect refused → Red dot); true re-binds the same port. Provisioned state is preserved."
    )]
    async fn set_reachable(
        &self,
        Parameters(p): Parameters<SetReachableParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut devices = self.devices.lock().await;
        let dev = devices
            .get_mut(&p.device_id)
            .ok_or_else(|| not_found(&p.device_id))?;
        if p.reachable {
            dev.bring_up().await.map_err(io_err)?;
        } else {
            dev.bring_down().await;
        }
        ok(dev.info(&p.device_id))
    }

    #[tool(
        description = "Snapshot one or all provisioned devices (id, base_url, port, reachable)."
    )]
    async fn status(
        &self,
        Parameters(q): Parameters<DeviceQuery>,
    ) -> Result<CallToolResult, ErrorData> {
        let devices = self.devices.lock().await;
        let list: Vec<_> = devices
            .iter()
            .filter(|(id, _)| match &q.device_id {
                Some(want) => want == id.as_str(),
                None => true,
            })
            .map(|(id, dev)| dev.info(id))
            .collect();
        ok(json!({ "devices": list }))
    }

    #[tool(
        description = "Tear down one device (device_id set) or all (omitted), freeing ports + temp dirs."
    )]
    async fn teardown(
        &self,
        Parameters(q): Parameters<DeviceQuery>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut devices = self.devices.lock().await;
        let torn: Vec<String> = match q.device_id {
            Some(id) => devices.remove(&id).map(|_| id).into_iter().collect(),
            None => {
                let ids: Vec<String> = devices.keys().cloned().collect();
                devices.clear();
                ids
            }
        };
        ok(json!({ "torn_down": torn }))
    }
}

#[tool_handler]
impl ServerHandler for MockComma {}
