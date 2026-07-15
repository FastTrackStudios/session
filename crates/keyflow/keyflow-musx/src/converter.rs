//! EnigmaXML → MusicXML tree conversion — a faithful port of
//! `musx2mxl/converter.py`.
//!
//! The Finale document (`root`) is read through [`crate::xml_in`] helpers that
//! stand in for the original lxml `find`/`xpath` calls; the MusicXML document is
//! built with the mutable tree in [`crate::xml_out`].

use std::collections::HashMap;

use roxmltree::Node;

use crate::helper::*;
use crate::xml_in::*;
use crate::xml_out::*;

const DIVISIONS: i64 = 16; // nb divisions per quarter note

// Finale bracket style
const PIANO_BRACE: &str = "3";

type N<'a, 'i> = Node<'a, 'i>;

fn parse_i64(s: Option<&str>) -> Option<i64> {
    s.and_then(|s| s.trim().parse().ok())
}

/// `math.ceil((edu * DIVISIONS) / 1024)` for an EDU offset string.
fn edu_offset(edu: &str) -> Option<String> {
    parse_i64(Some(edu)).map(|v| {
        let ceil = ((v * DIVISIONS) as f64 / 1024.0).ceil() as i64;
        ceil.to_string()
    })
}

/// `(edu * DIVISIONS) // 1024` (floor) for a duration in EDUs.
fn edu_duration(dura: i64) -> i64 {
    (dura * DIVISIONS).div_euclid(1024)
}

fn between(start: Option<&str>, val: &str, end: Option<&str>) -> bool {
    match (parse_i64(start), parse_i64(Some(val)), parse_i64(end)) {
        (Some(s), Some(v), Some(e)) => s <= v && v <= e,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Lookups
// ---------------------------------------------------------------------------

fn lookup_note_alter(root: N, entnum: &str) -> HashMap<String, bool> {
    let mut map = HashMap::new();
    for note_alter in grandchildren(root, "details", "noteAlter") {
        if attr(note_alter, "entnum").as_deref() == Some(entnum) && has_child(note_alter, "noteID")
        {
            if let Some(note_id) = path_text(note_alter, &["noteID"]) {
                let enharmonic = has_child(note_alter, "enharmonic");
                map.insert(note_id, enharmonic);
            }
        }
    }
    map
}

struct Expression {
    staff_assign: Option<String>,
    horz_edu_off: Option<String>,
    category_type: String,
    vert_meas_expr_align: Option<String>,
    text: String,
}

fn lookup_meas_expressions(root: N, meas_spec_cmper: &str) -> Vec<Expression> {
    let mut expressions = Vec::new();
    for mea in grandchildren(root, "others", "measExprAssign") {
        if attr(mea, "cmper").as_deref() != Some(meas_spec_cmper) || !has_child(mea, "textExprID") {
            continue;
        }
        let Some(text_expr_id) = path_text(mea, &["textExprID"]) else {
            continue;
        };
        let staff_assign = path_text(mea, &["staffAssign"]);
        let horz_edu_off = path_text(mea, &["horzEduOff"]);

        let Some(text_expr_def) = find_by_attr(root, "others", "textExprDef", "cmper", &text_expr_id)
        else {
            continue;
        };
        let Some(text_id_key) = path_text(text_expr_def, &["textIDKey"]) else {
            continue;
        };
        let vert_meas_expr_align = path_text(text_expr_def, &["vertMeasExprAlign"]);
        let Some(category_id) = path_text(text_expr_def, &["categoryID"]) else {
            continue;
        };

        let mut expression_text = None;
        let mut category_type = None;
        if let Some(text_block) = find_by_attr(root, "others", "textBlock", "cmper", &text_id_key) {
            if let Some(markings_category) =
                find_by_attr(root, "others", "markingsCategory", "cmper", &category_id)
            {
                let text_id = path_text(text_block, &["textID"]);
                category_type = path_text(markings_category, &["categoryType"]);
                if let Some(text_id) = text_id {
                    expression_text =
                        find_by_attr(root, "texts", "expression", "number", &text_id).and_then(text);
                }
            }
        } else {
            tracing::debug!("textBlock with cmper {text_id_key} not found.");
        }

        if let (Some(text_val), Some(category_type)) = (expression_text, category_type) {
            expressions.push(Expression {
                staff_assign,
                horz_edu_off,
                category_type,
                vert_meas_expr_align,
                text: text_val,
            });
        }
    }
    expressions
}

struct TxtRepeat {
    top_staff_only: bool,
    staff_list: Option<String>,
    rpt_text: Option<String>,
}

fn lookup_txt_repeats(root: N, meas_spec_cmper: &str) -> Vec<TxtRepeat> {
    let mut txt_repeats = Vec::new();
    for tra in grandchildren(root, "others", "textRepeatAssign") {
        if attr(tra, "cmper").as_deref() != Some(meas_spec_cmper) {
            continue;
        }
        let top_staff_only = has_child(tra, "topStaffOnly");
        let staff_list = path_text(tra, &["staffList"]);
        let repnum = path_text(tra, &["repnum"]);
        if let Some(repnum) = repnum {
            if let Some(trt) = find_by_attr(root, "others", "textRepeatText", "cmper", &repnum) {
                let rpt_text = path_text(trt, &["rptText"]);
                txt_repeats.push(TxtRepeat {
                    top_staff_only,
                    staff_list,
                    rpt_text,
                });
            } else {
                tracing::debug!("textRepeatText with cmper {repnum} not found.");
            }
        }
    }
    txt_repeats
}

struct MeasSmartShape {
    shape_type: Option<String>,
    start_meas: Option<String>,
    start_inst: Option<String>,
    start_edu: Option<String>,
    end_meas: Option<String>,
    end_inst: Option<String>,
    end_edu: Option<String>,
    start_entry: Option<String>,
}

fn lookup_meas_smart_shapes(root: N, meas_spec_cmper: &str) -> Vec<MeasSmartShape> {
    let mut shapes = Vec::new();
    for mark in grandchildren(root, "others", "smartShapeMeasMark") {
        if attr(mark, "cmper").as_deref() != Some(meas_spec_cmper) {
            continue;
        }
        let Some(shape_num) = path_text(mark, &["shapeNum"]) else {
            continue;
        };
        let Some(smart_shape) = find_by_attr(root, "others", "smartShape", "cmper", &shape_num)
        else {
            tracing::debug!("smartShape with cmper {shape_num} not found");
            continue;
        };
        shapes.push(MeasSmartShape {
            shape_type: path_text(smart_shape, &["shapeType"]),
            start_meas: path_text(smart_shape, &["startTermSeg", "endPt", "meas"]),
            start_inst: path_text(smart_shape, &["startTermSeg", "endPt", "inst"]),
            start_edu: path_text(smart_shape, &["startTermSeg", "endPt", "edu"]),
            start_entry: path_text(smart_shape, &["startTermSeg", "endPt", "entryNum"]),
            end_meas: path_text(smart_shape, &["endTermSeg", "endPt", "meas"]),
            end_inst: path_text(smart_shape, &["endTermSeg", "endPt", "inst"]),
            end_edu: path_text(smart_shape, &["endTermSeg", "endPt", "edu"]),
        });
    }
    shapes
}

fn lookup_block_text(root: N, id: &str) -> String {
    let Some(text_block) = find_by_attr(root, "others", "textBlock", "cmper", id) else {
        return String::new();
    };
    let Some(text_id) = path_text(text_block, &["textID"]) else {
        return String::new();
    };
    match find_by_attr(root, "texts", "blockText", "number", &text_id).and_then(text) {
        Some(t) => replace_music_symbols(&remove_styling_tags(&t)),
        None => {
            tracing::debug!("blockText with number {text_id} not found.");
            String::new()
        }
    }
}

fn lookup_suffix(root: N, suffix_cmper: Option<&str>) -> String {
    let mut suffix_str = String::new();
    if let Some(suffix_cmper) = suffix_cmper {
        for chord_suffix in grandchildren(root, "others", "chordSuffix") {
            if attr(chord_suffix, "cmper").as_deref() != Some(suffix_cmper)
                || !has_child(chord_suffix, "suffix")
            {
                continue;
            }
            if let Some(suffix) = path_text(chord_suffix, &["suffix"]) {
                if suffix == "209" {
                    suffix_str.push('b');
                } else if let Ok(code) = suffix.parse::<u32>() {
                    if code >= 20 {
                        if let Some(c) = char::from_u32(code) {
                            suffix_str.push(c);
                        }
                    } else {
                        suffix_str.push_str(&suffix);
                    }
                } else {
                    suffix_str.push_str(&suffix);
                }
            }
        }
    }
    suffix_str
}

struct Chord {
    root_scale_num: Option<String>,
    root_alter: Option<String>,
    show_alt_bass: bool,
    bass_scale_num: Option<String>,
    bass_alter: Option<String>,
    bass_position: Option<String>,
    suffix_text: String,
    horz_edu: Option<String>,
}

fn lookup_chords(root: N, staff_spec_cmper: &str, meas_spec_cmper: &str) -> Vec<Chord> {
    let mut chords = Vec::new();
    for ca in grandchildren(root, "details", "chordAssign") {
        if attr(ca, "cmper1").as_deref() != Some(staff_spec_cmper)
            || attr(ca, "cmper2").as_deref() != Some(meas_spec_cmper)
        {
            continue;
        }
        let root_scale_num = path_text(ca, &["rootScaleNum"]);
        let mut root_alter = path_text(ca, &["rootAlter"]);
        let show_alt_bass = has_child(ca, "showAltBass");
        let bass_scale_num = path_text(ca, &["bassScaleNum"]);
        let bass_alter = path_text(ca, &["bassAlter"]);
        let bass_position = path_text(ca, &["bassPosition"]);
        let suffix_cmper = path_text(ca, &["suffix"]);
        let horz_edu = path_text(ca, &["horzEdu"]);
        let mut suffix_text = lookup_suffix(root, suffix_cmper.as_deref());

        if suffix_text == "es" {
            suffix_text = String::new();
            root_alter = Some("-1".to_string());
        }
        if suffix_text == "is" {
            suffix_text = String::new();
            root_alter = Some("1".to_string());
        }

        chords.push(Chord {
            root_scale_num,
            root_alter,
            show_alt_bass,
            bass_scale_num,
            bass_alter,
            bass_position,
            suffix_text,
            horz_edu,
        });
    }
    chords
}

struct StaffGroup {
    start_inst: Option<String>,
    end_inst: Option<String>,
    full_name: Option<String>,
    abbrv_name: Option<String>,
    bracket_id: Option<String>,
}

fn lookup_staff_groups(root: N) -> Vec<StaffGroup> {
    let mut groups = Vec::new();
    for sg in grandchildren(root, "details", "staffGroup") {
        if sg.has_attribute("part") {
            continue;
        }
        let full_id = path_text(sg, &["fullID"]);
        let abbrv_id = path_text(sg, &["abbrvID"]);
        groups.push(StaffGroup {
            start_inst: path_text(sg, &["startInst"]),
            end_inst: path_text(sg, &["endInst"]),
            full_name: full_id.map(|id| lookup_block_text(root, &id)),
            abbrv_name: abbrv_id.map(|id| lookup_block_text(root, &id)),
            bracket_id: path_text(sg, &["bracket", "id"]),
        });
    }
    groups
}

/// `find_staff_group_name` — join the names of every group that spans the staff.
fn find_staff_group_name(full: bool, staff_spec_cmper: &str, staff_groups: &[StaffGroup]) -> Option<String> {
    let mut names = Vec::new();
    for group in staff_groups {
        let name = if full { &group.full_name } else { &group.abbrv_name };
        if between(group.start_inst.as_deref(), staff_spec_cmper, group.end_inst.as_deref()) {
            if let Some(name) = name {
                if !name.is_empty() {
                    names.push(name.clone());
                }
            }
        }
    }
    if names.is_empty() {
        None
    } else {
        Some(names.join(" "))
    }
}

fn get_piano_brace_staff_group<'g>(
    staff_spec_cmper: &str,
    staff_groups: &'g [StaffGroup],
) -> Option<&'g StaffGroup> {
    staff_groups.iter().find(|g| {
        g.bracket_id.as_deref() == Some(PIANO_BRACE)
            && g.start_inst != g.end_inst
            && between(g.start_inst.as_deref(), staff_spec_cmper, g.end_inst.as_deref())
    })
}

