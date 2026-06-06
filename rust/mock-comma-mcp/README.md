# mock-comma-mcp

An MCP server that wraps the in-repo [`mock-copyparty`](../mock-copyparty) fixture so the
agent can drive **hermetic, deterministic UI tests** (Phase B): provision mock Comma
"devices", inject server states, and toggle reachability for the green/blue/red connectivity
dot — no real device or internet.

Registered as a stdio server in the repo-root [`.mcp.json`](../../.mcp.json); Claude Code
launches it on demand (`cargo run -q -p mock-comma-mcp`). Built with the official
[`rmcp`](https://crates.io/crates/rmcp) SDK.

## Tools

| tool | input | result | behavior |
|------|-------|--------|----------|
| `provision_device` | `device_id`, `fixture`, `password?` | `{device_id, fixture, host, port, base_url, reachable}` | Build a fixture tree and serve it on a fresh, **stable** port. Re-provisioning an existing `device_id` replaces it. |
| `set_state` | `device_id`, `fixture` | device info | Rebuild the device with a different fixture **on the same port** (the app's configured URL stays valid). |
| `set_reachable` | `device_id`, `reachable` | device info | `false` closes the listening socket (TCP connect refused → **Red** dot); `true` re-binds the same port. Provisioned state is preserved. |
| `status` | `device_id?` | `{devices:[…]}` | Snapshot of one or all devices (for assertions). |
| `teardown` | `device_id?` | `{torn_down:[…]}` | Drop one or all devices, freeing ports + temp dirs. |

`fixture` ∈ `single_drive` | `gap_split` | `partial` | `size_mismatch`:
- **single_drive** — one route, 3 consecutive segments → one Complete drive.
- **gap_split** — a segment-index gap (0,1,3) → two drives.
- **partial** — a recording last segment with a partial file set.
- **size_mismatch** — `qcamera.ts` served honestly (600 B) but the listing advertises 1200 B,
  so the downloaded file mismatches its recorded size → the core classifies `SizeMismatch`
  (drive `Partial`/resumable).

## Consumption (B2/B3)

`host` is `127.0.0.1`; the **Android emulator** reaches it at `10.0.2.2:<port>` (iOS sim uses
`127.0.0.1:<port>`). A UI run: `provision_device` → point the app at the device → drive the UI
(Maestro / mobile-mcp) → toggle `set_reachable` to verify Red/Green and start a download for
Blue → `teardown`. Blue-vs-Green is derived in the core from reachability + active job, not by
this server.

## Test

`cargo test -p mock-comma-mcp` spawns the built binary and drives the real MCP stdio protocol
(`tests/it_mcp.rs`): lists the five tools, then exercises provision → reachable → unreachable →
reachable-same-port → set_state → teardown.
