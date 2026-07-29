//! MusicXML → notation [`Score`] importer.
//!
//! Walks the typed `musicxml` crate model the same way
//! `keyflow-orchestra/score/parse.rs` does (divisions-tick cursor,
//! backup/forward, chord stacking, inherited attributes) but preserves the
//! WRITTEN score — pitch spelling, note types, dots, tuplets, rests, clefs —
//! instead of flattening to a playback timeline.

use musicxml::datatypes::{StartStop, StartStopContinue, YesNo};
use musicxml::elements::{
    ArticulationsType, AudibleType, GraceType, MeasureElement, NotationContentTypes, Note as XmlNote,
    NoteType, PartElement, PartListElement, ScorePartwise,
};

use crate::model::{
    AttrChange, Chord, Clef, ClefSign, Event, Measure, Note, NoteTypeValue, Part, Rest, Score, Step,
    Transposition, WrittenDuration, WrittenPitch,
};

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("read failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad .mxl archive: {0}")]
    Mxl(String),
    #[error("musicxml parse error: {0}")]
    Parse(String),
}

/// Parse a `.musicxml` or `.mxl` file into a notation [`Score`].
pub fn import_file(path: impl AsRef<std::path::Path>) -> Result<Score, ImportError> {
    let data = std::fs::read(path.as_ref())?;
    import_bytes(data)
}

/// Parse in-memory MusicXML (raw XML or a compressed `.mxl` zip).
pub fn import_bytes(data: Vec<u8>) -> Result<Score, ImportError> {
    let xml = if data.starts_with(b"PK") {
        extract_mxl(data)?
    } else {
        data
    };
    let score = musicxml::read_score_data_partwise(xml).map_err(ImportError::Parse)?;
    Ok(score_from_partwise(&score))
}

/// Pull the score XML out of a compressed `.mxl` (container.xml rootfile,
/// falling back to the first plausible XML entry) — same recovery as
/// keyflow-orchestra; the musicxml crate's own zip path fails on multi-file
/// archives and inner names like `score.xml`.
fn extract_mxl(data: Vec<u8>) -> Result<Vec<u8>, ImportError> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(data))
        .map_err(|e| ImportError::Mxl(e.to_string()))?;

    let mut root_path: Option<String> = None;
    if let Ok(mut container) = archive.by_name("META-INF/container.xml") {
        let mut text = String::new();
        if container.read_to_string(&mut text).is_ok()
            && let Some(idx) = text.find("full-path=\"")
        {
            let rest = &text[idx + "full-path=\"".len()..];
            if let Some(end) = rest.find('"') {
                root_path = Some(rest[..end].to_string());
            }
        }
    }
    let root_path = match root_path {
        Some(p) => p,
        None => {
            let mut found = None;
            for i in 0..archive.len() {
                let name = archive
                    .by_index(i)
                    .map_err(|e| ImportError::Mxl(e.to_string()))?
                    .name()
                    .to_string();
                let lower = name.to_lowercase();
                if !name.starts_with("META-INF")
                    && (lower.ends_with(".musicxml") || lower.ends_with(".xml"))
                {
                    found = Some(name);
                    break;
                }
            }
            found.ok_or_else(|| ImportError::Mxl("no score XML in archive".into()))?
        }
    };
    let mut file = archive
        .by_name(&root_path)
        .map_err(|e| ImportError::Mxl(format!("missing rootfile {root_path}: {e}")))?;
    let mut xml = Vec::new();
    file.read_to_end(&mut xml)
        .map_err(|e| ImportError::Mxl(format!("decompress failed: {e}")))?;
    Ok(xml)
}

/// Decode XML entities (iteratively, so double-encoded `&amp;amp;` resolves).
fn decode_entities(s: &str) -> String {
    let mut cur = s.to_string();
    for _ in 0..3 {
        if !cur.contains('&') {
            break;
        }
        let next = cur
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'");
        if next == cur {
            break;
        }
        cur = next;
    }
    cur
}

