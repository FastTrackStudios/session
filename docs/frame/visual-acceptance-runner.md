# Visual Acceptance Runner

Use this to run parity diffs for a directory of rendered PNG captures.

## Command

```bash
tools/frame/run_visual_acceptance.sh \
  /path/to/actual-pngs \
  /path/to/expected-pngs \
  2.0 \
  --out-json docs/frame/reports/visual-acceptance.json \
  --out-md docs/frame/reports/visual-acceptance.md
```

- The script matches `*.png` files by relative path under `expected`.
- It invokes `frame-ui`'s `visual_diff` binary for each pair.
- It prints a JSON summary and exits non-zero on failures/missing outputs.
- Optional `--out-json` writes the full summary payload to disk.
- Optional `--out-md` writes a human-readable report for bead closure evidence.

## Output

JSON fields:
- `total`, `passed`, `failed`, `missingActual`
- `results[]` entries with per-image diff metrics and status
