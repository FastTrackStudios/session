//! The engine in a browser: daw-standalone in-process, the session opened
//! from text.
//!
//! The native app opens a project from disk, stands up the `daw` facade on
//! a tokio runtime, and reads the session back ([`crate::open`],
//! `StudioSession::open`). A page has no disk and no threads, so this does
//! the same from the project's text (fetched by the page) and awaits
//! everything instead of blocking: `load_rpp_text` into a `Standalone`,
//! `build_in_process_daw`, `daw::init_from_parts`, then the same read-back
//! and row plan ([`crate::studio::Planner`]). The engine clients
//! ([`crate::engine::Applier`], [`crate::engine::Transport`]) have web
//! implementations that find the facade this installs.

use std::rc::Rc;
use std::sync::Arc;

use crate::studio::{Planner, StudioSession};

/// The scene the web demo lays the arrangement out by — the desktop app's.
const SCENE: &str = "drum-mixing";

/// The engine, as the page's panels reach it.
#[derive(Clone)]
pub struct EngineRef {
    applier: Rc<Option<crate::engine::Applier>>,
}

impl PartialEq for EngineRef {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.applier, &other.applier)
    }
}

impl EngineRef {
    /// Carry out an edit. Dropped (logged) with no engine.
    pub fn send(&self, edit: crate::engine::Edit) {
        match self.applier.as_ref() {
            Some(applier) => applier.send(edit),
            None => tracing::debug!(?edit, "no engine to carry out the edit"),
        }
    }

    /// Where the play cursor is, in seconds.
    #[must_use]
    pub fn position(&self) -> f64 {
        crate::engine::Transport::shared().map_or(0.0, |t| t.read().0)
    }
}

/// Open a session from its project text (and its chart's, if it has one):
/// the engine stood up in-process, and the session read back as the
/// studio lays it out.
///
/// # Errors
///
/// The project did not parse, the facade did not come up, or the project
/// could not be read back.
pub async fn open(
    name: &str,
    rpp_text: &str,
    chart_text: Option<&str>,
) -> eyre::Result<(EngineRef, StudioSession)> {
    let standalone = daw_standalone::sync::Standalone::new();
    let summary = daw_standalone::project_loader::load_rpp_text(
        &standalone,
        name,
        &format!("{name}.RPP"),
        rpp_text,
    )
    .map_err(|e| eyre::eyre!("{name} did not parse: {e}"))?;
    // Prepared the way the desktop app prepares it: organized into
    // folders, the song built from its chart, the click and guide made.
    crate::prepare::steps(&standalone, &summary.project_guid, true, chart_text, true)
        .map_err(|e| eyre::eyre!("preparing {name}: {e}"))?;
    let bundle = daw_standalone::bootstrap::build_in_process_daw(standalone.clone())
        .await
        .map_err(|e| eyre::eyre!("build_in_process_daw: {e:?}"))?;
    daw::init_from_parts(bundle.daw.clone());
    // The facade and the engine live as long as the page.
    std::mem::forget(bundle);

    // Say which step fails: `fetch` answers only yes or no.
    let facade = daw_control::Daw::try_get().ok_or_else(|| eyre::eyre!("the daw facade is not up"))?;
    let current = facade
        .current_project()
        .await
        .map_err(|e| eyre::eyre!("no current project: {e}"))?;
    current
        .tracks()
        .all()
        .await
        .map_err(|e| eyre::eyre!("the tracks could not be read: {e}"))?;
    let raw = daw_ui::studio::project::fetch()
        .await
        .ok_or_else(|| eyre::eyre!("could not read {name} back"))?;
    let planner = Planner {
        raw: Arc::new(raw),
        scene: Some(SCENE),
        kinds: Arc::new(crate::plan::Kinds::from_text(rpp_text)),
    };
    let (project, rows) = planner.plan(&planner.raw);
    let previews = crate::midi::Previews::default();
    previews
        .fill(
            project
                .0
                .items
                .values()
                .flatten()
                .filter(|item| project.0.is_midi(&item.guid))
                .map(|item| (item.guid.clone(), item.length.as_seconds()))
                .collect(),
        )
        .await;
    let chart = chart_text.and_then(|text| {
        keyflow::parse(text)
            .inspect_err(|e| tracing::error!(error = %e, "chart: could not parse"))
            .ok()
            .map(Arc::new)
    });
    let engine = EngineRef {
        applier: Rc::new(crate::engine::Applier::start()),
    };
    Ok((
        engine,
        StudioSession {
            project,
            rows,
            previews,
            chart,
            planner,
        },
    ))
}
