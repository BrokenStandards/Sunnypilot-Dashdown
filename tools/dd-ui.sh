#!/usr/bin/env bash
# Run a Dashdown Maestro flow against the connected device, with args, so common
# device-setup tasks are one command instead of many UI taps.
#
# Usage:
#   tools/dd-ui.sh add_device NAME=escape2020 IP=192.168.1.100 [PORT=8080] [WIFI_IP=…]
#   tools/dd-ui.sh remove_device NAME=escape2020
#   tools/dd-ui.sh clear_devices
#
# Tip: after it runs, dump the screen once (mobile-mcp list-elements) to see the
# end state and decide the next action — instead of many list calls mid-flow.
set -euo pipefail

DIR="$(cd "$(dirname "$0")/.." && pwd)"
FLOW="$DIR/android/maestro/${1:?usage: dd-ui.sh <flow> [KEY=VAL ...]}.yaml"
shift || true
[ -f "$FLOW" ] || { echo "no such flow: $FLOW" >&2; ls "$DIR/android/maestro/" >&2; exit 1; }

SERIAL="${ANDROID_SERIAL:-$(adb devices | awk 'NR>1 && $2=="device"{print $1; exit}')}"
[ -n "$SERIAL" ] || { echo "no adb device connected" >&2; exit 1; }

ENVARGS=()
have_port=0
have_wifi=0
for kv in "$@"; do
  ENVARGS+=( -e "$kv" )
  case "$kv" in PORT=*) have_port=1 ;; WIFI_IP=*) have_wifi=1 ;; esac
done
# Defaults for optional add_device args (Maestro can't default a `${VAR}` and a
# flow `env:` block would shadow these -e values). Harmless for other flows.
[ "$have_port" = 1 ] || ENVARGS+=( -e "PORT=8080" )
[ "$have_wifi" = 1 ] || ENVARGS+=( -e "WIFI_IP=" )

# Maestro needs a JDK 17+; default to the project's JDK if JAVA_HOME isn't a 17+.
export JAVA_HOME="${JAVA_HOME:-/usr/lib/jvm/java-17-openjdk}"
export PATH="$JAVA_HOME/bin:$HOME/.maestro/bin:$PATH"
exec maestro --device "$SERIAL" test "${ENVARGS[@]}" "$FLOW"
