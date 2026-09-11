//! Load a REAPER `.rpp` project into a [`Standalone`] backend.
//!
//! Parses with `dawfile-reaper` and populates `ProjectState` with
//! tracks, items, takes, markers, regions, tempo points, hardware
//! outputs, and routing edges. Returns a `LoadedProject` summary so
//! callers can sanity-check what made it across.
//!
//! Scope (per current standalone capabilities): everything except
//! FX processing. FX entries on the source RPP are skipped (the
//! standalone FX impl is synthetic; loading real plugin state would
//! require a plugin host). Automation envelopes are also skipped for
//! now — the audio-graph integration will need them but the proto
//! `Automation` trait already works against the runtime envelopes
//! created via `add_point`, so loaded envelopes can land in a follow-up.
//!
//! Feature-gated under `rpp-project` (pulls dawfile-reaper, not WASM-
//! compatible due to rayon). For browser use cases, build the
//! project state directly via the proto trait methods.

#![cfg(any(feature = "rpp-project", feature = "rpp-project-wasm"))]

use std::sync::atomic::{AtomicUsize, Ordering};

use dawfile_reaper::types::item::{SourceType as RppSourceType, Take as RppTake};
use dawfile_reaper::types::project::{DecodeOptions, ReaperProject};
use uuid::Uuid;

use daw_proto::item::{FadeShape, Item, SourceType};
use daw_proto::primitives::{Duration, PositionInSeconds, Tempo, TimeSignature};
use daw_proto::project::ProjectInfo;
use daw_proto::track::Track;
use daw_proto::{Marker, Region, Take, TempoPoint};

use crate::sync::{ItemEntry, Standalone, TakeList, TrackExt};

/// Like [`load_rpp`] but pulls audio bytes through the project's
/// [`MediaBay`](crate::media_bay::MediaBay) resolver instead of an
/// inline closure. Caller must install a [`BayFileResolver`](
/// crate::media_bay::BayFileResolver) on the bay first — native apps
/// use [`FsFileResolver`](crate::media_bay::FsFileResolver), browser
/// apps install a JS-backed one.
///
/// ```ignore
/// use daw_standalone::media_bay::FsFileResolver;
/// use daw_standalone::project_loader::load_rpp_via_bay;
///
/// daw.media_bay().set_file_resolver(Box::new(FsFileResolver));
/// let (proj, audio) = load_rpp_via_bay(&daw, "Song", &path, &rpp_text)?;
/// ```
#[cfg(feature = "decode")]
pub fn load_rpp_via_bay(
    daw: &Standalone,
    project_name: &str,
    project_path: &str,
    rpp_text: &str,
) -> Result<
    (
        LoadedProject,
        crate::audio_engine::materialize::MaterializeReport,
    ),
    String,
> {
    let proj = load_rpp_text(daw, project_name, project_path, rpp_text)?;
    let audio = crate::audio_engine::materialize::materialize_via_bay(daw, &proj.project_guid)?;
    Ok((proj, audio))
}

/// One-shot wrapper that loads structure AND materializes audio.
///
/// Equivalent to `load_rpp_text(...)` followed by
/// `materialize_audio(...)`, returning both reports. Available when
/// the `decode` feature is on (so symphonia is available).
///
/// ```ignore
/// use daw_standalone::project_loader::load_rpp;
///
/// let (proj, audio) = load_rpp(&daw, "Song", "/tmp/song.rpp", &rpp_text, |path| {
///     std::fs::read(path).map_err(|e| e.to_string())
/// })?;
/// eprintln!("loaded {} tracks, {} audio sources", proj.track_count, audio.loaded);
/// ```
#[cfg(feature = "decode")]
pub fn load_rpp<F>(
    daw: &Standalone,
    project_name: &str,
    project_path: &str,
    rpp_text: &str,
    resolver: F,
) -> Result<
    (
        LoadedProject,
        crate::audio_engine::materialize::MaterializeReport,
    ),
    String,
>
where
    F: FnMut(&str) -> Result<Vec<u8>, String>,
{
    let proj = load_rpp_text(daw, project_name, project_path, rpp_text)?;
    let audio =
        crate::audio_engine::materialize::materialize_audio(daw, &proj.project_guid, resolver);
    Ok((proj, audio))
}

/// REAPER native colour (Windows `COLORREF`, `0x..BBGGRR` + the
/// `0x1000000` custom flag) → the canonical `0xRRGGBB` the daw-proto
/// types carry.
fn native_color_to_rgb(c: u32) -> u32 {
    let r = c & 0xff;
    let g = (c >> 8) & 0xff;
    let b = (c >> 16) & 0xff;
    (r << 16) | (g << 8) | b
}

/// Summary of what was loaded.
#[derive(Debug, Default)]
pub struct LoadedProject {
    pub project_guid: String,
    pub track_count: usize,
    pub item_count: usize,
    pub take_count: usize,
    pub marker_count: usize,
    pub region_count: usize,
    pub tempo_point_count: usize,
    pub hw_output_count: usize,
    /// Warnings emitted during the load (e.g. unsupported FX skipped).
    pub warnings: Vec<String>,
}

/// Parse RPP text and populate a fresh project in `daw`. Returns the
/// seeded project's GUID + a summary of what was loaded.
pub fn load_rpp_text(
    daw: &Standalone,
    project_name: &str,
    project_path: &str,
    rpp_text: &str,
) -> Result<LoadedProject, String> {
    let rpp =
        dawfile_reaper::parse_rpp_file(rpp_text).map_err(|e| format!("rpp parse failed: {e:?}"))?;
    let project = ReaperProject::from_rpp_project_with_options(&rpp, DecodeOptions::full())
        .map_err(|e| format!("rpp decode failed: {e:?}"))?;

    let project_guid = Uuid::new_v4().to_string();
    daw.seed_project(ProjectInfo {
        guid: project_guid.clone(),
        name: project_name.to_string(),
        path: project_path.to_string(),
    });

    let mut summary = LoadedProject {
        project_guid: project_guid.clone(),
        ..Default::default()
    };

    populate_tracks(daw, &project_guid, &project, &mut summary);
    populate_markers_regions(daw, &project_guid, &project, &mut summary);
    populate_tempo(daw, &project_guid, &project, &mut summary);
    populate_routing(daw, &project_guid, &project, &mut summary);
    populate_fx_chains(daw, &project_guid, &project, &mut summary);

    Ok(summary)
}

