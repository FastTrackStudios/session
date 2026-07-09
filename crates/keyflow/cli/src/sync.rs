//! `kf sync` / `kf bench` / `kf models` — lyrics-to-audio alignment glue.
//!
//! The heavy work lives in the `keyflow-sync` crate; this module only adapts it
//! to the CLI and bridges the parsed `Chart` to the chart-agnostic
//! `SectionLyrics` the aligner consumes. Embedded ONNX inference is gated behind
//! the `onnx` cargo feature so the stock `kf` binary pulls no native lib.

use std::path::Path;

use keyflow::Chart;
use keyflow_sync::karaoke::{VoiceNote, VoiceTrack};
use keyflow_sync::pipeline::SectionLyrics;

/// Notated duration of a melody note in quarter-note beats.
fn note_quarters(note: &keyflow::chart::melody::MelodyNote) -> f32 {
    let base = 4.0 / (note.duration.max(1) as f32);
    let base = if note.dotted { base * 1.5 } else { base };
    if note.triplet { base * 2.0 / 3.0 } else { base }
}

/// Extract the chart's `voice` melody tracks into per-voice MIDI note lists with
/// **notated** timing (from durations + tempo + section order). Each distinct
/// voice name gets its own MIDI channel (skipping ch 9 / drums) and keeps its
/// color. Pitch is resolved via keyflow's relative-octave logic → MIDI number.
pub fn chart_to_voices(chart: &Chart, bpm: f32, time_sig: (u8, u8)) -> Vec<VoiceTrack> {
    use std::collections::HashMap;
    let spq = 60.0 / bpm; // seconds per quarter note
    let measure_quarters = time_sig.0 as f32 * (4.0 / time_sig.1 as f32);

    let mut order: Vec<String> = Vec::new();
    let mut by_voice: HashMap<String, Vec<VoiceNote>> = HashMap::new();
    let mut colors: HashMap<String, Option<String>> = HashMap::new();
    let mut sec_start = 0.0f32;

    for section in &chart.sections {
        let mcount = section
            .section
            .measure_count
            .unwrap_or_else(|| section.measures().len());
        for track in section.melody_tracks() {
            let Some(melody) = &track.melody else {
                continue;
            };
            let name = track.name.clone().unwrap_or_else(|| "voice".into());
            if !order.contains(&name) {
                order.push(name.clone());
                colors.insert(name.clone(), track.color.clone());
            }
            let mut m = melody.clone();
            m.resolve_absolute_octaves();
            let mut t = sec_start;
            let bucket = by_voice.entry(name).or_default();
            for note in &m.notes {
                let dur_s = note_quarters(note) * spq;
                if !note.is_rest() {
                    if let Some(pc) = note.pitch_class() {
                        let oct = note.octave.unwrap_or(4) as i16;
                        let midi = ((oct + 1) * 12 + pc as i16).clamp(0, 127) as u8;
                        bucket.push(VoiceNote {
                            midi,
                            lyric: note.lyric.clone(),
                            start: t,
                            end: t + dur_s,
                        });
                    }
                }
                t += dur_s;
            }
        }
        sec_start += mcount as f32 * measure_quarters * spq;
    }

    order
        .into_iter()
        .enumerate()
        .map(|(i, name)| {
            let mut ch = i as u8;
            if ch >= 9 {
                ch += 1; // keep voices off the GM drum channel
            }
            let notes = by_voice.remove(&name).unwrap_or_default();
            let color = colors.get(&name).cloned().flatten();
            VoiceTrack {
                name,
                color,
                channel: ch & 0x0F,
                notes,
            }
        })
        .collect()
}

/// Options for `kf voices`.
pub struct VoicesOpts<'a> {
    pub chart: &'a Path,
    pub out_dir: &'a Path,
    pub reaper: bool,
}

/// `kf voices` — emit a multi-voice melody+lyrics MIDI (one track/channel per
/// colored vocal part) from a chart's `voice` lanes. Notated timing; no audio
/// needed. With `--reaper`, also writes a colored per-voice Reaper project.
pub fn cmd_voices(opts: VoicesOpts<'_>) -> Result<(), String> {
    use keyflow_sync::karaoke::voices_midi;

    let text = std::fs::read_to_string(opts.chart).map_err(|e| e.to_string())?;
    let chart = keyflow::parse(text.as_str()).map_err(|e| e.to_string())?;
    let lane = keyflow_sync::lanes::parse(&text).map_err(|e| e.to_string())?;
    let voices = chart_to_voices(&chart, lane.bpm, lane.time_sig);
    if voices.is_empty() {
        return Err("chart has no `voice {}` lanes (melody + lyrics)".into());
    }
    for v in &voices {
        let lyric_notes = v.notes.iter().filter(|n| n.lyric.is_some()).count();
        println!(
            "voice {:<8} ch{:<2} color {:<8} {} notes ({} syllables)",
            v.name,
            v.channel,
            v.color.as_deref().unwrap_or("-"),
            v.notes.len(),
            lyric_notes
        );
    }

    std::fs::create_dir_all(opts.out_dir).map_err(|e| e.to_string())?;
    let stem = opts
        .chart
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "song".into());
    let midi = voices_midi(&voices, lane.bpm).map_err(|e| e.to_string())?;
    let mid_path = opts.out_dir.join(format!("{stem}.mid"));
    std::fs::write(&mid_path, &midi).map_err(|e| e.to_string())?;
    println!("wrote {}", mid_path.display());

    if opts.reaper {
        #[cfg(feature = "reaper")]
        {
            let rpp = export_reaper_voices(&voices, lane.bpm, lane.time_sig)?;
            let rpp_path = opts.out_dir.join(format!("{stem}.rpp"));
            std::fs::write(&rpp_path, rpp).map_err(|e| e.to_string())?;
            println!("wrote {}", rpp_path.display());
        }
        #[cfg(not(feature = "reaper"))]
        eprintln!("note: --reaper needs the `reaper` feature; skipping .rpp");
    }
    Ok(())
}

