//! MusicXML measure walk — port of `css_engine.lua parsePart` on the typed
//! `musicxml` crate model. Converts divisions-tick cursors to QN as it goes;
//! handles backup/forward, chords, grace notes, ties, inherited meters,
//! mid-measure attributes, `<sound>`/`<direction>` tempo + dynamics, and
//! `<harmony>` symbols.

use musicxml::datatypes::StartStop;
use musicxml::elements::{
    ArticulationsType, AudibleType, DirectionType, DirectionTypeContents, DynamicsType,
    MeasureElement, NotationContentTypes, Note, NoteType, OrnamentType, PartElement, ScorePartwise,
    TechnicalContents,
};

use super::{
    ArtSet, DegreeKind, DynamicPoint, HarmonyDegree, HarmonyPoint, Marking, MarkingKind,
    MeterPoint, Part, RawNote, Score, ScoreMeta, TempoPoint, dynamic_from_text, dynamic_mark_value,
};
use crate::Error;

/// Decode XML entities (iteratively, so double-encoded `&amp;amp;` from lazy
/// exporters resolves all the way to `&`).
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

/// Parse a `.musicxml` or `.mxl` file into a [`Score`] (all parts eagerly).
pub fn load(path: impl AsRef<std::path::Path>) -> Result<Score, Error> {
    let data =
        std::fs::read(path.as_ref()).map_err(|e| Error::Parse(format!("read failed: {e}")))?;
    let xml = if data.starts_with(b"PK") {
        extract_mxl(data)?
    } else {
        data
    };
    let score = musicxml::read_score_data_partwise(xml).map_err(Error::Parse)?;
    Ok(score_from_partwise(&score))
}

/// Pull the score XML out of a compressed `.mxl`: follow the
/// `META-INF/container.xml` rootfile, falling back to the first plausible
/// XML entry. (The musicxml crate's own zip path fails on multi-file
/// archives and on inner names like `score.xml`.)
fn extract_mxl(data: Vec<u8>) -> Result<Vec<u8>, Error> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(data))
        .map_err(|e| Error::Parse(format!("bad .mxl zip: {e}")))?;

    let mut root_path: Option<String> = None;
    if let Ok(mut container) = archive.by_name("META-INF/container.xml") {
        let mut text = String::new();
        if container.read_to_string(&mut text).is_ok() {
            if let Some(idx) = text.find("full-path=\"") {
                let rest = &text[idx + "full-path=\"".len()..];
                if let Some(end) = rest.find('"') {
                    root_path = Some(rest[..end].to_string());
                }
            }
        }
    }
    let root_path = match root_path {
        Some(p) => p,
        None => {
            // fallback: first top-level XML entry outside META-INF
            let mut found = None;
            for i in 0..archive.len() {
                let name = archive
                    .by_index(i)
                    .map_err(|e| Error::Parse(e.to_string()))?
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
            found.ok_or_else(|| Error::Parse("no score XML in .mxl archive".into()))?
        }
    };
    let mut file = archive
        .by_name(&root_path)
        .map_err(|e| Error::Parse(format!("missing rootfile {root_path}: {e}")))?;
    let mut xml = Vec::new();
    file.read_to_end(&mut xml)
        .map_err(|e| Error::Parse(format!("decompress failed: {e}")))?;
    Ok(xml)
}

pub fn score_from_partwise(score: &ScorePartwise) -> Score {
    // Part id → display name from <part-list>.
    let mut names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for group in &score.content.part_list.content.content {
        if let musicxml::elements::PartListElement::ScorePart(sp) = group {
            names.insert(
                (*sp.attributes.id).clone(),
                decode_entities(&sp.content.part_name.content),
            );
        }
    }

    let mut parts = Vec::new();
    for part in &score.content.part {
        let id: String = (*part.attributes.id).clone();
        let name = names
            .get(&id)
            .cloned()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| id.clone());
        parts.push(parse_part(part, id, name));
    }

    let meta = score_meta(&parts);
    Score {
        work_title: score
            .content
            .work
            .as_ref()
            .and_then(|w| w.content.work_title.as_ref())
            .map(|t| decode_entities(&t.content)),
        parts,
        meta,
    }
}