pub fn score_from_partwise(score: &ScorePartwise) -> Score {
    // Part id → (name, abbreviation) from <part-list>.
    let mut meta: std::collections::HashMap<String, (String, Option<String>)> =
        std::collections::HashMap::new();
    for group in &score.content.part_list.content.content {
        if let PartListElement::ScorePart(sp) = group {
            meta.insert(
                (*sp.attributes.id).clone(),
                (
                    decode_entities(&sp.content.part_name.content),
                    sp.content
                        .part_abbreviation
                        .as_ref()
                        .map(|a| decode_entities(&a.content)),
                ),
            );
        }
    }

    let mut creators: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if let Some(ident) = &score.content.identification {
        for c in &ident.content.creator {
            if let Some(kind) = &c.attributes.r#type {
                creators.insert((**kind).clone(), decode_entities(&c.content));
            }
        }
    }

    let mut parts = Vec::new();
    for part in &score.content.part {
        let id: String = (*part.attributes.id).clone();
        let (name, abbreviation) = meta
            .get(&id)
            .cloned()
            .filter(|(n, _)| !n.is_empty())
            .unwrap_or_else(|| (id.clone(), None));
        parts.push(import_part(part, id, name, abbreviation));
    }

    Score {
        work_title: score
            .content
            .work
            .as_ref()
            .and_then(|w| w.content.work_title.as_ref())
            .map(|t| decode_entities(&t.content)),
        movement_title: score
            .content
            .movement_title
            .as_ref()
            .map(|t| decode_entities(&t.content)),
        composer: creators.get("composer").cloned(),
        arranger: creators.get("arranger").cloned(),
        lyricist: creators.get("lyricist").cloned(),
        parts,
    }
}

fn import_part(
    part: &musicxml::elements::Part,
    id: String,
    name: String,
    abbreviation: Option<String>,
) -> Part {
    let mut measures: Vec<Measure> = Vec::new();
    let mut staves: u8 = 1;
    let mut transpose: Option<Transposition> = None;

    // Inherited state across measures.
    let mut divisions: u32 = 1;
    let mut cur_time: (u32, u32) = (4, 4);

    for pel in &part.content {
        let PartElement::Measure(xml_measure) = pel else {
            continue;
        };
        let mut measure = Measure {
            number: (*xml_measure.attributes.number).clone(),
            implicit: xml_measure.attributes.implicit == Some(YesNo::Yes),
            ..Measure::default()
        };
        // Cursor in divisions ticks from measure start. i64: <backup> can
        // briefly undershoot on malformed input.
        let mut cursor: i64 = 0;
        let mut prev_onset: i64 = 0;

        for el in &xml_measure.content {
            match el {
                MeasureElement::Attributes(attrs) => {
                    let mut change = AttrChange {
                        tick: cursor.max(0) as u32,
                        ..AttrChange::default()
                    };
                    if let Some(d) = &attrs.content.divisions {
                        divisions = (*d.content).max(1);
                        change.divisions = Some(divisions);
                    }
                    if let Some(key) = attrs.content.key.first()
                        && let musicxml::elements::KeyContents::Explicit(e) = &key.content
                    {
                        change.key_fifths = Some(*e.fifths.content);
                    }
                    if let Some(time) = attrs.content.time.first()
                        && time.content.senza_misura.is_none()
                        && let Some(sig) = time.content.beats.first()
                        && let (Ok(beats), Ok(bt)) = (
                            sig.beats.content.parse::<u32>(),
                            sig.beat_type.content.parse::<u32>(),
                        )
                    {
                        cur_time = (beats, bt);
                        change.time = Some(cur_time);
                    }
                    if let Some(s) = &attrs.content.staves {
                        let n = *s.content as u8;
                        staves = staves.max(n);
                        change.staves = Some(n);
                    }
                    for clef in &attrs.content.clef {
                        let staff = clef
                            .attributes
                            .number
                            .as_ref()
                            .map(|n| **n)
                            .unwrap_or(1);
                        change.clefs.push((staff, convert_clef(clef)));
                    }
                    if let Some(t) = attrs.content.transpose.first() {
                        transpose = Some(Transposition {
                            diatonic: t
                                .content
                                .diatonic
                                .as_ref()
                                .map(|d| d.content as i8)
                                .unwrap_or(0),
                            chromatic: *t.content.chromatic.content as i8,
                            octave_change: t
                                .content
                                .octave_change
                                .as_ref()
                                .map(|o| o.content)
                                .unwrap_or(0),
                        });
                    }
                    measure.attributes.push(change);
                }

                MeasureElement::Backup(b) => {
                    cursor -= i64::from(*b.content.duration.content);
                    cursor = cursor.max(0);
                }

                MeasureElement::Forward(f) => {
                    cursor += i64::from(*f.content.duration.content);
                }

                MeasureElement::Note(note) => {
                    import_note(note, &mut measure, &mut cursor, &mut prev_onset, &mut staves);
                }

                _ => {}
            }
        }

        measure.divisions = divisions;
        // A bar is the inherited meter long; only content overflow (malformed
        // input) extends it, so nothing gets clipped downstream.
        let meter_ticks = (u64::from(cur_time.0) * 4 * u64::from(divisions)
            / u64::from(cur_time.1.max(1))) as u32;
        let content_ticks = measure
            .events
            .iter()
            .map(|e| match e {
                Event::Chord(c) => c.tick + c.duration,
                Event::Rest(r) => r.tick + r.duration,
            })
            .max()
            .unwrap_or(0);
        measure.len_ticks = meter_ticks.max(content_ticks);
        measures.push(measure);
    }

    Part {
        id,
        name,
        abbreviation,
        staves,
        transpose,
        measures,
    }
}