fn populate_tracks(
    daw: &Standalone,
    project_guid: &str,
    project: &ReaperProject,
    summary: &mut LoadedProject,
) {
    static ITEM_COUNTER: AtomicUsize = AtomicUsize::new(0);

    let _ = daw.with_project_mut(project_guid, |p| {
        // Default project tempo / time signature from the tempo
        // envelope (or fall back to 120/4-4).
        if let Some(env) = &project.tempo_envelope {
            p.transport.tempo = Tempo::from_bpm(env.default_tempo.max(1.0));
            let (num, denom) = env.default_time_signature;
            p.transport.time_signature = TimeSignature::new(num.max(1) as u32, denom.max(1) as u32);
        } else if let Some((bpm, num, denom, _)) = project.properties.tempo {
            // No tempo envelope: the header `TEMPO <bpm> <num> <denom>`
            // is the project's one tempo, and everything that converts
            // time — the grid, the ruler, quantize targets — reads it
            // from here. r[impl drums.group.tempo]
            p.transport.tempo = Tempo::from_bpm(bpm.max(1.0));
            p.transport.time_signature = TimeSignature::new(num.max(1) as u32, denom.max(1) as u32);
        }

        // Master section: `MASTER_VOLUME vol pan …` + `MASTERMUTESOLO`.
        if let Some((vol, pan, ..)) = project.properties.master_volume {
            p.master_volume = vol.max(0.0);
            p.master_pan = pan.clamp(-1.0, 1.0);
        }
        if let Some(ms) = project.properties.master_mute_solo {
            p.master_muted = ms & 1 != 0;
        }

        for (idx, rt) in project.tracks.iter().enumerate() {
            // Synthesize a GUID — REAPER's track GUIDs aren't always
            // exposed by dawfile-reaper. Use track_id when available.
            let guid = rt
                .track_id
                .clone()
                .unwrap_or_else(|| Uuid::new_v4().to_string());

            let (volume, pan) = rt
                .volpan
                .as_ref()
                .map(|v| (v.volume, v.pan))
                .unwrap_or((1.0, 0.0));
            let (muted, soloed) = rt
                .mutesolo
                .as_ref()
                .map(|m| {
                    let solo =
                        !matches!(m.solo, dawfile_reaper::types::track::TrackSoloState::NoSolo);
                    (m.mute, solo)
                })
                .unwrap_or((false, false));
            let (folder_depth, is_folder) = rt
                .folder
                .as_ref()
                .map(|f| {
                    use dawfile_reaper::types::track::FolderState as FS;
                    let depth = match f.folder_state {
                        FS::Regular => 0,
                        FS::FolderParent => 1,
                        // ISBUS's second field is the depth DELTA: a
                        // last-in-folder track can close several
                        // nested folders at once (`ISBUS 2 -3`).
                        FS::LastInFolder => f.indentation.min(-1),
                        FS::Unknown(_) => 0,
                    };
                    (depth, depth > 0)
                })
                .unwrap_or((0, false));

            // Fixed item lanes (REAPER 7 comping). Lane count comes
            // from LANENAME (one token per lane); fall back to the
            // largest item lane index + 1 when names are absent. The
            // play mask comes from LANESOLO's first 64 bits; a track
            // with lanes but no LANESOLO plays lane 0.
            let lane_count: u32 = rt
                .lane_names
                .as_ref()
                .map(|ln| ln.lane_count.max(0) as u32)
                .filter(|&c| c > 0)
                .unwrap_or_else(|| {
                    if rt.fixed_lanes.is_some() {
                        rt.items
                            .iter()
                            .filter_map(|i| i.lane)
                            .max()
                            .map(|l| (l.max(0) as u32) + 1)
                            .unwrap_or(0)
                    } else {
                        0
                    }
                });
            let lane_play_mask: u64 = if lane_count == 0 {
                0
            } else {
                rt.lane_solo
                    .as_ref()
                    .map(|ls| {
                        (ls.playing_lanes as u32 as u64)
                            | ((ls.unknown_field_2 as u32 as u64) << 32)
                    })
                    .filter(|&m| m != 0)
                    .unwrap_or(1) // lanes on, no LANESOLO → lane 0 plays
            };
            let lane_names: Vec<String> = rt
                .lane_names
                .as_ref()
                .map(|ln| ln.lane_names.clone())
                .unwrap_or_default();
            // Display mode from FIXEDLANES (empirical, verified across
            // the session corpus): field 3 = "show only the playing
            // lane"; bitfield &8 = big lanes (vs small).
            let lane_display = match rt.fixed_lanes.as_ref() {
                Some(fl) if fl.show_play_only_lane => daw_proto::track::LaneDisplay::One,
                Some(fl) if fl.bitfield & 8 != 0 => daw_proto::track::LaneDisplay::Big,
                _ => daw_proto::track::LaneDisplay::Small,
            };

            let grouping = decode_grouping(
                rt.group_flags.as_deref().unwrap_or(&[]),
                rt.group_flags_high.as_deref().unwrap_or(&[]),
            );

            let track = Track {
                guid: guid.clone(),
                automation_mode: {
                    use daw_proto::primitives::AutomationMode as P;
                    use dawfile_reaper::types::track::AutomationMode as R;
                    match rt.automation_mode {
                        R::TrimRead => P::TrimRead,
                        R::Read => P::Read,
                        R::Touch => P::Touch,
                        R::Write => P::Write,
                        R::Latch => P::Latch,
                        R::Unknown(_) => P::TrimRead,
                    }
                },
                input_monitor: {
                    use daw_proto::track::InputMonitoringMode as P;
                    use dawfile_reaper::types::track::MonitorMode as R;
                    match rt.record.as_ref().map(|r| r.monitor) {
                        Some(R::On) => P::Normal,
                        Some(R::Auto) => P::NotWhenPlaying,
                        _ => P::Off,
                    }
                },
                index: idx as u32,
                name: rt.name.clone(),
                color: rt.peak_color.map(|c| native_color_to_rgb(c as u32)),
                muted,
                soloed,
                armed: rt.record.as_ref().map(|r| r.armed).unwrap_or(false),
                phase_inverted: rt.invert_phase,
                selected: rt.selected,
                volume,
                pan,
                parent_guid: None, // resolved in a second pass once we
                // have folder nesting, currently
                // tracked only via folder_depth
                folder_depth,
                is_folder,
                lane_count,
                lane_play_mask,
                lane_names,
                lane_display,
                grouping,
                visible_in_tcp: rt
                    .show_in_mixer
                    .as_ref()
                    .map(|s| s.show_in_track_list)
                    .unwrap_or(true),
                visible_in_mixer: rt
                    .show_in_mixer
                    .as_ref()
                    .map(|s| s.show_in_mixer)
                    .unwrap_or(true),
                fx_count: 0, // FX not loaded (synthetic standalone)
                input_fx_count: 0,
                // The project's own `TRACKHEIGHT`. Absent, or zero —
                // REAPER's sentinel for automatic — means the panel
                // picks its own, so it stays `None` rather than becoming
                // a track nought pixels tall.
                height: rt
                    .track_height
                    .as_ref()
                    .map(|h| h.height)
                    .filter(|h| *h > 0)
                    .map(|h| h as u32),
                // The project file's own answer, not a default: a track
                // muted out of the master bus must not read as sending to
                // it just because nobody asked the routing service.
                parent_send: rt.master_send.as_ref().map(|m| m.enabled).unwrap_or(true),
                record_input: daw_proto::track::RecordInput::None,
            };
            p.tracks.push(track);

            // Extended (non-proto) fields.
            let parent_send_enabled = rt.master_send.as_ref().map(|m| m.enabled).unwrap_or(true);
            p.track_ext.insert(
                guid.clone(),
                TrackExt {
                    num_channels: rt.channel_count.max(1).min(128),
                    record_input: daw_proto::track::RecordInput::None,
                    parent_send_enabled,
                    tcp_height_pixels: 0,
                },
            );

            // Track automation envelopes (VOLENV2 / PANENV2 / …).
            for env in &rt.envelopes {
                if let Some((key, data)) = convert_track_envelope(env) {
                    p.envelopes.insert((guid.clone(), key), data);
                }
            }

            // Items on this track.
            let track_items = p.items_by_track.entry(guid.clone()).or_default();
            for (item_idx, ri) in rt.items.iter().enumerate() {
                let item_guid = ri
                    .item_guid
                    .clone()
                    .unwrap_or_else(|| Uuid::new_v4().to_string());
                let mut item = Item::default();
                item.guid = item_guid.clone();
                item.track_guid = guid.clone();
                item.index = item_idx as u32;
                item.position = PositionInSeconds::from_seconds(ri.position);
                item.length = Duration::from_seconds(ri.length);
                item.snap_offset = Duration::from_seconds(ri.snap_offset);
                item.muted = ri.mute.as_ref().map(|m| m.muted).unwrap_or(false);
                item.selected = ri.selected;
                item.volume = ri.volpan.as_ref().map(|v| v.item_trim).unwrap_or(1.0);
                if let Some(fi) = &ri.fade_in {
                    item.fade_in_length = Duration::from_seconds(fi.time);
                    item.fade_in_shape = fade_curve_to_shape(fi.curve_type);
                }
                if let Some(fo) = &ri.fade_out {
                    item.fade_out_length = Duration::from_seconds(fo.time);
                    item.fade_out_shape = fade_curve_to_shape(fo.curve_type);
                }
                item.color = ri.color.map(|c| native_color_to_rgb(c as u32));
                item.loop_source = ri.loop_source;
                // Fixed-lane membership only matters on lane-enabled
                // tracks (YPOS also appears for free item positioning).
                item.fixed_lane = if lane_count > 0 {
                    ri.lane.map(|l| l.max(0) as u32)
                } else {
                    None
                };
                item.take_count = ri.takes.len().max(1) as u32;
                // proto `Item` doesn't carry `channel_mode` yet — drop.

                ITEM_COUNTER.fetch_add(1, Ordering::Relaxed);
                p.items.insert(item_guid.clone(), ItemEntry { item });
                track_items.push(item_guid.clone());

                // Takes.
                let mut takes_out = Vec::with_capacity(ri.takes.len().max(1));
                for (take_idx, rt_take) in ri.takes.iter().enumerate() {
                    let take = build_take(&item_guid, take_idx as u32, rt_take, summary);
                    // r[impl drums.open.stretch-markers]
                    // `SM` lines are keyed per take; the third token is the
                    // marker's slope (daw-proto's field of the same name).
                    if !rt_take.stretch_markers.is_empty() {
                        let mut markers: Vec<daw_proto::StretchMarker> = rt_take
                            .stretch_markers
                            .iter()
                            .map(|sm| daw_proto::StretchMarker {
                                position: sm.position,
                                source_position: sm.source_position,
                                slope: sm.rate.unwrap_or(0.0),
                            })
                            .collect();
                        markers.sort_by(|a, b| a.position.total_cmp(&b.position));
                        p.stretch_markers.insert(take.guid.clone(), markers);
                    }
                    // If this is a MIDI take, decode its event stream
                    // into MidiNote entries on `p.midi_notes`. The
                    // renderer reads from this map to feed VST3i /
                    // CLAPi at playback.
                    if take.is_midi
                        && let Some(src) = rt_take.source.as_ref()
                        && let Some(midi) = src.midi_data.as_ref()
                    {
                        let decoded = decode_midi_source(midi);
                        if !decoded.notes.is_empty() {
                            p.midi_notes.insert(take.guid.clone(), decoded.notes);
                        }
                        if !decoded.ccs.is_empty() {
                            p.midi_ccs.insert(take.guid.clone(), decoded.ccs);
                        }
                        if !decoded.pitch_bends.is_empty() {
                            p.midi_pitch_bends
                                .insert(take.guid.clone(), decoded.pitch_bends);
                        }
                        if !decoded.program_changes.is_empty() {
                            p.midi_program_changes
                                .insert(take.guid.clone(), decoded.program_changes);
                        }
                        if !decoded.sysex.is_empty() {
                            p.midi_sysex.insert(take.guid.clone(), decoded.sysex);
                        }
                        if !decoded.channel_pressures.is_empty() {
                            p.midi_channel_pressures
                                .insert(take.guid.clone(), decoded.channel_pressures);
                        }
                        if !decoded.poly_pressures.is_empty() {
                            p.midi_poly_pressures
                                .insert(take.guid.clone(), decoded.poly_pressures);
                        }
                    }
                    takes_out.push(take);
                }
                // Which take plays.
                //
                // `TAKE SEL` is REAPER's own marker for the active take and
                // is authoritative when present. The GUID match below is the
                // older path and only works for an item whose FIRST take is
                // written inline (so the item carries that take's `GUID`).
                //
                // A comped item is not written that way. It opens with
                // `TAKE NULL` — an empty comp-lane slot, which occupies a
                // take index exactly as REAPER counts it — so there is no
                // item-level `GUID` to match, `position` finds nothing, and
                // the old `unwrap_or(0)` landed on a null slot. The item
                // then reported an `Empty` active take and every consumer
                // that keeps only audio dropped it: on a real session
                // (`set in stone`) the tom trigger tracks composed to 1.3%
                // non-zero at -66 dBFS while their source files were full
                // 317s recordings. It presented as "the trigger tracks hold
                // no audio".
                //
                // Falling back to the first take that has a source keeps a
                // pathological item (nulls only, no SEL) playing something
                // real rather than silence.
                let active_idx = ri
                    .takes
                    .iter()
                    .position(|t| t.is_selected)
                    .or_else(|| {
                        ri.take_guid
                            .as_ref()
                            .and_then(|g| ri.takes.iter().position(|t| t.take_guid.as_ref() == Some(g)))
                    })
                    .or_else(|| ri.takes.iter().position(|t| t.source.is_some()))
                    .unwrap_or(0) as u32;
                if !takes_out.is_empty() {
                    p.takes.insert(
                        item_guid.clone(),
                        TakeList {
                            active_idx,
                            takes: takes_out,
                        },
                    );
                    summary.take_count += ri.takes.len();
                }

                summary.item_count += 1;
            }
            summary.track_count += 1;
        }

        // Second pass: resolve parent_guid via folder_depth so the
        // standalone view reflects REAPER's nesting.
        resolve_folder_parents(&mut p.tracks);
    });

    let _ = ITEM_COUNTER; // silence unused-warning when this file is the only consumer
}