// ---------------------------------------------------------------------------
// Top-level conversion
// ---------------------------------------------------------------------------

/// Convert a parsed EnigmaXML document (+ optional metadata) to a MusicXML
/// document string.
pub fn convert(root: N, meta_root: Option<N>) -> String {
    let score_partwise = element("score-partwise", &[("version", "4.0")]);

    if let Some(meta_root) = meta_root {
        handle_meta_data(&score_partwise, meta_root);
    }
    let part_list = sub(&score_partwise, "part-list");

    let time_sig_do_abrv_common = find_path(
        root,
        &["options", "timeSignatureOptions", "timeSigDoAbrvCommon"],
    )
    .is_some();
    let time_sig_do_abrv_cut =
        find_path(root, &["options", "timeSignatureOptions", "timeSigDoAbrvCut"]).is_some();

    let staff_groups = lookup_staff_groups(root);

    let staff_specs: Vec<N> = grandchildren(root, "others", "staffSpec")
        .into_iter()
        .filter(|s| attr(*s, "cmper").as_deref().map_or(false, |c| c != "32767"))
        .collect();

    let mut i = 1;
    let mut part_ids: HashMap<String, String> = HashMap::new();
    for staff_spec in &staff_specs {
        let staff_spec = *staff_spec;
        let Some(staff_spec_cmper) = attr(staff_spec, "cmper") else {
            continue;
        };
        let inst_uuid = path_text(staff_spec, &["instUuid"]).unwrap_or_default();

        let full_name = match path_text(staff_spec, &["fullName"]) {
            Some(id) => Some(lookup_block_text(root, &id)),
            None => find_staff_group_name(true, &staff_spec_cmper, &staff_groups),
        };
        let abbrv_name = match path_text(staff_spec, &["abbrvName"]) {
            Some(id) => Some(lookup_block_text(root, &id)),
            None => find_staff_group_name(false, &staff_spec_cmper, &staff_groups),
        };

        let piano_staff_group = get_piano_brace_staff_group(&staff_spec_cmper, &staff_groups);
        if piano_staff_group.is_none()
            || piano_staff_group.unwrap().start_inst.as_deref() == Some(staff_spec_cmper.as_str())
        {
            let part_id = format!("P{i}");
            i += 1;
            part_ids.insert(staff_spec_cmper.clone(), part_id.clone());
            let score_part = sub_attrs(&part_list, "score-part", &[("id", &part_id)]);
            sub_text(&score_part, "part-name", full_name.as_deref().unwrap_or(""));
            if let Some(abbrv) = abbrv_name.as_deref().filter(|s| !s.is_empty()) {
                sub_text(&score_part, "part-abbreviation", abbrv);
            }

            let (instrument_name, instrument_sound) = translate_instrument(&inst_uuid);
            // Python guards with `if instrument_name:` — an empty name is falsy.
            if let Some(instrument_name) = instrument_name.filter(|s| !s.is_empty()) {
                let score_instrument =
                    sub_attrs(&score_part, "score-instrument", &[("id", &format!("{part_id}-I1"))]);
                sub_text(&score_instrument, "instrument-name", &instrument_name);
                if let Some(instrument_sound) = instrument_sound.filter(|s| !s.is_empty()) {
                    sub_text(&score_instrument, "instrument-sound", &instrument_sound);
                }
            }
        }
    }

    let mut handle_tempo = true;

    let meas_specs: Vec<N> = grandchildren(root, "others", "measSpec")
        .into_iter()
        .filter(|m| !m.has_attribute("shared") && !m.has_attribute("part"))
        .collect();
    let nb_measures = meas_specs.len();

    for staff_spec in &staff_specs {
        let staff_spec = *staff_spec;
        let Some(staff_spec_cmper) = attr(staff_spec, "cmper") else {
            continue;
        };
        let Some(part_id) = part_ids.get(&staff_spec_cmper).cloned() else {
            continue;
        };
        let part = sub_attrs(&score_partwise, "part", &[("id", &part_id)]);

        let piano_staff_group = get_piano_brace_staff_group(&staff_spec_cmper, &staff_groups);

        let transp_key_adjust =
            parse_i64(path_text(staff_spec, &["transposition", "keysig", "adjust"]).as_deref())
                .unwrap_or(0);
        let transp_interval =
            parse_i64(path_text(staff_spec, &["transposition", "keysig", "interval"]).as_deref())
                .unwrap_or(0);

        let mut current_key: Option<i64> = Some(-1);
        let mut current_beats: Option<String> = None;
        let mut current_divbeat: Option<String> = None;
        let mut current_clef_single: Option<String> = None;
        let mut current_clef_multi: Option<HashMap<i64, Option<String>>> = None;
        let mut ending_cnt = 0;

        for (meas_idx, meas_spec) in meas_specs.iter().enumerate() {
            let meas_spec = *meas_spec;
            let Some(meas_spec_cmper) = attr(meas_spec, "cmper") else {
                continue;
            };
            let measure = sub_attrs(&part, "measure", &[("number", &meas_spec_cmper)]);
            let beats = path_text(meas_spec, &["beats"]);
            let divbeat = path_text(meas_spec, &["divbeat"]);
            let key_ = path_text(meas_spec, &["keySig", "key"]);
            let mut barline_ = path_text(meas_spec, &["barline"]).unwrap_or_else(|| "normal".to_string());
            if meas_idx == nb_measures - 1 {
                barline_ = "final".to_string();
            }
            let for_rep_bar = has_child(meas_spec, "forRepBar");
            let bac_rep_bar = has_child(meas_spec, "bacRepBar");
            let bar_ending = has_child(meas_spec, "barEnding");
            let has_smart_shape = has_child(meas_spec, "hasSmartShape");
            let txt_repeats_flag = has_child(meas_spec, "txtRepeats");
            let has_chord = has_child(meas_spec, "hasChord");

            let txt_repeats = if txt_repeats_flag {
                lookup_txt_repeats(root, &meas_spec_cmper)
            } else {
                Vec::new()
            };
            let meas_smart_shapes = if has_smart_shape {
                lookup_meas_smart_shapes(root, &meas_spec_cmper)
            } else {
                Vec::new()
            };

            for txt_repeat in &txt_repeats {
                if (txt_repeat.top_staff_only && staff_spec_cmper == "1")
                    || txt_repeat.staff_list.as_deref() == Some(staff_spec_cmper.as_str())
                {
                    match txt_repeat.rpt_text.as_deref() {
                        Some("%") => {
                            let direction = sub_attrs(&measure, "direction", &[("placement", "above")]);
                            let dt = sub(&direction, "direction-type");
                            sub(&dt, "segno");
                        }
                        Some("\u{00DE}") => {
                            let direction = sub_attrs(&measure, "direction", &[("placement", "above")]);
                            let dt = sub(&direction, "direction-type");
                            sub(&dt, "coda");
                        }
                        other => {
                            let direction = sub_attrs(&measure, "direction", &[("placement", "below")]);
                            let dt = sub(&direction, "direction-type");
                            sub_text(&dt, "words", other.unwrap_or(""));
                        }
                    }
                }
            }

            for shape in &meas_smart_shapes {
                handle_meas_smart_shape(&measure, shape, &meas_spec_cmper, &staff_spec_cmper);
            }

            let key: Option<i64> = key_.as_deref().and_then(|k| k.parse().ok());
            // key_ present but non-numeric shouldn't happen; None means no keySig.
            let key = if key_.is_none() { None } else { key };

            let mut attributes: Option<El> = None;
            if meas_idx == 0 {
                attributes = Some(handle_divisions(&measure));
            }
            if key != current_key {
                attributes = Some(handle_key_change(
                    &measure,
                    attributes,
                    key,
                    transp_key_adjust,
                    transp_interval,
                ));
                current_key = key;
            }
            if beats != current_beats || divbeat != current_divbeat {
                attributes = Some(handle_time_change(
                    &measure,
                    attributes,
                    beats.as_deref().unwrap_or("0"),
                    divbeat.as_deref().unwrap_or("0"),
                    time_sig_do_abrv_common,
                    time_sig_do_abrv_cut,
                ));
                current_beats = beats.clone();
                current_divbeat = divbeat.clone();
            }

            if for_rep_bar || bar_ending {
                let left_barline = sub_attrs(&measure, "barline", &[("location", "left")]);
                if bar_ending {
                    ending_cnt += 1;
                    let ending = sub_attrs(
                        &left_barline,
                        "ending",
                        &[("number", &ending_cnt.to_string()), ("type", "start")],
                    );
                    set_text(&ending, &format!("{ending_cnt}."));
                }
                if for_rep_bar {
                    sub_text(&left_barline, "bar-style", "heavy-light");
                    sub_attrs(&left_barline, "repeat", &[("direction", "forward")]);
                }
            }

            let cur_beats_i = parse_i64(current_beats.as_deref()).unwrap_or(0);
            let cur_divbeat_i = parse_i64(current_divbeat.as_deref()).unwrap_or(0);

            if let Some(piano_staff_group) = piano_staff_group {
                let mut staff_id = 1;
                let mut clef_ids: HashMap<i64, Option<String>> = HashMap::new();
                let mut prev = false;
                let piano_staffs: Vec<String> = staff_specs
                    .iter()
                    .filter_map(|s| attr(*s, "cmper"))
                    .filter(|c| {
                        between(
                            piano_staff_group.start_inst.as_deref(),
                            c,
                            piano_staff_group.end_inst.as_deref(),
                        )
                    })
                    .collect();
                for piano_staff_spec_cmper in &piano_staffs {
                    if prev {
                        let backup = sub(&measure, "backup");
                        sub_text(
                            &backup,
                            "duration",
                            &((cur_beats_i * cur_divbeat_i * DIVISIONS) / 1024).to_string(),
                        );
                    }
                    if has_chord {
                        let chords = lookup_chords(root, piano_staff_spec_cmper, &meas_spec_cmper);
                        handle_chords(&measure, &chords, key, transp_key_adjust, staff_id);
                    }
                    let clef_id = process_gfholds(
                        piano_staff_spec_cmper,
                        &meas_spec_cmper,
                        Some(staff_id),
                        &measure,
                        root,
                        meas_spec,
                        &mut handle_tempo,
                        &barline_,
                        bac_rep_bar,
                        bar_ending,
                        ending_cnt,
                        cur_beats_i,
                        cur_divbeat_i,
                        key,
                        transp_key_adjust,
                        transp_interval,
                    );
                    clef_ids.insert(staff_id, clef_id);
                    staff_id += 1;
                    prev = true;
                }
                if current_clef_multi.as_ref() != Some(&clef_ids) {
                    attributes = Some(handle_multi_staff_clef_change(
                        root, &measure, attributes, &clef_ids,
                    ));
                    current_clef_multi = Some(clef_ids);
                }
            } else {
                if has_chord {
                    let chords = lookup_chords(root, &staff_spec_cmper, &meas_spec_cmper);
                    handle_chords(&measure, &chords, key, transp_key_adjust, 1);
                }
                let clef_id = process_gfholds(
                    &staff_spec_cmper,
                    &meas_spec_cmper,
                    None,
                    &measure,
                    root,
                    meas_spec,
                    &mut handle_tempo,
                    &barline_,
                    bac_rep_bar,
                    bar_ending,
                    ending_cnt,
                    cur_beats_i,
                    cur_divbeat_i,
                    key,
                    transp_key_adjust,
                    transp_interval,
                );
                if clef_id != current_clef_single {
                    attributes = Some(handle_clef_change(root, &measure, attributes, clef_id.as_deref()));
                    current_clef_single = clef_id;
                }
            }

            if let Some(attributes) = attributes {
                reorder_children(
                    &attributes,
                    &[
                        "footnote",
                        "level",
                        "divisions",
                        "key",
                        "time",
                        "staves",
                        "part-symbol",
                        "instruments",
                        "clef",
                        "staff-details",
                        "transpose",
                        "for-part",
                        "directive",
                        "measure-style",
                    ],
                );
            }
        }
    }

    serialize_document(&score_partwise)
}

