#!/usr/bin/env bash
# Run the Android on-device (instrumented) tests with the mock-copyparty fixture wired in, so the
# "live" tests (sync / download / connectivity / background-sync) actually execute instead of
# self-skipping. One command for both local runs and CI.
#
# What it does:
#   1. builds the host mock-copyparty binary (cheap if cached),
#   2. starts it with a data port + a control port (runtime state injection),
#   3. adb reverse both ports so the device/emulator reaches the host mock,
#   4. runs the :core + :app connected tests, passing mockPort/controlPort instrumentation args,
#   5. tears the mock down on exit.
#
# Usage:
#   tools/run-android-e2e.sh                 # both modules, all connected tests
#   tools/run-android-e2e.sh -Pandroid.testInstrumentationRunnerArguments.class=org.sunnypilot.dashdown.SyncSessionWorkerLiveTest
#   ANDROID_SERIAL=emulator-5554 tools/run-android-e2e.sh   # pick a device when several are attached
#
# Env: MOCK_PORT (8099), CONTROL_PORT (8098), FIXTURE (single_drive), GRADLE_TASKS
#      (":core:connectedDebugAndroidTest :app:connectedDebugAndroidTest").
set -euo pipefail

DIR="$(cd "$(dirname "$0")/.." && pwd)"
MOCK_PORT="${MOCK_PORT:-8099}"
CONTROL_PORT="${CONTROL_PORT:-8098}"
FIXTURE="${FIXTURE:-single_drive}"
GRADLE_TASKS="${GRADLE_TASKS:-:core:connectedDebugAndroidTest :app:connectedDebugAndroidTest}"

# JDK 17+ is required by the Gradle build; default to the project's toolchain if the caller (e.g.
# CI's setup-java) hasn't exported one.
export JAVA_HOME="${JAVA_HOME:-/usr/lib/jvm/java-17-openjdk}"
# Point cargo-ndk at the local NDK only when that path actually exists (local dev). On CI runners
# the SDK lives elsewhere and cargo-ndk auto-detects it from ANDROID_SDK_ROOT — don't clobber that.
if [ -z "${ANDROID_NDK_HOME:-}" ] && [ -d /opt/android-sdk/ndk/27.3.13750724 ]; then
  export ANDROID_NDK_HOME=/opt/android-sdk/ndk/27.3.13750724
fi

echo "==> building host mock-copyparty"
cargo build -q -p mock-copyparty --manifest-path "$DIR/Cargo.toml"

echo "==> starting mock-copyparty (fixture=$FIXTURE data=$MOCK_PORT control=$CONTROL_PORT)"
"$DIR/target/debug/mock-copyparty" --fixture "$FIXTURE" --port "$MOCK_PORT" --control-port "$CONTROL_PORT" &
MOCK_PID=$!
cleanup() {
  kill "$MOCK_PID" 2>/dev/null || true
  adb reverse --remove "tcp:$MOCK_PORT" 2>/dev/null || true
  adb reverse --remove "tcp:$CONTROL_PORT" 2>/dev/null || true
}
trap cleanup EXIT

# Wait for the control port to answer (mock is up).
for _ in $(seq 1 30); do
  if curl -sf "http://127.0.0.1:$CONTROL_PORT/status" >/dev/null 2>&1; then break; fi
  sleep 0.5
done
curl -sf "http://127.0.0.1:$CONTROL_PORT/status" >/dev/null || { echo "mock did not come up" >&2; exit 1; }

echo "==> adb reverse (device reaches the host mock over loopback)"
adb wait-for-device
adb reverse "tcp:$MOCK_PORT" "tcp:$MOCK_PORT"
adb reverse "tcp:$CONTROL_PORT" "tcp:$CONTROL_PORT"

echo "==> connected tests: $GRADLE_TASKS"
# shellcheck disable=SC2086
"$DIR/android/gradlew" -p "$DIR/android" $GRADLE_TASKS \
  -Pandroid.testInstrumentationRunnerArguments.mockPort="$MOCK_PORT" \
  -Pandroid.testInstrumentationRunnerArguments.controlPort="$CONTROL_PORT" \
  --no-daemon --stacktrace "$@"
