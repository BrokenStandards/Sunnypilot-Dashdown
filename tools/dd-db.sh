#!/usr/bin/env bash
# Inspect the Dashdown app's on-device SQLite index without a pile of adb/sqlite
# calls. Pulls the live DB (+WAL) off the connected device and runs a preset or a
# custom query.
#
# Usage:
#   tools/dd-db.sh [devices|identity|drives|segments|schema|"<SQL>"]
#   ANDROID_SERIAL=192.168.1.210:5555 tools/dd-db.sh identity
#
# Presets:
#   devices   id, name, dongle_label, IPs, port
#   identity  pinned hostname + cert fp + last-good base (device_identity)
#   drives    per-device drive/segment counts
#   segments  total segment count
#   schema    table names
# Anything else is run verbatim as SQL.
set -euo pipefail

PKG=org.sunnypilot.dashdown
REMOTE="/sdcard/Android/data/$PKG/files/dashdown"
SERIAL="${ANDROID_SERIAL:-$(adb devices | awk 'NR>1 && $2=="device"{print $1; exit}')}"
[ -n "$SERIAL" ] || { echo "no adb device connected" >&2; exit 1; }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
for f in index.sqlite index.sqlite-wal index.sqlite-shm; do
  adb -s "$SERIAL" pull "$REMOTE/$f" "$TMP/" >/dev/null 2>&1 || true
done
[ -f "$TMP/index.sqlite" ] || { echo "no index.sqlite on $SERIAL (add a device first?)" >&2; exit 1; }

case "${1:-devices}" in
  devices)  Q="SELECT id,name,dongle_label,hotspot_ip,wifi_ip,port FROM device;";;
  identity) Q="SELECT device_id,hostname,substr(cert_sha256,1,16)||'…' fp,last_good_base FROM device_identity;";;
  drives)   Q="SELECT device_id,count(*) drives,COALESCE(sum(segment_count),0) segs FROM drive GROUP BY device_id;";;
  segments) Q="SELECT count(*) segments FROM segment;";;
  schema)   Q="SELECT name FROM sqlite_master WHERE type='table' ORDER BY name;";;
  *)        Q="$1";;
esac

python3 - "$TMP/index.sqlite" "$Q" <<'PY'
import sqlite3, sys
db, q = sys.argv[1], sys.argv[2]
c = sqlite3.connect(db)
try:
    cur = c.execute(q)
    rows = cur.fetchall()
    cols = [d[0] for d in cur.description] if cur.description else []
    if cols:
        print(" | ".join(cols))
        print("-" * 60)
    for r in rows:
        print(" | ".join("" if v is None else str(v) for v in r))
    if cur.description and not rows:
        print("(no rows)")
except Exception as e:
    print("query error:", e, file=sys.stderr); sys.exit(1)
PY