/// Shape of one `<note>`: grace/cue flags, chord membership, sounding
/// content, duration ticks, and ties.
struct NoteShape<'a> {
    grace: bool,
    cue: bool,
    chord_member: bool,
    audible: &'a AudibleType,
    duration: u32,
    tie_start: bool,
    tie_stop: bool,
}

fn note_shape(note: &XmlNote) -> NoteShape<'_> {
    match &note.content.info {
        NoteType::Grace(g) => match &g.info {
            GraceType::Cue(c) => NoteShape {
                grace: true,
                cue: true,
                chord_member: c.chord.is_some(),
                audible: &c.audible,
                duration: 0,
                tie_start: false,
                tie_stop: false,
            },
            GraceType::Normal(n) => {
                let (tie_start, tie_stop) = tie_flags(&n.tie);
                NoteShape {
                    grace: true,
                    cue: false,
                    chord_member: n.chord.is_some(),
                    audible: &n.audible,
                    duration: 0,
                    tie_start,
                    tie_stop,
                }
            }
        },
        NoteType::Cue(c) => NoteShape {
            grace: false,
            cue: true,
            chord_member: c.chord.is_some(),
            audible: &c.audible,
            duration: *c.duration.content,
            tie_start: false,
            tie_stop: false,
        },
        NoteType::Normal(n) => {
            let (tie_start, tie_stop) = tie_flags(&n.tie);
            NoteShape {
                grace: false,
                cue: false,
                chord_member: n.chord.is_some(),
                audible: &n.audible,
                duration: *n.duration.content,
                tie_start,
                tie_stop,
            }
        }
    }
}

