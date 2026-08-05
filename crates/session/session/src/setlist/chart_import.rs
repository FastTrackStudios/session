//! Interpret keyflow chart text into a laid-out session song structure.
//!
//! The keyflow chart is the single source of *what* — section types, order,
//! bar counts, tempo/key. [`chart_to_layout`] turns that into absolute-time
//! [`LaidSection`]s a setlist/demo can stamp. This is the bridge behind live
//! keyflow-text editing (edit the chart → re-lay the sections) and the Praise
//! demo song; the sung/audio *when* comes separately (guide analysis / lyric
//! sync).

use crate::keyflow::actions::SectionKind;

/// Failure modes of [`chart_to_layout`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChartImportError {
    /// The keyflow parser rejected the text.
    Parse(String),
    /// The chart parsed but has no sections.
    Empty,
}

impl std::fmt::Display for ChartImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "chart parse failed: {e}"),
            Self::Empty => write!(f, "chart has no sections"),
        }
    }
}

impl std::error::Error for ChartImportError {}

/// One section placed on the song timeline (absolute seconds from the region
/// start, count-in included at the front).
#[derive(Debug, Clone, PartialEq)]
pub struct LaidSection {
    /// Session section kind (marker/colour driver).
    pub kind: SectionKind,
    /// Custom label from a quoted chart comment (`Interlude "Breakdown"`).
    pub label: Option<String>,
    /// Auto-assigned occurrence number ("Verse 1"), if any.
    pub number: Option<u32>,
    /// Start position in seconds (from region start; count-in is at 0).
    pub start_seconds: f64,
    /// End position in seconds.
    pub end_seconds: f64,
    /// Length in measures.
    pub measures: u32,
}

/// A keyflow chart laid out as a playable song skeleton.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartLayout {
    /// Song title from the chart header.
    pub title: Option<String>,
    /// Artist from the chart header.
    pub artist: Option<String>,
    /// Tempo in BPM (defaults to 120 if the chart omits it).
    pub tempo_bpm: f64,
    /// Time-signature numerator.
    pub time_sig_num: u32,
    /// Time-signature denominator.
    pub time_sig_den: u32,
    /// Initial key (note name, e.g. "A"), if the chart declares one.
    pub key: Option<String>,
    /// Count-in duration before the first musical downbeat (0 if no leading
    /// CountIn section).
    pub count_in_seconds: f64,
    /// The first musical downbeat, in seconds (== `count_in_seconds`).
    pub song_start_seconds: f64,
    /// End of the last section, in seconds.
    pub song_end_seconds: f64,
    /// All sections in order, count-in first (if present).
    pub sections: Vec<LaidSection>,
}

const DEFAULT_TEMPO_BPM: f64 = 120.0;

/// Parse keyflow chart text and lay its sections out on a seconds timeline.
///
/// Measure length is `numerator * 60 / bpm` (worship charts are `*/4`, so the
/// beat unit is a quarter). Sections are placed contiguously from region start
/// 0; a leading `CountIn` section occupies the front, so the first musical
/// section starts at `count_in_seconds`. A section with no explicit bar count
/// falls back to its kind's default.
pub fn chart_to_layout(chart_text: &str) -> Result<ChartLayout, ChartImportError> {
    let chart =
        keyflow::text::chart::parse_chart(chart_text).map_err(ChartImportError::Parse)?;
    if chart.sections.is_empty() {
        return Err(ChartImportError::Empty);
    }

    let tempo_bpm = chart
        .tempo
        .map(|t| t.bpm)
        .filter(|b| *b > 0.0)
        .unwrap_or(DEFAULT_TEMPO_BPM);
    let (num, den) = chart
        .time_signature
        .map(|ts| (ts.numerator, ts.denominator))
        .unwrap_or((4, 4));
    // Beat = 60/bpm; a measure is `num` beats (beat unit assumed quarter, i.e.
    // den == 4 — true for these worship charts).
    let measure_secs = f64::from(num.max(1)) * 60.0 / tempo_bpm;
    let key = chart.initial_key.as_ref().map(|k| k.root.name.clone());

    let mut sections: Vec<LaidSection> = Vec::with_capacity(chart.sections.len());
    let mut cursor_measures: u32 = 0;
    for cs in &chart.sections {
        let s = &cs.section;
        let kind = SectionKind::from_section_type(&s.section_type);
        let measures = s
            .measure_count
            .map(|m| m as u32)
            .filter(|m| *m > 0)
            .unwrap_or_else(|| kind.default_measure_count());
        let start_seconds = f64::from(cursor_measures) * measure_secs;
        cursor_measures += measures;
        let end_seconds = f64::from(cursor_measures) * measure_secs;
        sections.push(LaidSection {
            kind,
            label: s.comment.clone(),
            number: s.number,
            start_seconds,
            end_seconds,
            measures,
        });
    }

    // The leading CountIn section (if any) is the count-in; the first musical
    // downbeat is right after it.
    let count_in_seconds = sections
        .first()
        .filter(|s| s.kind == SectionKind::CountIn)
        .map(|s| s.end_seconds - s.start_seconds)
        .unwrap_or(0.0);
    let song_end_seconds = sections.last().map(|s| s.end_seconds).unwrap_or(0.0);

    Ok(ChartLayout {
        title: chart.metadata.title.clone(),
        artist: chart.metadata.artist.clone(),
        tempo_bpm,
        time_sig_num: num,
        time_sig_den: den,
        key,
        count_in_seconds,
        song_start_seconds: count_in_seconds,
        song_end_seconds,
        sections,
    })
}
