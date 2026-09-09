# Local source-preservation fix

Vendored from FastTrackStudios/daw v0.0.14,
commit b8a884f79e54b22a65dab2cab405efcaaf1aa9bf.
The manifest makes inherited workspace dependencies explicit.

The upstream exporter emits take properties but omits SOURCE chunks when building
new items or adding takes. A split therefore saves its new piece without audio.
`src/rpp/export/sources.rs` supplies missing file or opaque source chunks after
structural reconciliation, using the document and its object store. Existing
source chunks remain intact. Missing opaque objects produce an export error.

The root Cargo patch applies this fix to the standalone backend as well as direct
consumers. Remove it after adopting an upstream release containing the fix.
Regression coverage lives in the expression editor standalone tests
`rpp_save_sources` and `practice_real`. Upstream corpus tests also require the sibling
`dawfile-reaper/tests/fixtures` tree, which is not included in this vendor copy.

The take property writer also resolves each complete take run by GUID before
writing fields: GUID follows SOFFS in RPP, so matching only when reaching GUID
corrupted offsets on projects containing empty take slots.

New take delimiters preserve active selection with TAKE SEL. The importer derives
unique identities for unnamed take slots using their index; assigning every null
slot the same identity caused reconciliation to discard slots.