/// All MIDI event types decoded from an RPP `MidiSource`.
struct DecodedMidiSource {
    notes: Vec<daw_proto::midi::MidiNote>,
    ccs: Vec<daw_proto::midi::MidiCC>,
    pitch_bends: Vec<daw_proto::midi::MidiPitchBend>,
    program_changes: Vec<daw_proto::midi::MidiProgramChange>,
    sysex: Vec<daw_proto::midi::MidiSysEx>,
    channel_pressures: Vec<daw_proto::midi::MidiChannelPressure>,
    poly_pressures: Vec<daw_proto::midi::MidiPolyPressure>,
}

/// Walk a parsed RPP `MidiSource`, demultiplex the delta-tick event
/// stream into the proto's typed per-take collections (notes, CCs,
/// pitch bends, program changes, SysEx). The renderer reads from
/// these collections and feeds them per-block to the track's
/// instrument plugin.
///
/// Time conversion: REAPER's MIDI source stores deltas at
/// `ticks_per_qn` (typically 960). All proto types use *quarter notes*
/// for `start_ppq` / `position_ppq`, so we divide accumulated ticks
/// by `ticks_per_qn`.
///
/// Aftertouch (channel pressure) and poly-pressure are currently
/// dropped — they need their own proto vectors before we can route
/// them. SysEx is preserved verbatim including the leading 0xF0 /
/// trailing 0xF7 framing bytes (REAPER's `E` lines store the raw
/// MIDI bytes per the spec).
fn decode_midi_source(midi: &dawfile_reaper::types::item::MidiSource) -> DecodedMidiSource {
    use daw_proto::midi::{
        MidiCC, MidiChannelPressure, MidiNote, MidiPitchBend, MidiPolyPressure, MidiProgramChange,
        MidiSysEx,
    };
    let tpq = midi.ticks_per_qn.max(1) as f64;
    let mut notes: Vec<MidiNote> = Vec::new();
    let mut ccs: Vec<MidiCC> = Vec::new();
    let mut pitch_bends: Vec<MidiPitchBend> = Vec::new();
    let mut program_changes: Vec<MidiProgramChange> = Vec::new();
    let mut sysex: Vec<MidiSysEx> = Vec::new();
    let mut channel_pressures: Vec<MidiChannelPressure> = Vec::new();
    let mut poly_pressures: Vec<MidiPolyPressure> = Vec::new();
    // (channel, pitch) → (start_tick, velocity, index_into_notes)
    let mut pending_notes: std::collections::HashMap<(u8, u8), (u64, u8, usize)> =
        std::collections::HashMap::new();
    let mut tick: u64 = 0;
    let mut next_note_idx: u32 = 0;
    let to_ppq = |t: u64| (t as f64) / tpq;

    for ev in &midi.events {
        tick = tick.saturating_add(ev.delta_ticks as u64);
        let Some(&status) = ev.bytes.first() else {
            continue;
        };
        let typ = status & 0xF0;
        let channel = status & 0x0F;
        match typ {
            0x90 => {
                let pitch = ev.bytes.get(1).copied().unwrap_or(0) & 0x7F;
                let velocity = ev.bytes.get(2).copied().unwrap_or(0) & 0x7F;
                if velocity == 0 {
                    if let Some((start_tick, vel, idx)) = pending_notes.remove(&(channel, pitch))
                        && let Some(n) = notes.get_mut(idx) {
                            n.length_ppq = to_ppq(tick.saturating_sub(start_tick));
                            n.velocity = vel;
                        }
                } else {
                    let idx = notes.len();
                    notes.push(MidiNote {
                        index: next_note_idx,
                        channel,
                        pitch,
                        velocity,
                        start_ppq: to_ppq(tick),
                        length_ppq: 0.0,
                        selected: false,
                        muted: false,
                    });
                    next_note_idx += 1;
                    pending_notes.insert((channel, pitch), (tick, velocity, idx));
                }
            }
            0x80 => {
                let pitch = ev.bytes.get(1).copied().unwrap_or(0) & 0x7F;
                if let Some((start_tick, vel, idx)) = pending_notes.remove(&(channel, pitch))
                    && let Some(n) = notes.get_mut(idx) {
                        n.length_ppq = to_ppq(tick.saturating_sub(start_tick));
                        n.velocity = vel;
                    }
            }
            0xB0 => {
                // Control Change.
                let controller = ev.bytes.get(1).copied().unwrap_or(0) & 0x7F;
                let value = ev.bytes.get(2).copied().unwrap_or(0) & 0x7F;
                let idx = ccs.len() as u32;
                ccs.push(MidiCC {
                    index: idx,
                    channel,
                    controller,
                    value,
                    position_ppq: to_ppq(tick),
                    selected: false,
                });
            }
            0xC0 => {
                // Program Change.
                let program = ev.bytes.get(1).copied().unwrap_or(0) & 0x7F;
                let idx = program_changes.len() as u32;
                program_changes.push(MidiProgramChange {
                    index: idx,
                    channel,
                    program,
                    position_ppq: to_ppq(tick),
                });
            }
            0xE0 => {
                // Pitch Bend: LSB then MSB, both 7-bit, combine to
                // a 14-bit unsigned then subtract 8192 for signed.
                let lsb = ev.bytes.get(1).copied().unwrap_or(0) & 0x7F;
                let msb = ev.bytes.get(2).copied().unwrap_or(0) & 0x7F;
                let unsigned = ((msb as u16) << 7) | lsb as u16;
                let signed = (unsigned as i32 - 8192) as i16;
                let idx = pitch_bends.len() as u32;
                pitch_bends.push(MidiPitchBend {
                    index: idx,
                    channel,
                    value: signed,
                    position_ppq: to_ppq(tick),
                    selected: false,
                });
            }
            0xF0
                // SysEx (status 0xF0) — store the entire frame
                // verbatim including the trailing 0xF7. Other 0xFn
                // realtime / system messages (clock, start, stop,
                // active sensing) aren't currently routed.
                if status == 0xF0 => {
                    let idx = sysex.len() as u32;
                    sysex.push(MidiSysEx {
                        index: idx,
                        position_ppq: to_ppq(tick),
                        data: ev.bytes.clone(),
                    });
                }
            0xA0 => {
                // Poly Pressure (per-note aftertouch).
                let note = ev.bytes.get(1).copied().unwrap_or(0) & 0x7F;
                let pressure = ev.bytes.get(2).copied().unwrap_or(0) & 0x7F;
                let idx = poly_pressures.len() as u32;
                poly_pressures.push(MidiPolyPressure {
                    index: idx,
                    channel,
                    note,
                    pressure,
                    position_ppq: to_ppq(tick),
                    selected: false,
                });
            }
            0xD0 => {
                // Channel Pressure (mono aftertouch).
                let pressure = ev.bytes.get(1).copied().unwrap_or(0) & 0x7F;
                let idx = channel_pressures.len() as u32;
                channel_pressures.push(MidiChannelPressure {
                    index: idx,
                    channel,
                    pressure,
                    position_ppq: to_ppq(tick),
                    selected: false,
                });
            }
            _ => {
                // 0xF1-0xFE realtime / system messages — drop.
            }
        }
    }
    DecodedMidiSource {
        notes,
        ccs,
        pitch_bends,
        program_changes,
        sysex,
        channel_pressures,
        poly_pressures,
    }
}

