# keyflow-sync

Lyrics-to-audio **forced alignment** + pluggable **stem separation** for Keyflow.

Given a song's audio and its *known* lyrics (from a `.kf` chart), it produces a
**timing-map sidecar**: per-word start/end times with a confidence rating, keyed
back to the chart by `(section, word)`. The `.kf` is never modified — it stays the
single source of *what* (rhythm + lyrics); the sidecar is the single source of
*when*. Slides, karaoke, and MIDI lyric events are all derivations of
`chart + sidecar`.

This is the free/OSS path to what AudioShake's LyricSync does commercially.

## Pipeline

```
audio.wav ─▶ [separate] vocals ─▶ wav2vec2/MMS CTC emissions ─▶ forced-align trellis ─▶ words+confidence ─▶ sidecar
              (Demucs, optional)    (acoustic model)              (pure Rust)            (rating gate)
```

* **Separation** is the biggest quality lever — speech acoustic models fall apart
  on a full mix. It's a trait ([`StemSeparator`]) so backends (Demucs-ONNX,
  external `demucs`, a future online API) can be benchmarked head-to-head.
* **The trellis** (`align::trellis`) is the standard CTC forced-alignment Viterbi
  over the blank-extended state sequence. Pure Rust, no model, fully unit-tested —
  the one piece no off-the-shelf crate hands you.
* **The rating system** falls out of the alignment: each word's mean CTC
  probability → `Verified` / `Review` / `Failed`. Gates which sections are
  trustworthy enough to drive karaoke / CloneHero export.

## Build profiles

| Feature | What you get | Native deps |
|---|---|---|
| *(default)* | trellis, tokenizer, SI-SDR bench, sidecar, external (shell-out) backends | none |
| `onnx` | embedded wav2vec2 + HT-Demucs via ONNX Runtime — no external tools | ort (`libonnxruntime`, statically linked) |
| `cuda` | GPU acceleration via the CUDA execution provider (CPU fallback at runtime) | CUDA 12 + cuDNN 9 at runtime |
| `download` | `kf models pull` (pure-Rust TLS via ureq) | none |

Both ONNX models run windowed (HT-Demucs in 7.8 s segments, wav2vec2 in 30 s
windows with overlap-add), so a full-length song fits in GPU memory rather than
attempting one OOM-sized forward pass.

Model **weights are never in the binary** — they're pulled into a cache dir
(`$KEYFLOW_MODELS_DIR`, else `$XDG_CACHE_HOME/keyflow/models`) on demand.

## CLI

```sh
kf models list                          # registry + what's cached
kf models pull mms-fa                    # aligner    (needs --features download)
kf models pull htdemucs-vocals          # separator
kf sync song.wav chart.kf --separate -o timing.kf.json   # align (needs --features onnx)
kf sync song.wav chart.kf --lyrics-file lyrics.txt       # align a known transcript, not the chart
kf bench reference.wav estimate.wav --tool htdemucs      # SI-SDR/SDR, compare splitters
```

### Choosing an aligner model

| id | model | notes |
|----|-------|-------|
| `mms-fa` (default) | MMS-300M forced aligner | **Best for singing** — purpose-built aligner, 158 languages. ~3× higher confidence than wav2vec2 on sung vocals. Weights are **CC-BY-NC-4.0** (non-commercial); the keyflow code stays MIT/Apache since weights are downloaded, not bundled. |
| `w2v2-960h` | wav2vec2-base-960h | English speech model, permissive license. Lower confidence on singing. |

Rating cutoffs are model/material-dependent. `kf sync` defaults to `--ratings
singing` (verified ≥0.30, review ≥0.15); use `--ratings speech` for spoken word,
or override with `--verified-threshold` / `--review-threshold`.

### ASR comparison (`kf transcribe`)

Forced alignment needs known lyrics; `kf transcribe` runs free ASR (no
transcript) as an independent baseline — useful to see what's *actually* sung
(structure, repeats, instrumental sections) and to sanity-check alignment.

```sh
kf transcribe song.wav --separate --engine ctc       # wav2vec2 greedy CTC (fast)
kf transcribe song.wav --separate --engine whisper    # Whisper via candle (--features whisper)
```

- **`ctc`** — greedy-decodes the CTC emissions already used for alignment. Free,
  but **produces gibberish on singing** (a speech model with no language model).
  Its failure is the point: it's *why* constrained forced alignment beats free
  ASR on sung vocals.
- **`whisper`** — pure-Rust [candle] Whisper (`whisper-base.en`, auto-downloaded).
  Its language model recovers real words from singing and reveals the recording's
  true structure. Production decoding:
  - **Temperature fallback** — greedy (T=0), retried at higher temperatures with
    sampling if the output is too repetitive (gzip ratio) or low-probability,
    so repetitive sung vamps decode cleanly instead of looping.
  - **No-speech gate** — windows where `<|nospeech|>` is likely and confidence
    low are skipped.
  - **Energy VAD** — on the separated vocal stem, near-silent (instrumental)
    windows are skipped before decoding, which kills most hallucination over
    instrumental sections.
  - **Word-level timestamps** via `--align` (WhisperX-style): the recovered
    transcript is forced-aligned with the CTC aligner (`mms-fa`) for precise
    per-word times — more accurate than Whisper's native DTW, and it reuses the
    alignment stack. Needs the `onnx` feature.

  Build GPU Whisper with `whisper-cuda` (candle CUDA backend; needs nvcc + the
  CUDA toolkit at build time). Residual hallucinations at instrumental edges are
  inherent to Whisper-on-music and get flagged low-confidence by the rating gate.

