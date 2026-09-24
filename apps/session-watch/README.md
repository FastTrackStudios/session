# Session on the wrist

The Session watch app. Two jobs:

- **Guide**: where the band is in the song. It shows the section, the bar
  within it (`3/8`), the beat, the tempo, the count into the song, what
  comes next ("→ Chorus in 2") and the song as a strip of coloured
  sections.
- **Haptic click**: the beat as a tap on the wrist, in time with the
  tracks. Beat one of each bar and the count-in get the firm tap
  (`.start`); the other beats get the light one (`.click`).

It is the companion of the Session iPhone app (`app.fasttrackstudio.session`).
The bundle id is `app.fasttrackstudio.session.watchkitapp`.

## How it gets its data

**The iPhone relays a guide feed over WatchConnectivity.** The watch
itself does no timing at all.

```
Task (live set: presence, clock)                    ── vox/WebSocket ──  iPhone Session app
  leader's SyncPosition, stamped in Task's clock                            │ SharedClock (shared − phone)
                                                                            │ Guide::watch_timeline() → GuideTimeline
                                                                            │ WatchRelay: pings the watch → ClockEstimator (watch − phone)
                                                                            ▼
                                        WatchConnectivity (Bluetooth) ── feed: next 6 s of beats, in the WATCH's clock
                                                                            ▼
                                                                  watch: fire each beat at at_us − haptic lead
```

These are the reasons for this path:

- **The watch can't join a live set the way other peers do.** watchOS lets
  an app open a WebSocket, or any socket, only inside an audio-streaming
  session (Apple TN3135). That rules out vox over Task's `/vox`, and a
  Rust core on the watch wouldn't change it. vox has no HTTP transport,
  and the tier-3 `arm64_32-apple-watchos` targets would need `-Zbuild-std`
  on nightly.
- **The phone already has everything.** It is in the live set, it follows
  Task's clock (`SharedClock`), and it has the song's tempo map and
  sections open. WatchConnectivity goes over the Bluetooth link the watch
  always has to its phone, so it works without Wi-Fi or signal at the
  venue.
- **There is one code path.** The beats come from
  `session_guide::midi::click_notes`, which is the same grid that stamps
  the Click track the band hears. It is laid over
  `session_guide::midi::segments_in_span`, which `Guide::generate` now
  uses too. The count and the section cues come from the guide's own
  `CueSchedule`. The watch receives `WatchBeat`s with the bar, the section
  bar and the count already filled in. It shows fields and fires timers,
  and nothing else.
- **The phone runs the clock estimation.** It pings the watch four times a
  second. The watch answers with its continuous monotonic clock, stamped
  when the ping arrives and again when it replies. The phone feeds these
  exchanges into the same `ClockEstimator` (`daw-transport-sync`) that the
  shared clock uses, then converts every beat's instant from Task's clock
  to the phone's clock and on to the watch's before sending it.

The Rust side is `crates/session/watch` (`session-watch-guide`), re-exported
from the facade as `session::watch_guide`. The wire types are
`session_proto::watch::{WatchMessage, WatchGuideFeed, WatchBeat, …}`. The
Swift mirror, `SessionWatch/Generated/WatchGuide.generated.swift`, is
generated from them:

```bash
cargo run -p session-proto --example gen_watch_swift -- guide \
  > apps/session-watch/SessionWatch/Generated/WatchGuide.generated.swift
```

The relay degrades safely:

- A feed carries the next **6 s** of beats. The watch taps nothing past the
  last one, so if the phone goes quiet, the click stops within a few bars
  instead of drifting on its own.
- A start, stop, seek or song change starts a new `run`. Beats from an old
  run are dropped.
- A beat is tapped once per run, however many feeds resend it. A beat that
  comes due more than 40 ms late is skipped rather than played late.
- Until both clock mappings are known (`clock_locked`), the watch shows the
  guide but doesn't tap.

## Keeping it running with the wrist down

The app uses a **mindfulness extended runtime session**
(`WKBackgroundModes: mindfulness`). It keeps the app *frontmost* with the
screen dimmed for up to an hour, so the main-queue timers, the haptics and
the WatchConnectivity link all keep running. Apple's Breathe app uses the
same arrangement to pace breathing with taps.

A workout session would also work, and it has no hour limit. It was not
chosen for these reasons:

- It needs HealthKit authorisation and the HealthKit entitlement.
- It runs the heart-rate sensor.
- It records a workout, or has to throw one away.
- App Review could question it for a metronome.

The cost of this choice: a continuous run longer than an hour needs a
wrist raise. The app taps `.notification` when the session is about to
expire, and raising the wrist starts a new session.

