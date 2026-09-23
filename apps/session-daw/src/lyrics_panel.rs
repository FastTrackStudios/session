//! The lyrics panel: the song's words, following the song, in two views.
//!
//! - **Audience** — what a room reads: the words alone, large and
//!   centred on a dark field, a slide (or a line, or a section) at a time.
//!   Nothing else on screen.
//! - **Performer** — what the stage reads: the section it is in, how far
//!   through it, its lines with the one being sung lit, and what comes
//!   next.
//!
//! The lines are the song's own: the LINES track's items, one a line,
//! labelled with its text ([`session::lyrics::stamp_lines`] puts them
//! there from a synced `.lrc` when a song is prepared). Sections and
//! slides are derived from them against the song's section regions
//! ([`session::lyrics`]), so a deeper layer always brings the ones above
//! it. Words and syllables show once a song has them.
//!
//! Inline styles throughout (see the repo's CLAUDE.md on Blitz).

use dioxus::prelude::*;
use session::lyrics::{Layer, LyricSection, Lyrics, SLIDE_LINES, SectionSpan, Slide};

use crate::shell::{ACCENT, DIM, RULE, TEXT};
use crate::studio::StudioSession;

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

/// The song's lyrics as the panel shows them: lines, sections (with the
/// colour of the region each came from), slides.
#[derive(Clone, PartialEq)]
struct Words {
    lyrics: Lyrics,
    sections: Vec<LyricSection>,
    colors: Vec<String>,
    slides: Vec<Slide>,
}

impl Words {
    fn of(session: &StudioSession) -> Self {
        let project = &session.project;
        let lyrics = project
            .tracks
            .iter()
            .find(|t| t.name.trim().eq_ignore_ascii_case(session::lyrics::LINES_TRACK))
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
        Self {
            lyrics,
            sections,
            colors: regions
                .iter()
                .map(|r| r.color.clone().unwrap_or_else(|| ACCENT.to_owned()))
                .collect(),
            slides,
        }
    }

    /// The section the song is in at `at`, or the first before it starts.
    fn section_at(&self, at: f64) -> Option<usize> {
        self.sections
            .iter()
            .rposition(|s| s.start <= at + 1e-6)
            .or_else(|| (!self.sections.is_empty()).then_some(0))
    }

    /// The slide showing at `at`: the one sung, or held through a gap
    /// after it, or the first before any has started.
    fn slide_at(&self, at: f64) -> Option<usize> {
        self.slides
            .iter()
            .rposition(|s| s.start <= at + 1e-6)
            .or_else(|| (!self.slides.is_empty()).then_some(0))
    }

    /// The line to show at `at`: the one sung, or the last before a gap,
    /// or the first before any.
    fn line_shown(&self, at: f64) -> Option<usize> {
        self.lyrics
            .line_at(at)
            .or_else(|| self.lyrics.line_reached(at))
            .or_else(|| (!self.lyrics.lines.is_empty()).then_some(0))
    }
}

/// The lyrics panel.
#[component]
pub fn LyricsPanel() -> Element {
    let session: StudioSession = use_context();
    let words = use_hook(|| Words::of(&session));
    let reading = crate::progress::use_reading();
    let mut view = use_signal(|| View::Performer);
    let mut layer = use_signal(|| Layer::Slide);
    let at = reading().at;
    let has = words.lyrics.layers();
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; width:100%; height:100%; background:#0b0c0f; \
                    color:{TEXT}; font-family:system-ui, sans-serif;",
            div {
                style: "position:absolute; top:0; left:0; right:0; height:34px; display:flex; \
                        align-items:center; gap:4px; padding:0 10px; overflow-x:auto; \
                        border-bottom:1px solid {RULE}; background:#111216;",
                for each in View::ALL {
                    Pill {
                        label: each.name().to_owned(),
                        on: view() == each,
                        available: true,
                        pick: move |()| view.set(each),
                    }
                }
                if view() == View::Audience {
                    div { style: "width:12px; flex:none;" }
                    // What the room sees a step at a time. The song-wide
                    // and finer layers are the Performer's business.
                    for each in [Layer::Section, Layer::Slide, Layer::Line] {
                        Pill {
                            label: each.name().to_owned(),
                            on: layer() == each,
                            available: has.contains(&each),
                            pick: move |()| layer.set(each),
                        }
                    }
                }
            }
            div {
                style: "position:absolute; top:35px; left:0; right:0; bottom:0; overflow:hidden;",
                if words.lyrics.lines.is_empty() {
                    div {
                        style: "padding:18px 22px; color:{DIM}; font-size:13px; line-height:1.6;",
                        "No lyrics for this song yet. Put a synced .lrc beside its chart \
                         (session-cli lyrics fetch) and prepare the song again."
                    }
                } else if view() == View::Audience {
                    Audience { words: words.clone(), layer: layer(), at }
                } else {
                    Performer { words: words.clone(), at }
                }
            }
        }
    }
}