/// Pull each section's lyrics out of a parsed chart, reconstructing words from
/// the syllable stream (`word_initial` marks word boundaries). The `.kf`
/// remains the source of truth — this is a read-only projection for alignment.
// Consumed only by the ONNX alignment path; lean builds compile it but don't call it.
#[cfg_attr(not(feature = "onnx"), allow(dead_code))]
pub fn extract_section_lyrics(chart: &Chart) -> Vec<SectionLyrics> {
    let mut out = Vec::new();
    for (i, section) in chart.sections.iter().enumerate() {
        let Some(line) = section.lyrics_track().and_then(|t| t.lyrics.as_ref()) else {
            continue;
        };
        if line.syllables.is_empty() {
            continue;
        }
        let mut words: Vec<String> = Vec::new();
        let mut cur = String::new();
        for syl in &line.syllables {
            if syl.word_initial && !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
            cur.push_str(&syl.text);
        }
        if !cur.is_empty() {
            words.push(cur);
        }
        if !words.is_empty() {
            out.push(SectionLyrics {
                section: i as u32,
                text: words.join(" "),
            });
        }
    }
    out
}

// ---- kf models --------------------------------------------------------------

pub fn cmd_models_list() -> Result<(), String> {
    let reg = keyflow_sync::models::load_registry().map_err(|e| e.to_string())?;
    println!(
        "Models cache: {}\n",
        keyflow_sync::models::models_dir().display()
    );
    for m in &reg.models {
        let present = if keyflow_sync::models::is_present(m) {
            "✓ cached"
        } else {
            "  remote"
        };
        println!("{present}  {:<12} {:?}  — {}", m.id, m.kind, m.description);
    }
    Ok(())
}

pub fn cmd_models_pull(id: &str) -> Result<(), String> {
    let reg = keyflow_sync::models::load_registry().map_err(|e| e.to_string())?;
    let entry = keyflow_sync::models::find(&reg, id)
        .ok_or_else(|| format!("no model with id {id:?} in the registry (try `kf models list`)"))?;
    let path = keyflow_sync::models::pull(&entry).map_err(|e| e.to_string())?;
    println!("{id} ready at {}", path.display());
    Ok(())
}

// ---- kf bench ---------------------------------------------------------------

pub fn cmd_bench(reference: &Path, estimate: &Path, tool: &str) -> Result<(), String> {
    use keyflow_sync::AudioBuffer;
    let r = AudioBuffer::load_wav(reference).map_err(|e| e.to_string())?;
    let e = AudioBuffer::load_wav(estimate).map_err(|e| e.to_string())?;
    let scores =
        keyflow_sync::bench::score_stem(&r.samples, &e.samples).map_err(|x| x.to_string())?;
    println!("tool        SI-SDR     SDR");
    println!(
        "{tool:<10}  {:>6.1} dB  {:>6.1} dB",
        scores.si_sdr, scores.sdr
    );
    Ok(())
}

// ---- kf sync ----------------------------------------------------------------

/// Options for `kf sync`, threaded from the clap subcommand.
#[cfg_attr(not(feature = "onnx"), allow(dead_code))]
pub struct SyncOpts<'a> {
    pub audio: &'a Path,
    pub output: &'a Path,
    pub separate: bool,
    pub model_id: &'a str,
    /// Align against this plain-text lyrics file (one global section) instead of
    /// the chart's lyrics. Useful when the chart has no lyrics track yet, or to
    /// align a known song-order transcript.
    pub lyrics_file: Option<&'a Path>,
    /// Confidence cutoffs for the verified/review/failed rating buckets.
    pub thresholds: keyflow_sync::RatingThresholds,
    /// `<star>` token log-prob (None disables it). Absorbs non-transcript audio
    /// (ad-libs, instrumental, repeats) so mismatches don't drag words off.
    pub star_score: Option<f32>,
    /// Align each section independently against the remaining audio (anchored),
    /// instead of one global monotonic pass. Robust to recordings that repeat
    /// sections more than the transcript lists.
    pub per_section: bool,
}

