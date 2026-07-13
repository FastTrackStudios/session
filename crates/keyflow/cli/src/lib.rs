//! keyflow-cli — the `kf` command-line surface as a library.
//!
//! Former `apps/keyflow-cli` binary crate, kept as a crates/keyflow/cli
//! subcrate (its clap tree + render pipeline are too large to inline into
//! the `keyflow` facade cleanly). Embedders call [`cli_main`] with
//! pre-split argv: the thin `kf` binary (src/main.rs) and the unified
//! `fts keyflow` / `fts kf` subcommand mount the same surface.

use std::io::Read;
use std::path::PathBuf;

mod docs;
mod orchestra;
mod sync;

use clap::{Parser, Subcommand, ValueEnum};
use keyflow::Chart;
use keyflow::engraver::export::pdf::PdfSerializer;
use keyflow::engraver::export::svg::{SvgExportConfig, SvgSerializer};
use keyflow::engraver::fonts::ChartFontBundle;
use keyflow::engraver::layout::chart::{
    Breakpoint, ChartLayoutConfig, ChartLayoutEngine, ChartLayoutResult, LayoutMode,
};
use keyflow::engraver::style::MStyle;

/// Layout preset choice for the `png` / `svg` subcommands.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum PresetMode {
    /// A4 paginated, Master Rhythm preset.
    Page,
    /// Content-sized, minimal margins.
    Snippet,
    /// iReal Pro-style breakpoint-driven layout.
    Responsive,
}

/// Breakpoint choice for `--mode responsive`.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum BreakpointArg {
    Phone,
    Tablet,
    Desktop,
}

impl BreakpointArg {
    fn to_engraver(self) -> Breakpoint {
        match self {
            Self::Phone => Breakpoint::Phone,
            Self::Tablet => Breakpoint::Tablet,
            Self::Desktop => Breakpoint::Desktop,
        }
    }

    /// Default viewport width (points) for this breakpoint.
    /// Picked roughly in the middle of each breakpoint's range so the
    /// rendered output looks representative without the caller passing --width.
    fn default_width_pt(self) -> f64 {
        match self {
            Self::Phone => 375.0, // typical phone CSS width / DPI_SCALE
            Self::Tablet => 720.0,
            Self::Desktop => 1280.0,
        }
    }
}

