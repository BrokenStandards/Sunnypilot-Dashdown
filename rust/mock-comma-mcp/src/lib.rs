//! `mock-comma-mcp` — an MCP server wrapping the in-repo `mock-copyparty`
//! fixture so the agent can drive hermetic, deterministic UI tests: provision
//! mock "devices", inject fixture states, and toggle reachability for the
//! green/blue/red connectivity dot. See [`server::MockComma`].

pub mod server;

pub use server::MockComma;