#[cfg(feature = "onnx")]
pub fn cmd_sync(chart: &Chart, opts: SyncOpts<'_>) -> Result<(), String> {
    use keyflow_sync::align::tokenizer::Vocab;
    use keyflow_sync::align::wav2vec2::Wav2Vec2Onnx;
    use keyflow_sync::audio::AudioBuffer;
    use keyflow_sync::models;
    use keyflow_sync::separator::demucs::DemucsOnnx;
    use keyflow_sync::{TimingMap, pipeline};

    let sections = match opts.lyrics_file {
        Some(path) => {
            let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            // Split on blank lines into stanzas → one section each (so anchored
            // mode can place each independently). Within a stanza, collapse
            // newlines to spaces.
            text.split("\n\n")
                .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|s| !s.is_empty())
                .enumerate()
                .map(|(i, text)| pipeline::SectionLyrics {
                    section: i as u32,
                    text,
                })
                .collect()
        }
        None => extract_section_lyrics(chart),
    };
    if sections.is_empty() {
        return Err("no lyrics to align: chart has no Lyrics track — pass --lyrics-file".into());
    }

    let audio = AudioBuffer::load_wav(opts.audio).map_err(|e| e.to_string())?;
    println!(
        "loaded {} ({:.1}s @ {} Hz)",
        opts.audio.display(),
        audio.duration_secs(),
        audio.sample_rate
    );

    // Resolve + load the aligner model from the cache.
    let reg = models::load_registry().map_err(|e| e.to_string())?;
    let entry = models::find(&reg, opts.model_id)
        .ok_or_else(|| format!("unknown model {:?} (see `kf models list`)", opts.model_id))?;
    let model_path = models::local_path(&entry);
    if !model_path.exists() {
        return Err(format!(
            "model {:?} not downloaded — run `kf models pull {}`",
            opts.model_id, opts.model_id
        ));
    }
    // Prefer a sibling <model>.vocab.json (HF {token:id} map); fall back to the
    // built-in wav2vec2 960h labels.
    let vocab_path = model_path.with_extension("vocab.json");
    let vocab = if vocab_path.exists() {
        let text = std::fs::read_to_string(&vocab_path).map_err(|e| e.to_string())?;
        Vocab::from_vocab_map_json(&text).map_err(|e| e.to_string())?
    } else {
        Vocab::wav2vec2_en_960h()
    };
    // wav2vec2 conv stride is 320 samples at 16 kHz → exactly 50 frames/sec.
    let model = Wav2Vec2Onnx::load(&model_path, vocab, 16_000, 50.0).map_err(|e| e.to_string())?;

    // Optional vocal isolation first — the biggest quality lever.
    let separator = if opts.separate {
        let sep_entry = models::find(&reg, "htdemucs-vocals")
            .ok_or_else(|| "no htdemucs-vocals entry in registry for --separate".to_string())?;
        let sep_path = models::local_path(&sep_entry);
        if !sep_path.exists() {
            return Err(
                "htdemucs-vocals model not downloaded — run `kf models pull htdemucs-vocals`"
                    .into(),
            );
        }
        println!("separating vocals (htdemucs-ft, this is the slow part)…");
        Some(DemucsOnnx::load(&sep_path, "htdemucs-vocals").map_err(|e| e.to_string())?)
    } else {
        None
    };

    let (vocals, sep_name) = pipeline::prepare_vocals(
        &audio,
        separator
            .as_ref()
            .map(|s| s as &dyn keyflow_sync::StemSeparator),
    )
    .map_err(|e| e.to_string())?;
    if let Some(name) = sep_name {
        println!("separated vocals via {name}");
    }

    let audio_id = opts.audio.display().to_string();
    let map: TimingMap = if opts.per_section {
        println!("aligning {} sections (anchored)…", sections.len());
        pipeline::run_alignment_anchored(
            &model,
            &vocals,
            &sections,
            &audio_id,
            opts.model_id,
            opts.thresholds,
            opts.star_score,
        )
    } else {
        pipeline::run_alignment(
            &model,
            &vocals,
            &sections,
            &audio_id,
            opts.model_id,
            opts.thresholds,
            opts.star_score,
        )
    }
    .map_err(|e| e.to_string())?;

    map.write_file(opts.output).map_err(|e| e.to_string())?;
    let (v, r, f) = map.rating_summary();
    println!(
        "wrote {} ({} words: {v} verified, {r} review, {f} failed)",
        opts.output.display(),
        map.words.len()
    );
    Ok(())
}

/// Load audio, optionally separate vocals, load the aligner, and forced-align
/// `sections` against it. Shared by `kf sync` and `kf karaoke`.
#[cfg(feature = "onnx")]
fn align_known_lyrics(
    audio_path: &Path,
    sections: &[keyflow_sync::pipeline::SectionLyrics],
    separate: bool,
    model_id: &str,
    star_score: Option<f32>,
    thresholds: keyflow_sync::RatingThresholds,
) -> Result<(keyflow_sync::TimingMap, keyflow_sync::AudioBuffer), String> {
    use keyflow_sync::align::tokenizer::Vocab;
    use keyflow_sync::align::wav2vec2::Wav2Vec2Onnx;
    use keyflow_sync::audio::AudioBuffer;
    use keyflow_sync::separator::demucs::DemucsOnnx;
    use keyflow_sync::{models, pipeline};

    let audio = AudioBuffer::load_wav(audio_path).map_err(|e| e.to_string())?;
    println!(
        "loaded {} ({:.1}s @ {} Hz)",
        audio_path.display(),
        audio.duration_secs(),
        audio.sample_rate
    );
    let reg = models::load_registry().map_err(|e| e.to_string())?;
    let entry = models::find(&reg, model_id)
        .ok_or_else(|| format!("unknown model {model_id:?} (see `kf models list`)"))?;
    let model_path = models::local_path(&entry);
    if !model_path.exists() {
        return Err(format!(
            "model {model_id:?} not downloaded — run `kf models pull {model_id}`"
        ));
    }
    let vocab_path = model_path.with_extension("vocab.json");
    let vocab = if vocab_path.exists() {
        Vocab::from_vocab_map_json(
            &std::fs::read_to_string(&vocab_path).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?
    } else {
        Vocab::wav2vec2_en_960h()
    };
    let model = Wav2Vec2Onnx::load(&model_path, vocab, 16_000, 50.0).map_err(|e| e.to_string())?;

    let separator = if separate {
        let se = models::find(&reg, "htdemucs-vocals")
            .ok_or_else(|| "no htdemucs-vocals entry".to_string())?;
        let sp = models::local_path(&se);
        if !sp.exists() {
            return Err(
                "htdemucs-vocals not downloaded — run `kf models pull htdemucs-vocals`".into(),
            );
        }
        println!("separating vocals…");
        Some(DemucsOnnx::load(&sp, "htdemucs-vocals").map_err(|e| e.to_string())?)
    } else {
        None
    };
    let (vocals, sep_name) = pipeline::prepare_vocals(
        &audio,
        separator
            .as_ref()
            .map(|s| s as &dyn keyflow_sync::StemSeparator),
    )
    .map_err(|e| e.to_string())?;
    if let Some(name) = sep_name {
        println!("separated vocals via {name}");
    }
    let map = pipeline::run_alignment(
        &model,
        &vocals,
        sections,
        &audio_path.display().to_string(),
        model_id,
        thresholds,
        star_score,
    )
    .map_err(|e| e.to_string())?;
    Ok((map, audio))
}

/// Options for `kf karaoke`.
#[cfg_attr(not(feature = "onnx"), allow(dead_code))]
pub struct KaraokeOpts<'a> {
    pub chart: &'a Path,
    pub audio: &'a Path,
    pub out_dir: &'a Path,
    pub separate: bool,
    pub reaper: bool,
}

