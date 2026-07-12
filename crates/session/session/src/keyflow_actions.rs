use daw::module::ModuleContext;
use daw::service::transport::service::Transport as TransportService;
use daw::service::{Markers, MusicalPosition, ProjectContext, Projects, Region, Regions, TempoMap};
use keyflow::sections::colors_for_section_type;
use session_proto::SectionType;
use session_proto::ruler_lanes::{CoreLane, FtsLane, InstrumentLane, classify_marker_lane};
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use tracing::{info, warn};

const TOUCH_EPSILON_SECONDS: f64 = 0.001;
const MAX_SECTION_TIME_SELECTION_SECONDS: f64 = 60.0 * 60.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyflowAction {
    InsertSection(SectionKind),
    InsertMarker(MarkerKind),
    ConvertMarkersToSessionFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionKind {
    Intro,
    Verse,
    PreChorus,
    Chorus,
    Bridge,
    Outro,
    Instrumental,
    Solo,
    Hits,
    Interlude,
    Breakdown,
    Vamp,
    CountIn,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerKind {
    CountIn,
    Start,
    End,
    SongStart,
    SongEnd,
}

#[derive(Debug, Clone)]
struct SectionRegion {
    id: u32,
    start: f64,
    end: f64,
    section_type: SectionType,
}

pub fn init(_ctx: &ModuleContext) {}

pub fn action_for_id(action_id: &str) -> Option<KeyflowAction> {
    let normalized = normalized_action_id(action_id);
    let suffix = normalized
        .strip_prefix("fts.session.")
        .unwrap_or(normalized.as_str());
    match suffix {
        "insert_intro_region" => Some(KeyflowAction::InsertSection(SectionKind::Intro)),
        "insert_verse_region" => Some(KeyflowAction::InsertSection(SectionKind::Verse)),
        "insert_pre_chorus_region" => Some(KeyflowAction::InsertSection(SectionKind::PreChorus)),
        "insert_chorus_region" => Some(KeyflowAction::InsertSection(SectionKind::Chorus)),
        "insert_bridge_region" => Some(KeyflowAction::InsertSection(SectionKind::Bridge)),
        "insert_outro_region" => Some(KeyflowAction::InsertSection(SectionKind::Outro)),
        "insert_instrumental_region" => {
            Some(KeyflowAction::InsertSection(SectionKind::Instrumental))
        }
        "insert_solo_region" => Some(KeyflowAction::InsertSection(SectionKind::Solo)),
        "insert_hits_region" => Some(KeyflowAction::InsertSection(SectionKind::Hits)),
        "insert_interlude_region" => Some(KeyflowAction::InsertSection(SectionKind::Interlude)),
        "insert_breakdown_region" => Some(KeyflowAction::InsertSection(SectionKind::Breakdown)),
        "insert_vamp_region" => Some(KeyflowAction::InsertSection(SectionKind::Vamp)),
        "insert_count_in_region" => Some(KeyflowAction::InsertSection(SectionKind::CountIn)),
        "insert_end_region" => Some(KeyflowAction::InsertSection(SectionKind::End)),
        "insert_count_in_marker" => Some(KeyflowAction::InsertMarker(MarkerKind::CountIn)),
        "insert_start_marker" => Some(KeyflowAction::InsertMarker(MarkerKind::Start)),
        "insert_end_marker" => Some(KeyflowAction::InsertMarker(MarkerKind::End)),
        "insert_songstart_marker" => Some(KeyflowAction::InsertMarker(MarkerKind::SongStart)),
        "insert_songend_marker" => Some(KeyflowAction::InsertMarker(MarkerKind::SongEnd)),
        "convert_markers_to_session_format" => Some(KeyflowAction::ConvertMarkersToSessionFormat),
        _ => None,
    }
}

fn normalized_action_id(action_id: &str) -> String {
    action_id
        .trim()
        .to_lowercase()
        .strip_prefix("fts_session_")
        .map(|suffix| {
            format!(
                "insert_{}",
                suffix.strip_prefix("insert_").unwrap_or(suffix)
            )
        })
        .unwrap_or_else(|| action_id.trim().to_lowercase())
}

pub fn dispatch<D>(daw: &D, action: KeyflowAction)
where
    D: Projects + TransportService + Markers + Regions + TempoMap,
{
    tracing::info!(?action, "[session] Dispatching Keyflow action");
    if let Err(err) = run_action(daw, action) {
        tracing::error!(?err, "[session] Keyflow action failed");
    } else {
        tracing::info!(?action, "[session] Keyflow action completed");
    }
}

fn run_action<D>(daw: &D, action: KeyflowAction) -> eyre::Result<()>
where
    D: Projects + TransportService + Markers + Regions + TempoMap,
{
    let started = Instant::now();
    ensure_action_lane(daw, action);

    match action {
        KeyflowAction::InsertSection(kind) => insert_section_region(daw, kind)?,
        KeyflowAction::InsertMarker(kind) => {
            insert_marker(daw, kind)?;
            normalize_marker_lanes(daw)?;
        }
        KeyflowAction::ConvertMarkersToSessionFormat => {
            convert_markers_to_session_format(daw)?;
            normalize_marker_lanes(daw)?;
        }
    }

    hide_stray_lanes(daw);

    info!(
        ?action,
        elapsed_ms = started.elapsed().as_millis(),
        "[session] Keyflow action timing",
    );
    Ok(())
}

fn ensure_action_lane<D>(daw: &D, action: KeyflowAction)
where
    D: Projects,
{
    ensure_core_lanes(daw);

    let lane = match action {
        KeyflowAction::InsertSection(_) | KeyflowAction::ConvertMarkersToSessionFormat => {
            FtsLane::Core(CoreLane::Sections)
        }
        KeyflowAction::InsertMarker(kind) => classify_marker_lane(kind.name()),
    };
    // RULER_LANE_NAME uses 0-based name_key_index, *not* 1-based
    // lane_index. Mixing them rewrote lane 2 to "SECTIONS" every
    // insert and left an extra unnamed lane above SONG. See the
    // matching fix in ensure_core_lanes.
    let key_idx = match lane {
        FtsLane::Core(core) => core.name_key_index(),
        // Instrument lanes follow CoreLane::count() and stay 1-based
        // for now; if they ever break the same way, swap to a
        // dedicated `name_key_index` on InstrumentLane.
        FtsLane::Instrument(_) => lane.lane_index(),
    };
    daw.set_ruler_lane_name(ProjectContext::Current, key_idx, lane.display_name());
}

fn ensure_core_lanes<D>(daw: &D)
where
    D: Projects,
{
    let project = ProjectContext::Current;
    for lane in CoreLane::all() {
        // RULER_LANE_NAME:N / RULER_LANE_FLAGS:N use the 0-based
        // *name-table* index (N=0 names the leftmost UI lane), not
        // the 1-based `lane_index` markers/regions use via
        // `I_LANENUMBER`. Passing `lane_index` (1-based) here named
        // lane 2 in the UI and left an empty unnamed lane 1 above
        // SONG. Use `name_key_index` so the rename hits the lane the
        // user actually sees.
        let key_idx = lane.name_key_index();
        daw.set_ruler_lane_name(project.clone(), key_idx, lane.display_name());
        // SECTIONS = 8 (default region lane), SONG = 4 (default
        // marker lane). The flag steers *future* inserts; the
        // explicit `set_lane` calls in normalize_section_regions /
        // place_marker_via_action handle existing entities.
        daw.set_project_info(
            project.clone(),
            &format!("RULER_LANE_FLAGS:{}", key_idx),
            lane.flags() as f64,
        );
    }
}

/// Hide any ruler lanes that aren't part of the FTS layout and have no
/// markers/regions on them. REAPER's fresh-project "1" fallback lane can
/// linger after FTS lanes are registered; this clears it without touching
/// lanes the user has actually populated.
fn hide_stray_lanes<D>(daw: &D)
where
    D: Projects + Markers + Regions,
{
    let project = ProjectContext::Current;
    let valid_names: HashSet<&str> = CoreLane::all()
        .iter()
        .map(|l| l.display_name())
        .chain(InstrumentLane::all().iter().map(|l| l.display_name()))
        .collect();

    let mut used_lanes: HashSet<u32> = HashSet::new();
    for m in Markers::all(daw, project.clone()) {
        if let Some(lane) = m.lane {
            used_lanes.insert(lane);
        }
    }
    for r in Regions::all(daw, project.clone()) {
        if let Some(lane) = r.lane {
            used_lanes.insert(lane);
        }
    }

    // Probe a fixed range past the named-lanes count instead of
    // stopping at `ruler_lane_count` — REAPER sometimes leaves a
    // trailing empty lane after a sequence of `RULER_LANE_ORDER`
    // inserts, and that empty lane sits beyond what `ruler_lane_count`
    // reports. 12 is generous (we currently use 3 core lanes and
    // ~8 instrument lanes; anything past that is the user's).
    //
    // Both `RULER_LANE_HIDDEN:N` and `I_LANENUMBER` use the same
    // 0-based index now, so a marker with `lane=Some(N)` shows up in
    // `used_lanes` with the same N we'd hide.
    for key_idx in 0u32..12 {
        let name = daw.get_ruler_lane_name(project.clone(), key_idx);
        if valid_names.contains(name.trim()) || used_lanes.contains(&key_idx) {
            continue;
        }
        daw.set_project_info(
            project.clone(),
            &format!("RULER_LANE_HIDDEN:{key_idx}"),
            1.0,
        );
    }
}

fn convert_markers_to_session_format<D>(daw: &D) -> eyre::Result<()>
where
    D: Projects + Markers + Regions,
{
    let project = ProjectContext::Current;

    let project_name = daw
        .info(project.clone())
        .ok()
        .map(|info| info.name)
        .filter(|name| !name.is_empty() && name != "Untitled")
        .unwrap_or_else(|| "Song".to_string());

    // Markers with no time component can't anchor a region — drop them
    // up front so subsequent indexing maps 1:1 with `marker_positions`.
    let mut all_markers: Vec<_> = Markers::all(daw, project.clone())
        .into_iter()
        .filter(|m| m.position.seconds().is_some())
        .collect();
    all_markers.sort_by(|a, b| {
        a.position
            .seconds()
            .unwrap()
            .total_cmp(&b.position.seconds().unwrap())
    });

    let marker_positions: Vec<f64> = all_markers
        .iter()
        .map(|m| m.position.seconds().unwrap())
        .collect();

    let songstart_pos = all_markers
        .iter()
        .find(|m| m.name.trim().eq_ignore_ascii_case("SONGSTART"))
        .and_then(|m| m.position.seconds());
    let songend_pos = all_markers
        .iter()
        .find(|m| m.name.trim().eq_ignore_ascii_case("SONGEND"))
        .and_then(|m| m.position.seconds());

    let section_lane = CoreLane::Sections.lane_index();
    let song_lane = CoreLane::Song.lane_index();

    let mut converted_section_ends: Vec<f64> = Vec::new();
    let mut converted_section_starts: Vec<f64> = Vec::new();

    for (i, marker) in all_markers.iter().enumerate() {
        let Some(id) = marker.id else { continue };
        let Some(section_type) = parse_region_section_type(&marker.name) else {
            continue;
        };

        let start = marker_positions[i];
        let end = marker_positions
            .iter()
            .skip(i + 1)
            .find(|&&p| p > start + TOUCH_EPSILON_SECONDS)
            .copied()
            .or(songend_pos)
            .unwrap_or(start);

        if end <= start {
            warn!(
                marker = %marker.name,
                start,
                "[session] Convert: skipping marker — no end position found",
            );
            continue;
        }

        let region_id = Regions::add(daw, project.clone(), start, end, &marker.name)?;
        Regions::set_lane(daw, project.clone(), region_id, Some(section_lane))?;
        Regions::set_color(
            daw,
            project.clone(),
            region_id,
            section_type_color(section_type),
        )?;
        Markers::remove(daw, project.clone(), id)?;

        converted_section_starts.push(start);
        converted_section_ends.push(end);
    }

    let song_start =
        songstart_pos.or_else(|| converted_section_starts.iter().cloned().reduce(f64::min));
    let song_end = songend_pos.or_else(|| converted_section_ends.iter().cloned().reduce(f64::max));

    if let (Some(start), Some(end)) = (song_start, song_end)
        && end > start
    {
        let already_has_song_region = Regions::all(daw, project.clone())
            .iter()
            .any(|r| r.lane == Some(song_lane));
        if !already_has_song_region {
            let id = Regions::add(daw, project.clone(), start, end, &project_name)?;
            Regions::set_lane(daw, project.clone(), id, Some(song_lane))?;
        }
    }

    info!(
        sections_converted = converted_section_starts.len(),
        project = %project_name,
        "[session] Convert markers to session format",
    );
    Ok(())
}

fn insert_marker<D>(daw: &D, kind: MarkerKind) -> eyre::Result<()>
where
    D: TransportService + Markers + TempoMap,
{
    let started = Instant::now();
    let project = ProjectContext::Current;
    let position = edit_cursor_position(daw, project.clone());
    let name = kind.name();
    let lane = classify_marker_lane(name).lane_index();
    let id = Markers::add(daw, project.clone(), position, name)?;
    Markers::set_color(daw, project.clone(), id, kind.default_color())?;
    Markers::set_lane(daw, project.clone(), id, Some(lane))?;
    if kind == MarkerKind::CountIn {
        let cursor_position = two_measures_after(daw, project.clone(), position);
        TransportService::set_position(daw, project, cursor_position)?;
    }
    info!(
        ?kind,
        elapsed_ms = started.elapsed().as_millis(),
        "[session] Keyflow marker insert timing",
    );
    Ok(())
}

fn two_measures_after<D>(daw: &D, project: ProjectContext, position: f64) -> f64
where
    D: TempoMap,
{
    let (measure, beat, fraction) = daw.time_to_musical(project.clone(), position);
    daw.musical_to_time(project, measure + 2, beat, fraction)
}

fn insert_section_region<D>(daw: &D, kind: SectionKind) -> eyre::Result<()>
where
    D: TransportService + Regions + TempoMap,
{
    let started = Instant::now();
    let project = ProjectContext::Current;
    let position = edit_cursor_position(daw, project.clone());
    let time_selection = daw.get_time_selection(project.clone());
    let regions = Regions::all(daw, project.clone());
    let used_time_selection = time_selection.as_ref().is_some_and(|selection| {
        is_valid_time_selection(selection.start_seconds, selection.end_seconds)
    });
    let (start, end) = infer_insert_bounds(daw, kind, position, time_selection.as_ref(), &regions);
    info!(
        kind = ?kind,
        position,
        time_selection_start = ?time_selection.as_ref().map(|selection| selection.start_seconds),
        time_selection_end = ?time_selection.as_ref().map(|selection| selection.end_seconds),
        insert_start = start,
        insert_end = end,
        insert_length_seconds = end - start,
        "[session] Keyflow section bounds",
    );
    info!("[session] Keyflow section insert: carving overlapping regions");
    carve_overlapping_section_regions(daw, &regions, start, end)?;
    info!("[session] Keyflow section insert: carve complete");
    info!("[session] Keyflow section insert: adding region");
    let id = Regions::add(
        daw,
        project.clone(),
        start,
        end,
        &kind.section_type().abbreviation(),
    )?;
    info!(id, "[session] Keyflow section insert: add complete");
    info!(id, "[session] Keyflow section insert: setting color");
    Regions::set_color(
        daw,
        project.clone(),
        id,
        section_type_color(kind.section_type()),
    )?;
    info!(id, "[session] Keyflow section insert: color complete");
    info!(
        end,
        "[session] Keyflow section insert: moving cursor to end"
    );
    TransportService::set_position(daw, project.clone(), end)?;
    info!("[session] Keyflow section insert: cursor move complete");
    if used_time_selection {
        info!("[session] Keyflow section insert: clearing consumed time selection");
        TransportService::clear_time_selection(daw, project.clone())?;
    }

    info!("[session] Keyflow section insert: reloading regions");
    let regions = Regions::all(daw, project);
    info!(
        region_count = regions.len(),
        "[session] Keyflow section insert: reloaded regions",
    );
    info!("[session] Keyflow section insert: normalizing section regions");
    normalize_section_regions(daw, regions)?;
    info!("[session] Keyflow section insert: normalize complete");
    info!(
        ?kind,
        elapsed_ms = started.elapsed().as_millis(),
        "[session] Keyflow section insert timing",
    );
    Ok(())
}

fn edit_cursor_position<D>(daw: &D, project: ProjectContext) -> f64
where
    D: TransportService,
{
    daw.get_state(project.clone())
        .edit_position
        .time
        .map(|position| position.as_seconds())
        .unwrap_or_else(|| daw.get_position(project))
}

fn infer_insert_bounds<D>(
    daw: &D,
    kind: SectionKind,
    position: f64,
    time_selection: Option<&daw::service::LoopRegion>,
    regions: &[Region],
) -> (f64, f64)
where
    D: TempoMap,
{
    if let Some(selection) = time_selection
        .filter(|selection| is_valid_time_selection(selection.start_seconds, selection.end_seconds))
    {
        return (selection.start_seconds, selection.end_seconds);
    }
    if let Some(selection) = time_selection.filter(|selection| {
        is_valid_range(selection.start_seconds, selection.end_seconds)
            && !is_valid_time_selection(selection.start_seconds, selection.end_seconds)
    }) {
        warn!(
            kind = ?kind,
            selection_start = selection.start_seconds,
            selection_end = selection.end_seconds,
            selection_length_seconds = selection.end_seconds - selection.start_seconds,
            "[session] Ignoring oversized Keyflow section time selection",
        );
    }

    let musical_end = default_section_end(daw, kind, position);
    let next_section_start = regions
        .iter()
        .filter(|region| region.start_seconds() > position + TOUCH_EPSILON_SECONDS)
        .filter(|region| region.lane == Some(CoreLane::Sections.lane_index()))
        .map(Region::start_seconds)
        .min_by(f64::total_cmp);
    let end = choose_default_end(position, musical_end, next_section_start);

    (position, end)
}

fn carve_overlapping_section_regions<D>(
    daw: &D,
    regions: &[Region],
    insert_start: f64,
    insert_end: f64,
) -> eyre::Result<()>
where
    D: Regions,
{
    let section_lane = CoreLane::Sections.lane_index();
    let project = ProjectContext::Current;

    for region in regions {
        if region.lane != Some(section_lane) && parse_region_section_type(&region.name).is_none() {
            continue;
        }
        let Some(id) = region.id else {
            continue;
        };

        let start = region.start_seconds();
        let end = region.end_seconds();
        if end <= insert_start + TOUCH_EPSILON_SECONDS
            || start >= insert_end - TOUCH_EPSILON_SECONDS
        {
            continue;
        }

        if start < insert_start - TOUCH_EPSILON_SECONDS {
            info!(
                id,
                region_start = start,
                region_end = end,
                new_end = insert_start,
                "[session] Carving section region: trimming right edge",
            );
            Regions::set_bounds(daw, project.clone(), id, start, insert_start)?;
        } else if end > insert_end + TOUCH_EPSILON_SECONDS {
            info!(
                id,
                region_start = start,
                region_end = end,
                new_start = insert_end,
                "[session] Carving section region: trimming left edge",
            );
            Regions::set_bounds(daw, project.clone(), id, insert_end, end)?;
        } else {
            info!(
                id,
                region_start = start,
                region_end = end,
                "[session] Carving section region: removing covered region",
            );
            Regions::remove(daw, project.clone(), id)?;
        }
    }

    Ok(())
}

fn choose_default_end(position: f64, musical_end: f64, next_section_start: Option<f64>) -> f64 {
    next_section_start
        .filter(|next| next.is_finite())
        .map(|next| next.min(musical_end))
        .unwrap_or(musical_end)
        .max(position + TOUCH_EPSILON_SECONDS)
}

fn is_valid_range(start: f64, end: f64) -> bool {
    start.is_finite() && end.is_finite() && end > start + TOUCH_EPSILON_SECONDS
}

fn is_valid_time_selection(start: f64, end: f64) -> bool {
    is_valid_range(start, end) && end - start <= MAX_SECTION_TIME_SELECTION_SECONDS
}

fn default_section_end<D>(daw: &D, kind: SectionKind, position: f64) -> f64
where
    D: TempoMap,
{
    let project = ProjectContext::Current;
    let (measure, beat, fraction) = daw.time_to_musical(project.clone(), position);
    let start = musical_position_from_fraction(measure, beat, fraction);
    let end = MusicalPosition::new(
        start.measure + kind.default_measure_count() as i32,
        start.beat,
        start.subdivision,
    );
    let end_fraction = end.subdivision as f64 / 1000.0;
    daw.musical_to_time(project, end.measure, end.beat, end_fraction)
}

fn musical_position_from_fraction(measure: i32, beat: i32, fraction: f64) -> MusicalPosition {
    let subdivision = if fraction.is_finite() {
        (fraction * 1000.0).round() as i32
    } else {
        0
    };
    MusicalPosition::new(measure, beat, subdivision.clamp(0, 999))
}

fn normalize_section_regions<D>(daw: &D, regions: Vec<Region>) -> eyre::Result<()>
where
    D: Regions,
{
    let existing: HashMap<u32, (String, Option<u32>, Option<u32>)> = regions
        .iter()
        .filter_map(|region| Some((region.id?, (region.name.clone(), region.lane, region.color))))
        .collect();
    let normalized = normalized_section_updates(regions);
    let project = ProjectContext::Current;
    for update in normalized {
        let SectionUpdate {
            id,
            name,
            section_type,
        } = update;
        let desired_color = section_type_color(section_type);
        let current = existing.get(&id);
        if current
            .map(|(current_name, _, _)| current_name != &name)
            .unwrap_or(true)
        {
            info!(id, name, "[session] Normalizing section region: rename");
            Regions::rename(daw, project.clone(), id, &name)?;
        }
        if current
            .map(|(_, _, current_color)| *current_color != Some(desired_color))
            .unwrap_or(true)
        {
            info!(
                id,
                color = desired_color,
                "[session] Normalizing section region: set color",
            );
            Regions::set_color(daw, project.clone(), id, desired_color)?;
        }
        // Force the SECTIONS lane assignment too. `Regions::add`
        // lands in REAPER's default region lane, which is only
        // guaranteed to be SECTIONS after `ensure_core_lanes` has
        // written `RULER_LANE_FLAGS:N=8` — and that flag only steers
        // *future* inserts, not existing ones. Without this loop
        // applying the lane explicitly, freshly-stamped section
        // regions stick in lane 0 (REAPER's unnamed default).
        let section_lane = CoreLane::Sections.lane_index();
        if current
            .map(|(_, current_lane, _)| *current_lane != Some(section_lane))
            .unwrap_or(true)
        {
            info!(
                id,
                lane = section_lane,
                "[session] Normalizing section region: set lane",
            );
            Regions::set_lane(daw, project.clone(), id, Some(section_lane))?;
        }
    }
    Ok(())
}

fn normalize_marker_lanes<D>(daw: &D) -> eyre::Result<()>
where
    D: Markers,
{
    let project = ProjectContext::Current;
    for marker in Markers::all(daw, project.clone()) {
        let Some(id) = marker.id else {
            continue;
        };
        if let Some(color) = marker_color_for_name(&marker.name)
            && marker.color != Some(color)
        {
            Markers::set_color(daw, project.clone(), id, color)?;
        }
        let lane = classify_marker_lane(&marker.name).lane_index();
        if marker.lane != Some(lane) {
            Markers::set_lane(daw, project.clone(), id, Some(lane))?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
struct SectionUpdate {
    id: u32,
    name: String,
    section_type: SectionType,
}

#[cfg(test)]
fn normalized_section_names(regions: Vec<Region>) -> Vec<(u32, String)> {
    normalized_section_updates(regions)
        .into_iter()
        .map(|update| (update.id, update.name))
        .collect()
}

fn normalized_section_updates(regions: Vec<Region>) -> Vec<SectionUpdate> {
    // Song boundaries: SONG-lane regions delimit songs on a shared timeline.
    // Section numbering resets at each song, so "Verse 2" means the second
    // verse *within its song*, not the second across the whole project.
    let song_lane = CoreLane::Song.lane_index();
    let mut song_starts: Vec<f64> = regions
        .iter()
        .filter(|region| region.lane == Some(song_lane))
        .map(|region| region.start_seconds())
        .collect();
    song_starts.sort_by(f64::total_cmp);

    let mut sections: Vec<_> = regions
        .into_iter()
        .filter(|region| {
            region.lane == Some(CoreLane::Sections.lane_index())
                || parse_region_section_type(&region.name).is_some()
        })
        .filter_map(|region| {
            Some(SectionRegion {
                id: region.id?,
                start: region.start_seconds(),
                end: region.end_seconds(),
                section_type: parse_region_section_type(&region.name)?,
            })
        })
        .collect();

    sections.sort_by(|a, b| {
        a.start
            .total_cmp(&b.start)
            .then_with(|| a.end.total_cmp(&b.end))
    });

    // Which song a section belongs to = the last SONG start at or before it.
    // Sections before the first song (or when there are no song regions) fall
    // into bucket 0, preserving the single-song / whole-project behavior.
    let song_of = |start: f64| -> usize {
        song_starts
            .partition_point(|&song_start| song_start <= start)
            .saturating_sub(1)
    };

    let mut by_song: std::collections::BTreeMap<usize, Vec<SectionRegion>> =
        std::collections::BTreeMap::new();
    for section in sections {
        by_song.entry(song_of(section.start)).or_default().push(section);
    }

    let mut updates = Vec::new();
    for (_song, song_sections) in by_song {
        updates.extend(number_sections_in_song(song_sections));
    }
    updates
}

/// Number one song's sections: each section type is numbered by occurrence
/// within the song (Verse 1, Verse 2, …), touching sections of the same type
/// collapsing into one lettered group.
fn number_sections_in_song(sections: Vec<SectionRegion>) -> Vec<SectionUpdate> {
    let mut by_type: HashMap<String, Vec<SectionRegion>> = HashMap::new();
    for section in sections {
        by_type
            .entry(section.section_type.key())
            .or_default()
            .push(section);
    }

    let mut updates = Vec::new();
    for mut same_type in by_type.into_values() {
        same_type.sort_by(|a, b| {
            a.start
                .total_cmp(&b.start)
                .then_with(|| a.end.total_cmp(&b.end))
        });
        let groups = section_groups(&same_type);
        let multiple_groups = groups.len() > 1;

        for (group_index, group) in groups.iter().enumerate() {
            let group_number = group_index + 1;
            let multiple_in_group = group.len() > 1;

            for (letter_index, section_index) in group.iter().enumerate() {
                let section = &same_type[*section_index];
                let new_name = section_name(
                    &section.section_type,
                    multiple_groups,
                    group_number,
                    multiple_in_group,
                    letter_index,
                );
                updates.push(SectionUpdate {
                    id: section.id,
                    name: new_name,
                    section_type: section.section_type.clone(),
                });
            }
        }
    }

    updates
}

fn section_groups(sections: &[SectionRegion]) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (idx, section) in sections.iter().enumerate() {
        if let Some(group) = groups.last_mut()
            && let Some(prev_idx) = group.last()
        {
            let prev = &sections[*prev_idx];
            if (prev.end - section.start).abs() <= TOUCH_EPSILON_SECONDS {
                group.push(idx);
                continue;
            }
        }
        groups.push(vec![idx]);
    }
    groups
}

fn section_name(
    section_type: &SectionType,
    multiple_groups: bool,
    group_number: usize,
    multiple_in_group: bool,
    letter_index: usize,
) -> String {
    let base = section_type.abbreviation();
    match (multiple_groups, multiple_in_group) {
        (false, false) => base,
        (false, true) => format!("{} {}", base, alpha_suffix(letter_index)),
        (true, false) => format!("{} {}", base, group_number),
        (true, true) => format!("{} {}{}", base, group_number, alpha_suffix(letter_index)),
    }
}

fn alpha_suffix(index: usize) -> String {
    let mut n = index;
    let mut chars = Vec::new();
    loop {
        chars.push((b'A' + (n % 26) as u8) as char);
        n /= 26;
        if n == 0 {
            break;
        }
        n -= 1;
    }
    chars.iter().rev().collect()
}

fn parse_region_section_type(name: &str) -> Option<SectionType> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut token = trimmed.split_whitespace().next()?;
    if token.ends_with(':') {
        token = &token[..token.len() - 1];
    }

    SectionType::parse(token)
        .ok()
        .or_else(|| SectionType::parse(trimmed).ok())
}

fn section_type_color(section_type: SectionType) -> u32 {
    if matches!(section_type, SectionType::CountIn) {
        return MarkerKind::CountIn.default_color();
    }

    colors_for_section_type(&section_type)
        .bright
        .to_reaper_native() as u32
}

fn marker_color_for_name(name: &str) -> Option<u32> {
    let upper = name.trim().to_uppercase();
    match upper.as_str() {
        "COUNT-IN" | "COUNT IN" | "COUNTIN" => Some(MarkerKind::CountIn.default_color()),
        "SONGSTART" => Some(MarkerKind::SongStart.default_color()),
        "SONGEND" => Some(MarkerKind::SongEnd.default_color()),
        "=START" => Some(MarkerKind::Start.default_color()),
        "=END" => Some(MarkerKind::End.default_color()),
        _ => None,
    }
}

impl SectionKind {
    fn section_type(self) -> SectionType {
        match self {
            Self::Intro => SectionType::Intro,
            Self::Verse => SectionType::Verse,
            Self::PreChorus => SectionType::Pre(Box::new(SectionType::Chorus)),
            Self::Chorus => SectionType::Chorus,
            Self::Bridge => SectionType::Bridge,
            Self::Outro => SectionType::Outro,
            Self::Instrumental => SectionType::Instrumental,
            Self::Solo => SectionType::Solo,
            Self::Hits => SectionType::Hits,
            Self::Interlude => SectionType::Interlude,
            Self::Breakdown => SectionType::Breakdown,
            Self::Vamp => SectionType::Vamp,
            Self::CountIn => SectionType::CountIn,
            Self::End => SectionType::End,
        }
    }

    fn default_measure_count(self) -> u32 {
        self.verified_default_measure_count().unwrap_or(4)
    }

    fn verified_default_measure_count(self) -> Option<u32> {
        match self {
            Self::CountIn => Some(2),
            Self::Verse | Self::Chorus | Self::Bridge => Some(8),
            Self::Intro | Self::Outro | Self::Instrumental => Some(4),
            Self::PreChorus
            | Self::Solo
            | Self::Hits
            | Self::Interlude
            | Self::Breakdown
            | Self::Vamp
            | Self::End => None,
        }
    }
}

impl MarkerKind {
    fn name(self) -> &'static str {
        match self {
            Self::CountIn => "COUNT-IN",
            Self::Start => "=START",
            Self::End => "=END",
            Self::SongStart => "SONGSTART",
            Self::SongEnd => "SONGEND",
        }
    }

    fn default_color(self) -> u32 {
        match self {
            Self::CountIn => reaper_native_rgb(0xEC4899), // pink-500
            Self::SongStart => reaper_native_rgb(0x22C55E), // green-500
            Self::Start => reaper_native_rgb(0x737373),   // neutral-500
            Self::End => reaper_native_rgb(0xDC2626),     // red-600
            Self::SongEnd => reaper_native_rgb(0x475569), // slate-600
        }
    }
}

#[cfg(target_os = "windows")]
const fn reaper_native_rgb(rgb: u32) -> u32 {
    let r = rgb & 0x0000FF;
    let g = rgb & 0x00FF00;
    let b = rgb & 0xFF0000;
    0x01000000 | (r << 16) | g | (b >> 16)
}

#[cfg(not(target_os = "windows"))]
const fn reaper_native_rgb(rgb: u32) -> u32 {
    0x01000000 | (rgb & 0xFFFFFF)
}

#[cfg(test)]
mod tests {
    use super::*;
    use daw::service::Region;

    fn region(id: u32, start: f64, end: f64, name: &str) -> Region {
        let mut region = Region::from_seconds(start, end, name.to_string());
        region.id = Some(id);
        region.lane = Some(CoreLane::Sections.lane_index());
        region
    }

    #[test]
    fn one_section_keeps_plain_abbreviation() {
        assert_eq!(
            normalized_section_names(vec![region(1, 0.0, 8.0, "Chorus")]),
            vec![(1, "CH".to_string())]
        );
    }

    #[test]
    fn touching_sections_get_letters() {
        assert_eq!(
            normalized_section_names(vec![region(1, 0.0, 8.0, "CH"), region(2, 8.0, 16.0, "CH"),]),
            vec![(1, "CH A".to_string()), (2, "CH B".to_string())]
        );
    }

    #[test]
    fn separated_sections_get_numbers() {
        assert_eq!(
            normalized_section_names(vec![region(1, 0.0, 8.0, "CH"), region(2, 20.0, 28.0, "CH"),]),
            vec![(1, "CH 1".to_string()), (2, "CH 2".to_string())]
        );
    }

    #[test]
    fn separated_touching_runs_get_numbers_and_letters() {
        assert_eq!(
            normalized_section_names(vec![
                region(1, 0.0, 8.0, "CH"),
                region(2, 8.0, 16.0, "CH"),
                region(3, 32.0, 40.0, "CH"),
            ]),
            vec![
                (1, "CH 1A".to_string()),
                (2, "CH 1B".to_string()),
                (3, "CH 2".to_string()),
            ]
        );
    }

    #[test]
    fn pre_chorus_uses_keyflow_abbreviation() {
        assert_eq!(
            section_name(
                &SectionType::Pre(Box::new(SectionType::Chorus)),
                false,
                1,
                false,
                0,
            ),
            "PRE-CH"
        );
    }

    #[test]
    fn default_end_is_capped_by_next_section() {
        assert_eq!(choose_default_end(4.0, 20.0, Some(12.0)), 12.0);
    }

    #[test]
    fn default_end_uses_musical_end_without_next_section() {
        assert_eq!(choose_default_end(4.0, 20.0, None), 20.0);
    }

    #[test]
    fn oversized_time_selection_is_not_valid_for_section_insert() {
        assert!(is_valid_time_selection(4.0, 12.0));
        assert!(!is_valid_time_selection(0.0, 64_000.0));
    }

    #[test]
    fn section_default_measure_counts_match_session_convention() {
        assert_eq!(SectionKind::Verse.default_measure_count(), 8);
        assert_eq!(SectionKind::Chorus.default_measure_count(), 8);
        assert_eq!(SectionKind::Bridge.default_measure_count(), 8);
        assert_eq!(SectionKind::Intro.default_measure_count(), 4);
        assert_eq!(SectionKind::Outro.default_measure_count(), 4);
        assert_eq!(SectionKind::Instrumental.default_measure_count(), 4);
        assert_eq!(SectionKind::CountIn.default_measure_count(), 2);
    }

    #[test]
    fn unverified_section_defaults_fall_back_to_four_measures() {
        assert_eq!(SectionKind::PreChorus.default_measure_count(), 4);
        assert_eq!(SectionKind::Solo.default_measure_count(), 4);
        assert_eq!(SectionKind::Hits.default_measure_count(), 4);
        assert_eq!(SectionKind::Interlude.default_measure_count(), 4);
        assert_eq!(SectionKind::Breakdown.default_measure_count(), 4);
        assert_eq!(SectionKind::Vamp.default_measure_count(), 4);
        assert_eq!(SectionKind::End.default_measure_count(), 4);
    }

    #[test]
    fn musical_position_from_fraction_converts_to_subdivision() {
        assert_eq!(
            musical_position_from_fraction(4, 2, 0.5),
            MusicalPosition::new(4, 2, 500)
        );
        assert_eq!(
            musical_position_from_fraction(4, 2, f64::INFINITY),
            MusicalPosition::new(4, 2, 0)
        );
    }

    #[test]
    fn section_colors_use_keyflow_palette() {
        assert_eq!(
            section_type_color(SectionType::Chorus),
            colors_for_section_type(&SectionType::Chorus)
                .bright
                .to_reaper_native() as u32
        );
        assert_eq!(
            section_type_color(SectionType::CountIn),
            MarkerKind::CountIn.default_color()
        );
    }

    #[test]
    fn structural_markers_have_default_colors() {
        assert_eq!(
            marker_color_for_name("COUNT-IN"),
            Some(MarkerKind::CountIn.default_color())
        );
        assert_eq!(
            marker_color_for_name("SONGSTART"),
            Some(MarkerKind::SongStart.default_color())
        );
        assert_eq!(
            marker_color_for_name("=END"),
            Some(MarkerKind::End.default_color())
        );
    }

    #[test]
    fn action_lookup_accepts_reaper_command_names() {
        assert_eq!(
            action_for_id("FTS_SESSION_INSERT_CHORUS_REGION"),
            Some(KeyflowAction::InsertSection(SectionKind::Chorus))
        );
        assert_eq!(
            action_for_id("FTS_SESSION_INSERT_SONGSTART_MARKER"),
            Some(KeyflowAction::InsertMarker(MarkerKind::SongStart))
        );
    }

    #[test]
    fn action_lookup_accepts_internal_action_ids() {
        assert_eq!(
            action_for_id("insert_chorus_region"),
            Some(KeyflowAction::InsertSection(SectionKind::Chorus))
        );
        assert_eq!(
            action_for_id("fts.session.insert_songstart_marker"),
            Some(KeyflowAction::InsertMarker(MarkerKind::SongStart))
        );
    }
}

// ── architect::actions declaration ──────────────────────────────────────
//
// `KeyflowAction` / `action_for_id` / `dispatch` above stay put — still the
// live path `daw_module.rs`'s dispatch chain calls into. Additive
// declarative layer only, mirroring `setlist_actions`'s migration: same
// twenty handlers, now with real `ActionMeta` instead of only living in
// `session_actions`'s `actions_proto::define_actions!` block (`lib.rs`).
// Namespace `FTS_SESSION` matches the existing REAPER command-id
// convention (`FTS_SESSION_INSERT_INTRO_REGION`, etc).

/// Bridges the twenty keyflow-insert actions onto `#[architect::actions]`.
/// Every method forwards to the existing synchronous `dispatch` — no
/// behavior change, just a declarative front door with real metadata.
pub struct KeyflowActionsImpl<D> {
    daw: D,
}

#[architect::actions(namespace = "FTS_SESSION")]
pub trait KeyflowActions {
    #[action(
        description = "Insert an Intro section region at the current edit cursor",
        category = "Session",
        group = "Edit"
    )]
    fn insert_intro_region(&self);
    #[action(
        description = "Insert a Verse section region at the current edit cursor",
        category = "Session",
        group = "Edit"
    )]
    fn insert_verse_region(&self);
    #[action(
        description = "Insert a Pre-Chorus section region at the current edit cursor",
        category = "Session",
        group = "Edit"
    )]
    fn insert_pre_chorus_region(&self);
    #[action(
        description = "Insert a Chorus section region at the current edit cursor",
        category = "Session",
        group = "Edit"
    )]
    fn insert_chorus_region(&self);
    #[action(
        description = "Insert a Bridge section region at the current edit cursor",
        category = "Session",
        group = "Edit"
    )]
    fn insert_bridge_region(&self);
    #[action(
        description = "Insert an Outro section region at the current edit cursor",
        category = "Session",
        group = "Edit"
    )]
    fn insert_outro_region(&self);
    #[action(
        description = "Insert an Instrumental section region at the current edit cursor",
        category = "Session",
        group = "Edit"
    )]
    fn insert_instrumental_region(&self);
    #[action(
        description = "Insert a Solo section region at the current edit cursor",
        category = "Session",
        group = "Edit"
    )]
    fn insert_solo_region(&self);
    #[action(
        description = "Insert a Hits section region at the current edit cursor",
        category = "Session",
        group = "Edit"
    )]
    fn insert_hits_region(&self);
    #[action(
        description = "Insert an Interlude section region at the current edit cursor",
        category = "Session",
        group = "Edit"
    )]
    fn insert_interlude_region(&self);
    #[action(
        description = "Insert a Breakdown section region at the current edit cursor",
        category = "Session",
        group = "Edit"
    )]
    fn insert_breakdown_region(&self);
    #[action(
        description = "Insert a Vamp section region at the current edit cursor",
        category = "Session",
        group = "Edit"
    )]
    fn insert_vamp_region(&self);
    #[action(
        description = "Insert a Count-In section region at the current edit cursor",
        category = "Session",
        group = "Edit"
    )]
    fn insert_count_in_region(&self);
    #[action(
        description = "Insert an End section region at the current edit cursor",
        category = "Session",
        group = "Edit"
    )]
    fn insert_end_region(&self);
    #[action(
        description = "Insert a Count-In marker on the MARKS ruler lane",
        category = "Session",
        group = "Edit"
    )]
    fn insert_count_in_marker(&self);
    #[action(
        description = "Insert an =START marker on the MARKS ruler lane",
        category = "Session",
        group = "Edit"
    )]
    fn insert_start_marker(&self);
    #[action(
        description = "Insert an =END marker on the MARKS ruler lane",
        category = "Session",
        group = "Edit"
    )]
    fn insert_end_marker(&self);
    #[action(
        description = "Insert a SONGSTART marker on the MARKS ruler lane",
        category = "Session",
        group = "Edit"
    )]
    fn insert_songstart_marker(&self);
    #[action(
        description = "Insert a SONGEND marker on the MARKS ruler lane",
        category = "Session",
        group = "Edit"
    )]
    fn insert_songend_marker(&self);
    #[action(
        description = "Convert plain section-name markers into FTS section regions and add a SONG-lane region named after the project",
        category = "Session",
        group = "Edit"
    )]
    fn convert_markers_to_session_format(&self);
}

