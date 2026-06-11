// Toggle the mock-copyparty DATA server up/down via its control port.
// Runs HOST-side inside Maestro's GraalJS engine (not on the device), so it
// reaches the control port over plain host loopback — no adb reverse needed.
//
// Env (passed per-runScript, never via a top-level flow `env:` block — that
// would shadow the harness `-e` values):
//   CONTROL  control-port number (e.g. 8098)
//   UP       "true" | "false"
var url = 'http://127.0.0.1:' + CONTROL + '/reachable'
var r = http.post(url, {
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({ up: UP === 'true' }),
})
output.ok = r.ok