#[derive(Parser)]
#[command(
    name = "kf",
    about = "Keyflow chart CLI — parse, inspect, and render charts"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Parse chart text and show the full structure
    Parse {
        /// Path to a .kf file, or "-" for stdin
        #[arg(default_value = "-")]
        input: String,
    },
    /// Render chart to PDF
    Pdf {
        /// Path to a .kf file, or "-" for stdin
        #[arg(default_value = "-")]
        input: String,
        /// Output PDF path
        #[arg(short, long, default_value = "chart.pdf")]
        output: PathBuf,
    },
    /// Import MIDI and render the generated chart to PDF
    MidiPdf {
        /// Path to a MIDI file, or "-" for stdin
        #[arg(default_value = "-")]
        input: String,
        /// Output PDF path
        #[arg(short, long, default_value = "chart.pdf")]
        output: PathBuf,
        /// Optional path to also write the generated .kf chart text
        #[arg(long)]
        chart_output: Option<PathBuf>,
    },
    /// Render chart to SVG (one file per page)
    Svg {
        /// Path to a .kf file, or "-" for stdin
        #[arg(default_value = "-")]
        input: String,
        /// Output SVG path (page number appended for multi-page)
        #[arg(short, long, default_value = "chart.svg")]
        output: PathBuf,
    },
    /// Render chart to SVG and print to stdout (for quick inspection)
    Render {
        /// Path to a .kf file, or "-" for stdin
        #[arg(default_value = "-")]
        input: String,
    },
    /// Render a directory of charts into a gallery (PNG per mode + index.md).
    Gallery {
        /// Directory containing .kf source files (recursively scanned, top level only).
        #[arg(short, long, default_value = "examples")]
        input_dir: PathBuf,
        /// Output directory for the rendered gallery.
        #[arg(short, long, default_value = "examples/gallery")]
        output_dir: PathBuf,
        /// Bitmap density factor (1.0 = 1 px per pt).
        #[arg(long, default_value_t = 1.5)]
        scale: f32,
    },
    /// Import a MusicXML (.musicxml / .mxl) file and render the converted chart.
    Musicxml {
        /// Path to a .musicxml or .mxl file.
        input: PathBuf,
        /// Output directory for the rendered chart (one PNG per variant).
        #[arg(short, long, default_value = "musicxml-out")]
        output_dir: PathBuf,
        /// Bitmap density factor (1.0 = 1 px per pt).
        #[arg(long, default_value_t = 2.0)]
        scale: f32,
    },
    /// Import a MusicXML (.musicxml / .mxl) file and emit Keyflow text.
    ///
    /// Prints the converted `.kf` to stdout by default so it can be piped or
    /// captured in tests. Pass `--output` to write a file instead. The default
    /// never touches the input-adjacent `.kf`, so a curated chart sitting next
    /// to its source is not clobbered.
    MusicxmlKf {
        /// Path to a .musicxml or .mxl file.
        input: PathBuf,
        /// Write the Keyflow text to this `.kf` path instead of stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Compare a MusicXML import against a .kf parse after both become Chart objects.
    MusicxmlCompare {
        /// Path to the source .musicxml or .mxl file.
        musicxml: PathBuf,
        /// Path to the hand-authored or exported .kf file.
        keyflow: PathBuf,
        /// Maximum number of differences to print.
        #[arg(long, default_value_t = 50)]
        max_diffs: usize,
        /// Also compare source MusicXML measure numbers and widths.
        #[arg(long)]
        include_source: bool,
    },
    /// Import every `*.musicxml` in a directory and render each into its own
    /// subfolder under `output-dir`. Mirrors `gallery` but for MusicXML input.
    MusicxmlGallery {
        /// Directory containing `*.musicxml` files (top level only).
        #[arg(short, long, default_value = "examples/png-project-charts")]
        input_dir: PathBuf,
        /// Output directory.
        #[arg(short, long, default_value = "examples/png-project-charts/rendered")]
        output_dir: PathBuf,
        /// Bitmap density factor (1.0 = 1 px per pt).
        #[arg(long, default_value_t = 1.5)]
        scale: f32,
    },
    /// Compare MusicXML measure width hints against generated measure spacing.
    MusicxmlSpacingCompare {
        /// Path to a .musicxml or .mxl file.
        input: PathBuf,
        /// Only print rows whose normalized relative error is at or above this fraction.
        #[arg(long, default_value_t = 0.25)]
        tolerance: f64,
    },
    /// Render chart to PNG (offline; uses resvg). Useful for visual review.
    Png {
        /// Path to a .kf file, or "-" for stdin
        #[arg(default_value = "-")]
        input: String,
        /// Output PNG path (page suffix added for multi-page).
        #[arg(short, long, default_value = "chart.png")]
        output: PathBuf,
        /// Layout preset.
        #[arg(short, long, value_enum, default_value_t = PresetMode::Page)]
        mode: PresetMode,
        /// Responsive breakpoint (only used when --mode responsive).
        #[arg(short, long, value_enum, default_value_t = BreakpointArg::Desktop)]
        breakpoint: BreakpointArg,
        /// Viewport width (points). Overrides the breakpoint's default.
        /// Only meaningful for --mode snippet / responsive.
        #[arg(short, long)]
        width: Option<f64>,
        /// Scale factor for the output bitmap (1.0 = 1px per pt at 72 DPI).
        /// 2.0 gives a Retina-resolution PNG.
        #[arg(long, default_value_t = 2.0)]
        scale: f32,
    },
    /// Preprocess a markdown docs tree: render ```` ```kf ```` blocks to inline
    /// SVG, writing a generated mirror tree a stock dodeca (`ddc`) build serves.
    ///
    /// dodeca is left untouched — authors keep writing ```` ```kf ```` in the
    /// source; point `ddc`'s `content` at `--out`. Run before `ddc build`, or
    /// with `--watch` alongside `ddc serve`.
    Docs {
        /// Source content directory (authored markdown).
        #[arg(short, long, default_value = "docs/content")]
        input: PathBuf,
        /// Generated output directory (what dodeca builds). Rebuilt each run.
        #[arg(short, long, default_value = "docs/.rendered")]
        output: PathBuf,
        /// Re-render whenever the source tree changes (dev loop).
        #[arg(long)]
        watch: bool,
    },
    /// Align a chart's lyrics to an audio file, writing a timing-map sidecar.
    ///
    /// Produces per-word start/end times with a confidence rating, keyed back to
    /// the chart by (section, word). The `.kf` is never modified — the sidecar is
    /// the single source of *when*. Needs `--features onnx` + `kf models pull`.
    Sync {
        /// Path to the audio file (WAV; transcode others with ffmpeg first).
        audio: PathBuf,
        /// Path to the `.kf` chart whose lyrics to align.
        chart: String,
        /// Output sidecar path (JSON).
        #[arg(short, long, default_value = "timing.kf.json")]
        output: PathBuf,
        /// Isolate vocals with Demucs before aligning (needs `kf models pull htdemucs`).
        #[arg(long)]
        separate: bool,
        /// Aligner model id from the registry.
        #[arg(long, default_value = "mms-fa")]
        model: String,
        /// Align against this plain-text lyrics file instead of the chart's lyrics.
        #[arg(long)]
        lyrics_file: Option<PathBuf>,
        /// Rating-threshold preset. `singing` uses lower cutoffs than `speech`
        /// because CTC confidence is inherently lower on sung vocals.
        #[arg(long, value_enum, default_value_t = RatingsPreset::Singing)]
        ratings: RatingsPreset,
        /// Override the "verified" confidence cutoff (0.0–1.0).
        #[arg(long)]
        verified_threshold: Option<f32>,
        /// Override the "review" confidence cutoff (0.0–1.0).
        #[arg(long)]
        review_threshold: Option<f32>,
        /// `<star>` token log-prob — absorbs non-transcript audio (ad-libs,
        /// instrumental, repeats) so mismatches don't drag words off. Lower is
        /// stronger absorption; ~ -2.5 is a good start.
        #[arg(long, default_value_t = -2.5)]
        star_score: f32,
        /// Disable the `<star>` token (align every frame to a lyric token).
        #[arg(long)]
        no_star: bool,
        /// Align each section independently against the remaining audio
        /// (anchored). Robust to recordings that repeat sections more than the
        /// lyrics list (live worship, jam outros). Sections come from the chart,
        /// or from blank-line-separated stanzas in --lyrics-file.
        #[arg(long)]
        per_section: bool,
    },
    /// Align a lane-format chart's lyrics to audio and emit karaoke artifacts:
    /// the sync lane (written back into the .kf), a lyrics MIDI (lyric meta
    /// events), an LRC, and optionally a Reaper project (audio + lyrics MIDI).
    Karaoke {
        /// Path to the lane-format `.kf` chart (with a `lyrics {}` lane).
        chart: PathBuf,
        /// Path to the audio file (WAV).
        audio: PathBuf,
        /// Output directory for the derived artifacts.
        #[arg(short, long, default_value = "karaoke-out")]
        out_dir: PathBuf,
        /// Isolate vocals with Demucs before aligning (recommended).
        #[arg(long)]
        separate: bool,
        /// Also export a Reaper `.rpp` (needs the `reaper` feature).
        #[arg(long)]
        reaper: bool,
    },
    /// Emit a multi-voice melody+lyrics MIDI from a chart's `voice {}` lanes —
    /// one track/MIDI-channel per colored vocal part. Notated timing; no audio.
    Voices {
        /// Path to the `.kf` chart with `voice {}` lanes.
        chart: PathBuf,
        /// Output directory.
        #[arg(short, long, default_value = "voices-out")]
        out_dir: PathBuf,
        /// Also export a colored per-voice Reaper project (needs `reaper` feature).
        #[arg(long)]
        reaper: bool,
    },
    /// Transcribe audio (CTC greedy ASR, no transcript) — an independent
    /// baseline to compare against `kf sync`. Reveals what's actually sung
    /// (extra repeats, ad-libs) and gives comparable word timings.
    Transcribe {
        /// Path to the audio file (WAV).
        audio: PathBuf,
        /// Write word timings to this sidecar (same format as `kf sync`).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Isolate vocals with Demucs first (recommended).
        #[arg(long)]
        separate: bool,
        /// ASR model id for the CTC engine — must have a word-separator token.
        #[arg(long, default_value = "w2v2-960h")]
        model: String,
        /// ASR engine: `ctc` (wav2vec2 greedy, fast, poor on singing) or
        /// `whisper` (candle, LM-backed, far better on singing).
        #[arg(long, value_enum, default_value_t = AsrEngine::Ctc)]
        engine: AsrEngine,
        /// (Whisper) Forced-align the transcript for precise word-level
        /// timestamps (WhisperX-style; needs the onnx feature + `mms-fa`).
        #[arg(long)]
        align: bool,
    },
    /// Recover a keyflow section arrangement from a song's Click + Guide
    /// (spoken-cue) tracks: detect tempo/bars from the click, transcribe the
    /// spoken section names, and emit chart text. Needs `--features onnx,whisper`.
    GuideChart {
        /// Path to the Click-track WAV (steady metronome).
        #[arg(long)]
        click: PathBuf,
        /// Path to the Guide/Cue-track WAV (spoken section names).
        #[arg(long)]
        guide: PathBuf,
        /// Assumed beats per bar (meter numerator).
        #[arg(long, default_value_t = 4)]
        beats_per_bar: u32,
        /// Drop recognized cues below this confidence (hallucination gate).
        #[arg(long, default_value_t = 0.15)]
        min_confidence: f32,
        /// Optional header line to prepend (e.g. "Song\n#A 127bpm 4/4").
        #[arg(long)]
        header: Option<String>,
        /// Write the recovered keyflow text here.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Score a separated stem against a reference (SI-SDR / SDR), for comparing
    /// stem-splitter tools head-to-head.
    Bench {
        /// Ground-truth reference stem (WAV).
        reference: PathBuf,
        /// Estimated stem to score (WAV).
        estimate: PathBuf,
        /// Label for the tool under test (shown in the table).
        #[arg(long, default_value = "tool")]
        tool: String,
    },
    /// Manage downloadable alignment / separation models.
    Models {
        #[command(subcommand)]
        action: ModelsAction,
    },
    /// Analyze a MusicXML/MXL score: instrumentation, articulations, ranges.
    Score {
        /// Path to a .musicxml or .mxl file.
        input: PathBuf,
    },
    /// Run the CSS orchestrator engine on a score (articulated MIDI summary).
    Orchestrate {
        /// Path to a .musicxml or .mxl file.
        input: PathBuf,
        /// Only process parts whose name contains this substring.
        #[arg(long)]
        part: Option<String>,
        /// Force an instrument profile (strings, woodwinds, brass,
        /// brass_trumpet, generic, harp) instead of name detection.
        #[arg(long)]
        profile: Option<String>,
    },
}

/// ASR engine for `kf transcribe`.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum AsrEngine {
    /// wav2vec2 greedy CTC decode (fast, but poor on singing).
    Ctc,
    /// Whisper via candle (LM-backed; far better on singing).
    Whisper,
}

/// Rating-threshold preset for `kf sync`.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum RatingsPreset {
    /// Cutoffs tuned for read speech (verified ≥0.70, review ≥0.40).
    Speech,
    /// Cutoffs tuned for sung vocals (verified ≥0.30, review ≥0.15).
    Singing,
}

#[derive(Subcommand)]
enum ModelsAction {
    /// List registry models and whether each is cached locally.
    List,
    /// Download a model into the cache (needs `--features download`).
    Pull {
        /// Model id (see `kf models list`).
        id: String,
    },
}

fn read_source(input: &str) -> Result<String, String> {
    if input == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| format!("Failed to read stdin: {e}"))?;
        Ok(buf)
    } else {
        std::fs::read_to_string(input).map_err(|e| format!("Failed to read {input}: {e}"))
    }
}

fn read_bytes(input: &str) -> Result<Vec<u8>, String> {
    if input == "-" {
        let mut buf = Vec::new();
        std::io::stdin()
            .read_to_end(&mut buf)
            .map_err(|e| format!("Failed to read stdin: {e}"))?;
        Ok(buf)
    } else {
        std::fs::read(input).map_err(|e| format!("Failed to read {input}: {e}"))
    }
}

fn parse_chart(source: &str) -> Result<Chart, String> {
    keyflow::parse(source).map_err(|e| format!("{e}"))
}

fn generate_midi_chart_text(input: &str) -> Result<String, String> {
    let bytes = read_bytes(input)?;
    keyflow::midi::generate_chart_text_from_midi_bytes(&bytes)
}

