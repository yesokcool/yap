#!/bin/zsh
# Native ↔ web voice interop: the web client in headless Chromium
# (web-voice-peer.mjs) + YapVoiceTests on an iOS simulator (synthetic mic),
# both on a local dev server. See apple/README.md › Testing.
#
#   apple/Tools/run-voice-interop.sh <simulator-udid> [derived-data-dir]
#
# Env: YAP_URL (default http://127.0.0.1:8789), YAP_KEY (testkey), YAP_ADMIN_KEY (testadmin).
# RED measurements (apple/README.md › Tests): YAP_IMPAIR=wifi|cell|loss5|loss2 (an impairment
# proxy between the two), YAP_RED=0 (redundancy off, to compare), YAP_MEASURE_SECONDS (default 6),
# YAP_PEER_SCENARIOS=voice (only the two voice scenarios), YAP_TESTS (default YapVoiceTests;
# e.g. YapVoiceTests/VoiceInteropTests for the interop alone).
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
TEST_RUNNER_YAP_PEER_STATUS=$STATUS TEST_RUNNER_YAP_RED=${YAP_RED:-1} TEST_RUNNER_YAP_IMPAIR=${YAP_IMPAIR:-} \
TEST_RUNNER_YAP_MEASURE_SECONDS=${YAP_MEASURE_SECONDS:-6} TEST_RUNNER_YAP_INTEROP_SCENARIOS=${YAP_PEER_SCENARIOS:-all} \
  xcodebuild test -scheme YapKit-Package -destination "platform=iOS Simulator,id=$UDID" \
    -derivedDataPath "$DD" -only-testing:${YAP_TESTS:-YapVoiceTests} -parallel-testing-enabled NO \
    -skipMacroValidation -skipPackagePluginValidation > "$WORK/xcodebuild.log" 2>&1
CODE=$?
set -e
wait $PEER 2>/dev/null || true
trap - EXIT

echo "── native (xcodebuild) ──"
grep -E "VOICE |RED |SCREEN |Test run|passed|failed|recorded an issue|error:" "$WORK/xcodebuild.log" | grep -v "^ *|" | grep -v "RED stats" | head -80
echo "── web peer ──"
cat "$WORK/web-peer.log"
echo "logs: $WORK"
exit $CODE
