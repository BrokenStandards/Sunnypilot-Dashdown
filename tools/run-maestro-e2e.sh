#!/usr/bin/env bash
# Run the Dashdown Maestro black-box UI suite against either the mock fixture or
# real hardware — one command. Complements the instrumented connected tests
# (tools/run-android-e2e.sh); this drives the actual user-facing UI.
#
#   tools/run-maestro-e2e.sh [TARGET] [MODE=mock|real] [IP=…] [PORT=…] [NAME=…]
#
#     TARGET   a flow name (e.g. drive_download), a flow file, or "all" / a
#              directory → the curated suite for the chosen MODE. Default: all.
#     MODE     mock (default) | real
#     IP/PORT  copyparty host/port for the parameterized flows
#     NAME     device name used by add_device/remove_device/drive_download/…
#
# MOCK (default): builds the debug APK + the mock binary, starts mock-copyparty
#   (data + control port), adb-reverses both, installs the APK, then runs the
#   suite with -e MODE=mock IP=127.0.0.1 PORT=$MOCK_PORT CONTROL=$CONTROL_PORT.
#   Mock lifecycle is duplicated from run-android-e2e.sh (~12 lines) on purpose —
#   that runner gates CI, so we don't refactor it; the mock CLI/endpoints are stable.
#
# REAL (MODE=real, IP=<comma>): requires an explicit ANDROID_SERIAL (the Pixel —
#   never "first adb device"); no mock, no adb reverse. Runs only the bounded
#   read-only flows (no control-port steps). Read-only is structural on the comma
#   (copyparty is a read-only volume); the only real risk is filling the Pixel, so
#   the suite downloads exactly ONE drive (qcamera-only previews, autoSync off).
#
# Examples:
#   ANDROID_SERIAL=emulator-5554 tools/run-maestro-e2e.sh                 # full mock suite
#   ANDROID_SERIAL=emulator-5554 tools/run-maestro-e2e.sh connectivity_refresh_mock
#   ANDROID_SERIAL=192.168.1.210:5555 tools/run-maestro-e2e.sh drive_download IP=192.168.1.100 PORT=8080 MODE=real
set -euo pipefail

DIR="$(cd "$(dirname "$0")/.." && pwd)"
MAESTRO_DIR="$DIR/android/maestro"

# ---- parse KEY=VAL args + an optional leading TARGET ------------------------
TARGET="all"
MODE="mock"
IP=""
PORT=""
NAME=""
for arg in "$@"; do
  case "$arg" in
    MODE=*) MODE="${arg#MODE=}" ;;
    IP=*)   IP="${arg#IP=}" ;;
    PORT=*) PORT="${arg#PORT=}" ;;
    NAME=*) NAME="${arg#NAME=}" ;;
    *=*)    echo "unknown KEY=VAL: $arg" >&2; exit 2 ;;
    *)      TARGET="$arg" ;;   # a flow name / file / "all" / directory
  esac
done

MOCK_PORT="${MOCK_PORT:-8099}"
CONTROL_PORT="${CONTROL_PORT:-8098}"
FIXTURE="${FIXTURE:-single_drive}"
APK="$DIR/android/app/build/outputs/apk/debug/app-debug.apk"

# Maestro needs JDK 17+; default to the project toolchain (same as dd-ui.sh).
export JAVA_HOME="${JAVA_HOME:-/usr/lib/jvm/java-17-openjdk}"
export PATH="$JAVA_HOME/bin:$HOME/.maestro/bin:$PATH"
if [ -z "${ANDROID_NDK_HOME:-}" ] && [ -d /opt/android-sdk/ndk/27.3.13750724 ]; then
  export ANDROID_NDK_HOME=/opt/android-sdk/ndk/27.3.13750724
fi

# ---- curated suites (order matters; each flow is self-contained via clearState)
# Mock: the full black-box surface, including the control-port live-refresh flows.
MOCK_SUITE=(empty_state drive_download play_drive star_drive manual_download_close \
            connectivity_refresh_mock drive_list_refresh_mock add_device remove_device)
# Real: only bounded, read-only flows — no control-port steps, one manual download.
REAL_SUITE=(empty_state add_device drive_download play_drive star_drive remove_device)

