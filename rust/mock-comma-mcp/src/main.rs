//! `mock-comma-mcp` stdio server entrypoint. Registered in `.mcp.json`; Claude
//! Code launches it as a stdio subprocess during UI test runs.

use rmcp::transport::stdio;
use rmcp::ServiceExt;

use mock_comma_mcp::MockComma;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // NOTE: stdout carries the JSON-RPC stream — never println! to it. Any
    // diagnostics must go to stderr.
    let service = MockComma::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
