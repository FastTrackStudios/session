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

use std::collections::HashMap;
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
/// `media` is where the takes' audio streams from: a Task share link to the
/// session's folder, whose `rendition/audio/<path>` answers each take's
/// source with its proxy ([`crate::web_audio`]). `None` opens it silent.
/// `guide` is a share link to the guide sample library (as Ogg) the click,
/// count and cue tracks play from; without it they play synthesized ticks
/// and beeps.
pub async fn open(
    name: &str,
    rpp_text: &str,
    chart_text: Option<&str>,
    media: Option<&str>,
    guide: Option<&str>,
) -> eyre::Result<(EngineRef, StudioSession)> {
    let standalone = daw_standalone::sync::Standalone::new();
    let summary = daw_standalone::project_loader::load_rpp_text(
        &standalone,
        name,
        &format!("{name}.RPP"),
        rpp_text,
    )
    .map_err(|e| eyre::eyre!("{name} did not parse: {e}"))?;
    // The guide instrument, before the guide tracks it plays on are made.
    let library = match guide {
        Some(base) => guide_library(base).await,
        None => HashMap::new(),
    };
    crate::guide_instrument::install(
        &standalone,
        crate::guide_instrument::Library::Files {
            files: Arc::new(library),
            ext: "ogg",
        },
    );
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
    crate::web_audio::install(standalone.clone(), &summary.project_guid);
    if let Some(base) = media {
        stream_stems(&standalone, &summary.project_guid, base);
    }

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
            chart_file: None,
            planner,
        },
    ))
}

/// Fetch every take's proxy from `base` and attach it as it arrives — all
/// at once, so the stems fill in together rather than one after another.
fn stream_stems(standalone: &daw_standalone::sync::Standalone, project: &str, base: &str) {
    let mut sources: Vec<(String, String)> = Vec::new();
    let _ = standalone.read_project(project, |p| {
        for list in p.takes.values() {
            for take in &list.takes {
                if let Some(path) = take.source_file_path.as_deref()
                    && !path.is_empty()
                    && !take.is_midi
                {
                    sources.push((take.guid.clone(), path.to_owned()));
                }
            }
        }
    });
    tracing::info!(stems = sources.len(), "audio: streaming the stems");
    for (take, path) in sources {
        let url = format!("{}/rendition/audio/{}", base.trim_end_matches('/'), url_path(&path));
        let project = project.to_owned();
        wasm_bindgen_futures::spawn_local(async move {
            match fetch_bytes(&url).await {
                Ok(bytes) => {
                    if let Err(e) = crate::web_audio::add_stem(&project, &take, bytes) {
                        tracing::warn!(path, error = %e, "audio: a stem did not open");
                    }
                }
                Err(e) => tracing::warn!(path, error = %e, "audio: a stem did not arrive"),
            }
        });
    }
}

/// A root-relative path as a URL path: each segment encoded.
fn url_path(path: &str) -> String {
    path.split('/')
        .map(|segment| String::from(js_sys::encode_uri_component(segment)))
        .collect::<Vec<_>>()
        .join("/")
}

/// A URL's whole body.
async fn fetch_bytes(url: &str) -> Result<Arc<[u8]>, String> {
    use wasm_bindgen::JsCast as _;
    let window = web_sys::window().ok_or("no window")?;
    let response = wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|e| format!("{e:?}"))?;
    let response: web_sys::Response = response.dyn_into().map_err(|_| "not a response")?;
    if !response.ok() {
        return Err(format!("HTTP {}", response.status()));
    }
    let body = response.array_buffer().map_err(|e| format!("{e:?}"))?;
    let body = wasm_bindgen_futures::JsFuture::from(body)
        .await
        .map_err(|e| format!("{e:?}"))?;
    Ok(js_sys::Uint8Array::new(&body).to_vec().into())
}

/// The guide library's files MIDI-mode playback needs, fetched together
/// from the library's share link. A file that does not arrive is left out
/// (the instrument synthesizes its slot).
async fn guide_library(base: &str) -> HashMap<String, Vec<u8>> {
    let wanted = session_guide::samples::library::files(
        crate::guide_instrument::CLICK,
        crate::guide_instrument::VOICE,
        "ogg",
    );
    let base = base.trim_end_matches('/');
    let fetches = wanted.into_iter().map(|path| async move {
        let url = format!("{base}/download/{}", url_path(&path));
        match fetch_bytes(&url).await {
            Ok(bytes) => Some((path, bytes.to_vec())),
            Err(e) => {
                tracing::warn!(path, error = %e, "guide: a sample did not arrive");
                None
            }
        }
    });
    let files: HashMap<String, Vec<u8>> = futures_util::future::join_all(fetches)
        .await
        .into_iter()
        .flatten()
        .collect();
    tracing::info!(samples = files.len(), "guide: the sample library arrived");
    files
}
