//! Real-protocol smoke + behavior: spawn the `mock-comma-mcp` binary and drive
//! it as an MCP client over stdio — verifying the tool surface and the
//! reachability / state behavior end-to-end (not just unit-level handlers).

use std::time::Duration;

use rmcp::model::CallToolRequestParams;
use rmcp::transport::TokioChildProcess;
use rmcp::ServiceExt;
use serde_json::{json, Value};

fn port_of(sc: &Value) -> u16 {
    sc.get("port").and_then(Value::as_u64).expect("port") as u16
}

async fn tcp_ok(port: u16) -> bool {
    matches!(
        tokio::time::timeout(
            Duration::from_millis(500),
            tokio::net::TcpStream::connect(("127.0.0.1", port)),
        )
        .await,
        Ok(Ok(_))
    )
}

#[tokio::test]
async fn mcp_tools_and_reachability_behavior() {
    let cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_mock-comma-mcp"));
    let client =
        ().serve(TokioChildProcess::new(cmd).expect("spawn server"))
            .await
            .expect("mcp handshake");

    // 1. The five tools are advertised.
    let tools = client.list_all_tools().await.unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    for want in [
        "provision_device",
        "set_state",
        "set_reachable",
        "status",
        "teardown",
        "add_segment",
        "add_drive",
        "remove_drive",
    ] {
        assert!(names.contains(&want), "missing tool {want}; got {names:?}");
    }

    // CallToolRequestParams is #[non_exhaustive]; build it via Deserialize.
    let call = |name: &'static str, args: Value| {
        let client = &client;
        async move {
            let params: CallToolRequestParams =
                serde_json::from_value(json!({"name": name, "arguments": args})).unwrap();
            client
                .call_tool(params)
                .await
                .unwrap()
                .structured_content
                .expect("structured content")
        }
    };

    // 2. Provision → reachable on the returned (stable) port.
    let sc = call(
        "provision_device",
        json!({"device_id":"d1","fixture":"single_drive"}),
    )
    .await;
    let port = port_of(&sc);
    assert_eq!(sc["reachable"], json!(true));
    assert!(tcp_ok(port).await, "provisioned device is reachable");
    assert_eq!(
        sc["routes"][0]["segments"],
        json!(3),
        "single_drive starts with 3 segments"
    );

    // 2b. add_segment grows the active drive live (served on the same port).
    let sc = call("add_segment", json!({"device_id":"d1"})).await;
    assert_eq!(
        sc["routes"][0]["segments"],
        json!(4),
        "add_segment appended one segment to the primary route"
    );

    // 3. set_reachable(false) → listening socket closed → connect refused (Red).
    call("set_reachable", json!({"device_id":"d1","reachable":false})).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !tcp_ok(port).await,
        "unreachable after set_reachable(false)"
    );

    // 4. set_reachable(true) → reachable again on the SAME port.
    let sc = call("set_reachable", json!({"device_id":"d1","reachable":true})).await;
    assert_eq!(port_of(&sc), port);
    assert!(tcp_ok(port).await, "reachable again on the same port");

    // 5. set_state(size_mismatch) keeps the same port.
    let sc = call(
        "set_state",
        json!({"device_id":"d1","fixture":"size_mismatch"}),
    )
    .await;
    assert_eq!(port_of(&sc), port);

    // 6. status lists the device; teardown frees it.
    let sc = call("status", json!({})).await;
    assert_eq!(sc["devices"].as_array().unwrap().len(), 1);
    call("teardown", json!({"device_id":"d1"})).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!tcp_ok(port).await, "torn down → unreachable");

    let _ = client.cancel().await;
}