/// Union tempo/meter/rehearsal marks across all parts, deduped by (qn) /
/// (qn, text) — port of `getScoreMeta`.
fn score_meta(parts: &[Part]) -> ScoreMeta {
    let mut tempos: Vec<TempoPoint> = Vec::new();
    let mut meters: Vec<MeterPoint> = Vec::new();
    let mut rehearsals: Vec<Marking> = Vec::new();
    let mut seen_t = std::collections::HashSet::new();
    let mut seen_m = std::collections::HashSet::new();
    let mut seen_r = std::collections::HashSet::new();
    for p in parts {
        for t in &p.tempos {
            if seen_t.insert(format!("{:.4}", t.qn)) {
                tempos.push(*t);
            }
        }
        for m in &p.meters {
            if seen_m.insert(m.qn_x10000) {
                meters.push(*m);
            }
        }
        for mk in &p.markings {
            if let MarkingKind::Rehearsal(text) = &mk.kind {
                if seen_r.insert(format!("{:.4}:{}", mk.qn, text)) {
                    rehearsals.push(mk.clone());
                }
            }
        }
    }
    tempos.sort_by(|a, b| a.qn.total_cmp(&b.qn));
    meters.sort_by_key(|m| m.qn_x10000);
    rehearsals.sort_by(|a, b| a.qn.total_cmp(&b.qn));
    ScoreMeta {
        tempos,
        meters,
        rehearsals,
    }
}

struct PendingGrace {
    pitch: i32,
    /// Grace lead time in QN, by written note type (16th=0.15, eighth=0.25, else 0.2).
    lead_qn: f64,
}

fn parse_part(part: &musicxml::elements::Part, id: String, name: String) -> Part {
    let mut notes: Vec<RawNote> = Vec::new();
    let mut dynamics: Vec<DynamicPoint> = Vec::new();
    let mut tempos: Vec<TempoPoint> = Vec::new();
    let mut meters: Vec<MeterPoint> = Vec::new();
    let mut markings: Vec<Marking> = Vec::new();
    let mut harmonies: Vec<HarmonyPoint> = Vec::new();
    let mut pending_graces: Vec<PendingGrace> = Vec::new();

    let mut divisions: f64 = 1.0;
    let mut measure_start_qn: f64 = 0.0;
    // Time signatures are INHERITED: once set they apply until changed.
    // Default 4/4. `senza-misura` only hides the display; it must NOT reset
    // the inherited meter (or sparse measures mis-size and everything drifts).
    let mut cur_beats: u32 = 4;
    let mut cur_beat_type: u32 = 4;
    let mut time_sig_qn: f64 = 4.0;
    let mut cur_fifths: i32 = 0;

    for pel in &part.content {
        let PartElement::Measure(measure) = pel else {
            continue;
        };
        let mut cursor: f64 = 0.0; // divisions ticks from measure start
        let mut max_cursor: f64 = 0.0;
        let mut prev_onset_cursor: f64 = 0.0; // onset of last non-chord note

        for el in &measure.content {
            match el {
                MeasureElement::Attributes(attrs) => {
                    if let Some(d) = &attrs.content.divisions {
                        divisions = f64::from((*d.content).max(1));
                    }
                    if let Some(key) = attrs.content.key.first() {
                        if let Some(f) = key_fifths(key) {
                            cur_fifths = f;
                        }
                    }
                    if let Some(time) = attrs.content.time.first() {
                        if time.content.senza_misura.is_none() {
                            if let Some((beats, bt)) = time_signature(time) {
                                time_sig_qn = beats as f64 * 4.0 / bt as f64;
                                cur_beats = beats;
                                cur_beat_type = bt;
                                meters.push(MeterPoint::new(measure_start_qn, beats, bt));
                            }
                        }
                        // senza-misura: intentionally ignored (meter stays inherited)
                    }
                }

                MeasureElement::Harmony(h) => {
                    if let Some(hp) = harmony_point(h, measure_start_qn + cursor / divisions) {
                        harmonies.push(hp);
                    }
                }

                MeasureElement::Sound(snd) => {
                    if let Some(bpm) = &snd.attributes.tempo {
                        tempos.push(TempoPoint {
                            qn: measure_start_qn + cursor / divisions,
                            bpm: **bpm,
                        });
                    }
                }

                MeasureElement::Backup(b) => {
                    cursor -= f64::from(*b.content.duration.content);
                    if cursor < 0.0 {
                        cursor = 0.0;
                    }
                }

                MeasureElement::Forward(f) => {
                    cursor += f64::from(*f.content.duration.content);
                    if cursor > max_cursor {
                        max_cursor = cursor;
                    }
                }

                MeasureElement::Direction(dir) => {
                    let off = dir
                        .content
                        .offset
                        .as_ref()
                        .map(|o| f64::from(*o.content))
                        .unwrap_or(0.0);
                    let qn = measure_start_qn + (cursor + off) / divisions;
                    let snd = dir.content.sound.as_ref();
                    if let Some(bpm) = snd.and_then(|s| s.attributes.tempo.as_ref()) {
                        tempos.push(TempoPoint { qn, bpm: **bpm });
                        markings.push(Marking {
                            qn,
                            kind: MarkingKind::Tempo(**bpm),
                        });
                    }

                    // dynamics: numeric from <sound>, name from the <dynamics> mark
                    let mut value: Option<f64> = snd
                        .and_then(|s| s.attributes.dynamics.as_ref())
                        .map(|d| **d);
                    let mut dyn_name: Option<String> = None;
                    for dt in &dir.content.direction_type {
                        direction_type_events(dt, qn, &mut value, &mut dyn_name, &mut markings);
                    }
                    if let Some(v) = value {
                        dynamics.push(DynamicPoint { qn, value: v });
                        markings.push(Marking {
                            qn,
                            kind: MarkingKind::Dynamic {
                                name: dyn_name,
                                value: v,
                            },
                        });
                    }
                }

                MeasureElement::Note(note) => {
                    handle_note(
                        note,
                        NoteCtx {
                            divisions,
                            measure_start_qn,
                            cur_beats,
                            cur_beat_type,
                            cur_fifths,
                        },
                        &mut cursor,
                        &mut max_cursor,
                        &mut prev_onset_cursor,
                        &mut pending_graces,
                        &mut notes,
                    );
                }

                _ => {}
            }
        }

        // Advance by the inherited meter length. Sparse measures are still a
        // full bar; only extend past the meter if content overflows it
        // (malformed input), to avoid dropping notes.
        let content_qn = max_cursor / divisions;
        let len_qn = if content_qn > time_sig_qn {
            content_qn
        } else {
            time_sig_qn
        };
        measure_start_qn += len_qn;
    }

    if tempos.is_empty() {
        tempos.push(TempoPoint {
            qn: 0.0,
            bpm: 120.0,
        });
    }
    tempos.sort_by(|a, b| a.qn.total_cmp(&b.qn));
    harmonies.sort_by(|a, b| a.qn.total_cmp(&b.qn));

    Part {
        id,
        name,
        notes,
        dynamics,
        tempos,
        meters,
        markings,
        harmonies,
    }
}