fn handle_meas_smart_shape(
    measure: &El,
    shape: &MeasSmartShape,
    meas_spec_cmper: &str,
    staff_spec_cmper: &str,
) {
    let shape_type = shape.shape_type.as_deref();
    match shape_type {
        Some("cresc") => {
            if shape.start_meas.as_deref() == Some(meas_spec_cmper)
                && shape.start_inst.as_deref() == Some(staff_spec_cmper)
            {
                let direction = sub_attrs(measure, "direction", &[("placement", "below")]);
                let dt = sub(&direction, "direction-type");
                if let Some(edu) = shape.start_edu.as_deref().and_then(edu_offset) {
                    sub_text(&direction, "offset", &edu);
                }
                sub_attrs(&dt, "wedge", &[("type", "crescendo")]);
            }
            if shape.end_meas.as_deref() == Some(meas_spec_cmper)
                && shape.start_inst.as_deref() == Some(staff_spec_cmper)
            {
                let direction = sub_attrs(measure, "direction", &[("placement", "below")]);
                let dt = sub(&direction, "direction-type");
                if let Some(edu) = shape.end_edu.as_deref().and_then(edu_offset) {
                    sub_text(&direction, "offset", &edu);
                }
                sub_attrs(&dt, "wedge", &[("type", "stop")]);
            }
        }
        Some("decresc") => {
            if shape.start_meas.as_deref() == Some(meas_spec_cmper)
                && shape.end_inst.as_deref() == Some(staff_spec_cmper)
            {
                let direction = sub_attrs(measure, "direction", &[("placement", "below")]);
                let dt = sub(&direction, "direction-type");
                if let Some(edu) = shape.start_edu.as_deref().and_then(edu_offset) {
                    sub_text(&direction, "offset", &edu);
                }
                sub_attrs(&dt, "wedge", &[("type", "diminuendo")]);
            }
            if shape.end_meas.as_deref() == Some(meas_spec_cmper)
                && shape.end_inst.as_deref() == Some(staff_spec_cmper)
            {
                let direction = sub_attrs(measure, "direction", &[("placement", "below")]);
                let dt = sub(&direction, "direction-type");
                if let Some(edu) = shape.end_edu.as_deref().and_then(edu_offset) {
                    sub_text(&direction, "offset", &edu);
                }
                sub_attrs(&dt, "wedge", &[("type", "stop")]);
            }
        }
        Some("octaveUp") | Some("octaveDown") | Some("slurUp") | Some("trill")
        | Some("smartLine") | Some("dashLine") | Some("trillExt") | Some("solidLine") => {}
        _ => {
            if shape.start_entry.is_none() {
                tracing::debug!("unhandled smart shape: {:?}", shape_type);
            }
        }
    }
}