fn fade_curve_to_shape(curve: dawfile_reaper::types::item::FadeCurveType) -> FadeShape {
    use dawfile_reaper::types::item::FadeCurveType as F;
    match curve {
        F::Linear => FadeShape::Linear,
        F::Square => FadeShape::FastStart, // closest stand-in; proto has no
        // Square fade.
        F::SlowStartEnd => FadeShape::SlowStartEnd,
        F::FastStart => FadeShape::FastStart,
        F::FastEnd => FadeShape::FastEnd,
        F::Bezier => FadeShape::SlowStartEnd, // proto has no Bezier; pick smoothest stand-in
        F::Unknown(_) => FadeShape::Linear,
    }
}

fn build_take(item_guid: &str, index: u32, rt: &RppTake, _summary: &mut LoadedProject) -> Take {
    let take_guid = rt
        .take_guid
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let (source_type, source_file_path) = match &rt.source {
        Some(src) => {
            let st = match src.source_type {
                // Every compressed/PCM audio source REAPER's `<SOURCE ...>`
                // tag can name — not just `WAVE`. Previously only `Wave`
                // mapped to `Audio`; `Vorbis` (ogg — what session's
                // generated projects use), `Mp3`, `Flac`, and
                // `OfflineWave` all fell into `Unknown`, which silently
                // skips waveform-preview fetching (anything gated on
                // `SourceType::Audio` treated these takes as non-audio).
                RppSourceType::Wave
                | RppSourceType::OfflineWave
                | RppSourceType::Mp3
                | RppSourceType::Flac
                | RppSourceType::Vorbis => SourceType::Audio,
                RppSourceType::Midi => SourceType::Midi,
                RppSourceType::Empty => SourceType::Empty,
                RppSourceType::Video | RppSourceType::Section | RppSourceType::Unknown(_) => {
                    SourceType::Unknown
                }
            };
            let file = if src.file_path.is_empty() {
                None
            } else {
                Some(src.file_path.clone())
            };
            (st, file)
        }
        None => (SourceType::Empty, None),
    };

    Take {
        guid: take_guid,
        item_guid: item_guid.to_string(),
        index,
        is_active: false, // patched up by caller using item.take_guid
        name: rt.name.clone(),
        color: None,
        volume: rt.volpan.as_ref().map(|v| v.take_volume).unwrap_or(1.0),
        play_rate: rt.playrate.as_ref().map(|pr| pr.rate).unwrap_or(1.0),
        pitch: 0.0,
        preserve_pitch: true,
        channel_mode: {
            use dawfile_reaper::types::item::ChannelMode as CM;
            match rt.channel_mode {
                CM::Normal => 0,
                CM::ReverseStereo => 1,
                CM::MonoDownmix => 2,
                CM::MonoLeft => 3,
                CM::MonoRight => 4,
                // dawfile stores the decoded channel; re-encode to
                // REAPER's raw CHANMODE value space.
                CM::MonoChannel(ch) => ch as u32 + 2,
                CM::StereChannel(ch) => ch as u32 + 66,
                CM::Unknown(v) => v.max(0) as u32,
            }
        },
        start_offset: Duration::from_seconds(rt.slip_offset.max(0.0)),
        source_type,
        source_file_path,
        source_length: None,
        source_sample_rate: None,
        source_channels: None,
        is_midi: matches!(source_type, SourceType::Midi),
        midi_note_count: None,
    }
}