fn import_note(
    note: &XmlNote,
    measure: &mut Measure,
    cursor: &mut i64,
    prev_onset: &mut i64,
    staves: &mut u8,
) {
    let shape = note_shape(note);
    let onset = if shape.chord_member {
        *prev_onset
    } else {
        *cursor
    };
    let tick = onset.max(0) as u32;

    let voice = note
        .content
        .voice
        .as_ref()
        .and_then(|v| v.content.parse().ok())
        .unwrap_or(1);
    let staff = note.content.staff.as_ref().map(|s| *s.content as u8).unwrap_or(1);
    *staves = (*staves).max(staff);

    let written = WrittenDuration {
        note_type: note.content.r#type.as_ref().map(|t| convert_type(&t.content)),
        dots: note.content.dot.len() as u8,
        tuplet: note.content.time_modification.as_ref().map(|tm| {
            (
                *tm.content.actual_notes.content as u8,
                *tm.content.normal_notes.content as u8,
            )
        }),
    };

    match shape.audible {
        AudibleType::Pitch(p) => {
            let pitch = WrittenPitch {
                step: convert_step(&p.content.step.content),
                alter: p.content.alter.as_ref().map(|a| *a.content as i8).unwrap_or(0),
                octave: *p.content.octave.content as i8,
            };
            push_pitched(
                note, &shape, measure, tick, voice, staff, written, pitch,
            );
        }
        AudibleType::Unpitched(u) => {
            // Percussion: keep the display position as the written pitch so
            // the note still lands on its staff line.
            let pitch = WrittenPitch {
                step: convert_step(&u.content.display_step.content),
                alter: 0,
                octave: *u.content.display_octave.content as i8,
            };
            push_pitched(
                note, &shape, measure, tick, voice, staff, written, pitch,
            );
        }
        AudibleType::Rest(r) => {
            measure.events.push(Event::Rest(Rest {
                tick,
                duration: shape.duration,
                voice,
                staff,
                written,
                measure_rest: r.attributes.measure == Some(YesNo::Yes),
            }));
        }
    }

    if !shape.chord_member && !shape.grace {
        *prev_onset = onset;
        *cursor = onset + i64::from(shape.duration);
    }
}

#[allow(clippy::too_many_arguments)]
fn push_pitched(
    note: &XmlNote,
    shape: &NoteShape<'_>,
    measure: &mut Measure,
    tick: u32,
    voice: u32,
    staff: u8,
    written: WrittenDuration,
    pitch: WrittenPitch,
) {
    let model_note = Note {
        pitch,
        tie_start: shape.tie_start,
        tie_stop: shape.tie_stop,
        accidental: note
            .content
            .accidental
            .as_ref()
            .map(|a| format!("{:?}", a.content).to_lowercase()),
    };

    // `<chord>` notes stack onto the previous chord event when it matches
    // this note's voice/onset shape; otherwise they open a new chord (some
    // exporters emit orphan chord flags after rests).
    if shape.chord_member
        && let Some(Event::Chord(last)) = measure.events.last_mut()
        && last.tick == tick
        && last.voice == voice
        && last.grace == shape.grace
    {
        last.notes.push(model_note);
        return;
    }

    let (articulations, slur_start, slur_stop) = read_notations(note, shape.cue);
    measure.events.push(Event::Chord(Chord {
        tick,
        duration: shape.duration,
        voice,
        staff,
        written,
        grace: shape.grace,
        notes: vec![model_note],
        slur_start,
        slur_stop,
        articulations,
    }));
}

/// Collect articulation tags + slur flags from `<notations>`.
fn read_notations(
    note: &XmlNote,
    cue: bool,
) -> (std::collections::BTreeSet<String>, bool, bool) {
    let mut art = std::collections::BTreeSet::new();
    if cue {
        art.insert("cue".to_string());
    }
    let mut slur_start = false;
    let mut slur_stop = false;
    for nots in &note.content.notations {
        for item in &nots.content.notations {
            match item {
                NotationContentTypes::Articulations(arts) => {
                    for a in &arts.content {
                        art.insert(articulation_tag(a).to_string());
                    }
                }
                NotationContentTypes::Fermata(_) => {
                    art.insert("fermata".to_string());
                }
                NotationContentTypes::Slur(sl) => match sl.attributes.r#type {
                    StartStopContinue::Start => slur_start = true,
                    StartStopContinue::Stop => slur_stop = true,
                    StartStopContinue::Continue => {}
                },
                NotationContentTypes::Tuplet(_) => {
                    art.insert("tuplet-bracket".to_string());
                }
                NotationContentTypes::Glissando(g) => {
                    art.insert(
                        match g.attributes.r#type {
                            StartStop::Start => "glissando-start",
                            StartStop::Stop => "glissando-stop",
                        }
                        .to_string(),
                    );
                }
                _ => {}
            }
        }
    }
    (art, slur_start, slur_stop)
}