#[derive(Clone, Copy)]
struct NoteCtx {
    divisions: f64,
    measure_start_qn: f64,
    cur_beats: u32,
    cur_beat_type: u32,
    cur_fifths: i32,
}

#[allow(clippy::too_many_arguments)]
fn handle_note(
    note: &Note,
    ctx: NoteCtx,
    cursor: &mut f64,
    max_cursor: &mut f64,
    prev_onset_cursor: &mut f64,
    pending_graces: &mut Vec<PendingGrace>,
    notes: &mut Vec<RawNote>,
) {
    let (is_grace, is_chord, audible, dur, ties) = note_shape(note);
    let onset_cursor = if is_chord {
        *prev_onset_cursor
    } else {
        *cursor
    };

    if is_grace {
        if let Some(AudibleType::Pitch(p)) = audible {
            pending_graces.push(PendingGrace {
                pitch: pitch_to_midi(p),
                lead_qn: grace_lead_qn(note),
            });
        }
    } else if let Some(AudibleType::Pitch(p)) = audible {
        let voice = note_voice(note);
        let (art, slur_start, slur_stop) = read_articulations(note);
        let onset = ctx.measure_start_qn + onset_cursor / ctx.divisions;

        // Flush pending grace notes as quick pickups just before this
        // principal note.
        if !pending_graces.is_empty() && !is_chord {
            let total: f64 = pending_graces.iter().map(|g| g.lead_qn).sum();
            let mut gt = onset - total;
            for g in pending_graces.drain(..) {
                let mut ga = ArtSet::default();
                ga.insert("grace");
                notes.push(RawNote {
                    onset: gt.max(0.0),
                    dur: g.lead_qn,
                    pitch: g.pitch,
                    voice,
                    tie_start: false,
                    tie_stop: false,
                    art: ga,
                    slur_start: false,
                    slur_stop: false,
                    beat_qn: onset_cursor / ctx.divisions,
                    beats: ctx.cur_beats,
                    beat_type: ctx.cur_beat_type,
                    fifths: ctx.cur_fifths,
                });
                gt += g.lead_qn;
            }
        }

        notes.push(RawNote {
            onset,
            dur: dur / ctx.divisions,
            pitch: pitch_to_midi(p),
            voice,
            tie_start: ties.0,
            tie_stop: ties.1,
            art,
            slur_start,
            slur_stop,
            beat_qn: onset_cursor / ctx.divisions,
            beats: ctx.cur_beats,
            beat_type: ctx.cur_beat_type,
            fifths: ctx.cur_fifths,
        });
    }

    if !is_chord && !is_grace {
        *prev_onset_cursor = onset_cursor;
        *cursor += dur;
        if *cursor > *max_cursor {
            *max_cursor = *cursor;
        }
    }
}