fn today_string() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let Ok(dur) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return String::new();
    };
    let days = (dur.as_secs() / 86_400) as i64;
    // Howard Hinnant's days-from-civil, inverted.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

fn add_credit(
    score_partwise: &El,
    page: i32,
    credit_type: &str,
    credit_words: &str,
    default_x: &str,
    default_y: &str,
    justify: &str,
    valign: &str,
    font_size: &str,
) {
    let credit = sub_attrs(score_partwise, "credit", &[("page", &page.to_string())]);
    sub_text(&credit, "credit-type", credit_type);
    let words = sub_attrs(
        &credit,
        "credit-words",
        &[
            ("default-x", default_x),
            ("default-y", default_y),
            ("justify", justify),
            ("valign", valign),
            ("font-size", font_size),
        ],
    );
    set_text(&words, credit_words);
}

fn handle_meta_data(score_partwise: &El, meta_root: N) {
    let identification = sub(score_partwise, "identification");
    let encoding = sub(&identification, "encoding");
    sub_text(
        &encoding,
        "software",
        &format!("musx2mxl {}", env!("CARGO_PKG_VERSION")),
    );
    let date = today_string();
    if !date.is_empty() {
        sub_text(&encoding, "encoding-date", &date);
    }

    if let Some(title) = find_path(meta_root, &["fileInfo", "title"]).and_then(text) {
        add_credit(
            score_partwise, 1, "title", &title, "616.935484", "1511.049022", "center", "top", "22",
        );
    }
    if let Some(subtitle) = find_path(meta_root, &["fileInfo", "subtitle"]).and_then(text) {
        add_credit(
            score_partwise, 1, "subtitle", &subtitle, "616.935484", "1453.898908", "center", "top",
            "14",
        );
    }
    if let Some(composer) = find_path(meta_root, &["fileInfo", "composer"]).and_then(text) {
        add_credit(
            score_partwise, 1, "composer", &composer, "1148.145796", "1411.049022", "right",
            "bottom", "10",
        );
    }
}

fn handle_divisions(measure: &El) -> El {
    let attributes = sub(measure, "attributes");
    sub_text(&attributes, "divisions", &DIVISIONS.to_string());
    attributes
}

struct ClefInfo {
    sign: String,
    line: String,
    clef_octave_change: String,
}

fn lookup_clef_info(root: N, clef_id: Option<&str>) -> ClefInfo {
    if let Some(clef_id) = clef_id {
        if let Some(clef_def) =
            find_by_attr(root, "clefOptions", "clefDef", "index", clef_id).or_else(|| {
                // clefOptions lives under <options>; search there too.
                find_path(root, &["options", "clefOptions"])
                    .map(|co| children(co, "clefDef"))
                    .and_then(|defs| defs.into_iter().find(|d| attr(*d, "index").as_deref() == Some(clef_id)))
            })
        {
            let clef_char = path_text(clef_def, &["clefChar"]);
            let (sign, clef_octave_change) = translate_clef_sign(clef_char.as_deref());
            let clef_y_disp = parse_i64(path_text(clef_def, &["clefYDisp"]).as_deref()).unwrap_or(0);
            let line = (5 + clef_y_disp.div_euclid(2)).to_string();
            return ClefInfo {
                sign,
                line,
                clef_octave_change: clef_octave_change.to_string(),
            };
        }
    }
    ClefInfo {
        sign: "G".to_string(),
        line: "2".to_string(),
        clef_octave_change: "0".to_string(),
    }
}