fn resolve_folder_parents(tracks: &mut [Track]) {
    // Stack-of-folder-guids walk: when entering a folder we push the
    // track guid onto the stack and mark following tracks until
    // depth decrement.
    let mut stack: Vec<String> = Vec::new();
    for t in tracks.iter_mut() {
        t.parent_guid = stack.last().cloned();
        // Apply depth AFTER setting parent (a folder's own parent is
        // the folder enclosing it, not itself).
        match t.folder_depth.signum() {
            1 => stack.push(t.guid.clone()),
            -1 => {
                for _ in 0..t.folder_depth.unsigned_abs() {
                    stack.pop();
                }
            }
            _ => {}
        }
    }
}

fn populate_markers_regions(
    daw: &Standalone,
    project_guid: &str,
    project: &ReaperProject,
    summary: &mut LoadedProject,
) {
    let _ = daw.with_project_mut(project_guid, |p| {
        // REAPER 7.62+ ruler lanes. A marker carries a lane index and
        // the project names the lanes (`RULERLANE 1 8 SONG 0 -1`), so
        // without the names a lane index is a bare number and the
        // grouping cannot be labelled. Stored where the `Project`
        // service's ruler-lane accessors already read from, so a loaded
        // project answers them the same way a hand-set one does.
        for lane in &project.ruler_lanes {
            if lane.index < 0 || lane.name.is_empty() {
                continue;
            }
            p.project_ext_state.insert(
                (
                    "daw-standalone:ruler_lanes".into(),
                    format!("{}", lane.index),
                ),
                lane.name.clone(),
            );
        }
        for mr in &project.markers_regions.markers {
            let id = next_id(&mut p.next_marker_id);
            let m = Marker {
                id: Some(id),
                position: daw_proto::Position::from_time(PositionInSeconds::from_seconds(
                    mr.position,
                )),
                name: mr.name.clone(),
                color: if mr.color == 0 {
                    None
                } else {
                    Some(native_color_to_rgb(mr.color as u32))
                },
                guid: if mr.guid.is_empty() {
                    None
                } else {
                    Some(mr.guid.clone())
                },
                lane: mr.lane.map(|l| l as u32),
            };
            p.markers.insert(id, m);
            summary.marker_count += 1;
        }
        for mr in &project.markers_regions.regions {
            let id = next_id(&mut p.next_region_id);
            let end = mr.end_position.unwrap_or(mr.position);
            let r = Region {
                id: Some(id),
                time_range: daw_proto::primitives::TimeRange::from_seconds(mr.position, end),
                name: mr.name.clone(),
                color: if mr.color == 0 {
                    None
                } else {
                    Some(native_color_to_rgb(mr.color as u32))
                },
                guid: if mr.guid.is_empty() {
                    None
                } else {
                    Some(mr.guid.clone())
                },
                lane: mr.lane.map(|l| l as u32),
            };
            p.regions.insert(id, r);
            summary.region_count += 1;
        }
    });
}