struct ChartCompareReport {
    total_diffs: usize,
    diffs: Vec<String>,
    truncated: bool,
}

fn compare_charts(
    left: &Chart,
    right: &Chart,
    include_source: bool,
    max_diffs: usize,
) -> ChartCompareReport {
    let mut report = ChartCompareReport {
        total_diffs: 0,
        diffs: Vec::new(),
        truncated: false,
    };

    push_diff_if(
        &mut report,
        max_diffs,
        left.metadata.title != right.metadata.title,
        format!(
            "metadata.title: musicxml={:?}, keyflow={:?}",
            left.metadata.title, right.metadata.title
        ),
    );
    push_diff_if(
        &mut report,
        max_diffs,
        left.tempo != right.tempo,
        format!(
            "tempo: musicxml={:?}, keyflow={:?}",
            left.tempo, right.tempo
        ),
    );
    push_diff_if(
        &mut report,
        max_diffs,
        left.initial_key != right.initial_key,
        format!(
            "initial_key: musicxml={:?}, keyflow={:?}",
            left.initial_key, right.initial_key
        ),
    );
    push_diff_if(
        &mut report,
        max_diffs,
        left.initial_time_signature != right.initial_time_signature,
        format!(
            "initial_time_signature: musicxml={:?}, keyflow={:?}",
            left.initial_time_signature, right.initial_time_signature
        ),
    );
    let left_measures = flatten_measures_expanding_repeats(left);
    let right_measures = flatten_measures(right);
    push_diff_if(
        &mut report,
        max_diffs,
        left_measures.len() != right_measures.len(),
        format!(
            "global measure count: musicxml={}, keyflow={}",
            left_measures.len(),
            right_measures.len()
        ),
    );

    for (left_idx, left_measure) in left_measures.iter().enumerate() {
        let measure_number = left_measure
            .source_measure_number
            .map(|n| n as usize)
            .unwrap_or(left_idx + 1);
        let Some(right_measure) = right_measures.get(left_idx) else {
            push_diff_if(
                &mut report,
                max_diffs,
                true,
                format!(
                    "expanded measure {} source measure {measure_number}: missing keyflow measure",
                    left_idx + 1
                ),
            );
            continue;
        };

        let left_sig = measure_signature(left_measure, include_source);
        let right_sig = measure_signature(right_measure, include_source);
        push_diff_if(
            &mut report,
            max_diffs,
            left_sig != right_sig,
            format!(
                "expanded measure {} source measure {measure_number}: musicxml={left_sig}, keyflow={right_sig}",
                left_idx + 1
            ),
        );
    }

    if report.total_diffs > report.diffs.len() {
        report.truncated = true;
    }
    report
}

fn flatten_measures(chart: &Chart) -> Vec<&keyflow::chart::types::Measure> {
    chart
        .sections
        .iter()
        .flat_map(|section| section.measures())
        .collect()
}

fn flatten_measures_expanding_repeats(chart: &Chart) -> Vec<&keyflow::chart::types::Measure> {
    let measures = flatten_measures(chart);
    let mut expanded = Vec::new();
    let mut repeat_start = 0usize;

    for (idx, measure) in measures.iter().enumerate() {
        if matches!(
            measure.start_repeat,
            keyflow::chart::notations::RepeatMark::Forward
        ) {
            repeat_start = idx;
        }

        expanded.push(*measure);

        if matches!(
            measure.end_repeat,
            keyflow::chart::notations::RepeatMark::Backward
        ) {
            let repeat_end = first_ending_start(&measures, repeat_start, idx).unwrap_or(idx + 1);
            for repeated in &measures[repeat_start..repeat_end] {
                expanded.push(*repeated);
            }
            repeat_start = idx + 1;
        }
    }

    expanded
}

fn first_ending_start(
    measures: &[&keyflow::chart::types::Measure],
    repeat_start: usize,
    repeat_end: usize,
) -> Option<usize> {
    measures[repeat_start..=repeat_end]
        .iter()
        .position(|measure| {
            measure
                .volta_start
                .as_ref()
                .map(|volta| volta.numbers.contains(&1))
                .unwrap_or(false)
        })
        .map(|offset| repeat_start + offset)
}

fn push_diff_if(report: &mut ChartCompareReport, max_diffs: usize, condition: bool, diff: String) {
    if !condition {
        return;
    }
    report.total_diffs += 1;
    if report.diffs.len() < max_diffs {
        report.diffs.push(diff);
    }
}

/// Canonical, encoding-independent string for one melody note.
///
/// The MusicXML importer and the `.kf` parser populate bookkeeping fields
/// (`position`, `scale_degree`, `octave_modifier`) differently for the same
/// music, so the signature compares only the audible content: pitch, resolved
/// octave, duration, articulation, and stacked pitches.
///
/// Tie markers are deliberately omitted: a sustain is implied by the repeated
/// pitch and duration, and the two importers disagree on whether interior
/// notes of a tie chain carry `tie_start`/`tie_stop` — comparing the flags
/// produces false diffs without adding musical information.
fn melody_note_signature(note: &keyflow::chart::melody::MelodyNote) -> String {
    let mut extras = note
        .extra_pitches
        .iter()
        .map(|(pitch, octave)| format!("{pitch}{octave:?}"))
        .collect::<Vec<_>>();
    extras.sort();
    format!(
        "{}{:?}/{}{}{}{}[{}]",
        note.pitch,
        note.octave,
        note.duration,
        if note.dotted { "." } else { "" },
        if note.triplet { "t" } else { "" },
        if note.is_rest() { "R" } else { "" },
        extras.join(","),
    )
}

/// A melody that carries no audible content — every note is a rest or an
/// invisible space. The MusicXML importer writes an empty bar as a
/// whole-measure rest while the `.kf` parser may emit a space (`s`) or nothing;
/// these all mean silence, so a silent melody is dropped from the signature.
fn melody_is_audible(melody: &keyflow::chart::melody::Melody) -> bool {
    melody
        .notes
        .iter()
        .any(|note| !note.is_rest() && !note.is_space())
}