/// Extract (is_grace, is_chord, audible, duration_ticks, (tie_start, tie_stop)).
fn note_shape(note: &Note) -> (bool, bool, Option<&AudibleType>, f64, (bool, bool)) {
    match &note.content.info {
        NoteType::Grace(g) => match &g.info {
            musicxml::elements::GraceType::Cue(c) => (
                true,
                c.chord.is_some(),
                Some(&c.audible),
                0.0,
                (false, false),
            ),
            musicxml::elements::GraceType::Normal(n) => (
                true,
                n.chord.is_some(),
                Some(&n.audible),
                0.0,
                tie_flags(&n.tie),
            ),
        },
        NoteType::Cue(c) => (
            false,
            c.chord.is_some(),
            Some(&c.audible),
            f64::from(*c.duration.content),
            (false, false),
        ),
        NoteType::Normal(n) => (
            false,
            n.chord.is_some(),
            Some(&n.audible),
            f64::from(*n.duration.content),
            tie_flags(&n.tie),
        ),
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

fn note_voice(note: &Note) -> u32 {
    note.content
        .voice
        .as_ref()
        .and_then(|v| v.content.parse().ok())
        .unwrap_or(1)
}

/// Grace lead time in QN by written type (Lua: 16th=0.15, eighth=0.25, else 0.2).
fn grace_lead_qn(note: &Note) -> f64 {
    use musicxml::datatypes::NoteTypeValue;
    match note.content.r#type.as_ref().map(|t| &t.content) {
        Some(NoteTypeValue::Sixteenth) => 0.15,
        Some(NoteTypeValue::Eighth) => 0.25,
        _ => 0.2,
    }
}

fn pitch_to_midi(p: &musicxml::elements::Pitch) -> i32 {
    use musicxml::datatypes::Step;
    let step = match p.content.step.content {
        Step::C => 0,
        Step::D => 2,
        Step::E => 4,
        Step::F => 5,
        Step::G => 7,
        Step::A => 9,
        Step::B => 11,
    };
    let oct = *p.content.octave.content as i32;
    let alter = p
        .content
        .alter
        .as_ref()
        .map(|a| *a.content as i32)
        .unwrap_or(0);
    (oct + 1) * 12 + step + alter
}

fn key_fifths(key: &musicxml::elements::Key) -> Option<i32> {
    match &key.content {
        musicxml::elements::KeyContents::Explicit(e) => Some(*e.fifths.content as i32),
        _ => None,
    }
}

fn time_signature(time: &musicxml::elements::Time) -> Option<(u32, u32)> {
    let sig = time.content.beats.first()?;
    let beats: u32 = sig.beats.content.parse().ok()?;
    let bt: u32 = sig.beat_type.content.parse().ok()?;
    Some((beats, bt))
}

/// Collect articulation/technical/ornament/gliss/slur flags — port of
/// `readArticulations`.
fn read_articulations(note: &Note) -> (ArtSet, bool, bool) {
    let mut art = ArtSet::default();
    let mut slur_start = false;
    let mut slur_stop = false;
    for nots in &note.content.notations {
        for item in &nots.content.notations {
            match item {
                NotationContentTypes::Articulations(arts) => {
                    for a in &arts.content {
                        art.insert(articulation_tag(a));
                    }
                }
                NotationContentTypes::Technical(tech) => {
                    for t in &tech.content {
                        art.insert(technical_tag(t));
                    }
                }
                NotationContentTypes::Fermata(_) => art.insert("fermata"),
                NotationContentTypes::Ornaments(orn) => {
                    for o in &orn.content.ornaments {
                        art.insert(ornament_tag(o));
                    }
                }
                NotationContentTypes::Glissando(g) => {
                    art.insert("glissando");
                    match g.attributes.r#type {
                        StartStop::Start => art.insert("glissStart"),
                        StartStop::Stop => art.insert("glissStop"),
                    }
                }
                NotationContentTypes::Slide(s) => {
                    art.insert("glissando");
                    match s.attributes.r#type {
                        StartStop::Start => art.insert("glissStart"),
                        StartStop::Stop => art.insert("glissStop"),
                    }
                }
                NotationContentTypes::Slur(sl) => {
                    use musicxml::datatypes::StartStopContinue;
                    match sl.attributes.r#type {
                        StartStopContinue::Start => slur_start = true,
                        StartStopContinue::Stop => slur_stop = true,
                        StartStopContinue::Continue => {}
                    }
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
        ArticulationsType::OtherArticulation(_) => "other-articulation",
    }
}

fn technical_tag(t: &TechnicalContents) -> &'static str {
    use TechnicalContents as T;
    match t {
        T::UpBow(_) => "up-bow",
        T::DownBow(_) => "down-bow",
        T::Harmonic(_) => "harmonic",
        T::OpenString(_) => "open-string",
        T::ThumbPosition(_) => "thumb-position",
        T::Fingering(_) => "fingering",
        T::Pluck(_) => "pluck",
        T::DoubleTongue(_) => "double-tongue",
        T::TripleTongue(_) => "triple-tongue",
        T::Stopped(_) => "stopped",
        T::SnapPizzicato(_) => "snap-pizzicato",
        T::Fret(_) => "fret",
        T::StringNumber(_) => "string",
        T::HammerOn(_) => "hammer-on",
        T::PullOff(_) => "pull-off",
        T::Bend(_) => "bend",
        T::Tap(_) => "tap",
        T::Heel(_) => "heel",
        T::Toe(_) => "toe",
        T::Fingernails(_) => "fingernails",
        T::Hole(_) => "hole",
        T::Arrow(_) => "arrow",
        T::Handbell(_) => "handbell",
        T::BrassBend(_) => "brass-bend",
        T::Flip(_) => "flip",
        T::Smear(_) => "smear",
        T::Open(_) => "open",
        T::HalfMuted(_) => "half-muted",
        T::HarmonMute(_) => "harmon-mute",
        T::Golpe(_) => "golpe",
        T::OtherTechnical(_) => "other-technical",
    }
}