fn next_id(counter: &mut u32) -> u32 {
    let id = *counter;
    *counter = counter.saturating_add(1);
    id
}

fn populate_tempo(
    daw: &Standalone,
    project_guid: &str,
    project: &ReaperProject,
    summary: &mut LoadedProject,
) {
    let Some(env) = &project.tempo_envelope else {
        return;
    };
    let _ = daw.with_project_mut(project_guid, |p| {
        for pt in &env.points {
            let mut tp = TempoPoint::default();
            tp.position =
                daw_proto::Position::from_time(PositionInSeconds::from_seconds(pt.position));
            tp.bpm = pt.tempo.max(1.0);
            if let Some(enc) = pt.time_signature_encoded {
                let num = (enc & 0xFFFF).max(1) as u32;
                let denom = ((enc >> 16) & 0xFFFF).max(1) as u32;
                tp.time_signature = Some(TimeSignature::new(num, denom));
            }
            p.tempo_points.push(tp);
            summary.tempo_point_count += 1;
        }
    });
}

/// Decode `GROUP_FLAGS` (+ `_HIGH`, groups 33–64) into the proto
/// grouping struct. Field order is REAPER's documented one (Track
/// Grouping Parameters dialog / Ultraschall API docs):
/// 1 Volume Lead, 2 Volume Follow, 3 Pan L, 4 Pan F, 5 Mute L,
/// 6 Mute F, 7 Solo L, 8 Solo F, 9 RecArm L, 10 RecArm F,
/// 11 Polarity L, 12 Polarity F, 13 AutoMode L, 14 AutoMode F,
/// 15 Reverse Volume, 16 Reverse Pan, 17 No-lead-when-following,
/// 18 Reverse Width, 19 Width L, 20 Width F, 21 VCA Lead,
/// 22 VCA Follow, 23 VCA pre-FX Follow, 24 Media-Edit L, 25 Media-Edit F.
fn decode_grouping(low: &[u32], high: &[u32]) -> daw_proto::track::TrackGrouping {
    // 1-based field → folded u64 mask (low 32 groups | high 32).
    let mask = |field: usize| -> u64 {
        let lo = low.get(field - 1).copied().unwrap_or(0) as u64;
        let hi = high.get(field - 1).copied().unwrap_or(0) as u64;
        lo | (hi << 32)
    };
    daw_proto::track::TrackGrouping {
        volume_lead: mask(1),
        volume_follow: mask(2),
        pan_lead: mask(3),
        pan_follow: mask(4),
        mute_lead: mask(5),
        mute_follow: mask(6),
        solo_lead: mask(7),
        solo_follow: mask(8),
        recarm_lead: mask(9),
        recarm_follow: mask(10),
        polarity_lead: mask(11),
        polarity_follow: mask(12),
        vca_lead: mask(21),
        vca_follow: mask(22),
        vca_prefx_follow: mask(23),
        media_edit_lead: mask(24),
        media_edit_follow: mask(25),
    }
}