fn measure_signature(measure: &keyflow::chart::types::Measure, include_source: bool) -> String {
    // Chords: compare the musical symbol sequence (plus any articulation
    // commands), not the rhythm-storage enum or absolute position. The `.kf`
    // parser encodes a whole-measure chord as `Default` while the MusicXML
    // importer encodes the same thing as `Slashes { count: 2 }`; both mean the
    // chord sounds for the whole measure, so the encoding must not diff.
    let chords = measure
        .chords
        .iter()
        .filter(|chord| chord.full_symbol != "s")
        .map(|chord| {
            if chord.commands.is_empty() {
                chord.full_symbol.clone()
            } else {
                format!("{}{:?}", chord.full_symbol, chord.commands)
            }
        })
        .collect::<Vec<_>>();
    // Melody: only audible voices, normalized per note. Silent (rest/space)
    // melodies are dropped so an empty MusicXML bar and a `.kf` `s`/rest bar
    // compare equal.
    let melodies = measure
        .melodies
        .iter()
        .filter(|melody| melody_is_audible(melody))
        .map(|melody| {
            melody
                .notes
                .iter()
                .map(melody_note_signature)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>();
    let source = if include_source {
        format!(
            ", source={:?}, width={:?}",
            measure.source_measure_number, measure.source_measure_width
        )
    } else {
        String::new()
    };

    // Musical-equivalence signature. Free staff text, instrument cues, and
    // figured bass are intentionally excluded: they are engraving annotations
    // that the MusicXML importer and the `.kf` parser format differently
    // (merged `<words>`, dropped accidentals, case/`*` variations), so they are
    // not part of a hard chords-and-notes equivalence check. The `rhythm`
    // element list is excluded too — it only echoes the chords (compared
    // above) plus silence markers.
    format!(
        "ts={:?}, repeat_count={}, start={:?}, end={:?}, volta={:?}, chords={:?}, dynamics={:?}, classical={:?}, hairpins={:?}, melodies={:?}{}",
        measure.time_signature,
        measure.repeat_count,
        measure.start_repeat,
        measure.end_repeat,
        measure.volta_start,
        chords,
        measure.dynamics,
        measure.classical_dynamics,
        measure.hairpins,
        melodies,
        source
    )
}

struct LayoutPipeline {
    font_bundle: ChartFontBundle,
    engine: ChartLayoutEngine,
}

impl LayoutPipeline {
    fn new() -> Result<Self, String> {
        let font_bundle = ChartFontBundle::new()?;
        let style: &'static MStyle = Box::leak(Box::new(MStyle::new()));
        let engine = font_bundle.create_layout_engine(style);
        Ok(Self {
            font_bundle,
            engine,
        })
    }

    fn layout(&self, chart: &Chart) -> ChartLayoutResult {
        let config = ChartLayoutConfig::master_rhythm().with_page_offsets(true);
        let mode = LayoutMode::paginated_a4();
        self.engine.layout_chart_with_config(chart, &mode, &config)
    }

    /// Layout with explicit preset + viewport for visual review.
    ///
    /// `width_pt` is ignored in Page mode (uses A4); meaningful for Snippet
    /// and Responsive where it sets the viewport width.
    fn layout_preset(
        &self,
        chart: &Chart,
        preset: PresetMode,
        breakpoint: BreakpointArg,
        width_pt: f64,
    ) -> ChartLayoutResult {
        let (mode, config) = match preset {
            PresetMode::Page => (
                LayoutMode::paginated_a4(),
                ChartLayoutConfig::master_rhythm().with_page_offsets(false),
            ),
            PresetMode::Snippet => (
                LayoutMode::snippet(width_pt),
                ChartLayoutConfig::snippet().with_page_offsets(false),
            ),
            PresetMode::Responsive => (
                LayoutMode::ContinuousScroll { width: width_pt },
                ChartLayoutConfig::responsive_for(breakpoint.to_engraver()),
            ),
        };
        self.engine.layout_chart_with_config(chart, &mode, &config)
    }

    /// Embed every named font the chart pipeline ever references into the SVG
    /// export config. Names must match the values used by HarmonyStyle /
    /// ChartFontBundle::configure_renderer so resvg can resolve them.
    fn with_embedded_fonts(&self, mut config: SvgExportConfig) -> SvgExportConfig {
        let leland = self.font_bundle.symbol_font_data().as_ref().clone();
        let leland_text = self.font_bundle.leland_text_font_data().as_ref().clone();
        let musejazz_text = self.font_bundle.text_font_data().as_ref().clone();
        let musejazz = self.font_bundle.musejazz_font_data().as_ref().clone();
        let chicago = self.font_bundle.chicago_font_data().as_ref().clone();
        let bravura = self.font_bundle.bravura_font_data().as_ref().clone();
        let freesans = self.font_bundle.freesans_font_data().as_ref().clone();

        config = config
            // SMuFL music font (Leland) — primary + legacy "Bravura" alias.
            .with_embedded_font("Leland", leland.clone())
            .with_embedded_font("Bravura", bravura)
            // Leland Text companion (alternate text/symbol font).
            .with_embedded_font("Leland Text", leland_text.clone())
            .with_embedded_font("LelandText", leland_text.clone())
            .with_embedded_font("Edwin", leland_text)
            // MuseJazz (music font) + MuseJazz Text (chord-symbol text font).
            .with_embedded_font("MuseJazz", musejazz)
            .with_embedded_font("MuseJazz Text", musejazz_text.clone())
            .with_embedded_font("MuseJazzText", musejazz_text)
            // Chicago — default document/title text.
            .with_embedded_font("Chicago", chicago.clone())
            .with_embedded_font("ChicagoFLF", chicago.clone())
            .with_embedded_font("FreeSans", freesans)
            .with_embedded_font("sans-serif", chicago);
        config
    }

    /// Inline-docs SVG: continuous-scroll, **content-cropped**, and **font-less**.
    ///
    /// Mirrors the editor's `render_svg_live`. The viewBox is shrink-wrapped to
    /// the drawn content (via `ChartLayoutResult::content_bounds`) so a one-system
    /// chart isn't marooned in a print page box, and fonts are *not* embedded —
    /// the SVG references families by name, resolved by [`Self::font_face_css`]
    /// injected once per page. Returns `None` for an empty / contentless chart.
    fn render_live_svg(&self, chart: &Chart) -> Option<String> {
        const PAD: f64 = 6.0;
        let mode = LayoutMode::ContinuousScroll { width: 800.0 };
        let config = ChartLayoutConfig::master_rhythm().with_page_offsets(true);
        let result = self.engine.layout_chart_with_config(chart, &mode, &config);
        let b = result.content_bounds()?;
        let config = SvgExportConfig::for_page(
            b.x0 - PAD,
            b.y0 - PAD,
            b.width() + 2.0 * PAD,
            b.height() + 2.0 * PAD,
        );
        let mut serializer = SvgSerializer::new(config);
        Some(serializer.serialize(&result.scene))
    }

    /// The `@font-face` rules for the engraving fonts, as standalone CSS. Inject
    /// once per page that has font-less charts (from [`Self::render_live_svg`]).
    fn font_face_css(&self) -> String {
        self.with_embedded_fonts(SvgExportConfig::default())
            .font_face_css()
    }

    /// SVG for ContinuousScroll layouts (single image, scene-sized).
    /// Returns `None` if the result has explicit pages — use [`export_svg_pages`] in that case.
    fn export_svg_continuous(&self, result: &ChartLayoutResult) -> Option<String> {
        if !result.pages.is_empty() {
            return None;
        }
        let config = self.with_embedded_fonts(SvgExportConfig::for_page(
            0.0,
            0.0,
            result.total_width,
            result.total_height,
        ));
        let mut serializer = SvgSerializer::new(config);
        Some(serializer.serialize(&result.scene))
    }

    fn export_svg_pages(&self, result: &ChartLayoutResult) -> Vec<String> {
        let mut pages = Vec::with_capacity(result.pages.len());
        for page in &result.pages {
            let config = self.with_embedded_fonts(SvgExportConfig::for_page(
                page.x_offset,
                page.y_offset,
                page.width,
                page.height,
            ));
            let mut serializer = SvgSerializer::new(config);
            pages.push(serializer.serialize(&result.scene));
        }
        pages
    }

    fn export_pdf(&self, result: &ChartLayoutResult) -> Result<Vec<u8>, String> {
        let svg_pages = self.export_svg_pages(result);
        // Register every font the SVG can reference, under the same names as
        // `with_embedded_fonts`, so svg2pdf's usvg resolves them exactly like
        // resvg does for the PNG export. Omitting any (previously Chicago, the
        // title font) made the PDF fall back to a system face and diverge.
        let leland = self.font_bundle.symbol_font_data();
        let leland_text = self.font_bundle.leland_text_font_data();
        let musejazz = self.font_bundle.text_font_data();
        let musejazz_music = self.font_bundle.musejazz_font_data();
        let chicago = self.font_bundle.chicago_font_data();
        let bravura = self.font_bundle.bravura_font_data();
        let freesans = self.font_bundle.freesans_font_data();

        PdfSerializer::serialize_from_svg(
            &svg_pages,
            &[
                ("Leland", leland.as_slice()),
                ("Bravura", bravura.as_slice()),
                ("Leland Text", leland_text.as_slice()),
                ("LelandText", leland_text.as_slice()),
                ("Edwin", leland_text.as_slice()),
                ("MuseJazz", musejazz_music.as_slice()),
                ("MuseJazz Text", musejazz.as_slice()),
                ("MuseJazzText", musejazz.as_slice()),
                ("Chicago", chicago.as_slice()),
                ("ChicagoFLF", chicago.as_slice()),
                ("FreeSans", freesans.as_slice()),
                ("sans-serif", chicago.as_slice()),
            ],
        )
        .map_err(|e| format!("Failed to export PDF: {e}"))
    }
}

/// HTML preamble for the gallery index page. Stylesheet inline so the
/// generated file works as a single drop-in artifact (no external assets).
const GALLERY_HTML_HEAD: &str = r#"<!DOCTYPE html>
<html lang="en"><head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width,initial-scale=1" />
<title>Keyflow chart gallery</title>
<style>
  :root { color-scheme: light dark; }
  body { font: 14px/1.5 -apple-system, system-ui, "Segoe UI", sans-serif;
         margin: 0; padding: 24px; max-width: 1400px; margin: 0 auto;
         background: #fafafa; color: #222; }
  @media (prefers-color-scheme: dark) {
    body { background: #1a1a1a; color: #ddd; }
    figure { background: #2a2a2a; }
    .src { color: #888; }
  }
  h1 { margin: 0 0 4px; font-size: 28px; }
  h2 { margin: 32px 0 4px; font-size: 20px; }
  .hint { color: #777; margin: 0 0 32px; }
  .src { color: #888; font-size: 12px; margin: 0 0 12px; }
  .chart { margin-bottom: 48px; padding-bottom: 24px; border-bottom: 1px solid #ddd; }
  @media (prefers-color-scheme: dark) { .chart { border-bottom-color: #333; } }
  .variants { display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
              gap: 16px; }
  figure { margin: 0; background: white; border-radius: 8px; padding: 12px;
           box-shadow: 0 1px 3px rgba(0,0,0,.1); display: flex; flex-direction: column;
           align-items: center; }
  figcaption { font-size: 12px; font-weight: 600; text-transform: uppercase;
               letter-spacing: 0.05em; color: #666; margin-bottom: 8px; }
  figure img { max-width: 100%; height: auto; border: 1px solid #eee; }
  @media (prefers-color-scheme: dark) {
    figure { background: #2a2a2a; }
    figure img { border-color: #444; }
    figcaption { color: #aaa; }
  }
  .failed .err { color: #c33; font-size: 12px; padding: 24px; text-align: center; }
  code { background: rgba(127,127,127,.15); padding: 2px 6px; border-radius: 3px; font-size: 12px; }
</style>
</head><body>
"#;

/// Render variants exposed by `kf gallery`. One row per chart in the index.
const GALLERY_VARIANTS: &[(&str, PresetMode, BreakpointArg, Option<f64>)] = &[
    ("page", PresetMode::Page, BreakpointArg::Desktop, None),
    ("phone", PresetMode::Responsive, BreakpointArg::Phone, None),
    (
        "tablet",
        PresetMode::Responsive,
        BreakpointArg::Tablet,
        None,
    ),
    (
        "desktop",
        PresetMode::Responsive,
        BreakpointArg::Desktop,
        None,
    ),
];

/// Render one chart to one variant; returns paths written.
fn render_variant_pngs(
    pipeline: &LayoutPipeline,
    chart: &Chart,
    preset: PresetMode,
    breakpoint: BreakpointArg,
    width_pt: f64,
    scale: f32,
    output_base: &std::path::Path,
) -> Result<Vec<PathBuf>, String> {
    let layout = pipeline.layout_preset(chart, preset, breakpoint, width_pt);
    let svgs: Vec<String> = if layout.pages.is_empty() {
        vec![
            pipeline
                .export_svg_continuous(&layout)
                .ok_or_else(|| "continuous layout produced no SVG".to_string())?,
        ]
    } else {
        pipeline.export_svg_pages(&layout)
    };

    let mut written = Vec::with_capacity(svgs.len());
    if svgs.len() == 1 {
        let path = output_base.with_extension("png");
        let png = svg_to_png(&svgs[0], scale, &pipeline.font_bundle)?;
        std::fs::write(&path, &png)
            .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
        written.push(path);
    } else {
        for (i, svg) in svgs.iter().enumerate() {
            let path = output_base.with_extension(format!("p{}.png", i + 1));
            let png = svg_to_png(svg, scale, &pipeline.font_bundle)?;
            std::fs::write(&path, &png)
                .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
            written.push(path);
        }
    }
    Ok(written)
}

/// Rasterize one SVG string to PNG bytes via resvg.
///
/// `scale` is the bitmap density factor (1.0 = 1 px per pt, 2.0 = Retina).
/// `font_bundle` registers the music fonts so chord/notation glyphs render —
/// usvg won't pick up @font-face base64 data on its own.
fn svg_to_png(svg: &str, scale: f32, font_bundle: &ChartFontBundle) -> Result<Vec<u8>, String> {
    let mut fontdb = usvg::fontdb::Database::new();
    // Every embedded font in the SVG must also live in the fontdb so usvg
    // can resolve `font-family` references. Without these, resvg silently
    // falls back to a system serif/sans.
    fontdb.load_font_data(font_bundle.symbol_font_data().as_ref().clone()); // Leland
    fontdb.load_font_data(font_bundle.text_font_data().as_ref().clone()); // MuseJazz Text
    fontdb.load_font_data(font_bundle.musejazz_font_data().as_ref().clone()); // MuseJazz
    fontdb.load_font_data(font_bundle.leland_text_font_data().as_ref().clone()); // Leland Text
    fontdb.load_font_data(font_bundle.chicago_font_data().as_ref().clone()); // ChicagoFLF
    fontdb.load_font_data(font_bundle.bravura_font_data().as_ref().clone());
    fontdb.load_font_data(font_bundle.freesans_font_data().as_ref().clone());
    fontdb.load_system_fonts();

    let opts = usvg::Options {
        fontdb: std::sync::Arc::new(fontdb),
        ..Default::default()
    };
    let tree = usvg::Tree::from_str(svg, &opts).map_err(|e| format!("usvg parse failed: {e}"))?;
    let size = tree.size();
    let pixmap_width = (size.width() * scale).ceil() as u32;
    let pixmap_height = (size.height() * scale).ceil() as u32;
    if pixmap_width == 0 || pixmap_height == 0 {
        return Err("rendered pixmap has zero size".to_string());
    }
    let mut pixmap = tiny_skia::Pixmap::new(pixmap_width, pixmap_height)
        .ok_or_else(|| format!("failed to allocate {pixmap_width}x{pixmap_height} pixmap"))?;
    // White background — engraver SVGs often have transparent areas outside the page.
    pixmap.fill(tiny_skia::Color::WHITE);
    let transform = tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    pixmap
        .encode_png()
        .map_err(|e| format!("PNG encode failed: {e}"))
}

/// Run the `kf` CLI from pre-split argv (element 0 is the program name).
///
/// Exits the process with status 1 on error (CLI semantics). Tracing/log
/// setup is the caller's job — the `kf` binary installs an env-filter
/// subscriber, `fts` installs its own.
pub fn cli_main<I, T>(args: I)
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = Cli::parse_from(args);

    if let Err(e) = run(cli) {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Commands::Parse { input } => {
            let source = read_source(&input)?;
            let chart = parse_chart(&source)?;

            // Print metadata
            println!("=== Chart Structure ===\n");
            println!(
                "Title:    {}",
                chart.metadata.title.as_deref().unwrap_or("(none)")
            );
            println!(
                "Artist:   {}",
                chart.metadata.artist.as_deref().unwrap_or("(none)")
            );
            if let Some(tempo) = &chart.tempo {
                println!("Tempo:    {}", tempo);
            }
            if let Some(ts) = &chart.initial_time_signature {
                println!("Time Sig: {}/{}", ts.numerator, ts.denominator);
            }
            if let Some(key) = &chart.initial_key {
                println!("Key:      {}", key);
            }
            println!();

            // Print round-tripped display format
            println!("=== Display (round-trip) ===\n");
            println!("{}", chart);

            // Print detailed section breakdown
            println!("\n=== Section Detail ===\n");
            for (i, section) in chart.sections.iter().enumerate() {
                let s = &section.section;
                let comment = s.comment.as_deref().unwrap_or("");
                let comment_str = if comment.is_empty() {
                    String::new()
                } else {
                    format!(" \"{}\"", comment)
                };
                println!(
                    "Section {}: {:?}{} ({} measures)",
                    i,
                    s.section_type,
                    comment_str,
                    section.measures().len()
                );
                for (j, measure) in section.measures().iter().enumerate() {
                    let chords: Vec<String> = measure
                        .chords
                        .iter()
                        .map(|c| c.full_symbol.clone())
                        .collect();
                    let melody_info = if measure.melodies.is_empty() {
                        String::new()
                    } else {
                        let total_notes: usize =
                            measure.melodies.iter().map(|m| m.notes.len()).sum();
                        format!(
                            " + {} melody ({} notes)",
                            measure.melodies.len(),
                            total_notes
                        )
                    };
                    println!("  Measure {}: [{}]{}", j, chords.join(", "), melody_info);
                }
                println!();
            }

            Ok(())
        }

        Commands::Pdf { input, output } => {
            let source = read_source(&input)?;
            let chart = parse_chart(&source)?;
            let pipeline = LayoutPipeline::new()?;
            // Use the same A4 paged layout the PNG export uses (Page preset,
            // page offsets off) so the PDF is page-for-page visually identical
            // to `kf png`. `layout()` arranges pages with offsets into one
            // continuous scene, which is for on-screen scrolling, not paper.
            let layout = pipeline.layout_preset(
                &chart,
                PresetMode::Page,
                BreakpointArg::Desktop,
                BreakpointArg::Desktop.default_width_pt(),
            );

            println!(
                "Layout: {} A4 page(s), {:.0}x{:.0} pt",
                layout.pages.len(),
                layout.pages.first().map_or(layout.total_width, |p| p.width),
                layout
                    .pages
                    .first()
                    .map_or(layout.total_height, |p| p.height),
            );

            let pdf_bytes = pipeline.export_pdf(&layout)?;
            std::fs::write(&output, &pdf_bytes)
                .map_err(|e| format!("Failed to write {}: {e}", output.display()))?;
            println!("Wrote {} ({} bytes)", output.display(), pdf_bytes.len());
            Ok(())
        }

        Commands::MidiPdf {
            input,
            output,
            chart_output,
        } => {
            let chart_text = generate_midi_chart_text(&input)?;
            if let Some(chart_output) = chart_output {
                std::fs::write(&chart_output, &chart_text)
                    .map_err(|e| format!("Failed to write {}: {e}", chart_output.display()))?;
                println!("Wrote {}", chart_output.display());
            }
            let chart = parse_chart(&chart_text)?;

            let pipeline = LayoutPipeline::new()?;
            let layout = pipeline.layout(&chart);

            println!(
                "Layout: {} page(s), {:.0}x{:.0} pt",
                layout.pages.len(),
                layout.total_width,
                layout.total_height
            );

            let pdf_bytes = pipeline.export_pdf(&layout)?;
            std::fs::write(&output, &pdf_bytes)
                .map_err(|e| format!("Failed to write {}: {e}", output.display()))?;
            println!("Wrote {} ({} bytes)", output.display(), pdf_bytes.len());
            Ok(())
        }

        Commands::Svg { input, output } => {
            let source = read_source(&input)?;
            let chart = parse_chart(&source)?;
            let pipeline = LayoutPipeline::new()?;
            let layout = pipeline.layout(&chart);

            let svg_pages = pipeline.export_svg_pages(&layout);

            if svg_pages.len() == 1 {
                std::fs::write(&output, &svg_pages[0])
                    .map_err(|e| format!("Failed to write {}: {e}", output.display()))?;
                println!("Wrote {}", output.display());
            } else {
                let stem = output
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("chart");
                let ext = output.extension().and_then(|s| s.to_str()).unwrap_or("svg");
                let dir = output.parent().unwrap_or(std::path::Path::new("."));

                for (i, svg) in svg_pages.iter().enumerate() {
                    let page_path = dir.join(format!("{}-p{}.{}", stem, i + 1, ext));
                    std::fs::write(&page_path, svg)
                        .map_err(|e| format!("Failed to write {}: {e}", page_path.display()))?;
                    println!("Wrote {}", page_path.display());
                }
            }
            Ok(())
        }

        Commands::Gallery {
            input_dir,
            output_dir,
            scale,
        } => {
            let mut entries: Vec<PathBuf> = std::fs::read_dir(&input_dir)
                .map_err(|e| format!("Failed to read {}: {e}", input_dir.display()))?
                .filter_map(|r| r.ok())
                .map(|d| d.path())
                .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("kf"))
                .collect();
            entries.sort();
            if entries.is_empty() {
                return Err(format!("No .kf files found in {}", input_dir.display()));
            }
            std::fs::create_dir_all(&output_dir)
                .map_err(|e| format!("Failed to create {}: {e}", output_dir.display()))?;

            let pipeline = LayoutPipeline::new()?;

            // Build index.md and index.html together.
            let mut md = String::from("# Keyflow chart gallery\n\n");
            md.push_str("Regenerate with `cargo run -p keyflow-cli -- gallery`.\n\n");

            let mut html = String::new();
            html.push_str(GALLERY_HTML_HEAD);
            html.push_str("<h1>Keyflow chart gallery</h1>\n");
            html.push_str("<p class=\"hint\">Regenerate with <code>cargo run -p keyflow-cli -- gallery</code>.</p>\n");

            for path in &entries {
                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .ok_or_else(|| format!("Bad filename: {}", path.display()))?;
                println!("→ {}", stem);

                let source = std::fs::read_to_string(path)
                    .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
                let chart = match parse_chart(&source) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("  ⚠ skipped ({stem}): parse error: {e}");
                        continue;
                    }
                };

                let chart_dir = output_dir.join(stem);
                std::fs::create_dir_all(&chart_dir)
                    .map_err(|e| format!("Failed to create {}: {e}", chart_dir.display()))?;

                let title = chart
                    .metadata
                    .title
                    .clone()
                    .unwrap_or_else(|| stem.to_string());

                md.push_str(&format!("## {title}\n\n"));
                md.push_str(&format!(
                    "Source: [`{}`](../{})\n\n",
                    path.display(),
                    path.display()
                ));
                md.push_str("| Variant | Preview |\n|---|---|\n");

                html.push_str(&format!("<section class=\"chart\">\n  <h2>{title}</h2>\n"));
                html.push_str(&format!(
                    "  <p class=\"src\">Source: <code>{}</code></p>\n",
                    path.display()
                ));
                html.push_str("  <div class=\"variants\">\n");

                for (variant_name, preset, breakpoint, width_override) in GALLERY_VARIANTS {
                    let width_pt = width_override.unwrap_or_else(|| breakpoint.default_width_pt());
                    let base = chart_dir.join(variant_name);
                    match render_variant_pngs(
                        &pipeline,
                        &chart,
                        *preset,
                        *breakpoint,
                        width_pt,
                        scale,
                        &base,
                    ) {
                        Ok(written) => {
                            for p in written {
                                let rel = p
                                    .strip_prefix(&output_dir)
                                    .unwrap_or(&p)
                                    .display()
                                    .to_string();
                                md.push_str(&format!(
                                    "| `{variant_name}` | <img src=\"{rel}\" width=\"320\" /> |\n"
                                ));
                                html.push_str(&format!(
                                    "    <figure class=\"variant variant-{variant_name}\"><figcaption>{variant_name}</figcaption><img src=\"{rel}\" alt=\"{title} {variant_name}\" loading=\"lazy\" /></figure>\n"
                                ));
                            }
                        }
                        Err(e) => {
                            eprintln!("  ⚠ {variant_name}: {e}");
                            md.push_str(&format!("| `{variant_name}` | ⚠ render failed |\n"));
                            html.push_str(&format!(
                                "    <figure class=\"variant variant-{variant_name} failed\"><figcaption>{variant_name}</figcaption><div class=\"err\">⚠ {e}</div></figure>\n"
                            ));
                        }
                    }
                }
                md.push('\n');
                html.push_str("  </div>\n</section>\n");
            }

            html.push_str("</body></html>\n");

            let md_path = output_dir.join("README.md");
            std::fs::write(&md_path, &md)
                .map_err(|e| format!("Failed to write {}: {e}", md_path.display()))?;
            println!("Wrote {}", md_path.display());

            let html_path = output_dir.join("index.html");
            std::fs::write(&html_path, &html)
                .map_err(|e| format!("Failed to write {}: {e}", html_path.display()))?;
            println!("Wrote {}", html_path.display());
            Ok(())
        }

        Commands::Musicxml {
            input,
            output_dir,
            scale,
        } => {
            let chart = keyflow_musicxml::import_file(&input)
                .map_err(|e| format!("musicxml import: {e}"))?;
            std::fs::create_dir_all(&output_dir)
                .map_err(|e| format!("Failed to create {}: {e}", output_dir.display()))?;
            let pipeline = LayoutPipeline::new()?;
            let stem = input
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("chart");
            // A4 page-mode only — matches the source PDF.
            let base = output_dir.join(stem);
            match render_variant_pngs(
                &pipeline,
                &chart,
                PresetMode::Page,
                BreakpointArg::Desktop,
                BreakpointArg::Desktop.default_width_pt(),
                scale,
                &base,
            ) {
                Ok(written) => {
                    for p in written {
                        println!("Wrote {}", p.display());
                    }
                }
                Err(e) => eprintln!("  ⚠ {e}"),
            }
            Ok(())
        }

        Commands::MusicxmlKf { input, output } => {
            let chart = keyflow_musicxml::import_file(&input)
                .map_err(|e| format!("musicxml import: {e}"))?;
            let keyflow_text = keyflow::text::chart::exporter::chart_to_keyflow(&chart);
            match output {
                Some(output) => {
                    if let Some(parent) = output.parent() {
                        if !parent.as_os_str().is_empty() {
                            std::fs::create_dir_all(parent).map_err(|e| {
                                format!("Failed to create {}: {e}", parent.display())
                            })?;
                        }
                    }
                    std::fs::write(&output, keyflow_text)
                        .map_err(|e| format!("Failed to write {}: {e}", output.display()))?;
                    eprintln!("Wrote {}", output.display());
                }
                None => {
                    print!("{keyflow_text}");
                }
            }
            Ok(())
        }

        Commands::MusicxmlCompare {
            musicxml,
            keyflow,
            max_diffs,
            include_source,
        } => {
            let musicxml_chart = keyflow_musicxml::import_file(&musicxml)
                .map_err(|e| format!("musicxml import: {e}"))?;
            let keyflow_source = std::fs::read_to_string(&keyflow)
                .map_err(|e| format!("Failed to read {}: {e}", keyflow.display()))?;
            let keyflow_chart = parse_chart(&keyflow_source)?;
            let report = compare_charts(&musicxml_chart, &keyflow_chart, include_source, max_diffs);

            println!("Compared MusicXML Chart to Keyflow Chart");
            println!("  MusicXML: {}", musicxml.display());
            println!("  Keyflow:  {}", keyflow.display());
            println!("  Differences: {}", report.total_diffs);
            for diff in &report.diffs {
                println!("- {diff}");
            }
            if report.truncated {
                println!("- ... more differences omitted; rerun with a higher --max-diffs");
            }

            if report.total_diffs == 0 {
                Ok(())
            } else {
                Err(format!(
                    "charts differ ({} differences found)",
                    report.total_diffs
                ))
            }
        }

        Commands::MusicxmlGallery {
            input_dir,
            output_dir,
            scale,
        } => {
            let mut entries: Vec<PathBuf> = std::fs::read_dir(&input_dir)
                .map_err(|e| format!("Failed to read {}: {e}", input_dir.display()))?
                .filter_map(|r| r.ok())
                .map(|d| d.path())
                .filter(|p| {
                    p.extension().and_then(|s| s.to_str()) == Some("musicxml")
                        || p.extension().and_then(|s| s.to_str()) == Some("mxl")
                })
                .collect();
            entries.sort();
            if entries.is_empty() {
                return Err(format!(
                    "No .musicxml / .mxl files found in {}",
                    input_dir.display()
                ));
            }
            std::fs::create_dir_all(&output_dir)
                .map_err(|e| format!("Failed to create {}: {e}", output_dir.display()))?;
            let pipeline = LayoutPipeline::new()?;

            let mut md = String::from("# MusicXML import gallery\n\n");
            md.push_str("Regenerate with `cargo run -p keyflow-cli -- musicxml-gallery`.\n\n");
            let mut html = String::new();
            html.push_str(GALLERY_HTML_HEAD);
            html.push_str("<h1>MusicXML import gallery</h1>\n");
            html.push_str("<p class=\"hint\">Regenerate with <code>cargo run -p keyflow-cli -- musicxml-gallery</code>.</p>\n");

            for path in &entries {
                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .ok_or_else(|| format!("Bad filename: {}", path.display()))?;
                println!("→ {}", stem);

                let chart = match keyflow_musicxml::import_file(path) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("  ⚠ {stem}: import error: {e}");
                        continue;
                    }
                };

                let chart_dir = output_dir.join(stem);
                std::fs::create_dir_all(&chart_dir)
                    .map_err(|e| format!("Failed to create {}: {e}", chart_dir.display()))?;

                let title = chart
                    .metadata
                    .title
                    .clone()
                    .unwrap_or_else(|| stem.to_string());

                md.push_str(&format!("## {title}\n\nSource: `{}`\n\n", path.display()));

                html.push_str(&format!("<section class=\"chart\">\n  <h2>{title}</h2>\n"));
                html.push_str(&format!(
                    "  <p class=\"src\">Source: <code>{}</code></p>\n",
                    path.display()
                ));
                html.push_str("  <div class=\"variants\">\n");

                // A4 page only — phone/tablet/desktop deferred until import is solid.
                {
                    let variant_name = "page";
                    let base = chart_dir.join(variant_name);
                    match render_variant_pngs(
                        &pipeline,
                        &chart,
                        PresetMode::Page,
                        BreakpointArg::Desktop,
                        BreakpointArg::Desktop.default_width_pt(),
                        scale,
                        &base,
                    ) {
                        Ok(written) => {
                            for p in written {
                                let rel = p
                                    .strip_prefix(&output_dir)
                                    .unwrap_or(&p)
                                    .display()
                                    .to_string();
                                md.push_str(&format!("![{rel}]({rel})\n\n"));
                                html.push_str(&format!(
                                    "    <figure class=\"variant variant-{variant_name}\"><figcaption>{variant_name}</figcaption><img src=\"{rel}\" alt=\"{title} {variant_name}\" loading=\"lazy\" /></figure>\n"
                                ));
                            }
                        }
                        Err(e) => {
                            eprintln!("  ⚠ {variant_name}: {e}");
                            md.push_str(&format!("| `{variant_name}` | ⚠ render failed |\n"));
                            html.push_str(&format!(
                                "    <figure class=\"variant variant-{variant_name} failed\"><figcaption>{variant_name}</figcaption><div class=\"err\">⚠ {e}</div></figure>\n"
                            ));
                        }
                    }
                }
                md.push('\n');
                html.push_str("  </div>\n</section>\n");
            }
            html.push_str("</body></html>\n");

            let md_path = output_dir.join("README.md");
            std::fs::write(&md_path, &md)
                .map_err(|e| format!("Failed to write {}: {e}", md_path.display()))?;
            println!("Wrote {}", md_path.display());

            let html_path = output_dir.join("index.html");
            std::fs::write(&html_path, &html)
                .map_err(|e| format!("Failed to write {}: {e}", html_path.display()))?;
            println!("Wrote {}", html_path.display());
            Ok(())
        }

        Commands::MusicxmlSpacingCompare { input, tolerance } => {
            let chart = keyflow_musicxml::import_file(&input)
                .map_err(|e| format!("musicxml import: {e}"))?;
            let pipeline = LayoutPipeline::new()?;
            let mode = LayoutMode::paginated_a4();
            let config = ChartLayoutConfig::master_rhythm().with_page_offsets(false);
            let report = pipeline
                .engine
                .compare_musicxml_widths(&chart, &mode, &config);

            println!("MusicXML spacing comparison: {}", input.display());
            println!("Compared measures: {}", report.compared);
            if let Some(err) = report.median_abs_error {
                println!("Median abs error: {:.1}%", err * 100.0);
            }
            if let Some(err) = report.p90_abs_error {
                println!("P90 abs error: {:.1}%", err * 100.0);
            }
            if let Some(err) = report.max_abs_error {
                println!("Max abs error: {:.1}%", err * 100.0);
            }
            println!();
            println!(
                "{:>4} {:>3} {:>3} {:>7} {:>8} {:>8} {:>8} {:>8} {:>7} {:>7} {:>7}",
                "meas",
                "sec",
                "sys",
                "xml",
                "xmlBody",
                "assigned",
                "xmlShare",
                "ourShare",
                "err%",
                "weight",
                "prefix"
            );

            let mut printed = 0usize;
            for row in &report.rows {
                let Some(err) = row.relative_error else {
                    continue;
                };
                if err.abs() < tolerance {
                    continue;
                }
                printed += 1;
                println!(
                    "{:>4} {:>3} {:>3} {:>7.1} {:>8.1} {:>8.1} {:>7.1}% {:>7.1}% {:>+6.1}% {:>7.2} {:>7.1}",
                    row.source_measure
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    row.section_idx,
                    row.system_idx,
                    row.source_measure_width.unwrap_or_default(),
                    row.adjusted_source_body_width.unwrap_or_default(),
                    row.assigned_width,
                    row.source_body_share.unwrap_or_default() * 100.0,
                    row.assigned_share * 100.0,
                    err * 100.0,
                    row.weight,
                    row.prefix_xml_units_removed,
                );
            }
            println!();
            println!(
                "Printed {printed} row(s) at tolerance >= {:.1}%. First-system XML widths are prefix-adjusted before comparison.",
                tolerance * 100.0
            );
            Ok(())
        }

        Commands::Png {
            input,
            output,
            mode,
            breakpoint,
            width,
            scale,
        } => {
            let source = read_source(&input)?;
            let chart = parse_chart(&source)?;
            let pipeline = LayoutPipeline::new()?;

            let viewport_pt = width.unwrap_or_else(|| breakpoint.default_width_pt());
            let layout = pipeline.layout_preset(&chart, mode, breakpoint, viewport_pt);

            // ContinuousScroll has no `pages`; render the whole scene as one image.
            let svgs: Vec<String> = if layout.pages.is_empty() {
                vec![
                    pipeline
                        .export_svg_continuous(&layout)
                        .ok_or_else(|| "continuous layout produced no SVG".to_string())?,
                ]
            } else {
                pipeline.export_svg_pages(&layout)
            };

            println!(
                "Layout: mode={:?}, breakpoint={:?}, viewport={:.0}pt, pages={}, total={:.0}x{:.0}pt",
                mode,
                breakpoint,
                viewport_pt,
                svgs.len().max(1),
                layout.total_width,
                layout.total_height,
            );

            if svgs.len() == 1 {
                let png = svg_to_png(&svgs[0], scale, &pipeline.font_bundle)?;
                std::fs::write(&output, &png)
                    .map_err(|e| format!("Failed to write {}: {e}", output.display()))?;
                println!("Wrote {} ({} bytes)", output.display(), png.len());
            } else {
                let stem = output
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("chart");
                let ext = output.extension().and_then(|s| s.to_str()).unwrap_or("png");
                let dir = output.parent().unwrap_or(std::path::Path::new("."));
                for (i, svg) in svgs.iter().enumerate() {
                    let path = dir.join(format!("{}-p{}.{}", stem, i + 1, ext));
                    let png = svg_to_png(svg, scale, &pipeline.font_bundle)?;
                    std::fs::write(&path, &png)
                        .map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
                    println!("Wrote {} ({} bytes)", path.display(), png.len());
                }
            }
            Ok(())
        }

        Commands::Render { input } => {
            let source = read_source(&input)?;
            let chart = parse_chart(&source)?;
            let pipeline = LayoutPipeline::new()?;
            let layout = pipeline.layout(&chart);
            let svg_pages = pipeline.export_svg_pages(&layout);

            for (i, svg) in svg_pages.iter().enumerate() {
                if svg_pages.len() > 1 {
                    eprintln!("--- Page {} ---", i + 1);
                }
                println!("{}", svg);
            }
            Ok(())
        }
        Commands::Docs {
            input,
            output,
            watch,
        } => {
            let pipeline = LayoutPipeline::new()?;
            let font_css = pipeline.font_face_css();
            // Chart text → font-less SVG; `None` (parse error / empty) makes the
            // preprocessor leave the block as its original fenced source.
            let render = |src: &str| -> Option<String> {
                let chart = parse_chart(src).ok()?;
                pipeline.render_live_svg(&chart)
            };

            let run_once = || -> Result<(), String> {
                let n = docs::build(&input, &output, &render, &font_css)?;
                eprintln!(
                    "kf docs: rendered {n} chart(s) — {} → {}",
                    input.display(),
                    output.display()
                );
                Ok(())
            };

            run_once()?;

            if watch {
                eprintln!("kf docs: watching {} (poll)…", input.display());
                let mut last = docs::tree_fingerprint(&input);
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    let now = docs::tree_fingerprint(&input);
                    if now != last {
                        last = now;
                        if let Err(e) = run_once() {
                            eprintln!("kf docs: {e}");
                        }
                    }
                }
            }
            Ok(())
        }

        Commands::Sync {
            audio,
            chart,
            output,
            separate,
            model,
            lyrics_file,
            ratings,
            verified_threshold,
            review_threshold,
            star_score,
            no_star,
            per_section,
        } => {
            let source = read_source(&chart)?;
            let parsed = parse_chart(&source)?;
            let mut thresholds = match ratings {
                RatingsPreset::Speech => keyflow_sync::RatingThresholds::SPEECH,
                RatingsPreset::Singing => keyflow_sync::RatingThresholds::SINGING,
            };
            if let Some(v) = verified_threshold {
                thresholds.verified = v;
            }
            if let Some(r) = review_threshold {
                thresholds.review = r;
            }
            sync::cmd_sync(
                &parsed,
                sync::SyncOpts {
                    audio: &audio,
                    output: &output,
                    separate,
                    model_id: &model,
                    lyrics_file: lyrics_file.as_deref(),
                    thresholds,
                    star_score: if no_star { None } else { Some(star_score) },
                    per_section,
                },
            )
        }

        Commands::Transcribe {
            audio,
            output,
            separate,
            model,
            engine,
            align,
        } => {
            let opts = sync::TranscribeOpts {
                audio: &audio,
                output: output.as_deref(),
                separate,
                model_id: &model,
                align,
            };
            match engine {
                AsrEngine::Ctc => sync::cmd_transcribe(opts),
                AsrEngine::Whisper => sync::cmd_transcribe_whisper(opts),
            }
        }
        Commands::GuideChart {
            click,
            guide,
            beats_per_bar,
            min_confidence,
            header,
            output,
        } => {
            let opts = sync::GuideChartOpts {
                click: &click,
                guide: &guide,
                beats_per_bar,
                min_confidence,
                header: header.as_deref(),
                output: output.as_deref(),
            };
            sync::cmd_guide_chart(opts)
        }

        Commands::Karaoke {
            chart,
            audio,
            out_dir,
            separate,
            reaper,
        } => sync::cmd_karaoke(sync::KaraokeOpts {
            chart: &chart,
            audio: &audio,
            out_dir: &out_dir,
            separate,
            reaper,
        }),

        Commands::Voices {
            chart,
            out_dir,
            reaper,
        } => sync::cmd_voices(sync::VoicesOpts {
            chart: &chart,
            out_dir: &out_dir,
            reaper,
        }),

        Commands::Bench {
            reference,
            estimate,
            tool,
        } => sync::cmd_bench(&reference, &estimate, &tool),

        Commands::Models { action } => match action {
            ModelsAction::List => sync::cmd_models_list(),
            ModelsAction::Pull { id } => sync::cmd_models_pull(&id),
        },
        Commands::Score { input } => orchestra::score_report(&input),
        Commands::Orchestrate {
            input,
            part,
            profile,
        } => orchestra::orchestrate(&input, part.as_deref(), profile.as_deref()),
    }
}

#[cfg(test)]
mod tests {
    use super::{melody_is_audible, melody_note_signature};
    use keyflow::chart::melody::{Melody, MelodyNote};

    /// A note is the same musical event whether or not it sits in the middle of
    /// a tie chain; the MusicXML importer and the `.kf` parser disagree on the
    /// tie flags, so the signature must ignore them.
    #[test]
    fn melody_signature_ignores_tie_markers() {
        let mut tied = MelodyNote::new("C#", 2);
        tied.octave = Some(3);
        tied.dotted = true;
        tied.tie_start = true;
        tied.tie_stop = true;

        let mut plain = MelodyNote::new("C#", 2);
        plain.octave = Some(3);
        plain.dotted = true;

        assert_eq!(
            melody_note_signature(&tied),
            melody_note_signature(&plain),
            "tie markers must not change the signature"
        );
    }

    /// Stacked pitches and octave are part of the audible content and must
    /// appear in the signature.
    #[test]
    fn melody_signature_keeps_pitch_octave_and_stack() {
        let mut note = MelodyNote::new("F#", 8);
        note.octave = Some(2);
        note.dotted = true;
        note.extra_pitches = vec![("C#".to_string(), Some(4))];
        note.extra_pitch_modifiers = vec![Default::default()];

        let sig = melody_note_signature(&note);
        assert!(sig.contains("F#"), "pitch present: {sig}");
        assert!(sig.contains("Some(2)"), "octave present: {sig}");
        assert!(sig.contains("C#Some(4)"), "stacked pitch present: {sig}");
    }

    /// A melody made only of rests/spaces carries no audible content and is
    /// dropped, so an empty MusicXML bar and a `.kf` `s`/rest bar compare equal.
    #[test]
    fn silent_melodies_are_not_audible() {
        let rest = Melody::with_notes(vec![MelodyNote::new("r", 2)]);
        let space = Melody::with_notes(vec![MelodyNote::new("s", 2)]);
        assert!(!melody_is_audible(&rest), "all-rest melody is silent");
        assert!(!melody_is_audible(&space), "all-space melody is silent");

        let real = Melody::with_notes(vec![MelodyNote::new("r", 8), MelodyNote::new("C#", 8)]);
        assert!(
            melody_is_audible(&real),
            "a melody with one real note is audible"
        );
    }
}
