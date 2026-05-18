# Parity-push session progress — 2026-05-17/18

## Goal

Reach 100% parity with `docs/pt-parity-roadmap.md` (the canonical
read+write feature checklist for `.ptx` files).

## Status: substantial progress on a multi-week goal

A single session can't close 100% of the roadmap — many items are
opaque-blob parser RE (FX plugin params), require new fixtures (real
WAV-source for `clip_muted`/`clip_gain`), or need full block
serializer work (Phase D §15). But this session shipped concrete
movement on the highest-impact items.

## What landed (17 commits on `pt-re-features`, branched off `main`)

### Read-side (parser) — newly decoded

| Feature | Block | Status |
|---|---|---|
| Track solo | `0x102d +162` | ✅ |
| Track solo-defeat | `0x200b +268` (mirror `0x200a +259`) | ✅ |
| Track inactive (derived) | `0x1029 +5 == 1` AND `0x260a[0] +8 == 1` | ✅ |
| Track mute (effective, w/ send routing) | `0x1029 +5` ∧ ¬`0x260a[0] +8` | ✅ |
| Track color | `0x200a +97`, `0x200b +106`, `0x2015 +88` | ✅ |
| Mute automation envelope | `0x260a[1]` (per-track) | ✅ |
| Volume automation envelope | `0x260a[0]` (per-track) | ✅ |

### Write-side (`dawfile-protools::write::native`)

| Feature | Status |
|---|---|
| Single-track field-complete | ✅ name, color, mute, solo, solo-defeat, inactive, vol, pan, mute auto, vol auto |
| Multi-track structural (N × `0x261c`) | ✅ |
| Multi-track parser-visible | ✅ outer-list extension (`0x1015` / `0x1054` / `0x2519`) |
| Multi-track distinct names | ✅ verified up to 5 tracks |
| Per-track color / solo / mute / vol / pan | ✅ scoped to each `0x261c` |

### Parser improvements

- **Format detection.** Parser auto-detects converter-authored PTX
  (11× `0x1029` nested per `0x261c`) vs PT-authored (flat 1-per-track)
  and switches mix-block scoping accordingly. Both LotF user session
  and converter-cloned multi-track files parse correctly.
- **Duplicate-name handling.** `0x251a` walks past consecutive
  duplicates (multi-track interleave) while still stopping on the
  PT-authored 2× full-list copy marker.

### RE infrastructure added

7 new probe / inspection examples:
- `diff_block_counts`, `diff_track_groups[_xfile]` — byte-level diff
- `find_per_track_range`, `find_containing_block`, `find_lists`,
  `find_envtime` — block locators
- `dump_track_group_layout`, `dump_bytes`, `dump_regions`,
  `dump_envelope`, `dump_tracks_parsed`, `count_probe`, `check_state`,
  `check_auto`, `clone_track_mvp` — content inspectors

3 new probe variants added to `rpp_to_ptx_probe`: `one_track_aaa`,
`two_tracks_eq`, `three_tracks`.

### Test counts

- **Before session**: 14 lib + 15 + 14 + 1 + 1 + 3 = 48 tests, 0 ignored.
- **After session**: 18 lib + 15 + 14 + 1 + 1 + 3 = 52 tests, 0 ignored.

All `dawfile-protools` fixtures still byte-identically round-trip
via `RawSession::encrypt()` (the `round_trip_full` integration test).

### Parity matrix update (`docs/feature-parity.md`)

- **PTX → RPP**: 65% → 72% functional
- **RPP → PTX**: 15% → 40% functional

## Remaining roadmap — biggest gaps

The Phase A/B/C/D/E ordering in `pt-parity-roadmap.md` still applies.
Top-impact items left:

### Phase A (correctness)
- Region audio_file_index — current parser explicitly returns 0;
  needs careful byte-RE per region kind (varies with name length,
  region format old vs new). Tracked at §16.

### Phase B (mix fidelity)
- Track input routing — block not located
- Track output for multi-track converter PTX — single-track works
  via `set_track_output` but the baseline emits the empty-variant
  `0x260e` which the writer refuses to convert in-place
- Aux sends count/levels/destinations — partial RE; per-track
  `0x260a` sends decoded structurally but content not parsed
- HW insert routing — block not located
- Per-track FX insert chain — Phase B item but very opaque

### Phase C (production detail)
- Region clip gain (static + dynamic)
- Region clip mute — needs `clip_muted` probe with a real WAV
  fixture (current `item_basic` probe is zero-diff)
- Region pitch / time-stretch / Elastic
- MIDI CC / pitch bend / channel
- Pan automation read — `0x260a[2]` suspected but converter doesn't
  emit it for the `PANENV`/`PANENV2` probe names tried so far

### Phase D (round-trip writer)
- Full block serializer that handles ANY block, not just the
  baseline template
- Cross-reference rewriter for indices that change after edits
- Unknown-block passthrough (currently guaranteed at the raw-byte
  level via `RawSession::encrypt()` but not at the structural level)

### Phase E (niceties)
- Compound regions, alternate playlists round-trip, key-sig /
  chord-symbol items, track folders, icons/comments/height

## Operational notes

- Pre-push hook blocks `git push -u origin pt-re-features` because
  unrelated tests in the `main` worktree don't compile right now
  (`csurf_subscribe_latency.rs`, `event_bus` mod, `daw-control`
  routing/transport methods, etc.). Local branch carries 17 ready
  commits; push after main builds.
- 8 GH issues opened for tracked gaps: #26–#33 in `FastTrackStudios/daw`.