/// Convert an RPP track envelope block into the runtime envelope map
/// entry. Returns `None` for envelope types the renderer doesn't
/// evaluate yet (width, tempo, FX-param blocks live elsewhere).
///
/// Value conventions translated to the renderer's:
/// - volume: linear gain, used as-is
/// - pan: RPP −1…1 (negative = left) → 0…1 (0.5 = centre)
/// - mute: RPP >0.5 = PLAY → ours >0.5 = MUTED (inverted)
fn convert_track_envelope(
    env: &dawfile_reaper::types::envelope::Envelope,
) -> Option<(crate::sync::EnvelopeKey, crate::sync::EnvelopeData)> {
    use daw_proto::automation::{EnvelopeShape, EnvelopeType};
    use daw_proto::primitives::AutomationMode;
    use dawfile_reaper::types::envelope::EnvelopePointShape as PS;

    enum ValueMap {
        Direct,
        Pan,
        Mute,
    }
    let (ty, map) = match env.envelope_type.as_str() {
        "VOLENV2" => (EnvelopeType::Volume, ValueMap::Direct),
        "VOLENV" => (EnvelopeType::VolumePrefx, ValueMap::Direct),
        "PANENV2" => (EnvelopeType::Pan, ValueMap::Pan),
        "PANENV" => (EnvelopeType::PanPrefx, ValueMap::Pan),
        "MUTEENV" | "MUTEENV2" => (EnvelopeType::Mute, ValueMap::Mute),
        _ => return None,
    };
    let points: Vec<daw_proto::automation::EnvelopePoint> = env
        .points
        .iter()
        .enumerate()
        .map(|(i, pt)| daw_proto::automation::EnvelopePoint {
            index: i as u32,
            time: PositionInSeconds::from_seconds(pt.position),
            value: match map {
                ValueMap::Direct => pt.value,
                ValueMap::Pan => (pt.value.clamp(-1.0, 1.0) + 1.0) / 2.0,
                ValueMap::Mute => 1.0 - pt.value.clamp(0.0, 1.0),
            },
            shape: match pt.shape {
                PS::Square => EnvelopeShape::Square,
                PS::SlowStartEnd => EnvelopeShape::SlowStartEnd,
                PS::FastStart => EnvelopeShape::FastStart,
                PS::FastEnd => EnvelopeShape::FastEnd,
                PS::Bezier => EnvelopeShape::Bezier,
                PS::Linear | PS::Default => EnvelopeShape::Linear,
            },
            tension: pt.bezier_tension.unwrap_or(0.0),
            selected: pt.selected.unwrap_or(false),
        })
        .collect();
    if points.is_empty() {
        return None;
    }
    let mut data = crate::sync::EnvelopeData::new();
    data.visible = env.visible;
    data.armed = env.armed;
    data.automation_mode = if env.active {
        AutomationMode::Read
    } else {
        AutomationMode::Off
    };
    data.points = points;
    Some((crate::sync::EnvelopeKey::Track(ty), data))
}

fn populate_routing(
    daw: &Standalone,
    project_guid: &str,
    project: &ReaperProject,
    summary: &mut LoadedProject,
) {
    // Track→track sends are stored in RPP on the DESTINATION track as
    // `AUXRECV <source idx> <mode> <vol> <pan> …` — walk every track's
    // receives and register the send on the source. This is the session's
    // actual mix path (e.g. a folder with MAINSEND 0 sending into its bus:
    // Drums → DRUM BUS → MIX BUS → master); dropping them silences any
    // project mixed through busses.
    let _ = daw.with_project_mut(project_guid, |p| {
        // Snapshot track GUIDs by index for AUXRECV resolution.
        let track_guids: Vec<String> = p.tracks.iter().map(|t| t.guid.clone()).collect();
        let track_names: Vec<String> = p.tracks.iter().map(|t| t.name.clone()).collect();
        for (dest_idx, rt) in project.tracks.iter().enumerate() {
            let Some(dest_guid) = track_guids.get(dest_idx).cloned() else {
                continue;
            };
            for recv in &rt.receives {
                let src_idx = recv.source_track_index;
                if src_idx < 0 {
                    continue;
                }
                let Some(src_guid) = track_guids.get(src_idx as usize).cloned() else {
                    continue;
                };
                let mut route = daw_proto::TrackRoute::default();
                route.route_type = daw_proto::RouteType::Send;
                route.source_track_guid = src_guid.clone();
                route.dest_track_guid = Some(dest_guid.clone());
                route.dest_track_name = track_names.get(dest_idx).cloned();
                route.volume = recv.volume;
                route.pan = recv.pan;
                route.muted = recv.mute;
                route.phase_inverted = recv.invert_polarity;
                route.send_mode = match recv.mode {
                    1 => daw_proto::routing::SendMode::PreFx,
                    3 => daw_proto::routing::SendMode::PostFx, // pre-fader
                    _ => daw_proto::routing::SendMode::PostFader,
                };
                let sends = p.sends.entry(src_guid).or_default();
                route.index = sends.len() as u32;
                sends.push(route);
            }
        }

        for (src_idx, rt) in project.tracks.iter().enumerate() {
            let Some(src_guid) = track_guids.get(src_idx).cloned() else {
                continue;
            };
            for hw in &rt.hardware_outputs {
                let mut route = daw_proto::TrackRoute::default();
                route.route_type = daw_proto::RouteType::HardwareOutput;
                route.source_track_guid = src_guid.clone();
                route.hw_output_index = Some(hw.output_index as u32);
                route.hw_output_name = Some(format!("HW {}", hw.output_index));
                route.volume = hw.volume;
                route.pan = hw.pan;
                route.muted = hw.mute;
                route.phase_inverted = hw.invert_polarity;
                route.source_channels = daw_proto::ChannelMapping {
                    start_channel: 0,
                    num_channels: rt.channel_count.max(1).min(128),
                };
                let outs = p.hw_outputs.entry(src_guid.clone()).or_default();
                let i = outs.len() as u32;
                route.index = i;
                outs.push(route);
                summary.hw_output_count += 1;
            }
        }
    });
}

// ────────────────────────────────────────────────────────────────────
// FX chain population
// ────────────────────────────────────────────────────────────────────

/// Walk each track's `<FXCHAIN>` and (in REAPER 7+) `<CONTAINER>`
/// nodes, instantiate the plugin through [`daw_proto::fx::Effects`],
/// and apply state via [`crate::rpp_state`] + [`Standalone::apply_plugin_state`].
///
/// Unsupported nodes (JS scripts, AU, video, plugins whose bundle
/// can't be resolved on this host) are recorded as warnings on
/// [`LoadedProject`] and skipped — the track still loads with its
/// audio + automation intact.
fn populate_fx_chains(
    daw: &Standalone,
    project_guid: &str,
    project: &ReaperProject,
    summary: &mut LoadedProject,
) {
    use daw_proto::fx::FxChainContext;
    use daw_proto::project::ProjectContext;

    // Build (track_guid, fx_chain) pairs. Tracks were added in order
    // so we can correlate by index against the source project.
    let track_guids: Vec<String> = daw
        .read_project(project_guid, |p| {
            p.tracks.iter().map(|t| t.guid.clone()).collect()
        })
        .unwrap_or_default();

    let ctx = ProjectContext::Project(project_guid.to_string());

    for (idx, rt) in project.tracks.iter().enumerate() {
        let Some(track_guid) = track_guids.get(idx).cloned() else {
            continue;
        };
        let Some(fxc) = rt.fx_chain.as_ref() else {
            continue;
        };
        for node in &fxc.nodes {
            apply_fx_node(
                daw,
                &ctx,
                FxChainContext::Track(track_guid.clone()),
                node,
                summary,
            );
        }
    }
}

