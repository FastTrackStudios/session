use daw::module::ModuleContext;
use daw::rpc::Project;
use daw::service::{MusicalPosition, Region};
use keyflow::sections::colors_for_section_type;
use session_proto::SectionType;
use session_proto::ruler_lanes::{CoreLane, FtsLane, classify_marker_lane};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use tokio::runtime::Runtime;
use tracing::{info, warn};

static RUNTIME: OnceLock<Arc<Runtime>> = OnceLock::new();

const TOUCH_EPSILON_SECONDS: f64 = 0.001;
const MAX_SECTION_TIME_SELECTION_SECONDS: f64 = 60.0 * 60.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyflowAction {
    InsertSection(SectionKind),
    InsertMarker(MarkerKind),
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

pub fn init(ctx: &ModuleContext) {
    let _ = RUNTIME.set(ctx.runtime.clone());
}

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

pub fn dispatch(action: KeyflowAction) {
    let Some(runtime) = RUNTIME.get() else {
        tracing::warn!("[session] Keyflow action fired before module runtime was initialized");
        return;
    };

    tracing::info!(?action, "[session] Dispatching Keyflow action");
    runtime.spawn(async move {
        tracing::info!(?action, "[session] Running Keyflow action");
        if let Err(err) = run_action(action).await {
            tracing::error!(?err, "[session] Keyflow action failed");
        } else {
            tracing::info!(?action, "[session] Keyflow action completed");
        }
    });
}

async fn run_action(action: KeyflowAction) -> eyre::Result<()> {
    let started = Instant::now();
    let daw = daw::get().ok_or_else(|| eyre::eyre!("DAW facade not initialized"))?;
    let project = daw.current_project().await?;
    ensure_action_lane(&project, action).await?;

    match action {
        KeyflowAction::InsertSection(kind) => insert_section_region(&project, kind).await?,
        KeyflowAction::InsertMarker(kind) => {
            insert_marker(&project, kind).await?;
            normalize_marker_lanes(&project).await?;
        }
    }

    info!(
        ?action,
        elapsed_ms = started.elapsed().as_millis(),
        "[session] Keyflow action timing",
    );
    Ok(())
}

async fn ensure_action_lane(project: &Project, action: KeyflowAction) -> eyre::Result<()> {
    let lane = match action {
        KeyflowAction::InsertSection(_) => FtsLane::Core(CoreLane::Sections),
        KeyflowAction::InsertMarker(kind) => classify_marker_lane(kind.name()),
    };
    project
        .set_ruler_lane_name(lane.lane_index(), lane.display_name())
        .await?;
    Ok(())
}

async fn insert_marker(project: &Project, kind: MarkerKind) -> eyre::Result<()> {
    let started = Instant::now();
    let position = project.transport().get_position().await?;
    let name = kind.name();
    let lane = classify_marker_lane(name).lane_index();
    let id = project.markers().add_in_lane(position, name, lane).await?;
    project
        .markers()
        .set_color(id, kind.default_color())
        .await?;
    info!(
        ?kind,
        elapsed_ms = started.elapsed().as_millis(),
        "[session] Keyflow marker insert timing",
    );
    Ok(())
}

async fn insert_section_region(project: &Project, kind: SectionKind) -> eyre::Result<()> {
    let started = Instant::now();
    let position = project.transport().get_position().await?;
    let time_selection = project.transport().get_time_selection().await?;
    let regions = project.regions().all().await?;
    let (start, end) =
        infer_insert_bounds(project, kind, position, time_selection.as_ref(), &regions).await?;
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
    let lane = CoreLane::Sections.lane_index();
    info!("[session] Keyflow section insert: carving overlapping regions");
    carve_overlapping_section_regions(project, &regions, start, end).await?;
    info!("[session] Keyflow section insert: carve complete");
    info!("[session] Keyflow section insert: adding region");
    let id = project
        .regions()
        .add_in_lane(start, end, &kind.section_type().abbreviation(), lane)
        .await?;
    info!(id, "[session] Keyflow section insert: add complete");
    info!(id, "[session] Keyflow section insert: setting color");
    project
        .regions()
        .set_color(id, section_type_color(kind.section_type()))
        .await?;
    info!(id, "[session] Keyflow section insert: color complete");
    info!(
        end,
        "[session] Keyflow section insert: moving cursor to end"
    );
    project.transport().set_position(end).await?;
    info!("[session] Keyflow section insert: cursor move complete");

    info!("[session] Keyflow section insert: reloading regions");
    let regions = project.regions().all().await?;
    info!(
        region_count = regions.len(),
        "[session] Keyflow section insert: reloaded regions",
    );
    info!("[session] Keyflow section insert: normalizing section regions");
    normalize_section_regions(project, regions).await?;
    info!("[session] Keyflow section insert: normalize complete");
    info!(
        ?kind,
        elapsed_ms = started.elapsed().as_millis(),
        "[session] Keyflow section insert timing",
    );
    Ok(())
}

async fn infer_insert_bounds(
    project: &Project,
    kind: SectionKind,
    position: f64,
    time_selection: Option<&daw::service::LoopRegion>,
    regions: &[Region],
) -> eyre::Result<(f64, f64)> {
    if let Some(selection) = time_selection
        .filter(|selection| is_valid_time_selection(selection.start_seconds, selection.end_seconds))
    {
        return Ok((selection.start_seconds, selection.end_seconds));
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

    let musical_end = default_section_end(project, kind, position).await?;
    let next_section_start = regions
        .iter()
        .filter(|region| region.start_seconds() > position + TOUCH_EPSILON_SECONDS)
        .filter(|region| region.lane == Some(CoreLane::Sections.lane_index()))
        .map(Region::start_seconds)
        .min_by(f64::total_cmp);
    let end = choose_default_end(position, musical_end, next_section_start);

    Ok((position, end))
}

async fn carve_overlapping_section_regions(
    project: &Project,
    regions: &[Region],
    insert_start: f64,
    insert_end: f64,
) -> eyre::Result<()> {
    let section_lane = CoreLane::Sections.lane_index();
    let regions_client = project.regions();

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
            regions_client.set_bounds(id, start, insert_start).await?;
        } else if end > insert_end + TOUCH_EPSILON_SECONDS {
            info!(
                id,
                region_start = start,
                region_end = end,
                new_start = insert_end,
                "[session] Carving section region: trimming left edge",
            );
            regions_client.set_bounds(id, insert_end, end).await?;
        } else {
            info!(
                id,
                region_start = start,
                region_end = end,
                "[session] Carving section region: removing covered region",
            );
            regions_client.remove(id).await?;
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

async fn default_section_end(
    project: &Project,
    kind: SectionKind,
    position: f64,
) -> eyre::Result<f64> {
    let tempo_map = project.tempo_map();
    let (measure, beat, fraction) = tempo_map.time_to_musical(position).await?;
    let start = musical_position_from_fraction(measure, beat, fraction);
    let end = MusicalPosition::new(
        start.measure + kind.default_measure_count() as i32,
        start.beat,
        start.subdivision,
    );
    let end_fraction = end.subdivision as f64 / 1000.0;
    Ok(tempo_map
        .musical_to_time(end.measure, end.beat, end_fraction)
        .await?)
}

fn musical_position_from_fraction(measure: i32, beat: i32, fraction: f64) -> MusicalPosition {
    let subdivision = if fraction.is_finite() {
        (fraction * 1000.0).round() as i32
    } else {
        0
    };
    MusicalPosition::new(measure, beat, subdivision.clamp(0, 999))
}

async fn normalize_section_regions(project: &Project, regions: Vec<Region>) -> eyre::Result<()> {
    let existing: HashMap<u32, (String, Option<u32>, Option<u32>)> = regions
        .iter()
        .filter_map(|region| Some((region.id?, (region.name.clone(), region.lane, region.color))))
        .collect();
    let normalized = normalized_section_updates(regions);
    let regions = project.regions();
    for update in normalized {
        let SectionUpdate {
            id,
            name,
            section_type,
        } = update;
        let desired_lane = Some(CoreLane::Sections.lane_index());
        let desired_color = Some(section_type_color(section_type));
        let current = existing.get(&id);
        if current
            .map(|(current_name, _, _)| current_name != &name)
            .unwrap_or(true)
        {
            info!(id, name, "[session] Normalizing section region: rename");
            regions.rename(id, &name).await?;
        }
        if current
            .map(|(_, current_lane, _)| *current_lane != desired_lane)
            .unwrap_or(true)
        {
            info!(
                id,
                lane = ?desired_lane,
                "[session] Normalizing section region: set lane",
            );
            regions.set_lane(id, desired_lane).await?;
        }
        if current
            .map(|(_, _, current_color)| *current_color != desired_color)
            .unwrap_or(true)
        {
            info!(
                id,
                color = desired_color.unwrap(),
                "[session] Normalizing section region: set color",
            );
            regions.set_color(id, desired_color.unwrap()).await?;
        }
    }
    Ok(())
}

async fn normalize_marker_lanes(project: &Project) -> eyre::Result<()> {
    let markers = project.markers();
    for marker in markers.all().await? {
        let Some(id) = marker.id else {
            continue;
        };
        let lane = classify_marker_lane(&marker.name).lane_index();
        if marker.lane != Some(lane) {
            markers.set_lane(id, Some(lane)).await?;
        }
        if let Some(color) = marker_color_for_name(&marker.name)
            && marker.color != Some(color)
        {
            markers.set_color(id, color).await?;
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
        n = n / 26;
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
