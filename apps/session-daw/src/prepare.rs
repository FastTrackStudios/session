//! Prepare a multitrack session for playing: organize it, build the song
//! from its chart, and generate the click and guide — on the engine this
//! window opened, before the view reads it.
//!
//! Each step is the shared one, not a window-only copy:
//! `dynamic_template::apply::organize` (the `--apply-buses` pass, run live),
//! `session::keyflow::from_chart::build_from_chart`, and
//! `session::guide::Guide::generate`. Each is optional; they run in that
//! order because the guide reads the song the chart stamps, and files its
//! tracks into the folder the organizer builds.

use std::path::PathBuf;

use daw::service::ProjectContext;

/// What to do to a freshly opened session.
#[derive(Debug, Default, Clone)]
pub struct Prepare {
    pub organize: bool,
    pub chart: Option<PathBuf>,
    pub guide: bool,
}

impl Prepare {
    /// From `FTS_BLITZ_ORGANIZE=1`, `FTS_BLITZ_CHART=<file.kf>` and
    /// `FTS_BLITZ_GUIDE=1`.
    #[must_use]
    pub fn from_env() -> Self {
        let on = |key: &str| std::env::var(key).is_ok_and(|v| !v.is_empty() && v != "0");
        Self {
            organize: on("FTS_BLITZ_ORGANIZE"),
            chart: std::env::var_os("FTS_BLITZ_CHART").map(PathBuf::from),
            guide: on("FTS_BLITZ_GUIDE"),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.organize && self.chart.is_none() && !self.guide
    }

    /// Run the requested steps against `opened`. Stops at the first step
    /// that fails — a guide built for a song the chart never stamped is
    /// worse than no guide.
    ///
    /// # Errors
    ///
    /// The step that failed, and why.
    #[cfg(feature = "native")]
    pub fn run(&self, opened: &crate::open::Opened) -> eyre::Result<()> {
        let runtime = crate::open::runtime().ok_or_else(|| eyre::eyre!("no engine runtime"))?;
        let _entered = runtime.enter();
        let chart = match &self.chart {
            Some(path) => Some(
                std::fs::read_to_string(path)
                    .map_err(|e| eyre::eyre!("chart {}: {e}", path.display()))?,
            ),
            None => None,
        };
        steps(
            &opened.daw,
            &opened.project_guid,
            self.organize,
            chart.as_deref(),
            self.guide,
        )
    }
}

/// The one `.kf` chart in the project's folder, if there is exactly one.
/// Two would be a guess, and a wrong chart is worse than none.
#[must_use]
pub fn chart_beside(project: &std::path::Path) -> Option<PathBuf> {
    let mut charts = std::fs::read_dir(project.parent()?)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("kf")));
    let chart = charts.next()?;
    charts.next().is_none().then_some(chart)
}

/// Prepare the song at `rpp` and save it as `Song.session` beside it —
/// what the app does the first time it opens a multitrack, done ahead of
/// time (a whole setlist before a service, say). Returns where the session
/// was written. Any session already there is replaced: this is the
/// explicit way to prepare again.
///
/// One song per process — the daw facade the steps go through is
/// process-wide, and it is bound to the first engine opened.
///
/// # Errors
///
/// The song did not open, a step failed (nothing is saved then), or the
/// session could not be written.
#[cfg(feature = "native")]
pub fn prepare_and_save(rpp: &std::path::Path, chart: Option<PathBuf>) -> eyre::Result<PathBuf> {
    let prepare = Prepare {
        organize: true,
        chart,
        guide: true,
    };
    let opened = crate::open::open_silent(rpp)?;
    prepare.run(&opened)?;
    let dir = rpp.with_extension("session");
    crate::session_file::save_session(&opened.daw, &opened.project_guid, &dir)
        .map_err(|e| eyre::eyre!("saving {}: {e}", dir.display()))
}

