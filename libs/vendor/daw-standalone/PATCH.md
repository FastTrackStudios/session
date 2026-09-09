# Local save fix

Vendored from FastTrackStudios/daw v0.0.14,
commit b8a884f79e54b22a65dab2cab405efcaaf1aa9bf.
The manifest supplies the upstream workspace dependencies explicitly.

`src/save.rs` exports active-take flags from `TakeList.active_idx`, the backend's
authoritative selection. Cached `Take.is_active` values are not synchronized by
the loader or selection API. Saving those flags selected the wrong recording
in new multi-take split pieces. The companion dawfile-standalone patch emits
TAKE SEL for those pieces. Covered by the Crescendum practice round-trip test.

Remove both Cargo patches after adopting an upstream release containing the fixes.