fn handle_multi_staff_clef_change(
    root: N,
    measure: &El,
    attributes: Option<El>,
    clef_ids: &HashMap<i64, Option<String>>,
) -> El {
    let attributes = attributes.unwrap_or_else(|| sub(measure, "attributes"));
    let mut staff_ids: Vec<&i64> = clef_ids.keys().collect();
    staff_ids.sort();
    for staff_id in staff_ids {
        let clef_info = lookup_clef_info(root, clef_ids[staff_id].as_deref());
        let clef = sub_attrs(&attributes, "clef", &[("number", &staff_id.to_string())]);
        sub_text(&clef, "sign", &clef_info.sign);
        sub_text(&clef, "line", &clef_info.line);
        if clef_info.clef_octave_change != "0" {
            sub_text(&clef, "clef-octave-change", &clef_info.clef_octave_change);
        }
    }
    attributes
}

fn handle_clef_change(root: N, measure: &El, attributes: Option<El>, clef_id: Option<&str>) -> El {
    let attributes = attributes.unwrap_or_else(|| sub(measure, "attributes"));
    let clef_info = lookup_clef_info(root, clef_id);
    let clef = sub(&attributes, "clef");
    sub_text(&clef, "sign", &clef_info.sign);
    sub_text(&clef, "line", &clef_info.line);
    if clef_info.clef_octave_change != "0" {
        sub_text(&clef, "clef-octave-change", &clef_info.clef_octave_change);
    }
    attributes
}

fn handle_chords(measure: &El, chords: &[Chord], key: Option<i64>, transp_key_adjust: i64, staff_id: i64) {
    for chord in chords {
        let suffix = translate_chord_suffix(Some(&chord.suffix_text));
        let harmony = sub(measure, "harmony");
        let chord_root = sub(&harmony, "root");
        let (step, alter) = translate_chord_step(
            key,
            transp_key_adjust,
            chord.root_scale_num.as_deref(),
            chord.root_alter.as_deref(),
        );
        sub_text(&chord_root, "root-step", &step);
        if alter != 0 {
            sub_text(&chord_root, "root-alter", &alter.to_string());
        }
        let kind = sub_text(&harmony, "kind", &suffix.kind);
        set_attr(&kind, "use-symbols", &suffix.use_symbols);
        set_attr(&kind, "parentheses-degrees", &suffix.parentheses_degrees);
        if !suffix.text.is_empty() {
            set_attr(&kind, "text", &suffix.text);
        }
        if chord.show_alt_bass {
            let bass = sub(&harmony, "bass");
            if chord.bass_position.as_deref() == Some("underRoot") {
                set_attr(&bass, "arrangement", "vertical");
            }
            let (bass_step, bass_alter) = translate_chord_step(
                key,
                transp_key_adjust,
                chord.bass_scale_num.as_deref(),
                chord.bass_alter.as_deref(),
            );
            sub_text(&bass, "bass-step", &bass_step);
            if bass_alter != 0 {
                sub_text(&bass, "bass-alter", &bass_alter.to_string());
            }
        }
        for degree in &suffix.degrees {
            let degree_ = sub(&harmony, "degree");
            sub_text(&degree_, "degree-value", &degree.degree_value.to_string());
            sub_text(&degree_, "degree-alter", &degree.degree_alter.to_string());
            sub_text(&degree_, "degree-type", &degree.degree_type);
        }
        if let Some(edu) = chord.horz_edu.as_deref().and_then(edu_offset) {
            sub_text(&harmony, "offset", &edu);
        }
        if staff_id > 1 {
            sub_text(&harmony, "staff", &staff_id.to_string());
        }
    }
}

fn handle_key_change(
    measure: &El,
    attributes: Option<El>,
    key: Option<i64>,
    transp_key_adjust: i64,
    transp_interval: i64,
) -> El {
    let (mode, fifths) = calculate_mode_and_key_fifths(key, transp_key_adjust);
    let attributes = attributes.unwrap_or_else(|| sub(measure, "attributes"));
    let key_ = sub(&attributes, "key");
    sub_text(&key_, "fifths", &fifths.to_string());
    sub_text(&key_, "mode", &mode);
    if transp_interval != 0 {
        let (diatonic, chromatic, octave_change) = calculate_transpose(transp_interval);
        let transpose = sub(&attributes, "transpose");
        sub_text(&transpose, "diatonic", &diatonic.to_string());
        sub_text(&transpose, "chromatic", &chromatic.to_string());
        if octave_change != 0 {
            sub_text(&transpose, "octave-change", &octave_change.to_string());
        }
    }
    attributes
}

fn handle_time_change(
    measure: &El,
    attributes: Option<El>,
    beats: &str,
    divbeat: &str,
    time_sig_do_abrv_common: bool,
    time_sig_do_abrv_cut: bool,
) -> El {
    let attributes = attributes.unwrap_or_else(|| sub(measure, "attributes"));
    let time_ = sub(&attributes, "time");
    let beats_ = sub(&time_, "beats");
    let beats_type = sub(&time_, "beat-type");
    let divbeat_i = parse_i64(Some(divbeat)).unwrap_or(0);
    let beats_i = parse_i64(Some(beats)).unwrap_or(0);
    if divbeat_i != 0 && divbeat_i % 1536 == 0 {
        set_text(&beats_type, "8");
        set_text(&beats_, &(beats_i * 3 * divbeat_i / 1536).to_string());
    } else if divbeat_i != 0 && 4096 % divbeat_i == 0 {
        set_text(&beats_, beats);
        set_text(&beats_type, &(4096 / divbeat_i).to_string());
        if beats == "4" && divbeat == "1024" && time_sig_do_abrv_common {
            set_attr(&time_, "symbol", "common");
        }
        if beats == "2" && divbeat == "2048" && time_sig_do_abrv_cut {
            set_attr(&time_, "symbol", "cut");
        }
    } else {
        tracing::debug!("Unknown divbeat {divbeat}");
    }
    attributes
}