/// `kf karaoke` — align a lane-format chart's lyrics to audio, then emit the
/// derived artifacts: sync lane (written back into the .kf), karaoke MIDI, LRC,
/// and optionally a Reaper project (audio + lyrics MIDI).
#[cfg(feature = "onnx")]
pub fn cmd_karaoke(opts: KaraokeOpts<'_>) -> Result<(), String> {
    use keyflow_sync::karaoke::{LyricEvent, lyrics_lrc, lyrics_midi};
    use keyflow_sync::{RatingThresholds, lanes};

    let chart_text = std::fs::read_to_string(opts.chart).map_err(|e| e.to_string())?;
    let chart = lanes::parse(&chart_text).map_err(|e| e.to_string())?;
    let sections = chart.playback_lyrics();
    if sections.is_empty() {
        return Err("chart has no `lyrics {}` lane / no lyric-bearing sections".into());
    }
    println!(
        "{} — {} bpm, {} lyric sections over a {}-section spine",
        chart.title,
        chart.bpm,
        sections.len(),
        chart.spine.len()
    );

    let (map, audio) = align_known_lyrics(
        opts.audio,
        &sections,
        opts.separate,
        "mms-fa",
        Some(-2.5),
        RatingThresholds::SINGING,
    )?;
    let (v, r, f) = map.rating_summary();
    println!(
        "aligned {} words: {v} verified, {r} review, {f} failed",
        map.words.len()
    );

    std::fs::create_dir_all(opts.out_dir).map_err(|e| e.to_string())?;
    let stem = opts
        .chart
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "song".into());

    // Derived artifacts from the aligned words.
    let events: Vec<LyricEvent> = map
        .words
        .iter()
        .map(|w| LyricEvent {
            word: w.text.clone(),
            start: w.start,
            end: w.end,
        })
        .collect();

    let midi = lyrics_midi(&events, chart.bpm, 60).map_err(|e| e.to_string())?;
    let midi_path = opts.out_dir.join(format!("{stem}.mid"));
    std::fs::write(&midi_path, &midi).map_err(|e| e.to_string())?;
    println!("wrote {}", midi_path.display());

    let lrc_path = opts.out_dir.join(format!("{stem}.lrc"));
    std::fs::write(&lrc_path, lyrics_lrc(&events)).map_err(|e| e.to_string())?;
    println!("wrote {}", lrc_path.display());

    // Write the sync lane back into the .kf (single shareable file).
    let sync_block = render_sync_lane(&chart, &map, opts.audio);
    let updated = upsert_block(&chart_text, "sync", &sync_block);
    std::fs::write(opts.chart, updated).map_err(|e| e.to_string())?;
    println!("updated sync lane in {}", opts.chart.display());

    if opts.reaper {
        #[cfg(feature = "reaper")]
        {
            // Self-contained project: copy the audio next to the .rpp and
            // reference it by relative name so the folder is portable.
            let ext = opts
                .audio
                .extension()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_else(|| "wav".into());
            let audio_name = format!("{stem}.{ext}");
            let audio_dest = opts.out_dir.join(&audio_name);
            if std::fs::canonicalize(opts.audio).ok() != std::fs::canonicalize(&audio_dest).ok() {
                std::fs::copy(opts.audio, &audio_dest).map_err(|e| e.to_string())?;
            }
            let rpp = export_reaper(
                &audio_name,
                &events,
                chart.bpm,
                chart.time_sig,
                audio.duration_secs(),
            )?;
            let rpp_path = opts.out_dir.join(format!("{stem}.rpp"));
            std::fs::write(&rpp_path, rpp).map_err(|e| e.to_string())?;
            println!("wrote {} (+ {})", rpp_path.display(), audio_name);
        }
        #[cfg(not(feature = "reaper"))]
        {
            let _ = &audio;
            eprintln!("note: --reaper needs the `reaper` feature; skipping .rpp");
        }
    }
    Ok(())
}

#[cfg(not(feature = "onnx"))]
pub fn cmd_karaoke(_opts: KaraokeOpts<'_>) -> Result<(), String> {
    Err("`kf karaoke` needs `--features onnx`.".into())
}

