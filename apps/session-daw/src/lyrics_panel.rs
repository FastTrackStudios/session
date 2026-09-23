//! The lyrics panel: the song's words, following the song, in two views.
//!
//! - **Audience** — what a room reads: the words alone, as large as the
//!   panel lets them be, centred on a dark field lit by the section's
//!   colour, a slide (or a line, or a section) at a time. A title card
//!   before the first line; nothing through an instrumental.
//! - **Performer** — what the stage reads: the song as a strip of its
//!   sections, the one it is in and how far through, the line being sung
//!   large with the next under it and the rest of the section after, and
//!   what comes next, counting down.
//!
//! Which view (and the Audience's layer) is a small switch in the panel's
//! top-right corner; everything else is the view.
//!
//! The lines are the song's own: the Lyrics track's items, one a line,
//! labelled with its text ([`session::lyrics::stamp_lines`] puts them
//! there from a synced `.lrc` when a song is prepared). Sections and
//! slides are derived from them against the song's section regions
//! ([`session::lyrics`]). The panel runs ahead of the song — words on a
//! screen are needed before they are sung ([`SWITCH_EARLY`],
//! [`LINE_EARLY`]).
//!
//! Inline styles throughout (see the repo's CLAUDE.md on Blitz).

use std::rc::Rc;
use std::time::Duration;

use dioxus::prelude::*;
use futures_util::StreamExt as _;
use session::lyrics::{Layer, LyricSection, Lyrics, SLIDE_LINES, SectionSpan, Slide};

use crate::shell::{ACCENT, DIM, RULE, TEXT};
use crate::studio::StudioSession;

/// How much sooner than the song a section or a slide comes up: words
/// on a screen have to be there before they are sung, so the change is
/// made ahead, the way a projectionist makes it.
const SWITCH_EARLY: f64 = 1.0;

/// How much sooner a line is lit than it is sung — enough to be read
/// into, not so much that it runs ahead of the singer.
const LINE_EARLY: f64 = 0.4;

/// The two ways the panel shows the words.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Audience,
    Performer,
}

impl View {
    const ALL: [Self; 2] = [Self::Audience, Self::Performer];

    const fn name(self) -> &'static str {
        match self {
            Self::Audience => "Audience",
            Self::Performer => "Performer",
        }
    }
}

/// What the panel shows — Audience or Performer, and the Audience's
/// layer. A host provides one above the songs so the choice holds from
/// song to song; without one each panel keeps its own.
#[derive(Clone, Copy, PartialEq)]
pub struct LyricsChoice {
    pub view: Signal<View>,
    pub layer: Signal<Layer>,
}

impl LyricsChoice {
    #[must_use]
    pub fn new() -> Self {
        Self {
            view: Signal::new(View::Performer),
            layer: Signal::new(Layer::Slide),
        }
    }
}

impl Default for LyricsChoice {
    fn default() -> Self {
        Self::new()
    }
}

/// The song's lyrics as the panel shows them: lines, sections (with the
/// colour of the region each came from), slides, and what the song is
/// called.
#[derive(Clone, PartialEq)]
struct Words {
    lyrics: Lyrics,
    sections: Vec<LyricSection>,
    colors: Vec<String>,
    slides: Vec<Slide>,
    title: String,
    artist: Option<String>,
    song: (f64, f64),
}

impl Words {
    fn of(session: &StudioSession) -> Self {
        let project = &session.project;
        let lyrics = project
            .tracks
            .iter()
            .find(|t| session::lyrics::is_lyrics_track(&t.name))
            .map(|track| {
                Lyrics::from_items(project.lane(&track.guid).iter().filter_map(|item| {
                    Some((item.position.as_seconds(), item.length.as_seconds(), project.title(item)?))
                }))
            })
            .unwrap_or_default();
        let mut regions: Vec<_> = project
            .sections
            .iter()
            .filter(|r| r.lane == crate::ruler::SECTIONS_ROW as u32)
            .collect();
        regions.sort_by(|a, b| a.start.total_cmp(&b.start));
        let spans: Vec<SectionSpan> = regions
            .iter()
            .map(|r| SectionSpan {
                name: r.name.clone(),
                start: r.start,
                end: r.end,
            })
            .collect();
        let sections = lyrics.sections(&spans);
        let slides = lyrics.slides(&sections, SLIDE_LINES);
        let meta = session.chart.as_ref().map(|c| &c.metadata);
        let song = (
            spans.first().map_or(0.0, |s| s.start),
            spans.last().map_or(0.0, |s| s.end),
        );
        Self {
            lyrics,
            sections,
            colors: regions
                .iter()
                .map(|r| r.color.clone().unwrap_or_else(|| ACCENT.to_owned()))
                .collect(),
            slides,
            title: meta.and_then(|m| m.title.clone()).unwrap_or_default(),
            artist: meta.and_then(|m| m.artist.clone()),
            song,
        }
    }