/// Lay an edited chart over the open session: rebuild the song's structure
/// from it (tempo, markers, section regions, key and chords — see
/// [`session::keyflow::from_chart::rebuild_from_chart`]) and, when `guide`,
/// regenerate the click, count and cues to match.
///
/// The guide needs nothing cleared first: it writes one item per role over
/// the whole song, from the count-in, and every song starts there — so the
/// old song's item always overlaps the new one's span, and generating
/// replaces it.
///
/// The live half of editing a chart: Organize mode calls this after each
/// pause in typing. Text that does not parse changes nothing.
///
/// # Errors
///
/// The chart does not parse, or a step failed.
pub fn apply_chart(
    daw: &daw_standalone::sync::Standalone,
    project_guid: &str,
    chart: &str,
    guide: bool,
) -> eyre::Result<session::keyflow::from_chart::ChartRebuilt> {
    let project = ProjectContext::Project(project_guid.to_owned());
    let rebuilt = session::keyflow::from_chart::rebuild_from_chart(daw, &project, chart)?;
    if guide {
        session::guide::Guide::new(daw.clone())
            .with_instrument(GUIDE_INSTRUMENT)
            .generate(session::guide::GuideScope::All)
            .map_err(|e| eyre::eyre!("guide: {e}"))?;
    }
    Ok(rebuilt)
}

/// The FX the Click / Count / Guide tracks play through
/// (`guide_instrument::IDENT`).
pub const GUIDE_INSTRUMENT: &str = "fts.guide";

/// The steps themselves, on any host: the native window's opened engine,
/// or the web demo's (which has the chart's text rather than a file).
///
/// # Errors
///
/// The step that failed, and why.
pub fn steps(
    daw: &daw_standalone::sync::Standalone,
    project_guid: &str,
    organize: bool,
    chart: Option<&str>,
    guide: bool,
) -> eyre::Result<()> {
    let project = ProjectContext::Project(project_guid.to_owned());

    let mut target = dynamic_template::apply::DawTarget::on(daw.clone(), project.clone());
    if organize {
        // Folders first — Drums, Bass, Guitars … with any wrapper the
        // multitrack came in taken apart — so the bus pass routes
        // groups rather than loose tracks.
        let arranged = target.arrange_into_groups()?;
        tracing::info!(
            unwrapped = ?arranged.unwrapped,
            folders = ?arranged.created,
            placed = arranged.placed,
            "prepare: arranged into groups"
        );
        let organized = dynamic_template::apply::organize(&mut target)
            .map_err(|e| eyre::eyre!("organize: {e}"))?;
        tracing::info!(
            tracks = organized.existing,
            buses = organized.buses.len(),
            unsorted = organized.unsorted.len(),
            "prepare: organized"
        );
    }
    if let Some(text) = chart {
        let built = session::keyflow::from_chart::build_from_chart(daw, &project, text)
            .map_err(|e| eyre::eyre!("chart: {e}"))?;
        tracing::info!(
            title = %built.title,
            bpm = built.tempo_bpm,
            sections = built.sections,
            "prepare: song built from chart"
        );
        // The key and the chords read from the ruler's CHORDS lane,
        // so the folder that holds them is out of the track panel
        // until someone opens it to edit — which the lane follows.
        target.hide_in_tcp("Keyflow")?;
    }
    if guide {
        session::guide::Guide::new(daw.clone())
            .with_instrument(GUIDE_INSTRUMENT)
            .generate(session::guide::GuideScope::All)
            .map_err(|e| eyre::eyre!("guide: {e}"))?;
        tracing::info!("prepare: click and guide generated");
        // The multitrack's own click and guide stems, renamed and
        // muted by the guide: kept for reference, out of the panel.
        target.hide_in_tcp("Click Audio")?;
        target.hide_in_tcp("Guide Audio")?;
    }
    if organize {
        // The template's top-level order — what is played to first,
        // then the song, then the instruments, routing last — and the
        // routing out of the track panel.
        target.order_top_level(
            &["Guide", "Keyflow"],
            &["CLICK + GUIDE BUS", "MIX BUS", "UNSORTED"],
        )?;
        target.hide_in_tcp("CLICK + GUIDE BUS")?;
        target.hide_in_tcp("MIX BUS")?;
    }
    Ok(())
}

#[cfg(test)]
mod chart_tests {
    use super::chart_beside;

    /// The chart beside a project is used only when it is the only one.
    #[test]
    fn a_session_folder_s_one_chart_is_its_chart() {
        let dir = std::env::temp_dir().join(format!("session-chart-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let project = dir.join("Song.RPP");
        std::fs::write(&project, "").unwrap();
        assert_eq!(chart_beside(&project), None, "no chart at all");

        std::fs::write(dir.join("Song.kf"), "").unwrap();
        assert_eq!(chart_beside(&project), Some(dir.join("Song.kf")));

        std::fs::write(dir.join("Other.KF"), "").unwrap();
        assert_eq!(chart_beside(&project), None, "two charts is a guess");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
