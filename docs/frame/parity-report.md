# Figma Parity Report

Generate a consolidated parity artifact from:
- feature coverage (`fts-figma-export` payload),
- optional visual acceptance results,
- optional performance snapshot JSON.

## Command

```bash
tools/frame/run_parity_pipeline.sh \
  tests/frame/fixtures/latest-bridge-export.json \
  --out-dir docs/frame/reports \
  --actual-png-dir /path/to/actual \
  --expected-png-dir /path/to/expected \
  --threshold 2.0 \
  --require-pass
```

Manual split flow is still available via:
- `tools/frame/capture_render_diagnostics.sh`
- `tools/frame/capture_import_diagnostics.sh`
- `tools/frame/run_visual_acceptance.sh`
- `tools/frame/generate_parity_report.sh`

## Output

- JSON report:
  - `featureCoverage`
  - `visualAcceptance` (if provided)
  - `performance` (if provided)
  - `renderDiagnostics` (if provided)
  - `importDiagnostics` (if provided)
  - `gates` summary
- Markdown report suitable for bead closure evidence.
- `--require-pass` makes pipeline exit non-zero when `gates.parityPass` is `false` (CI-friendly).