/// Render the `sync {}` lane: per spine section, the aligned word onsets.
#[cfg(feature = "onnx")]
fn render_sync_lane(
    chart: &keyflow_sync::lanes::LaneChart,
    map: &keyflow_sync::TimingMap,
    audio: &Path,
) -> String {
    use std::collections::BTreeMap;
    let mut by_section: BTreeMap<u32, Vec<&keyflow_sync::WordEntry>> = BTreeMap::new();
    for w in &map.words {
        by_section.entry(w.section).or_default().push(w);
    }
    let audio_name = audio
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut out = format!("sync source={audio_name} model=mms-fa {{\n");
    for (section, words) in by_section {
        let label = chart.label_at(section as usize).unwrap_or("?");
        let onsets: Vec<String> = words.iter().map(|w| format!("{:.2}", w.start)).collect();
        out.push_str(&format!("  [{section}:{label}] {}\n", onsets.join(" ")));
    }
    out.push('}');
    out
}

/// Remove every existing `name … { … }` block, then append the new one. The
/// header may carry metadata before the brace (`sync source=… model=… {`), so
/// the finder is line-anchored, not `name {`-literal.
#[cfg(feature = "onnx")]
fn upsert_block(text: &str, name: &str, block: &str) -> String {
    let mut s = text.to_string();
    while let Some(start) = find_block_start(&s, name) {
        match matching_brace(&s, start) {
            Some(end) => s.replace_range(start..end + 1, ""),
            None => break,
        }
    }
    let mut out = s.trim_end().to_string();
    out.push_str("\n\n");
    out.push_str(block);
    out.push('\n');
    out
}

/// Byte offset of a block whose line *starts* with `name` (a declaration, not a
/// `;` comment mention) and contains `{`.
#[cfg(feature = "onnx")]
fn find_block_start(text: &str, name: &str) -> Option<usize> {
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if let Some(after) = trimmed.strip_prefix(name) {
            let boundary = after.starts_with(|c: char| c.is_whitespace() || c == '{');
            if boundary && line.contains('{') {
                return Some(offset + (line.len() - trimmed.len()));
            }
        }
        offset += line.len();
    }
    None
}

