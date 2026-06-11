// Remove a route (drive) from the served fixture tree via the control port, so a
// flow can assert the row disappears on the app's own poll. Host-side.
//
// Env (per-runScript):
//   CONTROL  control-port number
//   ROUTE    route id to remove, e.g. "000009ee--maestroadd"
var url = 'http://127.0.0.1:' + CONTROL + '/remove_drive'
var r = http.post(url, {
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({ route: ROUTE }),
})
output.ok = r.ok