```sh
# Production: Whisper transcript → word-level forced alignment, GPU separation
kf transcribe song.wav --separate --engine whisper --align -o words.json
```

On "Build My Life", Whisper independently confirmed the diagnosis below: lyrics
through ~120 s, then ~110 s of instrumental — which is exactly why forced
alignment (correctly) rated the back half low-confidence.

### Robustness: the `<star>` token and transcript matching

A CTC forced aligner maps *every* transcript word onto the audio in one
monotonic sweep, so it assumes the transcript matches what's sung. Real
recordings break that: instrumental intros/interludes, ad-libs, and — for live
worship especially — **sections repeated more times than the lyrics sheet
lists**. `kf sync` enables a `<star>` token by default (`--star-score`, disable
with `--no-star`) that absorbs audio matching no lyric token, so instrumental
fills and ad-libs don't drag neighbouring words off.

Star handles *extra audio between transcript words*; it can't invent repeats the
transcript is missing. If a recording sings the bridge 4× but the transcript
lists it once, the back half won't align well — and the rating system correctly
flags those words low-confidence rather than placing them wrongly with false
confidence. Two ways to handle repeat-heavy recordings:

- Make the transcript match the arrangement (list the repeats), or
- `--per-section` (experimental): align each chart section / blank-line-separated
  stanza independently against the remaining audio. Currently weaker than the
  global pass for complete transcripts; useful for partial ones.

Measured on "Build My Life": a transcript covering only the sung-once sections
aligns at **72% verified / 6% failed** against the full 4-min audio (star
absorbs the rest), vs 45% verified when the full transcript over-/under-counts
the live repeats. The aligner is rarely the bottleneck — transcript-vs-recording
structure is.

Real numbers — "Build My Life" (4:23), MMS-FA + Demucs separation, RTX 4080,
49 s end-to-end: **138 verified / 75 review / 92 failed** of 305 words, mean
confidence 0.31 (vs 0.11 / 0 verified with wav2vec2 on the same input).

### GPU (CUDA)

Build with `--features onnx-cuda` (on the CLI) / `cuda` (on the crate). ort
downloads a CUDA-enabled onnxruntime and copies the provider dylibs next to the
binary. At runtime the CUDA EP needs CUDA 12, cuDNN 9, and `libstdc++` reachable
via `LD_LIBRARY_PATH`; it falls back to CPU if they're missing. Set
`KEYFLOW_REQUIRE_CUDA=1` to turn a failed CUDA init into a hard error instead of
a silent CPU fallback (useful for diagnosing library-path problems).

On NixOS, point `LD_LIBRARY_PATH` at the cudatoolkit, cuDNN, gcc, and driver
store paths, e.g.:

```sh
export LD_LIBRARY_PATH="$(nix eval --impure --raw nixpkgs#cudaPackages.cudatoolkit)/lib:\
$(nix eval --impure --raw nixpkgs#cudaPackages.cudnn.lib)/lib:\
$(nix eval --impure --raw nixpkgs#stdenv.cc.cc.lib)/lib:/run/opengl-driver/lib"
```

Reference: a 4:23 song (separation + alignment) runs in ~48 s on an RTX 4080,
pinning the GPU at ~100% during separation.

Compressed audio: transcode to WAV first (`ffmpeg -i in.mp3 -ar 44100 in.wav`) —
feeding lossy sources to a separator/aligner is the #1 cause of metallic artifacts.

## Benchmarking separators

`kf bench` reports **SI-SDR** (scale-invariant, the modern standard) and a
gain-sensitive SNR-style **SDR**. These are *not* `mir_eval.bss_eval` and aren't
comparable to published `bss_eval` figures — but they're directly comparable
*between tools* on the same reference, which is the point of a bake-off. Wire any
new `StemSeparator` and score its vocal stem against a MUSDB18 / MedleyDB
reference.

## Two integration seams (when enabling `onnx`)

Both are clearly marked in-code; everything else is done and tested.

1. **Model URLs** — `models.rs` ships a registry with `REPLACE_ME` placeholder
   URLs. Point them at your ONNX exports (or drop a `models.json` in the cache
   dir to override without recompiling).
2. **Tensor names** — `align/wav2vec2.rs` and `separator/demucs.rs` assume the
   standard export I/O names (`input_values`→`logits`; `mix`→`stems`). Adjust the
   `IN_NAME`/`OUT_NAME`/`SOURCE_ORDER` consts if your export differs. A sibling
   `<model>.vocab.json` (JSON array of labels) overrides the default wav2vec2
   English label set.
```