#[cfg(feature = "onnx")]
fn matching_brace(text: &str, from: usize) -> Option<usize> {
    let open = from + text[from..].find('{')?;
    let mut depth = 0;
    for (i, c) in text[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Convert a `#RRGGBB` (or `RRGGBB`) color to REAPER's native track-color int
/// (`0x01_00_00_00 | (B<<16) | (G<<8) | R`).
#[cfg(feature = "reaper")]
fn reaper_color(hex: &str) -> i32 {
    let h = hex.trim_start_matches('#');
    if h.len() >= 6 {
        if let (Ok(r), Ok(g), Ok(b)) = (
            u32::from_str_radix(&h[0..2], 16),
            u32::from_str_radix(&h[2..4], 16),
            u32::from_str_radix(&h[4..6], 16),
        ) {
            return (0x0100_0000 | (b << 16) | (g << 8) | r) as i32;
        }
    }
    0
}

/// Build a Reaper `.rpp` with one colored track per voice: each carries a MIDI
/// item whose notes (on the voice's MIDI channel) and `FF 05` lyric meta events
/// are placed at the voices' notated times. TIME timebase.
#[cfg(feature = "reaper")]
fn export_reaper_voices(
    voices: &[VoiceTrack],
    bpm: f32,
    time_sig: (u8, u8),
) -> Result<String, String> {
    use dawfile_reaper::RppSerialize;
    use dawfile_reaper::builder::ReaperProjectBuilder;
    use dawfile_reaper::types::item::{MidiEvent, MidiSourceEvent};

    const TPQN: u32 = 480;
    let to_ticks = |s: f32| -> u64 { (s.max(0.0) * (bpm / 60.0) * TPQN as f32).round() as u64 };
    let total = voices
        .iter()
        .flat_map(|v| v.notes.iter())
        .map(|n| n.end)
        .fold(0.0f32, f32::max)
        .max(1.0) as f64;

    let mut pb = ReaperProjectBuilder::new()
        .tempo_with_time_sig(bpm as f64, time_sig.0 as i32, time_sig.1 as i32)
        .sample_rate(44100);

    for v in voices {
        // Precompute (delta_ticks, midi, dur_ticks) so the builder closure
        // captures only a plain Vec.
        let ch = v.channel;
        let mut cmds: Vec<(u32, u8, u32)> = Vec::new();
        let mut prev = 0u64;
        for n in &v.notes {
            let on = to_ticks(n.start);
            let off = to_ticks(n.end.max(n.start + 0.05)).max(on + 1);
            cmds.push(((on.saturating_sub(prev)) as u32, n.midi, (off - on) as u32));
            prev = on;
        }
        let name = v.name.clone();
        pb = pb.track(name, move |t| {
            t.item(0.0, total, move |i| {
                i.midi(move |mut m| {
                    m = m.ticks_per_qn(TPQN);
                    for (d, midi, dur) in &cmds {
                        m = m.note(*d, ch, *midi, 80, *dur);
                    }
                    m
                })
            })
        });
    }

    let mut project = pb.build();
    // Per voice: set color + TIME timebase, and splice lyric meta events.
    for (vi, v) in voices.iter().enumerate() {
        let Some(track) = project.tracks.get_mut(vi) else {
            break;
        };
        track.beat = Some(0);
        if let Some(c) = &v.color {
            track.peak_color = Some(reaper_color(c));
        }
        let Some(src) = track
            .items
            .get_mut(0)
            .and_then(|i| i.takes.get_mut(0))
            .and_then(|tk| tk.source.as_mut())
            .and_then(|s| s.midi_data.as_mut())
        else {
            continue;
        };
        let mut abs: Vec<(u64, u8, MidiEvent)> = Vec::new();
        let mut acc = 0u64;
        for ev in &src.event_stream {
            if let MidiSourceEvent::Midi(e) = ev {
                acc += e.delta_ticks as u64;
                abs.push((acc, 1, e.clone()));
            }
        }
        for n in &v.notes {
            if let Some(l) = &n.lyric {
                let mut bytes = vec![0xFF, 0x05];
                bytes.extend_from_slice(l.as_bytes());
                abs.push((
                    to_ticks(n.start),
                    0,
                    MidiEvent {
                        delta_ticks: 0,
                        bytes,
                    },
                ));
            }
        }
        abs.sort_by_key(|(t, ord, _)| (*t, *ord));
        let mut prev = 0u64;
        let (mut events_out, mut stream) = (Vec::new(), Vec::new());
        for (t, _, mut e) in abs {
            e.delta_ticks = (t - prev) as u32;
            prev = t;
            stream.push(MidiSourceEvent::Midi(e.clone()));
            events_out.push(e);
        }
        src.events = events_out;
        src.event_stream = stream;
        src.has_data = true;
    }
    Ok(project.to_rpp_string())
}

/// Build a Reaper `.rpp`: an audio track with the song, and a "Lyrics" MIDI
/// track carrying a note + `FF 05` lyric meta event per word, placed at the
/// aligned times. Both tracks use TIME timebase so they lock to seconds.
#[cfg(all(feature = "reaper", feature = "onnx"))]
fn export_reaper(
    audio_ref: &str,
    events: &[keyflow_sync::karaoke::LyricEvent],
    bpm: f32,
    time_sig: (u8, u8),
    duration: f32,
) -> Result<String, String> {
    use dawfile_reaper::RppSerialize;
    use dawfile_reaper::builder::ReaperProjectBuilder;
    use dawfile_reaper::types::item::{MidiEvent, MidiSourceEvent};

    const TPQN: u32 = 480;
    let to_ticks = |s: f32| -> u64 { (s.max(0.0) * (bpm / 60.0) * TPQN as f32).round() as u64 };
    let dur = duration.max(1.0) as f64;

    // Audio track + lyrics MIDI track (a note per word). `audio_ref` is a
    // relative filename resolved against the .rpp's directory.
    let mut project = ReaperProjectBuilder::new()
        .tempo_with_time_sig(bpm as f64, time_sig.0 as i32, time_sig.1 as i32)
        .sample_rate(44100)
        .track("Audio", |t| {
            t.item(0.0, dur, |i| i.name("Audio").source_wave(audio_ref))
        })
        .track("Lyrics", |t| {
            t.item(0.0, dur, |i| {
                i.name("Lyrics").midi(|mut m| {
                    m = m.ticks_per_qn(TPQN);
                    let mut prev = 0u64;
                    for e in events {
                        let on = to_ticks(e.start);
                        let off = to_ticks(e.end.max(e.start + 0.1)).max(on + 1);
                        m = m.note(
                            (on.saturating_sub(prev)) as u32,
                            0,
                            60,
                            80,
                            (off - on) as u32,
                        );
                        prev = on;
                    }
                    m
                })
            })
        })
        .build();

    // Splice FF 05 lyric meta events into the lyrics MIDI source's event stream.
    if let Some(src) = project
        .tracks
        .get_mut(1)
        .and_then(|t| t.items.get_mut(0))
        .and_then(|i| i.takes.get_mut(0))
        .and_then(|tk| tk.source.as_mut())
        .and_then(|s| s.midi_data.as_mut())
    {
        // Reconstruct absolute ticks from the note stream, then merge in lyrics.
        let mut abs: Vec<(u64, u8, MidiEvent)> = Vec::new();
        let mut acc = 0u64;
        for ev in &src.event_stream {
            if let MidiSourceEvent::Midi(e) = ev {
                acc += e.delta_ticks as u64;
                abs.push((acc, 1, e.clone())); // notes after lyric at a tie
            }
        }
        for e in events {
            let mut bytes = vec![0xFF, 0x05];
            bytes.extend_from_slice(e.word.as_bytes());
            abs.push((
                to_ticks(e.start),
                0,
                MidiEvent {
                    delta_ticks: 0,
                    bytes,
                },
            ));
        }
        abs.sort_by_key(|(t, ord, _)| (*t, *ord));
        let mut prev = 0u64;
        let (mut events_out, mut stream) = (Vec::new(), Vec::new());
        for (t, _, mut e) in abs {
            e.delta_ticks = (t - prev) as u32;
            prev = t;
            stream.push(MidiSourceEvent::Midi(e.clone()));
            events_out.push(e);
        }
        src.events = events_out;
        src.event_stream = stream;
        src.has_data = true;
    }

    // TIME timebase: audio + MIDI lock to seconds, not beats.
    for t in &mut project.tracks {
        t.beat = Some(0);
    }

    Ok(project.to_rpp_string())
}

/// Options for `kf transcribe`.
#[cfg_attr(not(feature = "onnx"), allow(dead_code))]
pub struct TranscribeOpts<'a> {
    pub audio: &'a Path,
    pub output: Option<&'a Path>,
    pub separate: bool,
    pub model_id: &'a str,
    /// (Whisper only) Forced-align the recovered transcript with the CTC aligner
    /// for precise word-level timestamps (the WhisperX approach). Needs `onnx`.
    pub align: bool,
}

#[cfg(feature = "onnx")]
pub fn cmd_transcribe(opts: TranscribeOpts<'_>) -> Result<(), String> {
    use keyflow_sync::align::tokenizer::Vocab;
    use keyflow_sync::align::wav2vec2::Wav2Vec2Onnx;
    use keyflow_sync::audio::AudioBuffer;
    use keyflow_sync::separator::demucs::DemucsOnnx;
    use keyflow_sync::{RatingThresholds, models, pipeline};

    let audio = AudioBuffer::load_wav(opts.audio).map_err(|e| e.to_string())?;
    let reg = models::load_registry().map_err(|e| e.to_string())?;
    let entry = models::find(&reg, opts.model_id)
        .ok_or_else(|| format!("unknown model {:?} (see `kf models list`)", opts.model_id))?;
    let model_path = models::local_path(&entry);
    if !model_path.exists() {
        return Err(format!(
            "model {:?} not downloaded — run `kf models pull {}`",
            opts.model_id, opts.model_id
        ));
    }
    let vocab_path = model_path.with_extension("vocab.json");
    let vocab = if vocab_path.exists() {
        Vocab::from_vocab_map_json(
            &std::fs::read_to_string(&vocab_path).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?
    } else {
        Vocab::wav2vec2_en_960h()
    };
    let model = Wav2Vec2Onnx::load(&model_path, vocab, 16_000, 50.0).map_err(|e| e.to_string())?;

    let separator = if opts.separate {
        let se = models::find(&reg, "htdemucs-vocals")
            .ok_or_else(|| "no htdemucs-vocals entry".to_string())?;
        let sp = models::local_path(&se);
        if !sp.exists() {
            return Err(
                "htdemucs-vocals not downloaded — run `kf models pull htdemucs-vocals`".into(),
            );
        }
        println!("separating vocals…");
        Some(DemucsOnnx::load(&sp, "htdemucs-vocals").map_err(|e| e.to_string())?)
    } else {
        None
    };
    let (vocals, _) = pipeline::prepare_vocals(
        &audio,
        separator
            .as_ref()
            .map(|s| s as &dyn keyflow_sync::StemSeparator),
    )
    .map_err(|e| e.to_string())?;

    let map = pipeline::run_transcription(
        &model,
        &vocals,
        &opts.audio.display().to_string(),
        opts.model_id,
        RatingThresholds::SINGING,
    )
    .map_err(|e| e.to_string())?;

    // Print the heard transcript — compare this against the known lyrics to see
    // where the recording diverges (extra repeats, ad-libs, structure).
    let transcript: Vec<&str> = map.words.iter().map(|w| w.text.as_str()).collect();
    println!(
        "\n--- ASR transcript ({} words) ---\n{}\n",
        map.words.len(),
        transcript.join(" ")
    );
    if let Some(out) = opts.output {
        map.write_file(out).map_err(|e| e.to_string())?;
        println!(
            "wrote {} (word timings, comparable to a sync sidecar)",
            out.display()
        );
    }
    Ok(())
}

#[cfg(not(feature = "onnx"))]
pub fn cmd_transcribe(_opts: TranscribeOpts<'_>) -> Result<(), String> {
    Err("`kf transcribe` needs `--features onnx`.".into())
}

/// Separate vocals if requested and the `onnx` feature is built; otherwise pass
/// the audio through (Whisper tolerates mixes). Shared by the Whisper path.
#[cfg(feature = "whisper")]
#[allow(unused_variables)]
fn maybe_separate(
    audio: keyflow_sync::AudioBuffer,
    separate: bool,
) -> Result<keyflow_sync::AudioBuffer, String> {
    if !separate {
        return Ok(audio);
    }
    #[cfg(feature = "onnx")]
    {
        use keyflow_sync::{models, pipeline, separator::demucs::DemucsOnnx};
        let reg = models::load_registry().map_err(|e| e.to_string())?;
        let se = models::find(&reg, "htdemucs-vocals")
            .ok_or_else(|| "no htdemucs-vocals entry".to_string())?;
        let sp = models::local_path(&se);
        if !sp.exists() {
            return Err(
                "htdemucs-vocals not downloaded — run `kf models pull htdemucs-vocals`".into(),
            );
        }
        println!("separating vocals…");
        let sep = DemucsOnnx::load(&sp, "htdemucs-vocals").map_err(|e| e.to_string())?;
        let (vocals, _) =
            pipeline::prepare_vocals(&audio, Some(&sep as &dyn keyflow_sync::StemSeparator))
                .map_err(|e| e.to_string())?;
        Ok(vocals)
    }
    #[cfg(not(feature = "onnx"))]
    {
        eprintln!("warning: --separate needs the `onnx` feature; transcribing the full mix");
        Ok(audio)
    }
}

/// `kf transcribe --engine whisper` — LM-backed ASR baseline (candle).
#[cfg(feature = "whisper")]
pub fn cmd_transcribe_whisper(opts: TranscribeOpts<'_>) -> Result<(), String> {
    use keyflow_sync::audio::AudioBuffer;
    use keyflow_sync::whisper::{Segment, WhisperAsr, ensure_model};
    use keyflow_sync::{Rating, RatingThresholds, TimingMap};

    let audio = AudioBuffer::load_wav(opts.audio).map_err(|e| e.to_string())?;
    let vocals = maybe_separate(audio, opts.separate)?;
    let vocals = vocals.resample(16_000);

    let dir = ensure_model().map_err(|e| e.to_string())?;
    println!("loading whisper-base.en…");
    let mut asr = WhisperAsr::load(&dir).map_err(|e| e.to_string())?;
    println!("transcribing (whisper)…");
    let segments: Vec<Segment> = asr.transcribe(&vocals.samples).map_err(|e| e.to_string())?;

    let transcript: Vec<&str> = segments.iter().map(|s| s.text.as_str()).collect();
    println!(
        "\n--- Whisper transcript ({} segments) ---\n{}\n",
        segments.len(),
        transcript.join(" ")
    );
    for s in &segments {
        println!("  [{:6.2}-{:6.2}] {}", s.start, s.end, s.text);
    }

    // WhisperX-style: forced-align the recovered transcript for precise per-word
    // timestamps. Whisper's transcript matches the audio (right words, right
    // order, real repeats), so forced alignment of it lands cleanly.
    #[cfg(feature = "onnx")]
    if opts.align {
        return whisperx_align(&opts, &vocals, &segments);
    }
    #[cfg(not(feature = "onnx"))]
    if opts.align {
        eprintln!("warning: --align needs the `onnx` feature; writing phrase-level timings only");
    }

    if let Some(out) = opts.output {
        // Store as a sidecar: each segment's words share its span (rough word
        // timing), so it drops into the same comparison format.
        let mut map = TimingMap::new(&opts.audio.display().to_string(), "whisper-base.en", 50.0);
        let th = RatingThresholds::SINGING;
        let mut words = Vec::new();
        for s in &segments {
            let toks: Vec<&str> = s.text.split_whitespace().collect();
            let n = toks.len().max(1);
            let dur = (s.end - s.start) / n as f32;
            for (i, w) in toks.iter().enumerate() {
                words.push(keyflow_sync::WordTiming {
                    word: w.to_string(),
                    start: s.start + dur * i as f32,
                    end: s.start + dur * (i as f32 + 1.0),
                    confidence: s.confidence,
                });
            }
        }
        map.push_section(0, &words, th);
        map.write_file(out).map_err(|e| e.to_string())?;
        let (v, r, f) = map.rating_summary();
        let _ = Rating::Verified;
        println!(
            "wrote {} ({} words: {v} verified, {r} review, {f} failed)",
            out.display(),
            map.words.len()
        );
    }
    Ok(())
}

/// Strip bracketed/parenthesized non-lyric annotations Whisper emits
/// (`[Music]`, `(singing)`, …) and join the rest into a transcript string.
#[cfg(all(feature = "whisper", feature = "onnx"))]
fn clean_transcript(segments: &[keyflow_sync::whisper::Segment]) -> String {
    let mut out = String::new();
    for s in segments {
        let mut depth = 0i32;
        for ch in s.text.chars() {
            match ch {
                '[' | '(' => depth += 1,
                ']' | ')' => depth = (depth - 1).max(0),
                _ if depth == 0 => out.push(ch),
                _ => {}
            }
        }
        out.push(' ');
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Forced-align the Whisper transcript with the CTC aligner for word timings.
#[cfg(all(feature = "whisper", feature = "onnx"))]
fn whisperx_align(
    opts: &TranscribeOpts<'_>,
    vocals: &keyflow_sync::AudioBuffer,
    segments: &[keyflow_sync::whisper::Segment],
) -> Result<(), String> {
    use keyflow_sync::align::{align_words_star, tokenizer::Vocab, wav2vec2::Wav2Vec2Onnx};
    use keyflow_sync::{RatingThresholds, TimingMap, models};

    let transcript = clean_transcript(segments);
    if transcript.is_empty() {
        return Err("Whisper found no lyric words to align".into());
    }

    let reg = models::load_registry().map_err(|e| e.to_string())?;
    let entry =
        models::find(&reg, "mms-fa").ok_or_else(|| "no mms-fa aligner in registry".to_string())?;
    let path = models::local_path(&entry);
    if !path.exists() {
        return Err("aligner not downloaded — run `kf models pull mms-fa`".into());
    }
    let vocab_path = path.with_extension("vocab.json");
    let vocab = if vocab_path.exists() {
        Vocab::from_vocab_map_json(
            &std::fs::read_to_string(&vocab_path).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?
    } else {
        Vocab::wav2vec2_en_960h()
    };
    println!("aligning Whisper transcript (WhisperX-style)…");
    let model = Wav2Vec2Onnx::load(&path, vocab, 16_000, 50.0).map_err(|e| e.to_string())?;
    let words = align_words_star(&model, &vocals.samples, &transcript, Some(-2.5))
        .map_err(|e| e.to_string())?;

    let mut map = TimingMap::new(
        &opts.audio.display().to_string(),
        "whisper-base.en+mms-fa",
        50.0,
    );
    map.push_section(0, &words, RatingThresholds::SINGING);
    let (v, r, f) = map.rating_summary();
    println!(
        "{} words aligned: {v} verified, {r} review, {f} failed",
        map.words.len()
    );
    if let Some(out) = opts.output {
        map.write_file(out).map_err(|e| e.to_string())?;
        println!("wrote {}", out.display());
    }
    Ok(())
}

#[cfg(not(feature = "whisper"))]
pub fn cmd_transcribe_whisper(_opts: TranscribeOpts<'_>) -> Result<(), String> {
    Err("Whisper ASR needs `--features whisper` (or `whisper-cuda`).".into())
}

#[cfg(not(feature = "onnx"))]
pub fn cmd_sync(_chart: &Chart, _opts: SyncOpts<'_>) -> Result<(), String> {
    Err(
        "`kf sync` needs embedded inference: rebuild with `--features onnx` \
         (and `kf models pull mms-fa`). The lean binary ships without the ONNX runtime."
            .into(),
    )
}
