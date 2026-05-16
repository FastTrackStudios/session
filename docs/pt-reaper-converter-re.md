# RE notes — PT Reaper Converter v1.5.4

The third-party "PT Reaper Converter" app (Mac universal binary in
`~/Downloads/pt-reaper-converter-v1.5.4.dmg`) converts both directions
between `.ptx` and `.rpp` and supports far more fields than our parser
currently does — including correctly distinguishing track-level mute
from clip-level mute and send-level mute. Extracting some structural
hints here for future RE.

## Binaries

| Path | Size | Role |
|---|---|---|
| `PT Reaper Converter.app/Contents/MacOS/PT Reaper Converter` | 6.7 MB | Standalone converter (Swift, universal x86_64 + arm64). The actual `.ptx` parser. |
| `reaper_ptx_import.dylib` | 178 KB | REAPER plugin wrapper. Just spawns the standalone app via temp file. Not useful for RE. |

## Build characteristics

- Language: **Swift** (with Objective-C runtime, SwiftUI)
- Mangled class prefix: `_TtC19PT_Reaper_Converter` (Swift v3) → e.g.
  `_TtC19PT_Reaper_Converter9PTXParser` = `PT_Reaper_Converter.PTXParser`
- Some debug symbols stripped — only 6 `.swift` file paths survive
- Type metadata for PTX entities visible as plain ASCII strings

## Swift classes related to mute (read side)

```
PTXParser                — top-level .ptx reader
PTXTrack                 — a parsed track
PTXTrackSpec             — track definition (name, channels)
PTXTrackStateSpec        — track mix state (vol, pan, mute…)
PTXMutePoint             — single mute automation breakpoint
```

## Mute field nomenclature (used by the writer side; same vocab as read)

| Field | Meaning |
|---|---|
| `mainMute` | Track-level mute (the M button) — boolean |
| `mainMuteAutomation` | Time-varying main mute envelope |
| `clipMute` | Per-clip / per-region mute |
| `sendMuteAutomation` | Send-level mute over time |

The user's confusion about "wrongly muted" tracks in our parser may be
that `0x1029 +5` is **NOT** `mainMute`. It might be `clipMute` or a
default-clip-mute applied to bounced/print tracks. The converter app
treats these as separate concepts; our parser conflates them.

## Confirmed: `0x1029` is referenced in the binary

Binary scan for the immediate constant `0x1029` (LE bytes `29 10`) as
either a `cmp` or `push` instruction:

| File offset | Pattern |
|---|---|
| `0x10241b` | `3D 29 10 00 00` — `cmp eax, 0x1029` |
| `0x15b4a3` | `68 29 10 00 00` — `push 0x1029` |
| `0x1a0302` | `68 29 10 00 00` — `push 0x1029` |
| `0x2378b9` | `3D 29 10 00 00` — `cmp eax, 0x1029` |

So the parser does dispatch on content_type=`0x1029`. The work needed
is to follow the cmp/push targets to see what fields it reads and at
what payload offsets.

## What's blocking deeper analysis

1. Swift symbol mangling — Ghidra's auto-analysis identified only
   1229 functions of which only ~32 had recognized names.
2. Significant inlining + optimizations — the relevant logic is
   spread across many small leaves.
3. Standard ObjC/Swift analyzer in Ghidra does not unmangle Swift v3
   class refs reliably for this binary.

## Update: Swift type metadata recovered via ipsw

`ipsw swift-dump --demangle` extracts the complete Swift type model
from the binary even though function symbols are stripped, because
Swift embeds class/struct/enum layout metadata for reflection. Full
dump is in `docs/pt-reaper-converter-swift-dump.txt`. The mute-relevant
structs:

```swift
struct PTXTrackStateSpec {
    var color: UInt8?       // ← static color
    var active: Bool?
    var hidden: Bool?
    var solo: Bool?
    var soloSafe: Bool?
    var mute: Bool?         // ← static M button
    var sends: [String: PTXSendStateSpec]
}

struct PTXSendStateSpec {
    var mute: Bool?
    var preFader: Bool?
    var active: Bool?
}

struct PTXTrackSpec {
    var name: String
    var type: String
    var format: String
    var output: String?
    var input: String?
    var color: UInt8?
    var active: Bool
    var hidden: Bool
    var solo: Bool
    var soloSafe: Bool
    var volume: [(pos: Int, raw: Int)]?  // ← automation breakpoints
    var mute: [(pos: Int, raw: Int)]?    // ← MUTE AUTOMATION
    var pan: [(pos: Int, raw: Int)]?
    var surroundPan: PTXSurroundPanSpec?
    var sends: [String: PTXSendSpec]
    var playlists: [PTXPlaylistSpec]
    var notes: String
    var children: [String]
    var clips: [PTXClipSpec]
}

struct PTXClipSpec {
    var poolFile: String
    var start: Int
    var end: Int
    var sourceIn: Int
    var name: String?
    var fades: [PTXFadeSpec]
    var clipGain: PTXClipGain?
    var color: UInt8?
    var muted: Bool         // ← per-clip mute
}

struct PTXMutePoint {
    let positionSamples: Int
    let muted: Bool         // ← one automation breakpoint
}
```

