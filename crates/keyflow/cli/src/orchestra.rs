//! `kf score` / `kf orchestrate` — score analysis + CSS orchestrator engine.

use std::path::Path;

use keyflow_orchestra::analysis::analyze;
use keyflow_orchestra::{Config, ProfileKind, detect_profile, process_part};

/// `kf score <file>` — instrumentation / articulation / structure report.
pub fn score_report(input: &Path) -> Result<(), String> {
    let score = keyflow_orchestra::score::load(input).map_err(|e| e.to_string())?;
    let report = analyze(&score);

    println!(
        "=== Score: {} ===",
        report.title.as_deref().unwrap_or("(untitled)")
    );
    println!("Length:   {:.1} QN", report.length_qn);
    if let Some((lo, hi)) = report.bpm_range {
        if (lo - hi).abs() < f64::EPSILON {
            println!("Tempo:    {lo:.0} bpm");
        } else {
            println!(
                "Tempo:    {lo:.0}–{hi:.0} bpm ({} changes)",
                report.tempo_changes
            );
        }
    }
    let meters: Vec<String> = report
        .meters
        .iter()
        .map(|(n, d)| format!("{n}/{d}"))
        .collect();
    if !meters.is_empty() {
        println!("Meters:   {}", meters.join(", "));
    }
    if !report.rehearsal_marks.is_empty() {
        println!("Marks:    {}", report.rehearsal_marks.join(", "));
    }
    println!("\nParts ({}):", report.parts.len());
    for p in &report.parts {
        let range = p
            .range
            .map(|(lo, hi)| format!("{}–{}", note_name(lo), note_name(hi)))
            .unwrap_or_else(|| "-".into());
        let mut flags: Vec<&str> = Vec::new();
        if p.has_slurs {
            flags.push("slurs");
        }
        if p.has_grace {
            flags.push("grace");
        }
        if p.has_gliss {
            flags.push("gliss");
        }
        if p.max_chord_width > 1 {
            flags.push("divisi");
        }
        println!(
            "  {:<24} [{:<13}] {:>5} notes  range {:<9} voices={} width={}{}{}",
            p.name,
            p.profile.name(),
            p.note_count,
            range,
            p.voices.len(),
            p.max_chord_width,
            if flags.is_empty() { "" } else { "  " },
            flags.join(",")
        );
        if !p.articulations.is_empty() {
            let arts: Vec<&str> = p.articulations.iter().map(String::as_str).collect();
            println!("    articulations: {}", arts.join(", "));
        }
        if !p.dynamics.is_empty() {
            let dyns: Vec<&str> = p.dynamics.iter().map(String::as_str).collect();
            println!("    dynamics: {}", dyns.join(", "));
        }
    }
    Ok(())
}

/// `kf orchestrate <file>` — run the engine on every (or one) part and
/// summarize what would be written.
pub fn orchestrate(
    input: &Path,
    part_filter: Option<&str>,
    profile_override: Option<&str>,
) -> Result<(), String> {
    let score = keyflow_orchestra::score::load(input).map_err(|e| e.to_string())?;
    let mut total_notes = 0usize;
    let mut total_ccs = 0usize;
    for part in &score.parts {
        if let Some(f) = part_filter {
            if !part.name.to_lowercase().contains(&f.to_lowercase()) {
                continue;
            }
        }
        let mut cfg = Config::default();
        cfg.profile = match profile_override {
            Some(p) => parse_profile(p)?,
            None => detect_profile(&part.name),
        };
        cfg.tempo_map = Some(score.meta.tempos.clone());
        let out = process_part(part, &cfg);
        if out.empty {
            println!("--    '{}' (no notes)", part.name);
            continue;
        }
        let channels: Vec<String> = out.channels.iter().map(|c| c.to_string()).collect();
        println!(
            "OK    '{}'  [{}]  notes={} cc={} ch={{{}}}  span {:.1}..{:.1} QN",
            part.name,
            cfg.profile.name(),
            out.notes.len(),
            out.ccs.len(),
            channels.join(","),
            out.start_qn,
            out.end_qn
        );
        total_notes += out.notes.len();
        total_ccs += out.ccs.len();
    }
    println!("\nTotal: {total_notes} notes, {total_ccs} CC events");
    Ok(())
}

fn parse_profile(s: &str) -> Result<ProfileKind, String> {
    Ok(match s {
        "strings" => ProfileKind::Strings,
        "woodwinds" => ProfileKind::Woodwinds,
        "brass" => ProfileKind::Brass,
        "brass_trumpet" => ProfileKind::BrassTrumpet,
        "generic" => ProfileKind::Generic,
        "harp" => ProfileKind::Harp,
        other => return Err(format!("unknown profile '{other}'")),
    })
}

fn note_name(midi: i32) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!("{}{}", NAMES[midi.rem_euclid(12) as usize], midi / 12 - 1)
}