impl<D> KeyflowActions for KeyflowActionsImpl<D>
where
    D: Projects + TransportService + Markers + Regions + TempoMap,
{
    fn insert_intro_region(&self) {
        dispatch(&self.daw, KeyflowAction::InsertSection(SectionKind::Intro));
    }
    fn insert_verse_region(&self) {
        dispatch(&self.daw, KeyflowAction::InsertSection(SectionKind::Verse));
    }
    fn insert_pre_chorus_region(&self) {
        dispatch(
            &self.daw,
            KeyflowAction::InsertSection(SectionKind::PreChorus),
        );
    }
    fn insert_chorus_region(&self) {
        dispatch(&self.daw, KeyflowAction::InsertSection(SectionKind::Chorus));
    }
    fn insert_bridge_region(&self) {
        dispatch(&self.daw, KeyflowAction::InsertSection(SectionKind::Bridge));
    }
    fn insert_outro_region(&self) {
        dispatch(&self.daw, KeyflowAction::InsertSection(SectionKind::Outro));
    }
    fn insert_instrumental_region(&self) {
        dispatch(
            &self.daw,
            KeyflowAction::InsertSection(SectionKind::Instrumental),
        );
    }
    fn insert_solo_region(&self) {
        dispatch(&self.daw, KeyflowAction::InsertSection(SectionKind::Solo));
    }
    fn insert_hits_region(&self) {
        dispatch(&self.daw, KeyflowAction::InsertSection(SectionKind::Hits));
    }
    fn insert_interlude_region(&self) {
        dispatch(
            &self.daw,
            KeyflowAction::InsertSection(SectionKind::Interlude),
        );
    }
    fn insert_breakdown_region(&self) {
        dispatch(
            &self.daw,
            KeyflowAction::InsertSection(SectionKind::Breakdown),
        );
    }
    fn insert_vamp_region(&self) {
        dispatch(&self.daw, KeyflowAction::InsertSection(SectionKind::Vamp));
    }
    fn insert_count_in_region(&self) {
        dispatch(
            &self.daw,
            KeyflowAction::InsertSection(SectionKind::CountIn),
        );
    }
    fn insert_end_region(&self) {
        dispatch(&self.daw, KeyflowAction::InsertSection(SectionKind::End));
    }
    fn insert_count_in_marker(&self) {
        dispatch(&self.daw, KeyflowAction::InsertMarker(MarkerKind::CountIn));
    }
    fn insert_start_marker(&self) {
        dispatch(&self.daw, KeyflowAction::InsertMarker(MarkerKind::Start));
    }
    fn insert_end_marker(&self) {
        dispatch(&self.daw, KeyflowAction::InsertMarker(MarkerKind::End));
    }
    fn insert_songstart_marker(&self) {
        dispatch(
            &self.daw,
            KeyflowAction::InsertMarker(MarkerKind::SongStart),
        );
    }
    fn insert_songend_marker(&self) {
        dispatch(&self.daw, KeyflowAction::InsertMarker(MarkerKind::SongEnd));
    }
    fn convert_markers_to_session_format(&self) {
        dispatch(&self.daw, KeyflowAction::ConvertMarkersToSessionFormat);
    }
}

/// Registers all twenty keyflow actions with `backend`, dispatching each
/// through a fresh `KeyflowActionsImpl` bound to `daw`.
pub fn register_actions<D, B>(backend: &B, daw: D)
where
    D: Projects + TransportService + Markers + Regions + TempoMap + Send + Sync + 'static,
    B: ::architect::action::ActionBackend + ?Sized,
{
    register_keyflow_actions_actions(backend, std::sync::Arc::new(KeyflowActionsImpl { daw }));
}