#[component]
fn Pill(label: String, on: bool, available: bool, pick: EventHandler<()>) -> Element {
    let (bg, fg) = match (on, available) {
        (true, _) => (ACCENT, "#0b0c0e"),
        (false, true) => ("transparent", TEXT),
        (false, false) => ("transparent", "#4b4f57"),
    };
    rsx! {
        button {
            style: "flex:none; height:24px; padding:0 10px; border-radius:5px; border:1px solid {RULE}; \
                    background:{bg}; color:{fg}; font-size:11px; font-weight:600; cursor:pointer; \
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

/// The room's view: the words and nothing else, large, centred, on a
/// dark field that is lightest behind them.
#[component]
fn Audience(words: Words, layer: Layer, at: f64) -> Element {
    let lines = &words.lyrics.lines;
    let shown: Vec<usize> = match layer {
        Layer::Section => words
            .section_at(at)
            .map(|i| words.sections[i].lines.clone().collect())
            .unwrap_or_default(),
        Layer::Line => words.line_shown(at).into_iter().collect(),
        _ => words
            .slide_at(at)
            .map(|i| words.slides[i].lines.clone().collect())
            .unwrap_or_default(),
    };
    // Fewer words, larger: a line alone fills the width it can.
    let size = match (layer, shown.len()) {
        (Layer::Line, _) | (_, 1) => 56,
        (_, 2) => 48,
        (_, 3 | 4) => 38,
        _ => 30,
    };
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; width:100%; height:100%; display:flex; \
                    flex-direction:column; align-items:center; justify-content:center; \
                    padding:40px 48px; text-align:center; \
                    background:radial-gradient(ellipse at center, #1c2233 0%, #0b0d14 70%, #06070a 100%);",
            for k in shown {
                div {
                    key: "{k}",
                    style: "font-size:{size}px; line-height:1.18; font-weight:700; color:#ffffff; \
                            letter-spacing:-0.5px; margin:0 0 14px 0; max-width:100%;",
                    "{lines[k].text}"
                }
            }
        }
    }
}

/// The stage's view: where the song is, what is being sung, what is next.
#[component]
fn Performer(words: Words, at: f64) -> Element {
    let lines = &words.lyrics.lines;
    let Some(i) = words.section_at(at) else { return rsx! {} };
    let section = &words.sections[i];
    let color = words.colors.get(i).cloned().unwrap_or_else(|| ACCENT.to_owned());
    let through = ((at - section.start) / (section.end - section.start).max(1e-6)).clamp(0.0, 1.0) * 100.0;
    let current = words.lyrics.line_at(at);
    let next_line = words.line_shown(at).map_or(0, |k| k + usize::from(current.is_some() || at >= lines[k].start));
    let upcoming = words.sections.get(i + 1).map(|s| {
        let first = (!s.lines.is_empty()).then(|| lines[s.lines.start].text.clone());
        (s.name.clone(), (s.start - at).max(0.0), first, words.colors.get(i + 1).cloned())
    });
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; width:100%; height:100%; padding:16px 20px; overflow:auto;",
            // Where the song is: the section, in its colour, and how far in.
            div {
                style: "display:flex; align-items:baseline; gap:10px;",
                div {
                    style: "font-size:13px; font-weight:800; letter-spacing:1.5px; text-transform:uppercase; color:{color};",
                    "{section.name}"
                }
                div { style: "font-size:11px; color:{DIM};", "{i + 1} of {words.sections.len()}" }
            }
            div {
                style: "height:3px; margin:8px 0 16px 0; background:#1d1f25; border-radius:2px;",
                div { style: "height:3px; width:{through}%; background:{color}; border-radius:2px;" }
            }
            // The section's lines, the one being sung lit.
            if section.lines.is_empty() {
                div { style: "font-size:22px; color:{DIM}; font-style:italic;", "Instrumental" }
            }
            for k in section.lines.clone() {
                div {
                    key: "{k}",
                    style: if Some(k) == current {
                        format!("font-size:30px; line-height:1.25; font-weight:800; color:{TEXT}; margin:0 0 8px 0; \
                                 padding-left:10px; border-left:3px solid {color};")
                    } else if k < next_line {
                        format!("font-size:22px; line-height:1.3; font-weight:500; color:#5a5f69; margin:0 0 8px 0; padding-left:13px;")
                    } else {
                        format!("font-size:22px; line-height:1.3; font-weight:600; color:#aeb3bd; margin:0 0 8px 0; padding-left:13px;")
                    },
                    "{lines[k].text}"
                }
            }
            // What comes next.
            if let Some((name, due, first, next_color)) = upcoming {
                div {
                    style: "margin-top:20px; padding-top:12px; border-top:1px solid {RULE};",
                    div {
                        style: "display:flex; align-items:baseline; gap:10px;",
                        div {
                            style: "font-size:11px; font-weight:800; letter-spacing:1.5px; text-transform:uppercase; \
                                    color:{next_color.unwrap_or_else(|| DIM.to_owned())};",
                            "Next · {name}"
                        }
                        div { style: "font-size:11px; color:{DIM};", "in {due:.0}s" }
                    }
                    if let Some(first) = first {
                        div { style: "font-size:17px; color:{DIM}; margin-top:6px;", "{first}" }
                    }
                }
            }
        }
    }
}