fn apply_fx_node(
    daw: &Standalone,
    ctx: &daw_proto::project::ProjectContext,
    chain_ctx: daw_proto::fx::FxChainContext,
    node: &dawfile_reaper::types::fx_chain::FxChainNode,
    summary: &mut LoadedProject,
) {
    use dawfile_reaper::types::fx_chain::{FxChainNode, PluginType};
    match node {
        FxChainNode::Plugin(p) => {
            // Skip plugin formats we don't host yet (or never will,
            // like Video). JS would need a JSFX engine.
            match p.plugin_type {
                PluginType::Vst3 | PluginType::Clap | PluginType::Vst => {}
                _ => {
                    summary.warnings.push(format!(
                        "FX skipped: '{}' (unsupported format {:?})",
                        p.name, p.plugin_type
                    ));
                    return;
                }
            }
            // Resolve the bundle on disk. RPP stores just the
            // filename ("MUtility.vst3") so we walk standard plugin
            // search paths.
            let Some(path) = resolve_plugin_path(&p.file, &p.plugin_type) else {
                summary.warnings.push(format!(
                    "FX skipped: '{}' (bundle '{}' not found in plugin search paths)",
                    p.name, p.file
                ));
                return;
            };
            // Stand-up the plugin via the existing Effects::add
            // path (which dispatches by file extension into the
            // CLAP / VST3 host).
            let Some(fx_guid) =
                daw_proto::fx::Effects::add(daw, ctx.clone(), chain_ctx.clone(), path.as_str())
            else {
                summary.warnings.push(format!(
                    "FX add failed: '{}' (load_plugin returned no instance)",
                    p.name
                ));
                return;
            };
            // Restore state if the RPP carried any.
            if !p.state_data.is_empty() {
                let decode = match p.plugin_type {
                    PluginType::Clap => crate::rpp_state::reaper_clap_to_state(&p.state_data),
                    _ => crate::rpp_state::reaper_vst3_to_daw_state(&p.state_data),
                };
                match decode {
                    Ok(blob) => {
                        if let Err(e) = daw.apply_plugin_state(&fx_guid, &blob) {
                            summary
                                .warnings
                                .push(format!("FX state apply failed for '{}': {e}", p.name));
                        }
                    }
                    Err(e) => summary
                        .warnings
                        .push(format!("FX state decode failed for '{}': {e}", p.name)),
                }
            }
            // REAPER bypass = `enabled=false`. Effects::add starts
            // enabled so only call when we need to flip it off.
            if p.bypassed {
                let _ = daw_proto::fx::Effects::set_enabled(
                    daw,
                    ctx.clone(),
                    daw_proto::fx::FxTarget {
                        context: chain_ctx.clone(),
                        fx: daw_proto::fx::FxRef::Guid(fx_guid.clone()),
                    },
                    false,
                );
            }
            if p.offline {
                let _ = daw_proto::fx::Effects::set_offline(
                    daw,
                    ctx.clone(),
                    daw_proto::fx::FxTarget {
                        context: chain_ctx.clone(),
                        fx: daw_proto::fx::FxRef::Guid(fx_guid.clone()),
                    },
                    true,
                );
            }
        }
        FxChainNode::Container(c) => {
            // REAPER 7 FX containers. The proto layer doesn't have a
            // first-class container yet — flatten children into the
            // parent chain. State + routing within the container is
            // lost; record a warning so users know.
            summary.warnings.push(format!(
                "FX container '{}' flattened (REAPER 7 container layout not yet modeled)",
                c.name
            ));
            for child in &c.children {
                apply_fx_node(daw, ctx, chain_ctx.clone(), child, summary);
            }
        }
    }
}

/// Find a plugin bundle on disk given just a filename. Walks the
/// usual VST3 / CLAP search dirs:
///
/// - `$HOME/.vst3`, `/usr/lib/vst3`, `/usr/local/lib/vst3` (Linux)
/// - `~/Library/Audio/Plug-Ins/VST3`, `/Library/Audio/Plug-Ins/VST3` (macOS)
/// - `$HOME/.clap`, `/usr/lib/clap`, `/usr/local/lib/clap` (Linux)
///
/// Returns the absolute path if found. Bare filenames in `.rpp`
/// files are how REAPER refers to plugins; the host resolves them
/// against the same dirs the OS DAW would.
fn resolve_plugin_path(
    filename: &str,
    plugin_type: &dawfile_reaper::types::fx_chain::PluginType,
) -> Option<String> {
    use dawfile_reaper::types::fx_chain::PluginType;
    use std::path::PathBuf;

    // If the file is already an absolute path that exists, take it.
    let direct = PathBuf::from(filename);
    if direct.is_absolute() && direct.exists() {
        return Some(filename.to_string());
    }

    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut roots: Vec<PathBuf> = Vec::new();
    match plugin_type {
        PluginType::Vst3 => {
            if let Some(h) = &home {
                roots.push(h.join(".vst3"));
                #[cfg(target_os = "macos")]
                roots.push(h.join("Library/Audio/Plug-Ins/VST3"));
            }
            roots.push(PathBuf::from("/usr/lib/vst3"));
            roots.push(PathBuf::from("/usr/local/lib/vst3"));
            #[cfg(target_os = "macos")]
            roots.push(PathBuf::from("/Library/Audio/Plug-Ins/VST3"));
        }
        PluginType::Clap => {
            if let Some(h) = &home {
                roots.push(h.join(".clap"));
                #[cfg(target_os = "macos")]
                roots.push(h.join("Library/Audio/Plug-Ins/CLAP"));
            }
            roots.push(PathBuf::from("/usr/lib/clap"));
            roots.push(PathBuf::from("/usr/local/lib/clap"));
            #[cfg(target_os = "macos")]
            roots.push(PathBuf::from("/Library/Audio/Plug-Ins/CLAP"));
        }
        PluginType::Vst => {
            if let Some(h) = &home {
                roots.push(h.join(".vst"));
            }
            roots.push(PathBuf::from("/usr/lib/vst"));
            roots.push(PathBuf::from("/usr/local/lib/vst"));
        }
        _ => return None,
    }

    for root in roots {
        let candidate = root.join(filename);
        if candidate.exists() {
            return candidate.to_str().map(|s| s.to_string());
        }
    }
    None
}
