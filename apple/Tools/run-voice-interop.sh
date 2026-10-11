#!/bin/zsh
# Native ↔ web voice interop: the web client in headless Chromium
# (web-voice-peer.mjs) + YapVoiceTests on an iOS simulator (synthetic mic),
# both on a local dev server. See apple/README.md › Testing.
#
#   apple/Tools/run-voice-interop.sh <simulator-udid> [derived-data-dir]
#
# Env: YAP_URL (default http://127.0.0.1:8789), YAP_KEY (testkey), YAP_ADMIN_KEY (testadmin).
set -euo pipefail
HERE=${0:A:h}
UDID=${1:?usage: run-voice-interop.sh <simulator-udid> [derived-data-dir]}
WORK=$(mktemp -d -t yap-voice)
DD=${2:-$WORK/dd}
export YAP_URL=${YAP_URL:-http://127.0.0.1:8789}
export YAP_KEY=${YAP_KEY:-testkey}
export YAP_ADMIN_KEY=${YAP_ADMIN_KEY:-testadmin}
STATUS=$WORK/web-peer.json

node "$HERE/web-voice-peer.mjs" "$STATUS" > "$WORK/web-peer.log" 2>&1 &
PEER=$!
trap 'kill $PEER 2>/dev/null || true' EXIT

cd "$HERE/../YapKit"
set +e
TEST_RUNNER_YAP_TEST_URL=$YAP_URL TEST_RUNNER_YAP_TEST_KEY=$YAP_KEY TEST_RUNNER_YAP_TEST_ADMIN=$YAP_ADMIN_KEY \
TEST_RUNNER_YAP_PEER_STATUS=$STATUS \
  xcodebuild test -scheme YapKit-Package -destination "platform=iOS Simulator,id=$UDID" \
    -derivedDataPath "$DD" -only-testing:YapVoiceTests -parallel-testing-enabled NO \
    -skipMacroValidation -skipPackagePluginValidation > "$WORK/xcodebuild.log" 2>&1
CODE=$?
set -e
wait $PEER 2>/dev/null || true
trap - EXIT

echo "── native (xcodebuild) ──"
grep -E "VOICE |Test run|passed|failed|recorded an issue|error:" "$WORK/xcodebuild.log" | grep -v "^ *|" | head -60
echo "── web peer ──"
cat "$WORK/web-peer.log"
echo "logs: $WORK"
exit $CODE