fn articulation_tag(a: &ArticulationsType) -> &'static str {
    match a {
        ArticulationsType::Accent(_) => "accent",
        ArticulationsType::StrongAccent(_) => "strong-accent",
        ArticulationsType::Staccato(_) => "staccato",
        ArticulationsType::Tenuto(_) => "tenuto",
        ArticulationsType::DetachedLegato(_) => "detached-legato",
        ArticulationsType::Staccatissimo(_) => "staccatissimo",
        ArticulationsType::Spiccato(_) => "spiccato",
        ArticulationsType::Scoop(_) => "scoop",
        ArticulationsType::Plop(_) => "plop",
        ArticulationsType::Doit(_) => "doit",
        ArticulationsType::Falloff(_) => "falloff",
        ArticulationsType::BreathMark(_) => "breath-mark",
        ArticulationsType::Caesura(_) => "caesura",
        ArticulationsType::Stress(_) => "stress",
        ArticulationsType::Unstress(_) => "unstress",
        ArticulationsType::SoftAccent(_) => "soft-accent",
        ArticulationsType::OtherArticulation(_) => "other",
    }
}

fn tie_flags(ties: &[musicxml::elements::Tie]) -> (bool, bool) {
    let mut start = false;
    let mut stop = false;
    for t in ties {
        match t.attributes.r#type {
            StartStop::Start => start = true,
            StartStop::Stop => stop = true,
        }
    }
    (start, stop)
}

fn convert_clef(clef: &musicxml::elements::Clef) -> Clef {
    use musicxml::datatypes::ClefSign as Xml;
    Clef {
        sign: match clef.content.sign.content {
            Xml::G => ClefSign::G,
            Xml::F => ClefSign::F,
            Xml::C => ClefSign::C,
            Xml::Percussion => ClefSign::Percussion,
            Xml::TAB => ClefSign::Tab,
            Xml::Jianpu | Xml::None => ClefSign::None,
        },
        line: clef.content.line.as_ref().map(|l| *l.content as u8),
        octave_change: clef
            .content
            .clef_octave_change
            .as_ref()
            .map(|o| o.content)
            .unwrap_or(0),
    }
}

fn convert_step(step: &musicxml::datatypes::Step) -> Step {
    use musicxml::datatypes::Step as Xml;
    match step {
        Xml::A => Step::A,
        Xml::B => Step::B,
        Xml::C => Step::C,
        Xml::D => Step::D,
        Xml::E => Step::E,
        Xml::F => Step::F,
        Xml::G => Step::G,
    }
}

fn convert_type(t: &musicxml::datatypes::NoteTypeValue) -> NoteTypeValue {
    use musicxml::datatypes::NoteTypeValue as Xml;
    match t {
        Xml::Maxima => NoteTypeValue::Maxima,
        Xml::Long => NoteTypeValue::Long,
        Xml::Breve => NoteTypeValue::Breve,
        Xml::Whole => NoteTypeValue::Whole,
        Xml::Half => NoteTypeValue::Half,
        Xml::Quarter => NoteTypeValue::Quarter,
        Xml::Eighth => NoteTypeValue::Eighth,
        Xml::Sixteenth => NoteTypeValue::Sixteenth,
        Xml::ThirtySecond => NoteTypeValue::ThirtySecond,
        Xml::SixtyFourth => NoteTypeValue::SixtyFourth,
        Xml::OneHundredTwentyEighth => NoteTypeValue::OneHundredTwentyEighth,
        Xml::TwoHundredFiftySixth => NoteTypeValue::TwoHundredFiftySixth,
        Xml::FiveHundredTwelfth => NoteTypeValue::FiveHundredTwelfth,
        Xml::OneThousandTwentyFourth => NoteTypeValue::OneThousandTwentyFourth,
    }
}
