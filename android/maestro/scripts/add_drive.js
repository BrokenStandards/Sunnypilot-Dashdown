// Add a route (drive) to the served fixture tree via the control port, so a
// flow can assert it appears in the drives list on the app's own poll — with no
// manual refresh. Host-side (see set_reachable.js).
//
// Env (per-runScript):
//   CONTROL  control-port number
//   ROUTE    route id, e.g. "000009ee--maestroadd"
//   SEGS     segment count (string); defaults to 1
var url = 'http://127.0.0.1:' + CONTROL + '/add_drive'
var r = http.post(url, {
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({ route: ROUTE, segs: parseInt(SEGS, 10) || 1 }),
})
output.ok = r.ok
