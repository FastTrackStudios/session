#!/usr/bin/env bash
# The Session iPhone app and the Session watch app, together, on a paired
# iPhone + Apple Watch simulator — the end-to-end check of the watch feed:
# the phone's engine plays the demo setlist (FTS_DEMO=1), the relay pings the
# watch's clock over WatchConnectivity and sends it the beats, the watch
# taps them.
#
#   apps/session-watch/sim-pair.sh [PAIR_UDID]     # default: the first pair
#
# The phone app is built with the host toolchain (no nix shell, no dx — the
# host dx is a different dioxus from the tree's) and bundled by hand; this
# is a simulator bundle, never a release one (deploy-testflight.sh is).
# Logs: the phone's tracing on this terminal (RUST_LOG below), the watch's
# with `xcrun simctl spawn <watch> log stream --predicate 'subsystem ==
# "app.fasttrackstudio.session.watch"'`.
#
# Shut the simulators down afterwards (`xcrun simctl shutdown all`): a
# booted simulator adds a virtual audio device the desktop app's CoreAudio
# input can hang on.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode-beta.app/Contents/Developer}"
PAIR="${1:-$(xcrun simctl list pairs | awk '/^[0-9A-F-]{36}/{print $1; exit}')}"
WATCH="$(xcrun simctl list pairs | awk -v p="$PAIR" '$1==p{f=1;next} f&&/Watch:/{match($0,/[0-9A-F-]{36}/);print substr($0,RSTART,RLENGTH);exit}')"
PHONE="$(xcrun simctl list pairs | awk -v p="$PAIR" '$1==p{f=1;next} f&&/Phone:/{match($0,/[0-9A-F-]{36}/);print substr($0,RSTART,RLENGTH);exit}')"
[ -n "$WATCH" ] && [ -n "$PHONE" ] || { echo "no simulator pair $PAIR" >&2; exit 1; }
echo "pair $PAIR: phone $PHONE, watch $WATCH"

echo "=== phone app (session-desktop, iOS simulator) ==="
( cd "$ROOT" && cargo build -p session-desktop --target aarch64-apple-ios-sim \
    --no-default-features --features session,charts,watch )
APP="$ROOT/target/aarch64-apple-ios-sim/sim-pair/Session.app"
rm -rf "$APP" && mkdir -p "$APP"
# Built against the iOS 27 SDK, UIKit requires the UIScene lifecycle, which
# tao (dioxus' iOS shell) has not adopted: the app traps at launch
# ("NoSceneLifecycleAdoption"). The TestFlight build links the 26.x SDK and
# is unaffected; for this simulator run the binary is stamped as built
# against 26.0. (The real fix is scene adoption in tao/dioxus-mobile — a
# release blocker once the iOS build moves to the 27 SDK.)
xcrun vtool -set-build-version iossim 14.0 26.0 -replace \
    -output "$APP/session-desktop" "$ROOT/target/aarch64-apple-ios-sim/debug/session-desktop"
cat > "$APP/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleExecutable</key><string>session-desktop</string>
  <key>CFBundleIdentifier</key><string>app.fasttrackstudio.session</string>
  <key>CFBundleName</key><string>Session</string>
  <key>CFBundleDisplayName</key><string>Session</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.0.1</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>CFBundleSupportedPlatforms</key><array><string>iPhoneSimulator</string></array>
  <key>LSRequiresIPhoneOS</key><true/>
  <key>MinimumOSVersion</key><string>15.0</string>
  <key>UIDeviceFamily</key><array><integer>1</integer></array>
  <key>UILaunchScreen</key><dict/>
  <key>UIBackgroundModes</key><array><string>audio</string></array>
  <key>NSMicrophoneUsageDescription</key><string>Used for the audio session during setlist playback.</string>
</dict></plist>
PLIST

echo "=== watch app ==="
# A stale nix eval cache can name an xcodegen .drv a GC removed ("don't know
# how to recreate store derivation"): drop the cache and try once more.
# Only when project.yml is newer than the project it generates.
xcodegen() { ( cd "$ROOT/apps/session-watch" && nix run nixpkgs#xcodegen -- generate >/dev/null ); }
if [ ! -d "$ROOT/apps/session-watch/SessionWatch.xcodeproj" ] \
    || [ "$ROOT/apps/session-watch/project.yml" -nt "$ROOT/apps/session-watch/SessionWatch.xcodeproj" ]; then
    xcodegen || { rm -rf "$HOME/.cache/nix"; xcodegen; }
fi
xcodebuild -project "$ROOT/apps/session-watch/SessionWatch.xcodeproj" -scheme SessionWatch \
    -destination 'generic/platform=watchOS Simulator' -derivedDataPath "$ROOT/apps/session-watch/build" \
    CODE_SIGNING_ALLOWED=NO build | grep -E "error|BUILD" || true
WATCH_APP="$ROOT/apps/session-watch/build/Build/Products/Debug-watchsimulator/Session.app"

# Embedded, as the TestFlight build embeds it: WatchConnectivity pairs an
# iPhone app only with the watch app it carries (a watch app installed on
# its own reads as "not installed" to the phone).
mkdir -p "$APP/Watch"
cp -R "$WATCH_APP" "$APP/Watch/"
codesign --force --sign - "$APP/Watch/Session.app"
codesign --force --sign - "$APP"

echo "=== boot, install, launch ==="
xcrun simctl boot "$PHONE" 2>/dev/null || true
xcrun simctl boot "$WATCH" 2>/dev/null || true
xcrun simctl bootstatus "$PHONE" -b >/dev/null
xcrun simctl bootstatus "$WATCH" -b >/dev/null
open -a "$DEVELOPER_DIR/Applications/Simulator.app" 2>/dev/null || true
xcrun simctl install "$PHONE" "$APP"
# The phone's WatchConnectivity counts the watch app installed only if it
# lands while the phone app is running (installed with the phone app idle,
# the phone's session keeps saying "Watch app is not installed"): start it
# plainly first, install the watch app, then relaunch it below with the demo.
SIMCTL_CHILD_FTS_NO_AUDIO="${FTS_NO_AUDIO:-1}" xcrun simctl launch "$PHONE" app.fasttrackstudio.session >/dev/null
sleep 3
xcrun simctl install "$WATCH" "$WATCH_APP"
# The measured part is the clock and the timers: no haptic lead, so a tap's
# timer fires on the beat itself (the Taptic Engine's own latency is a
# device measurement — Calibrate).
xcrun simctl spawn "$WATCH" defaults write app.fasttrackstudio.session.watchkitapp leadClickMs -float 0
xcrun simctl spawn "$WATCH" defaults write app.fasttrackstudio.session.watchkitapp leadStrongMs -float 0
xcrun simctl launch --terminate-running-process "$WATCH" app.fasttrackstudio.session.watchkitapp
# On the Session workspace, the song and its playhead beside the watch's.
# FTS_NO_AUDIO: the simulator's audio server times out opening RemoteIO on
# this host (an abort, not an error), so the transport runs on the engine's
# soft clock — which stamps the same per-buffer sync snapshots.
SIMCTL_CHILD_FTS_DEMO=1 SIMCTL_CHILD_FTS_NO_AUDIO="${FTS_NO_AUDIO:-1}" SIMCTL_CHILD_FTS_OPEN_WORKSPACE="${FTS_OPEN_WORKSPACE:-session}" SIMCTL_CHILD_RUST_LOG="${RUST_LOG:-info,session_desktop::watch=debug,session_watch_guide=debug}" \
    xcrun simctl launch --console-pty --terminate-running-process "$PHONE" app.fasttrackstudio.session