fn ornament_tag(o: &OrnamentType) -> &'static str {
    use OrnamentType as O;
    match o {
        O::TrillMark(_) => "trill-mark",
        O::Turn(_) => "turn",
        O::DelayedTurn(_) => "delayed-turn",
        O::InvertedTurn(_) => "inverted-turn",
        O::DelayedInvertedTurn(_) => "delayed-inverted-turn",
        O::VerticalTurn(_) => "vertical-turn",
        O::InvertedVerticalTurn(_) => "inverted-vertical-turn",
        O::Shake(_) => "shake",
        O::WavyLine(_) => "wavy-line",
        O::Mordent(_) => "mordent",
        O::InvertedMordent(_) => "inverted-mordent",
        O::Schleifer(_) => "schleifer",
        O::Tremolo(_) => "tremolo",
        O::Haydn(_) => "haydn",
        O::OtherOrnament(_) => "other-ornament",
    }
}

fn direction_type_events(
    dt: &DirectionType,
    qn: f64,
    value: &mut Option<f64>,
    dyn_name: &mut Option<String>,
    markings: &mut Vec<Marking>,
) {
    match &dt.content {
        DirectionTypeContents::Dynamics(dyns) => {
            for d in dyns {
                for item in &d.content {
                    if let Some(name) = dynamics_tag(item) {
                        if dyn_name.is_none() {
                            *dyn_name = Some(name.to_string());
                            if value.is_none() {
                                *value = dynamic_mark_value(name);
                            }
                        }
                    }
                }
            }
        }
        DirectionTypeContents::Rehearsal(rehs) => {
            for r in rehs {
                if !r.content.is_empty() {
                    markings.push(Marking {
                        qn,
                        kind: MarkingKind::Rehearsal(r.content.clone()),
                    });
                }
            }
        }
        DirectionTypeContents::Words(words) => {
            for w in words {
                if w.content.is_empty() {
                    continue;
                }
                markings.push(Marking {
                    qn,
                    kind: MarkingKind::Words(w.content.clone()),
                });
                // a dynamic typed as plain text (no <dynamics> / <sound dynamics>)
                if value.is_none() {
                    if let Some((v, name)) = dynamic_from_text(&w.content) {
                        *value = Some(v);
                        *dyn_name = Some(name);
                    }
                }
            }
        }
        _ => {}
    }
}