# Resolve TARGET → a list of flow files.
flows=()
case "$TARGET" in
  all|"$MAESTRO_DIR"|"$MAESTRO_DIR"/|android/maestro|android/maestro/)
    names=(); [ "$MODE" = real ] && names=("${REAL_SUITE[@]}") || names=("${MOCK_SUITE[@]}")
    for n in "${names[@]}"; do flows+=("$MAESTRO_DIR/$n.yaml"); done
    ;;
  *)
    f="$TARGET"; [ -f "$f" ] || f="$MAESTRO_DIR/${TARGET%.yaml}.yaml"
    [ -f "$f" ] || { echo "no such flow: $TARGET" >&2; ls "$MAESTRO_DIR" >&2; exit 1; }
    flows+=("$f")
    ;;
esac

# ---- mode defaults ----------------------------------------------------------
if [ "$MODE" = real ]; then
  : "${IP:?MODE=real requires IP=<comma ip> (e.g. IP=192.168.1.100)}"
  PORT="${PORT:-8080}"
  NAME="${NAME:-escape2020}"
  # Never silently target the wrong device on a real run.
  SERIAL="${ANDROID_SERIAL:?MODE=real requires an explicit ANDROID_SERIAL (the Pixel); refusing to guess}"
else
  IP="${IP:-127.0.0.1}"
  PORT="${PORT:-$MOCK_PORT}"
  NAME="${NAME:-maestro}"
  SERIAL="${ANDROID_SERIAL:-$(adb devices | awk 'NR>1 && $2=="device"{print $1; exit}')}"
  [ -n "$SERIAL" ] || { echo "no adb device connected" >&2; exit 1; }
fi

echo "==> Maestro suite  mode=$MODE  device=$SERIAL  target=$TARGET"
echo "    device under test: NAME='$NAME' IP=$IP PORT=$PORT  (CONTROL=$CONTROL_PORT for mock)"
printf '    flows:'; for f in "${flows[@]}"; do printf ' %s' "$(basename "$f" .yaml)"; done; echo

# ---- mock control plane (mock mode only) ------------------------------------
MOCK_PID=""
cleanup() {
  [ -n "$MOCK_PID" ] && kill "$MOCK_PID" 2>/dev/null || true
  if [ "$MODE" != real ]; then
    adb -s "$SERIAL" reverse --remove "tcp:$MOCK_PORT" 2>/dev/null || true
    adb -s "$SERIAL" reverse --remove "tcp:$CONTROL_PORT" 2>/dev/null || true
  fi
}
trap cleanup EXIT

if [ "$MODE" != real ]; then
  echo "==> building host mock-copyparty"
  cargo build -q -p mock-copyparty --manifest-path "$DIR/Cargo.toml"
  echo "==> starting mock-copyparty (fixture=$FIXTURE data=$MOCK_PORT control=$CONTROL_PORT)"
  "$DIR/target/debug/mock-copyparty" --fixture "$FIXTURE" --port "$MOCK_PORT" --control-port "$CONTROL_PORT" &
  MOCK_PID=$!
  for _ in $(seq 1 30); do
    curl -sf "http://127.0.0.1:$CONTROL_PORT/status" >/dev/null 2>&1 && break
    sleep 0.5
  done
  curl -sf "http://127.0.0.1:$CONTROL_PORT/status" >/dev/null || { echo "mock did not come up" >&2; exit 1; }
  echo "==> adb reverse (device reaches the host mock over loopback)"
  adb -s "$SERIAL" wait-for-device
  adb -s "$SERIAL" reverse "tcp:$MOCK_PORT" "tcp:$MOCK_PORT"
  adb -s "$SERIAL" reverse "tcp:$CONTROL_PORT" "tcp:$CONTROL_PORT"
fi

# ---- build + install the app ------------------------------------------------
echo "==> assembling + installing the debug APK"
"$DIR/android/gradlew" -p "$DIR/android" :app:assembleDebug --no-daemon
adb -s "$SERIAL" install -r "$APK"

# ---- run the flows ----------------------------------------------------------
ENVARGS=(-e "MODE=$MODE" -e "NAME=$NAME" -e "IP=$IP" -e "PORT=$PORT" \
         -e "CONTROL=$CONTROL_PORT" -e "WIFI_IP=")
failed=()
for f in "${flows[@]}"; do
  name="$(basename "$f" .yaml)"
  echo; echo "==> flow: $name"
  if maestro --device "$SERIAL" test "${ENVARGS[@]}" "$f"; then
    echo "    PASS $name"
  else
    echo "    FAIL $name"
    failed+=("$name")
  fi
done

echo
if [ "${#failed[@]}" -eq 0 ]; then
  echo "==> all ${#flows[@]} flow(s) passed"
else
  echo "==> ${#failed[@]} flow(s) FAILED: ${failed[*]}"
  exit 1
fi
