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