    /// When a section comes up on screen: its downbeat, or its first
    /// sung line when that is a pickup ahead of it.
    fn shows_from(&self, section: &LyricSection) -> f64 {
        self.lyrics
            .lines
            .get(section.lines.start)
            .filter(|_| !section.lines.is_empty())
            .map_or(section.start, |first| first.start.min(section.start))
    }

    /// The section shown at `at` — up [`SWITCH_EARLY`] before it comes,
    /// or before its pickup does — or the first before any.
    fn section_at(&self, at: f64) -> Option<usize> {
        self.sections
            .iter()
            .rposition(|s| self.shows_from(s) <= at + SWITCH_EARLY)
            .or_else(|| (!self.sections.is_empty()).then_some(0))
    }

    /// The slide showing at `at`: up [`SWITCH_EARLY`] before it is sung,
    /// and held through a gap after it until the section changes.
    fn slide_at(&self, at: f64) -> Option<usize> {
        self.slides.iter().rposition(|s| s.start <= at + SWITCH_EARLY)
    }

    /// The line lit at `at`, [`LINE_EARLY`] ahead of the singer.
    fn line_lit(&self, at: f64) -> Option<usize> {
        self.lyrics.line_at(at + LINE_EARLY)
    }

    fn color(&self, section: usize) -> String {
        self.colors.get(section).cloned().unwrap_or_else(|| ACCENT.to_owned())
    }
}

/// A section colour at `alpha` (0–255), for washes: `#rrggbb` gains an
/// alpha byte; anything else is used as it is.
fn tint(color: &str, alpha: u8) -> String {
    if color.len() == 7 && color.starts_with('#') {
        format!("{color}{alpha:02x}")
    } else {
        color.to_owned()
    }
}

/// The panel's size in pixels, read back from the layout and kept up to
/// date — the Audience's words are sized to it.
fn use_size() -> (Signal<(f64, f64)>, Signal<Option<Rc<MountedData>>>) {
    let mut size = use_signal(|| (0.0_f64, 0.0_f64));
    let node = use_signal(|| None::<Rc<MountedData>>);
    use_future(move || async move {
        // A tick from a thread of its own: a panel resizes when a divider
        // or the window moves, and nothing tells it.
        let (tick, mut ticks) = futures_channel::mpsc::unbounded::<()>();
        std::thread::spawn(move || {
            while tick.unbounded_send(()).is_ok() {
                std::thread::sleep(Duration::from_millis(400));
            }
        });
        while ticks.next().await.is_some() {
            let Some(node) = node.peek().clone() else { continue };
            if let Ok(rect) = node.get_client_rect().await {
                let now = (rect.size.width, rect.size.height);
                if *size.peek() != now {
                    size.set(now);
                }
            }
        }
    });
    (size, node)
}

/// The lyrics panel.
#[component]
pub fn LyricsPanel() -> Element {
    let session: StudioSession = use_context();
    let words = use_hook(|| Words::of(&session));
    let reading = crate::progress::use_reading();
    let own = use_hook(LyricsChoice::new);
    let LyricsChoice { mut view, mut layer } = try_use_context::<LyricsChoice>().unwrap_or(own);
    let (size, mut node) = use_size();
    let at = reading().at;
    let has = words.lyrics.layers();
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; width:100%; height:100%; overflow:hidden; \
                    background:#07080b; color:{TEXT}; font-family:system-ui, sans-serif;",
            onmounted: move |e| node.set(Some(e.data())),
            if words.lyrics.lines.is_empty() {
                div {
                    style: "position:absolute; top:0; left:0; width:100%; height:100%; display:flex; \
                            align-items:center; justify-content:center; padding:24px; text-align:center; \
                            color:{DIM}; font-size:13px; line-height:1.6;",
                    "No lyrics for this song yet — put a synced .lrc beside its chart \
                     (session lyrics fetch) and prepare the song again."
                }
            } else if view() == View::Audience {
                Audience { words: words.clone(), layer: layer(), at, size: size() }
            } else {
                Performer { words: words.clone(), at, size: size() }
            }
            // The switch, out of the way in the corner.
            div {
                style: "position:absolute; top:8px; right:8px; display:flex; align-items:center; gap:2px; \
                        padding:2px; border-radius:7px; background:rgba(10,11,14,0.72); \
                        border:1px solid {RULE};",
                if view() == View::Audience {
                    for each in [Layer::Section, Layer::Slide, Layer::Line] {
                        Pill {
                            label: each.name().to_owned(),
                            on: layer() == each,
                            available: has.contains(&each),
                            pick: move |()| layer.set(each),
                        }
                    }
                    div { style: "width:1px; height:14px; margin:0 3px; background:{RULE};" }
                }
                for each in View::ALL {
                    Pill {
                        label: each.name().to_owned(),
                        on: view() == each,
                        available: true,
                        pick: move |()| view.set(each),
                    }
                }
            }
        }
    }
}