**Conclusion on mute architecture:** PT stores mute in FOUR distinct
places (static track mute, mute automation envelope, per-clip mute,
per-send mute). Our parser reads `0x1029 +5` which is most likely
`PTXTrackStateSpec.mute`. The user-visible mute state in PT's UI is
the *effective* mute = `mute || (automation evaluated at playhead)`.

For the over-muted tracks (SYZ, AC GTR, El Gtr 1, Bass Demo): their
`PTXTrackStateSpec.mute` is `true` but they likely have a mute
*automation envelope* that overrides it to `false` at playhead, OR
PT treats `active: false` as a distinct visual state from "muted".

For the under-muted Inst dups: those alternate playlists inherit
the parent's mute state via the PT mixer — not stored directly on
the alternate.

## Other recovered field models (selected highlights)

```swift
struct PTXClipGainBreakpoint {
    let positionSamples: Int
    let valueDB: Double
}

struct PTXMarker {
    let number: Int
    let name: String
    let positionSamples: Int
    let color: UInt8?       // ← MARKERS HAVE COLORS too
}

struct PTXAudioFileInfo {
    var filename: String
    var durationSamples: Int
    var sampleRate: Int
    var channels: Int
    var bitDepth: Int
    var hash1: Data?         // ← Avid Unique ID
    var hash2: Data?
    var fileUUID: Data?
}

struct PTXRegion {
    let name: String
    let index: Int
    var sampleRate: Int
    var durationSamples: Int
    var sourceIn: Int
    var sourceLength: Int
    var clipDuration: Int
    var sourceFile: String
    var sfIdx: Int          // ← the source-file index we couldn't read
}

struct PTXBus {
    let id: Int
    let name: String
    let format: String
    let channels: Int
    let isPhysicalOutput: Bool
    let parentBusId: Int?   // ← bus hierarchy
}

struct PTXFade {
    let trackName: String
    let regionName: String
    let timelineStart: Int
    let fadeDirection: String
    let curveSlope: String
    let curveShape: String
    let durationSamples: Int
    let crossfadeStart: Int
}
```

These give us the complete field set the upstream tool extracts.
Promote each as a roadmap target.

## Useful next steps (if continuing this thread)

1. Pull Ghidra's Swift symbolicator plugin or use `swift-demangle`
   manually on `_$s` and `_TtC` strings to give functions friendly
   names.
2. Manually mark the 4 `0x1029` call sites' parent functions, study
   them as the entry points for TrackMixSettings parsing.
3. Cross-reference `mainMute` / `clipMute` string offsets to find
   which parser produces the named fields.
4. Alternative cheaper path: invoke the converter on the
   Lord-of-the-Fight session and observe what mute state it emits
   for each track. The output `.rpp` becomes ground truth.

## Strings of interest (full list)

`/tmp/pt_reaper_converter` binary contains these PTX classes (search
`PTX[A-Z]` for the full list):

```
PTXAudioFileInfo, PTXAutomationWriter, PTXAutomationWriterError,
PTXBlocks, PTXBus, PTXBusSpec, PTXClipGain, PTXClipGainWriter,
PTXClipPlacement, PTXClipSpec, PTXClipWriter, PTXExpandError, PTXFade,
PTXFadeCurve, PTXFadeSpec, PTXFadeType, PTXFadeWriter,
PTXFadeWriterError, PTXFeatureWriterError, PTXFeatureWriters,
PTXFolderNesting, PTXInputRouting, PTXLinkingOptions,
PTXMappingAnomaly, PTXMarker, PTXMarkerSpec, PTXMeterEntry,
PTXMissingFile, PTXMissingFileKind, PTXMutePoint, PTXOutputRouting,
PTXPanPoint, PTXParser, PTXPlaylistSpec, PTXPoolError,
PTXPoolFileSpec, PTXPreflightResult, PTXRegion, PTXRegistry,
PTXRelinkCandidate, PTXSendSpec, PTXSendStateSpec, PTXSendWriter,
PTXSessionBuilderError, PTXSessionSpec, PTXStats, PTXSubPathSpec,
PTXSurroundPan, PTXSurroundPanSpec, PTXSurroundPanWriter,
PTXTempoEntry, PTXTrack, PTXTrackExpander, PTXTrackSpec,
PTXTrackStateSpec, PTXUnifiedClipPipeline, PTXVideoClip,
PTXVideoFile, PTXVideoRegion, PTXVolumeTransition
```

These names tell us EVERY feature the upstream tool can read/write.
Our parser covers ~5 of these (Tracks, Regions, Markers, Tempo,
SurroundPan partial). Big gaps for our roadmap: PlaylistSpec,
SendStateSpec, FadeWriter, AutomationWriter, VolumeTransition.
