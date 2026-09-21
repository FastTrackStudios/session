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

use crate::open::Opened;

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
    pub fn run(&self, opened: &Opened) -> eyre::Result<()> {
        let runtime = crate::open::runtime().ok_or_else(|| eyre::eyre!("no engine runtime"))?;
        let _entered = runtime.enter();
        let project = ProjectContext::Project(opened.project_guid.clone());

        if self.organize {
            let mut target =
                dynamic_template::apply::DawTarget::on(opened.daw.clone(), project.clone());
            let organized = dynamic_template::apply::organize(&mut target)
                .map_err(|e| eyre::eyre!("organize: {e}"))?;
            tracing::info!(
                tracks = organized.existing,
                buses = organized.buses.len(),
                unsorted = organized.unsorted.len(),
                "prepare: organized"
            );
        }
        if let Some(chart) = &self.chart {
            let text = std::fs::read_to_string(chart)
                .map_err(|e| eyre::eyre!("chart {}: {e}", chart.display()))?;
            let built = session::keyflow::from_chart::build_from_chart(&opened.daw, &project, &text)
                .map_err(|e| eyre::eyre!("chart: {e}"))?;
            tracing::info!(
                title = %built.title,
                bpm = built.tempo_bpm,
                sections = built.sections,
                "prepare: song built from chart"
            );
        }
        if self.guide {
            session::guide::Guide::new(opened.daw.clone())
                .generate(session::guide::GuideScope::All)
                .map_err(|e| eyre::eyre!("guide: {e}"))?;
            tracing::info!("prepare: click and guide generated");
        }
        Ok(())
    }
}