#[component]
fn Pill(label: String, on: bool, available: bool, pick: EventHandler<()>) -> Element {
    let (bg, fg) = match (on, available) {
        (true, _) => (ACCENT, "#0b0c0e"),
        (false, true) => ("transparent", "#c7cad1"),
        (false, false) => ("transparent", "#4b4f57"),
    };
    rsx! {
        button {
            style: "flex:none; height:20px; padding:0 8px; border-radius:5px; border:none; \
                    background:{bg}; color:{fg}; font-size:10px; font-weight:700; cursor:pointer; \
                    white-space:nowrap;",
            onclick: move |_| {
                if available {
                    pick.call(());
                }
            },
            "{label}"
        }
    }
}

/// The room's view: the words and nothing else, as large as the panel
/// allows, centred, on a dark field lit by the section's colour.
#[component]
fn Audience(words: Words, layer: Layer, at: f64, size: (f64, f64)) -> Element {
    let lines = &words.lyrics.lines;
    let section = words.section_at(at);
    let color = section.map_or_else(|| ACCENT.to_owned(), |i| words.color(i));
    let before_words = lines.first().is_some_and(|first| at + SWITCH_EARLY < first.start);
    let shown: Vec<usize> = if before_words {
        Vec::new()
    } else {
        match layer {
            Layer::Section => section.map(|i| words.sections[i].lines.clone().collect()).unwrap_or_default(),
            Layer::Line => words.lyrics.line_at(at + SWITCH_EARLY).into_iter().collect(),
            // A slide holds through a breath, but not into the next
            // section's instrumental: then the screen goes dark.
            _ => words
                .slide_at(at)
                .filter(|&i| Some(words.slides[i].section) == section)
                .map(|i| words.slides[i].lines.clone().collect())
                .unwrap_or_default(),
        }
    };
    let (w, h) = (size.0.max(1.0), size.1.max(1.0));
    // As large as fits: the height shared by the lines, the width by the
    // longest of them (a character is about half an em of this face).
    let longest = shown.iter().map(|&k| lines[k].text.chars().count()).max().unwrap_or(1).max(8) as f64;
    let count = shown.len().max(1) as f64;
    let font = ((h * 0.78) / (count * 1.2)).min((w * 0.86) / (longest * 0.52)).clamp(18.0, 140.0);
    let glow = tint(&color, 0x38);
    let title_size = (font * 0.9).min(72.0);
    let artist_gap = (font * 0.25).min(18.0);
    let artist_size = (font * 0.32).clamp(12.0, 26.0);
    let line_gap = font * 0.12;
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; width:100%; height:100%; display:flex; \
                    flex-direction:column; align-items:center; justify-content:center; \
                    padding:0 6%; text-align:center; \
                    background:radial-gradient(ellipse at 50% 55%, {glow} 0%, #0c0e16 55%, #040507 100%);",
            if before_words {
                div {
                    style: "font-size:{title_size}px; font-weight:800; letter-spacing:-1px; \
                            line-height:1.1; color:#ffffff;",
                    "{words.title}"
                }
                if let Some(artist) = &words.artist {
                    div {
                        style: "margin-top:{artist_gap}px; font-size:{artist_size}px; \
                                font-weight:500; letter-spacing:2px; text-transform:uppercase; color:#9aa3b5;",
                        "{artist}"
                    }
                }
            }
            for k in shown {
                div {
                    key: "{k}",
                    style: "font-size:{font}px; line-height:1.14; font-weight:800; letter-spacing:-0.02em; \
                            color:#ffffff; margin:0 0 {line_gap}px 0;",
                    "{lines[k].text}"
                }
            }
        }
    }
}