Not verified: the watchOS simulator refuses extended runtime sessions
("unknown error", code 1). This needs a device.

## Timing accuracy

| part | how it is known | size |
|---|---|---|
| phone ↔ Task clock (`SharedClock`) | its own round trip; the error is at most half the asymmetry | typically 1–2 ms on a good network |
| watch ↔ phone clock over WatchConnectivity | **modelled**: `cargo run -p session-watch-guide --example link_accuracy` runs the relay's own code against a simulated link | p95 ≈ 2 ms (quiet link) · 6.6 ms (typical) · 17 ms (poor, 10 ms one-way asymmetry). An asymmetric link sets the floor, and no clock sync can detect it |
| watch timer firing | **measured** in the app (Settings › Timing, and the `click` log category) | simulator (Series 11, watchOS 27): p50 0.07 ms, p95 0.11 ms over 480 beats. Device: not yet measured |
| Taptic Engine latency | **measured** on the device by Calibrate, using the accelerometer (10 taps per pattern, median, about ±5 ms resolution) and applied as the lead | default lead 30 ms until calibrated |

Expected on a device: the tap lands about **±5–10 ms** from the band's
beat on a good Bluetooth link, and about ±20 ms on a poor one. The
threshold for noticing audio-tactile asynchrony is about 20–40 ms. None of
this has been measured on a real watch yet. To measure it:

1. Run Calibrate.
2. Play the demo or a live set.
3. Read the Timing section and the `click` log:

   ```bash
   log stream --predicate 'subsystem == "app.fasttrackstudio.session.watch"'
   ```

To check the end-to-end result with a band, record the watch's tap and the
click track with a contact mic. That test is still to do.

## Running it

It needs Xcode 26 or later (27 beta here) and XcodeGen:

```bash
cd apps/session-watch
nix run nixpkgs#xcodegen -- generate
export DEVELOPER_DIR=/Applications/Xcode-beta.app/Contents/Developer

# Simulator: build, install and run the Demo (no phone needed).
xcodebuild -project SessionWatch.xcodeproj -scheme SessionWatch \
  -destination 'generic/platform=watchOS Simulator' -derivedDataPath build \
  CODE_SIGNING_ALLOWED=NO build
SIM=$(xcrun simctl list devices available | grep -m1 'Apple Watch Series 11 (46mm)' | grep -oE '[0-9A-F-]{36}')
xcrun simctl boot "$SIM"; open -a Simulator
xcrun simctl install "$SIM" build/Build/Products/Debug-watchsimulator/Session.app
SIMCTL_CHILD_FTS_DEMO=1 xcrun simctl launch "$SIM" app.fasttrackstudio.session.watchkitapp
# SIMCTL_CHILD_FTS_TAB=settings opens on the settings page.
```

The Demo replays `SessionWatch/Resources/demo-feed.json`, one song's feed
built by the Rust timeline. It includes a tempo change and a count-in.
Regenerate it with:

```bash
cargo run -p session-watch-guide --example demo_feed > apps/session-watch/SessionWatch/Resources/demo-feed.json
```

On a device, the watch app ships inside the Session iPhone app's
TestFlight build. Use `deploy-testflight.sh` with:

```
WATCH_APP=apps/session-watch
WATCH_SCHEME=SessionWatch
WATCH_PRODUCT=Session
WATCH_BUNDLE_ID=app.fasttrackstudio.session.watchkitapp
```

This needs an App Store provisioning profile for that bundle id. Register
the id first. See `docs/distribution-handoff.md` §7.

## Hooking up the iPhone side

The phone runs `session::watch_guide::Relay` once it has joined a live
set:

```rust
use session::watch_guide::{Lead, Relay, Snapshot, SongSnapshot, link::WcLink};

// Once, on joining a set (iOS, feature `session/watch-link`):
let relay = WcLink::activate().map(|link| Relay::spawn(Arc::new(link), move || Snapshot {
    set_title: set.title.clone(),
    // Rebuilt when a song opens, or when its tempo map or sections change:
    //   Guide::new(daw).watch_timeline()  — the song generate() stamps from.
    song: current_song(),                        // Option<SongSnapshot>
    // Playing together: the leader's stamp, the one a following desktop locks
    // to. Playing apart: this phone's own engine, carried into Task's clock.
    lead: Lead::from_presence(&presence.states())
        .or_else(|| local_snapshot().map(|p| Lead::local(song_key(), p, clock.offset_micros().unwrap_or(0.0)))),
    shared_offset_us: clock.offset_micros(),
    shared_round_trip_us: clock.round_trip_micros(),
}));
```

`collab.rs` is not wired up yet. It is being rewritten for Task-hosted live
sets in parallel, and this goes in its tick once that lands.