fn dynamics_tag(d: &DynamicsType) -> Option<&'static str> {
    use DynamicsType as D;
    Some(match d {
        D::P(_) => "p",
        D::Pp(_) => "pp",
        D::Ppp(_) => "ppp",
        D::Pppp(_) => "pppp",
        D::F(_) => "f",
        D::Ff(_) => "ff",
        D::Fff(_) => "fff",
        D::Ffff(_) => "ffff",
        D::Mp(_) => "mp",
        D::Mf(_) => "mf",
        D::Sf(_) => "sf",
        D::Sfz(_) => "sfz",
        D::Sffz(_) => "sffz",
        D::Fp(_) => "fp",
        D::Fz(_) => "fz",
        D::Rf(_) => "rf",
        D::Rfz(_) => "rfz",
        D::Sfp(_) => "sfp",
        _ => return None,
    })
}

fn harmony_point(h: &musicxml::elements::Harmony, qn: f64) -> Option<HarmonyPoint> {
    use musicxml::datatypes::Step;
    let sub = h.content.harmony.first()?;
    let root = sub.root.as_ref()?;
    let step = match root.content.root_step.content {
        Step::C => 0,
        Step::D => 2,
        Step::E => 4,
        Step::F => 5,
        Step::G => 7,
        Step::A => 9,
        Step::B => 11,
    };
    let alter = root
        .content
        .root_alter
        .as_ref()
        .map(|a| *a.content as i32)
        .unwrap_or(0);
    let kind = kind_tag(&sub.kind.content);
    let degrees = sub
        .degree
        .iter()
        .map(|d| {
            use musicxml::datatypes::DegreeTypeValue;
            HarmonyDegree {
                value: *d.content.degree_value.content as i32,
                alter: *d.content.degree_alter.content as i32,
                kind: match d.content.degree_type.content {
                    DegreeTypeValue::Add => DegreeKind::Add,
                    DegreeTypeValue::Alter => DegreeKind::Alter,
                    DegreeTypeValue::Subtract => DegreeKind::Subtract,
                },
            }
        })
        .collect();
    Some(HarmonyPoint {
        qn,
        root_pc: (step + alter).rem_euclid(12),
        kind,
        degrees,
    })
}

fn kind_tag(k: &musicxml::datatypes::KindValue) -> String {
    use musicxml::datatypes::KindValue as K;
    match k {
        K::Major => "major",
        K::Minor => "minor",
        K::Augmented => "augmented",
        K::Diminished => "diminished",
        K::Dominant => "dominant",
        K::MajorSeventh => "major-seventh",
        K::MinorSeventh => "minor-seventh",
        K::DiminishedSeventh => "diminished-seventh",
        K::AugmentedSeventh => "augmented-seventh",
        K::HalfDiminished => "half-diminished",
        K::MajorMinor => "major-minor",
        K::MajorSixth => "major-sixth",
        K::MinorSixth => "minor-sixth",
        K::DominantNinth => "dominant-ninth",
        K::MajorNinth => "major-ninth",
        K::MinorNinth => "minor-ninth",
        K::Dominant11th => "dominant-11th",
        K::Major11th => "major-11th",
        K::Minor11th => "minor-11th",
        K::Dominant13th => "dominant-13th",
        K::Major13th => "major-13th",
        K::Minor13th => "minor-13th",
        K::SuspendedSecond => "suspended-second",
        K::SuspendedFourth => "suspended-fourth",
        K::Neapolitan => "neapolitan",
        K::Italian => "italian",
        K::French => "french",
        K::German => "german",
        K::Pedal => "pedal",
        K::Power => "power",
        K::Tristan => "tristan",
        K::Other => "other",
        K::None => "none",
    }
    .to_string()
}
