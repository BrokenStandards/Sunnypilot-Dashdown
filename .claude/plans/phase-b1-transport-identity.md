# Phase B1 — Transport & identity: multi-IP auto-connect, HTTPS, device-pinned identity

> Phase B1 of [e2e-and-sync-roadmap.md](e2e-and-sync-roadmap.md). The original "Phase B" splits into **B1 (this:
> transport & identity)** and **B2 (background sync scheduler)**, built/merged in that order — B2
> sits on B1. On implementation this file is renamed `phase-b1-transport-identity.md` and the
> roadmap is updated to reflect the split. Branch `phase-b1-transport-identity` → PR → CI → merge.

## Context

The user wants the app to **connect and sync over whichever IP is up — the comma's own
hotspot or a shared/home Wi-Fi — without manually switching modes**, over **HTTPS**, verifying the
endpoint is **the same comma device** ("ping is up and it appears to be the same device hosting
copyparty"). Today the core picks a single IP from `Device::active_mode` and hardcodes `http://`
([model/mod.rs:362-371](rust/core/src/model/mod.rs)); there is no TLS or identity check.

Research (sunnypilot + copyparty source) established:
- sunnypilot serves copyparty as `…-p8080 -z -q` → **HTTP and HTTPS both on port 8080**
  (auto-detected), with a **self-signed cert**. So HTTPS needs no device change.
- The cert is unique per device only if the comma has `cfssl` (auto-generated, SANs = IPs), and
  copyparty **regenerates the leaf on network change** — so the leaf fingerprint is *not* stable
  across IP changes, and the bundled fallback cert is *shared* across installs. Cert-fingerprint
  pinning alone is therefore not a reliable device identity.
- **The stable identity is the copyparty server hostname** (e.g. `comma-e0e384a`), the device's
  persistent system hostname. copyparty renders it into the **HTML listing** as
  `<span id="srv_info"><span>{name}</span> // <span>… free of …</span></span>` (and the page
  `<title>`) — built from `args.name_html` in [httpcli.py:6979-7087]. It is **not** in `?ls=j` JSON,
  so the client reads it from one HTML GET. (DongleId/serial live in `/data/params`,`/persist` —
  not mounted — so the hostname is the available marker.)

**Identity model (decided):** the **hostname is the primary, stable identity anchor**; the
self-signed **cert is TOFU-pinned for transport security but tolerated to rotate when the hostname
still matches** (legitimate cfssl regeneration). Different hostname ⇒ not the same device ⇒ reject.

Outcome: a transport layer that auto-finds the device on any of its IPs over HTTPS and proves it's
the same comma — the foundation B2 (background scheduler), Phase C (live refresh), and downloads all
use.

## Design

### 1. Multi-IP resolution (replaces single `active_ip()` routing)
- `Device::candidate_ips()` → ordered `[last_good?, hotspot_ip, wifi_ip?]` (deduped). Keep the
  `active_mode`/`hotspot_ip`/`wifi_ip` fields as-is (no `Device` record change → no FFI/call-site
  churn); `active_mode` stops driving routing (becomes a vestigial hint).
- A `resolve` step (in `sync_engine`, used by `client_for` + `check_connectivity`): probe candidates,
  pick the first that is **TLS-reachable AND identity-matches**, persist it as `last_good_ip`. The
  red/green dot stays cheap (TCP probe across candidates); identity is enforced when a client is built.

### 2. HTTPS + cert handling — `copyparty_client`
- `base_url()` → `https://{ip}:8080/` (port stays the device's configured port).
- Build `reqwest` with a **custom rustls `ServerCertVerifier`** (via `use_preconfigured_tls`) that
  accepts the self-signed chain (no CA/hostname validation) and **captures the leaf SHA-256** into a
  shared cell. Reuses the existing `tls::ensure_crypto_provider()` ([tls.rs](rust/core/src/tls.rs)).

### 3. Identity — new `core/src/identity` module + internal DB table
- Persist per device in a **new `device_identity` table** (`device_id, hostname, cert_sha256,
  last_good_ip`) — NOT on the `Device` record (keeps the FFI surface + all `Device(...)` call sites
  stable). Additive migration; bump schema version.
- `fetch_identity(client)`: one HTML GET of the listing → extract the `srv_info` hostname (text in
  `id="srv_info"` before ` // `; fallback to `<title>` prefix). Returns `(hostname, cert_sha256)`.
- Decision on connect:
  - **No pin yet (TOFU):** store hostname + cert.
  - **hostname matches:** OK; if `cert_sha256` changed, **re-pin** (legit rotation).
  - **hostname differs:** reject → surfaced as not-the-device (connectivity Red / sync error).

### 4. FFI + Android edit UI
- Drop the manual mode toggle (`device_form_mode_toggle`) from the device-edit form
  ([DeviceEditScreen](android/app/src/main/java/org/sunnypilot/dashdown/ui/edit/DeviceEditScreen.kt)); both IPs are just optional fields. Show the auto-resolved active IP +
  verified device name as read-only status. Deprecate `set_active_mode` ([ffi/mod.rs:158](rust/core/src/ffi/mod.rs)).
- Regenerate UniFFI bindings (in-workspace `bindgen` crate) if the FFI surface changes (it shouldn't,
  beyond removing `set_active_mode`).

### 5. Mock support (extend Phase A's `mock-copyparty`)
- Optional **TLS mode**: generate a self-signed cert at runtime (`rcgen`) and serve via rustls;
  expose the cert fingerprint. A control/CLI knob to **set the device name** and **rotate the cert**
  (to test rotation tolerance).
- Serve a minimal **HTML listing** carrying `<span id="srv_info"><span>{name}</span> // …</span>`
  so the identity fetch works against the mock (today it only serves `?ls=j` + file GET).

## Files
- Rust core: [model/mod.rs](rust/core/src/model/mod.rs) (`candidate_ips`, https `base_url`), [copyparty_client/mod.rs](rust/core/src/copyparty_client/mod.rs) (TLS verifier +
  fingerprint capture + HTML identity fetch), new `core/src/identity/`, [connectivity/mod.rs](rust/core/src/connectivity/mod.rs)
  (multi-IP probe), [sync_engine/mod.rs](rust/core/src/sync_engine/mod.rs) (`resolve`, `client_for`, `check_connectivity`), [db/mod.rs](rust/core/src/db/mod.rs)
  (+`device_identity` table + migration), [ffi/mod.rs](rust/core/src/ffi/mod.rs) (drop `set_active_mode`).
- Mock: `rust/mock-copyparty` (TLS serve via `rcgen`+rustls; HTML `srv_info`; name/rotate knobs) — new deps.
- Android: [DeviceEditScreen.kt](android/app/src/main/java/org/sunnypilot/dashdown/ui/edit/DeviceEditScreen.kt) + its VM (remove mode toggle; show resolved IP + device name).

## Tests (build complete, test complete)
- **Rust unit:** cert verifier captures + pins the leaf SHA-256; `srv_info`/title hostname extraction
  from sample HTML; identity decision table (no-pin→store; name match + cert change→re-pin; name
  mismatch→reject).
- **Rust integration (TLS mock):** HTTPS connect to the self-signed mock captures identity; reconnect
  after **cert rotation, same name → accepted**; **different name → rejected**; **multi-IP** —
  device with two IPs, first unreachable → resolver picks the second; persisted `last_good_ip` is
  tried first next time.
- **Android:** device-edit screen no longer shows the manual mode toggle; both IP fields render.

## Verification
- `cargo test -p dashdown-core -p mock-copyparty` green; clippy/fmt clean; bindings regen + app builds.
- Manual (real hardware): point a device at the comma's IP(s); confirm **HTTPS** connect on :8080,
  hostname `comma-…` captured as identity, and that listing/connectivity work via the auto-resolved
  IP without a manual mode switch. (No device config changed.)

## Commit
Branch `phase-b1-transport-identity`; milestone commit once green; PR; merge after CI. Rename this
file to `phase-b1-transport-identity.md` and update the roadmap to show the B1/B2 split.