#[allow(clippy::too_many_arguments)]
fn process_frame(
    root: N,
    measure: &El,
    frame_spec_cmper: &str,
    frame_num: i64,
    staff_id: Option<i64>,
    key: Option<i64>,
    transp_key_adjust: i64,
    transp_interval: i64,
) {
    let voice = match staff_id {
        None => frame_num,
        Some(sid) => (sid - 1) * 4 + frame_num,
    };
    for frame_spec in grandchildren(root, "others", "frameSpec") {
        if attr(frame_spec, "cmper").as_deref() != Some(frame_spec_cmper) {
            continue;
        }
        let start_entry = path_text(frame_spec, &["startEntry"]);
        let end_entry = path_text(frame_spec, &["endEntry"]);
        if let (Some(start_entry), Some(end_entry)) = (start_entry, end_entry) {
            process_frame_entries(
                root,
                measure,
                &start_entry,
                &end_entry,
                staff_id,
                voice,
                key,
                transp_key_adjust,
                transp_interval,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn process_frame_entries(
    root: N,
    measure: &El,
    start_entnum: &str,
    end_entnum: &str,
    staff_id: Option<i64>,
    voice: i64,
    key: Option<i64>,
    transp_key_adjust: i64,
    transp_interval: i64,
) {
    let mut tuplet_attributes: Vec<TupletAttr> = Vec::new();
    let mut current_entnum = start_entnum.to_string();
    loop {
        let Some(current_entry) = find_by_attr(root, "entries", "entry", "entnum", &current_entnum)
        else {
            return;
        };
        process_entry(
            root,
            measure,
            current_entry,
            staff_id,
            voice,
            key,
            transp_key_adjust,
            transp_interval,
            &mut tuplet_attributes,
        );
        if current_entnum == end_entnum {
            return;
        }
        match attr(current_entry, "next") {
            Some(next) if !next.is_empty() => current_entnum = next,
            _ => return,
        }
    }
}

fn handle_tuplet_start(root: N, entry: N, notations: &El, tuplet_attributes: &mut Vec<TupletAttr>) {
    let Some(entnum) = attr(entry, "entnum") else {
        return;
    };
    let mut idx = tuplet_attributes
        .iter()
        .filter_map(|t| t.number.parse::<i64>().ok())
        .max()
        .unwrap_or(0);

    for tuplet_def in grandchildren(root, "details", "tupletDef") {
        if attr(tuplet_def, "entnum").as_deref() != Some(entnum.as_str())
            || !has_child(tuplet_def, "symbolicNum")
        {
            continue;
        }
        idx += 1;
        let number = idx.to_string();
        let symbolic_num = parse_i64(path_text(tuplet_def, &["symbolicNum"]).as_deref()).unwrap_or(0);
        let symbolic_dur = parse_i64(path_text(tuplet_def, &["symbolicDur"]).as_deref()).unwrap_or(1);
        let ref_num = parse_i64(path_text(tuplet_def, &["refNum"]).as_deref()).unwrap_or(0);
        let ref_dur = parse_i64(path_text(tuplet_def, &["refDur"]).as_deref()).unwrap_or(1);

        let tuplet = sub_attrs(notations, "tuplet", &[("number", &number), ("type", "start")]);
        if idx > 1 {
            let (actual_type, _) = calculate_type_and_dots(symbolic_dur);
            let (normal_type, _) = calculate_type_and_dots(ref_dur);
            let tuplet_actual = sub(&tuplet, "tuplet-actual");
            sub_text(&tuplet_actual, "tuplet-number", &symbolic_num.to_string());
            sub_text(&tuplet_actual, "tuplet-type", actual_type.as_deref().unwrap_or(""));
            let tuplet_normal = sub(&tuplet, "tuplet-normal");
            sub_text(&tuplet_normal, "tuplet-number", &ref_num.to_string());
            sub_text(&tuplet_normal, "tuplet-type", normal_type.as_deref().unwrap_or(""));
        }

        tuplet_attributes.push(TupletAttr {
            symbolic_num,
            symbolic_dur,
            ref_num,
            ref_dur,
            count: 0.0,
            number,
        });
    }
}

fn handle_smart_shape_detail(root: N, entry: N, notations: &El) {
    let Some(entnum) = attr(entry, "entnum") else {
        return;
    };
    for mark in grandchildren(root, "details", "smartShapeEntryMark") {
        if attr(mark, "entnum").as_deref() != Some(entnum.as_str()) {
            continue;
        }
        let Some(shape_num) = path_text(mark, &["shapeNum"]) else {
            continue;
        };
        if let Some(smart_shape) = find_by_attr(root, "others", "smartShape", "cmper", &shape_num) {
            let shape_type = path_text(smart_shape, &["shapeType"]);
            let start_entry = path_text(smart_shape, &["startTermSeg", "endPt", "entryNum"]);
            if shape_type.as_deref() == Some("slurAuto") || shape_type.as_deref() == Some("slurUp") {
                let slur_type = if start_entry.as_deref() == Some(entnum.as_str()) {
                    "start"
                } else {
                    "stop"
                };
                sub_attrs(notations, "slur", &[("number", "1"), ("type", slur_type)]);
            }
        } else {
            tracing::debug!("Smart shape with cmper {shape_num} not found.");
        }
    }
}

struct ArticDetail {
    char_main: Option<String>,
}

fn lookup_artic_detail(root: N, entnum: &str) -> Vec<ArticDetail> {
    let mut details = Vec::new();
    for aa in grandchildren(root, "details", "articAssign") {
        if attr(aa, "entnum").as_deref() != Some(entnum) || !has_child(aa, "articDef") {
            continue;
        }
        let Some(artic_def_cmper) = path_text(aa, &["articDef"]) else {
            continue;
        };
        if let Some(artic_def) = find_by_attr(root, "others", "articDef", "cmper", &artic_def_cmper) {
            details.push(ArticDetail {
                char_main: path_text(artic_def, &["charMain"]),
            });
        }
    }
    details
}

struct LyricDetail {
    number: String,
    syllabic: String,
    extend: bool,
    text: String,
}

fn lookup_lyric_details(root: N, entnum: &str) -> Vec<LyricDetail> {
    let mut details = Vec::new();
    for lv in grandchildren(root, "details", "lyrDataVerse") {
        if attr(lv, "entnum").as_deref() != Some(entnum) || !has_child(lv, "syll") {
            continue;
        }
        let Some(lyric_number) = path_text(lv, &["lyricNumber"]) else {
            continue;
        };
        let Some(syll) = path_text(lv, &["syll"]).and_then(|s| s.parse::<i64>().ok()) else {
            continue;
        };
        match find_by_attr(root, "texts", "verse", "number", &lyric_number).and_then(text) {
            Some(verse) => {
                let (text_val, syllabic, extend) = find_nth_syllabic(&verse, syll);
                details.push(LyricDetail {
                    number: lyric_number,
                    syllabic,
                    extend,
                    text: text_val,
                });
            }
            None => tracing::debug!("Verse not found with number= {lyric_number}"),
        }
    }
    details
}

fn add_rest_to_empty_measure(root: N, measure: &El, meas_spec_cmper: &str, staff_id: Option<i64>) {
    let first_gfhold = grandchildren(root, "details", "gfhold")
        .into_iter()
        .find(|g| attr(*g, "cmper2").as_deref() == Some(meas_spec_cmper) && has_child(*g, "frame1"));
    let Some(first_gfhold) = first_gfhold else {
        return;
    };
    let Some(frame) = path_text(first_gfhold, &["frame1"]) else {
        return;
    };
    let frame_spec = grandchildren(root, "others", "frameSpec").into_iter().find(|f| {
        attr(*f, "cmper").as_deref() == Some(frame.as_str())
            && has_child(*f, "startEntry")
            && has_child(*f, "endEntry")
    });
    let Some(frame_spec) = frame_spec else {
        return;
    };
    let Some(start_entnum) = path_text(frame_spec, &["startEntry"]) else {
        return;
    };
    let Some(end_entnum) = path_text(frame_spec, &["endEntry"]) else {
        return;
    };

    let mut current_entnum: Option<String> = None;
    let mut next_entnum = start_entnum;
    let mut dura = 0;
    while current_entnum.as_deref() != Some(end_entnum.as_str()) {
        let Some(entry) = find_by_attr(root, "entries", "entry", "entnum", &next_entnum) else {
            break;
        };
        current_entnum = Some(next_entnum.clone());
        next_entnum = attr(entry, "next").unwrap_or_default();
        dura += parse_i64(path_text(entry, &["dura"]).as_deref()).unwrap_or(0);
        if next_entnum.is_empty() {
            break;
        }
    }

    let (type_name, nb_dots) = calculate_type_and_dots(dura);
    let note = sub(measure, "note");
    sub(&note, "rest");
    sub_text(&note, "duration", &edu_offset(&dura.to_string()).unwrap_or_default());
    let voice = match staff_id {
        Some(sid) => (sid - 1) * 4 + 1,
        None => 1,
    };
    sub_text(&note, "voice", &voice.to_string());
    if let Some(type_name) = type_name {
        sub_text(&note, "type", &type_name);
    }
    for _ in 0..nb_dots {
        sub(&note, "dot");
    }
    if let Some(sid) = staff_id {
        sub_text(&note, "staff", &sid.to_string());
    }
}

#[allow(clippy::too_many_arguments)]
fn process_gfholds(
    staff_spec_cmper: &str,
    meas_spec_cmper: &str,
    staff_id: Option<i64>,
    measure: &El,
    root: N,
    meas_spec: N,
    _handle_tempo: &mut bool,
    barline_: &str,
    bac_rep_bar: bool,
    bar_ending: bool,
    ending_cnt: i64,
    current_beats: i64,
    current_divbeat: i64,
    key: Option<i64>,
    transp_key_adjust: i64,
    transp_interval: i64,
) -> Option<String> {
    let mut clef_id: Option<String> = None;

    let gfholds: Vec<N> = grandchildren(root, "details", "gfhold")
        .into_iter()
        .filter(|g| {
            attr(*g, "cmper1").as_deref() == Some(staff_spec_cmper)
                && attr(*g, "cmper2").as_deref() == Some(meas_spec_cmper)
        })
        .collect();

    if gfholds.is_empty() {
        let first_clef = grandchildren(root, "details", "gfhold")
            .into_iter()
            .filter(|g| attr(*g, "cmper1").as_deref() == Some(staff_spec_cmper))
            .find_map(|g| path_text(g, &["clefID"]));
        clef_id = first_clef;
        add_rest_to_empty_measure(root, measure, meas_spec_cmper, staff_id);
    }

    if has_child(meas_spec, "hasExpr") {
        let expressions = lookup_meas_expressions(root, meas_spec_cmper);
        for mut expression in expressions {
            let placement = if expression.vert_meas_expr_align.as_deref() == Some("belowStaffOrEntry")
            {
                "below"
            } else {
                "above"
            };

            let is_this_staff = expression.staff_assign.as_deref() == Some(staff_spec_cmper);

            if expression.category_type == "misc" && is_this_staff {
                if translate_dynamics(&expression.text).is_some() {
                    expression.category_type = "dynamics".to_string();
                } else {
                    let direction = sub_attrs(measure, "direction", &[("placement", placement)]);
                    let dt = sub(&direction, "direction-type");
                    add_direction_offset(&direction, expression.horz_edu_off.as_deref());
                    if let Some(sid) = staff_id {
                        sub_text(&direction, "staff", &sid.to_string());
                    }
                    let words = sub_text(&dt, "words", &remove_styling_tags(&expression.text));
                    set_attr(&words, "font-style", "italic");
                }
            }

            if expression.category_type == "dynamics" && is_this_staff {
                if let Some(dynamic_name) = translate_dynamics(&expression.text) {
                    let direction = sub_attrs(measure, "direction", &[("placement", placement)]);
                    let dt = sub(&direction, "direction-type");
                    add_direction_offset(&direction, expression.horz_edu_off.as_deref());
                    if let Some(sid) = staff_id {
                        sub_text(&direction, "staff", &sid.to_string());
                    }
                    let dynamics = sub(&dt, "dynamics");
                    sub(&dynamics, &dynamic_name);
                }
            } else if expression.category_type == "tempoAlts" && is_this_staff {
                let direction = sub_attrs(measure, "direction", &[("placement", placement)]);
                let dt = sub(&direction, "direction-type");
                add_direction_offset(&direction, expression.horz_edu_off.as_deref());
                if let Some(sid) = staff_id {
                    sub_text(&direction, "staff", &sid.to_string());
                }
                let words = sub_text(&dt, "words", &remove_styling_tags(&expression.text));
                set_attr(&words, "font-style", "italic");
            } else if expression.category_type == "expressiveText" && is_this_staff {
                let direction = sub_attrs(measure, "direction", &[("placement", placement)]);
                let dt = sub(&direction, "direction-type");
                add_direction_offset(&direction, expression.horz_edu_off.as_deref());
                if let Some(sid) = staff_id {
                    sub_text(&direction, "staff", &sid.to_string());
                }
                let words = sub_text(&dt, "words", &remove_styling_tags(&expression.text));
                set_attr(&words, "font-style", "italic");
            } else if expression.category_type == "techniqueText" && is_this_staff {
                let direction = sub_attrs(measure, "direction", &[("placement", placement)]);
                let dt = sub(&direction, "direction-type");
                add_direction_offset(&direction, expression.horz_edu_off.as_deref());
                if let Some(sid) = staff_id {
                    sub_text(&direction, "staff", &sid.to_string());
                }
                let words = sub_text(&dt, "words", &remove_styling_tags(&expression.text));
                set_attr(&words, "font-style", "italic");
            } else if expression.category_type == "tempoMarks" {
                let marks = translate_tempo_marks(&expression.text);
                let direction = sub_attrs(measure, "direction", &[("placement", placement)]);
                if let Some(words) = marks.words.as_deref().filter(|w| !w.is_empty()) {
                    let dt = sub(&direction, "direction-type");
                    add_direction_offset(&direction, expression.horz_edu_off.as_deref());
                    sub_text(&dt, "words", words);
                }
                if let (Some(beat_unit), Some(per_minute)) = (marks.beat_unit, marks.per_minute) {
                    let dt = sub(&direction, "direction-type");
                    add_direction_offset(&direction, expression.horz_edu_off.as_deref());
                    let metronome = sub_attrs(
                        &dt,
                        "metronome",
                        &[("parentheses", marks.parentheses.as_deref().unwrap_or("no"))],
                    );
                    sub_text(&metronome, "beat-unit", &beat_unit);
                    if marks.has_dot {
                        sub(&metronome, "beat-unit-dot");
                    }
                    sub_text(&metronome, "per-minute", &per_minute);
                }
            }
            // rehearsalMarks: not yet handled (matches source `pass`).
        }
    }

    for gfhold in &gfholds {
        let gfhold = *gfhold;
        if let Some(cid) = path_text(gfhold, &["clefID"]) {
            clef_id = Some(cid);
        }

        let mut has_prev_frame = false;
        for frame_num in 1..=4 {
            if let Some(frame) = path_text(gfhold, &[&format!("frame{frame_num}")]) {
                if has_prev_frame {
                    let backup = sub(measure, "backup");
                    sub_text(
                        &backup,
                        "duration",
                        &((current_beats * current_divbeat * DIVISIONS) / 1024).to_string(),
                    );
                }
                process_frame(
                    root,
                    measure,
                    &frame,
                    frame_num,
                    staff_id,
                    key,
                    transp_key_adjust,
                    transp_interval,
                );
                has_prev_frame = true;
            }
        }
    }

    let barline = sub_attrs(measure, "barline", &[("location", "right")]);
    sub_text(
        &barline,
        "bar-style",
        &translate_bar_style(barline_, bac_rep_bar, bar_ending),
    );
    if bar_ending {
        let ending = sub_attrs(
            &barline,
            "ending",
            &[("number", &ending_cnt.to_string()), ("type", "stop")],
        );
        set_text(&ending, &format!("{ending_cnt}."));
    }
    if bac_rep_bar {
        sub_attrs(&barline, "repeat", &[("direction", "backward"), ("winged", "none")]);
    }

    clef_id
}

fn add_direction_offset(direction: &El, horz_edu_off: Option<&str>) {
    if let Some(edu) = horz_edu_off.and_then(edu_offset) {
        sub_text(direction, "offset", &edu);
    }
}

const NOTE_ORDER: &[&str] = &[
    "grace",
    "chord",
    "pitch",
    "unpitched",
    "rest",
    "cue",
    "duration",
    "tie",
    "instrument",
    "footnote",
    "level",
    "voice",
    "type",
    "dot",
    "accidental",
    "time-modification",
    "stem",
    "notehead",
    "notehead-text",
    "staff",
    "beam",
    "notations",
    "lyric",
    "play",
    "listen",
];

/// Emit tuplet stops + a `<time-modification>` for the current entry, mutating
/// `tuplet_attributes` to drop finished tuplets. Shared by note and rest paths.
fn apply_tuplets(note: &El, notations: &El, dura: i64, tuplet_attributes: &mut Vec<TupletAttr>) {
    if tuplet_attributes.is_empty() {
        return;
    }
    let is_nested = tuplet_attributes.len() > 1;
    count_tuplet(tuplet_attributes, dura);

    let mut actual_notes = 1_i64;
    let mut normal_notes = 1_i64;
    let mut finished: Vec<String> = Vec::new();
    for attributes in tuplet_attributes.iter() {
        actual_notes *= attributes.symbolic_num;
        normal_notes *= attributes.ref_num;
        if (attributes.count - attributes.symbolic_num as f64).abs() < 1e-9 {
            sub_attrs(
                notations,
                "tuplet",
                &[("number", &attributes.number), ("type", "stop")],
            );
            finished.push(attributes.number.clone());
        }
    }
    tuplet_attributes.retain(|a| !finished.contains(&a.number));

    let time_modification = sub(note, "time-modification");
    sub_text(&time_modification, "actual-notes", &actual_notes.to_string());
    sub_text(&time_modification, "normal-notes", &normal_notes.to_string());
    if is_nested {
        if let Some(first) = tuplet_attributes.first() {
            let (normal_type, _) = calculate_type_and_dots(first.symbolic_dur);
            sub_text(&time_modification, "normal-type", normal_type.as_deref().unwrap_or(""));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn process_entry(
    root: N,
    measure: &El,
    entry: N,
    staff_id: Option<i64>,
    voice: i64,
    key: Option<i64>,
    transp_key_adjust: i64,
    transp_interval: i64,
    tuplet_attributes: &mut Vec<TupletAttr>,
) {
    let dura = parse_i64(path_text(entry, &["dura"]).as_deref()).unwrap_or(0);
    let is_note = has_child(entry, "isNote");
    let note_detail = has_child(entry, "noteDetail");
    let lyric_detail = has_child(entry, "lyricDetail");
    let artic_detail = has_child(entry, "articDetail");
    let entnum = attr(entry, "entnum").unwrap_or_default();

    let note_alter_map = if note_detail {
        lookup_note_alter(root, &entnum)
    } else {
        HashMap::new()
    };
    let artic_details = if artic_detail {
        lookup_artic_detail(root, &entnum)
    } else {
        Vec::new()
    };

    let grace_note = has_child(entry, "graceNote");
    let tuplet_start = has_child(entry, "tupletStart");
    let smart_shape_detail = has_child(entry, "smartShapeDetail");

    if is_note {
        let notes = children(entry, "note");
        for (idx, note_) in notes.iter().enumerate() {
            let note_ = *note_;
            let note = sub(measure, "note");
            if idx == 0 && lyric_detail {
                for lyric_detail in lookup_lyric_details(root, &entnum) {
                    let lyric = sub_attrs(
                        &note,
                        "lyric",
                        &[("name", "verse"), ("number", &lyric_detail.number)],
                    );
                    sub_text(&lyric, "syllabic", &lyric_detail.syllabic);
                    sub_text(&lyric, "text", &lyric_detail.text);
                    if lyric_detail.extend {
                        sub(&lyric, "extend");
                    }
                }
            }
            if idx > 0 {
                sub(&note, "chord");
            }
            if grace_note {
                sub_attrs(&note, "grace", &[("slash", "no")]);
            }
            let pitch = sub(&note, "pitch");
            let harm_lev = parse_i64(path_text(note_, &["harmLev"]).as_deref()).unwrap_or(0);
            let harm_alt = parse_i64(path_text(note_, &["harmAlt"]).as_deref()).unwrap_or(0);
            let enharmonic = attr(note_, "id")
                .and_then(|id| note_alter_map.get(&id).copied())
                .unwrap_or(false);
            let (step_value, alter_value, octave_value) = calculate_step_alter_and_octave(
                harm_lev,
                harm_alt,
                key,
                transp_key_adjust,
                transp_interval,
                enharmonic,
            );
            sub_text(&pitch, "step", &step_value);
            if alter_value != 0 {
                sub_text(&pitch, "alter", &alter_value.to_string());
            }
            sub_text(&pitch, "octave", &octave_value);
            if !grace_note {
                sub_text(&note, "duration", &edu_duration(dura).to_string());
            }

            if has_child(note_, "tieStart") {
                sub_attrs(&note, "tie", &[("type", "start")]);
            }
            if has_child(note_, "tieEnd") {
                sub_attrs(&note, "tie", &[("type", "stop")]);
            }

            sub_text(&note, "voice", &voice.to_string());
            let (type_name, nb_dots) = calculate_type_and_dots(dura);
            if let Some(type_name) = &type_name {
                sub_text(&note, "type", type_name);
                for _ in 0..nb_dots {
                    sub(&note, "dot");
                }
            }
            if let Some(sid) = staff_id {
                sub_text(&note, "staff", &sid.to_string());
            }

            if idx == 0 {
                let notations = sub(&note, "notations");
                if smart_shape_detail {
                    handle_smart_shape_detail(root, entry, &notations);
                }
                if tuplet_start {
                    handle_tuplet_start(root, entry, &notations, tuplet_attributes);
                }
                apply_tuplets(&note, &notations, dura, tuplet_attributes);

                if artic_detail {
                    let articulations = sub(&notations, "articulations");
                    for art_detail in &artic_details {
                        let (tag_name, ty) =
                            translate_articulation(art_detail.char_main.as_deref().unwrap_or(""));
                        let articulation = sub(&articulations, &tag_name);
                        if let Some(ty) = ty {
                            set_attr(&articulation, "type", &ty);
                        }
                    }
                }

                if child_count(&notations) == 0 {
                    remove_child(&note, &notations);
                }
            }

            reorder_children(&note, NOTE_ORDER);
        }
    } else {
        let note = sub(measure, "note");
        sub(&note, "rest");
        sub_text(&note, "duration", &edu_duration(dura).to_string());
        sub_text(&note, "voice", &voice.to_string());
        let (type_name, nb_dots) = calculate_type_and_dots(dura);
        if let Some(type_name) = &type_name {
            sub_text(&note, "type", type_name);
            for _ in 0..nb_dots {
                sub(&note, "dot");
            }
            if let Some(sid) = staff_id {
                sub_text(&note, "staff", &sid.to_string());
            }
        }
        let notations = sub(&note, "notations");
        if smart_shape_detail {
            handle_smart_shape_detail(root, entry, &notations);
        }
        if tuplet_start {
            handle_tuplet_start(root, entry, &notations, tuplet_attributes);
        }
        apply_tuplets(&note, &notations, dura, tuplet_attributes);

        if child_count(&notations) == 0 {
            remove_child(&note, &notations);
        }

        reorder_children(&note, NOTE_ORDER);
    }
}