/// The stage's view: where the song is, what is being sung, what is next.
#[component]
fn Performer(words: Words, at: f64, size: (f64, f64)) -> Element {
    let lines = &words.lyrics.lines;
    let Some(i) = words.section_at(at) else { return rsx! {} };
    let section = &words.sections[i];
    let color = words.color(i);
    let wash = tint(&color, 0x1c);
    let through = ((at - section.start) / (section.end - section.start).max(1e-6)).clamp(0.0, 1.0) * 100.0;
    let lit = words.line_lit(at);
    // The line in focus — lit, or else the next to come in this section.
    let focus = lit
        .or_else(|| section.lines.clone().find(|&k| lines[k].start > at))
        .or_else(|| section.lines.clone().last());
    let w = size.0.max(1.0);
    let big = (w / 17.0).clamp(22.0, 44.0);
    let mid = (big * 0.62).max(16.0);
    let small = (big * 0.48).max(13.0);
    let upcoming = words.sections.get(i + 1).map(|s| {
        let first = (!s.lines.is_empty()).then(|| lines[s.lines.start].text.clone());
        let due = (words.shows_from(s) - at).max(0.0);
        let span = (words.shows_from(s) - section.start).max(1.0);
        (s.name.clone(), due, (1.0 - due / span).clamp(0.0, 1.0) * 100.0, first, words.color(i + 1))
    });
    let (song_start, song_end) = words.song;
    let song_span = (song_end - song_start).max(1e-6);
    let strip: Vec<(f64, String, f64)> = words
        .sections
        .iter()
        .enumerate()
        .map(|(k, s)| {
            let opacity = if k == i {
                1.0
            } else if k < i {
                0.28
            } else {
                0.5
            };
            ((s.end - s.start) / song_span * 100.0, words.color(k), opacity)
        })
        .collect();
    let line_style = |k: usize| match focus {
        Some(f) if k == f => format!(
            "font-size:{big}px; line-height:1.18; font-weight:800; color:{}; margin:4px 0 10px 0; \
             padding-left:12px; border-left:4px solid {color};",
            if lit == Some(k) { "#ffffff" } else { "#d9dce2" }
        ),
        Some(f) if k == f + 1 => format!(
            "font-size:{mid}px; line-height:1.25; font-weight:700; color:#aab0bb; margin:0 0 8px 0; padding-left:16px;"
        ),
        Some(f) if k < f => format!(
            "font-size:{small}px; line-height:1.3; font-weight:500; color:#4a4f58; margin:0 0 6px 0; padding-left:16px;"
        ),
        _ => format!(
            "font-size:{small}px; line-height:1.3; font-weight:600; color:#7d838e; margin:0 0 6px 0; padding-left:16px;"
        ),
    };
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; width:100%; height:100%; display:flex; flex-direction:column; \
                    background:linear-gradient(180deg, {wash} 0%, #07080b 38%);",
            // The song as a strip of its sections, the one it is in lit.
            div {
                style: "flex:none; display:flex; gap:2px; height:6px; margin:12px 190px 0 16px;",
                for (k, (width, fill, opacity)) in strip.into_iter().enumerate() {
                    div {
                        key: "{k}",
                        style: "flex:none; width:{width}%; height:6px; border-radius:2px; background:{fill}; opacity:{opacity};",
                    }
                }
            }
            // Where it is: the section, and how far through.
            div {
                style: "flex:none; display:flex; align-items:center; gap:10px; padding:12px 16px 0 16px;",
                div {
                    style: "padding:3px 10px; border-radius:6px; background:{color}; color:#0b0c0e; \
                            font-size:13px; font-weight:800; letter-spacing:1px; text-transform:uppercase;",
                    "{section.name}"
                }
                div { style: "font-size:11px; color:{DIM};", "{i + 1} / {words.sections.len()}" }
            }
            div {
                style: "flex:none; height:3px; margin:10px 16px 0 16px; background:#1b1d23; border-radius:2px;",
                div { style: "height:3px; width:{through}%; background:{color}; border-radius:2px;" }
            }
            // What is sung: the line in focus large, the next under it,
            // the rest of the section after; what has gone, faint.
            div {
                style: "flex:1; min-height:0; overflow:hidden; padding:14px 16px 0 16px;",
                if section.lines.is_empty() {
                    div { style: "font-size:{mid}px; color:{DIM}; font-style:italic; margin-top:6px;", "Instrumental" }
                }
                for k in section.lines.clone() {
                    div { key: "{k}", style: line_style(k), "{lines[k].text}" }
                }
            }
            // What comes next, counting down.
            if let Some((name, due, closing, first, next_color)) = upcoming {
                div {
                    style: "flex:none; margin:8px 12px 12px 12px; padding:10px 12px; border-radius:8px; \
                            background:#101217; border:1px solid {RULE};",
                    div {
                        style: "display:flex; align-items:center; gap:8px;",
                        div { style: "width:8px; height:8px; border-radius:2px; background:{next_color};" }
                        div {
                            style: "font-size:11px; font-weight:800; letter-spacing:1.5px; text-transform:uppercase; color:{next_color};",
                            "Next · {name}"
                        }
                        div { style: "flex:1;" }
                        div { style: "font-size:12px; font-weight:700; color:#c7cad1;", "{due:.0}s" }
                    }
                    div {
                        style: "height:2px; margin-top:7px; background:#1f2229; border-radius:1px;",
                        div { style: "height:2px; width:{closing}%; background:{next_color}; border-radius:1px;" }
                    }
                    if let Some(first) = first {
                        div { style: "font-size:{small}px; color:{DIM}; margin-top:7px;", "{first}" }
                    }
                }
            }
        }
    }
}
