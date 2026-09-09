//! The workstation window: the whole project, not just the editor.
//!
//! ```text
//! +--------------------+-------+
//! | TCP | Arrangement  | Mixer |
//! +--------------------+ (MCP) |
//! | Expression editor  |       |
//! +--------------------+-------+
//! ```
//!
//! The panels are daw-ui's **native components family** — the traced
//! vector TCP, arrangement and mixer that the REAPER theme's art is
//! exported from (`daw_ui::components`, PR #279's main window). Not the
//! WALTER `panels` family: that one executes theme images, and a window
//! is not a theme editor. Composition follows
//! `daw_ui::components::main_window` — the same row pitch
//! (`geometry::tcp::ROW_H` + 1px divider), the same ruler spacer in the
//! TCP column — with the mixer moved to a full-height right column and
//! the expression editor docked under the arrangement.
//!
//! Data comes through the in-process daw facade — the same `Standalone`
//! the drum host edits, served over a vox memory link by
//! [`daw::standalone::bootstrap::build_in_process_daw`]. One backend,
//! three faces: the arrangement shows the items the quantizer cuts, the
//! mixer moves the faders the renderer reads, and the editor writes the
//! edits — all visibly the same project.

mod mixer;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use dioxus::prelude::*;

use daw::service::{Fx, Item, Track};
use daw::standalone::Standalone;
use daw_theme_art::geometry::mcp::STRIP_W;
use daw_theme_art::geometry::tcp::{ROW_H, ROW_W};
use daw_ui::components::arrangement_view::{
    ArrangePreview, ItemPreview, plan_rows, waveform_from_peaks,
};
use daw_ui::components::tcp::TrackRow;
use daw_ui::controls::{ControlSync, MeterFeed, use_daw_tracks, use_track_store};
use daw_ui::panels::native::NativeTransportBar;

use expression_editor_core::Editor;
use expression_editor_ui::ExpressionEditor;

use crate::app::{HostCallbacks, host_callbacks};
use crate::drum_host::SharedDrumHost;

/// The mixer column's width — four native strips before scrolling.
pub const MIXER_W: f64 = (STRIP_W as f64 + 1.0) * 4.0 + 2.0;
/// The arrange pane's share of the left column.
pub const ARRANGE_FRACTION: f64 = 0.45;
/// The transport band, per the main window.
const TRANSPORT_H: f64 = 40.0;
/// The ruler's region lane — REAPER's colored section bands.
const REGION_H: f64 = 16.0;
/// The ruler's marker lane — numbered flags under the regions.
const MARKER_H: f64 = 14.0;
/// The arrange view's own ruler — the TCP column's spacer must match it
/// exactly, per the main window's alignment contract.
const ARR_RULER_H: f32 = 26.0;
/// The FX insert band over the mixer strips, when any track has a chain.
const FX_BAND_H: f64 = 144.0;

/// How far the shared viewport must travel before the panes re-cut what
/// they mount. Each must stay comfortably under the overscan the panes
/// keep mounted past their edges, or a scroll would reach unmounted
/// ground before anyone had noticed it moved.
const COMMIT_X: f32 = 160.0;
/// Vertically, in the same units the pane scrolls: three track rows.
const COMMIT_Y: f32 = 3.0 * (ROW_H + 1.0);
/// The timeline's vertical step — a screenful, not a few rows. It
/// re-renders far more per step than the track panel does, so it is told
/// far less often; `ArrangeCanvas`'s vertical overscan covers the drift.
const COMMIT_TIMELINE_Y: f32 = 360.0;

/// The Blitz cursor/scheme fixes every native window embeds (the same
/// three lines the REAPER test panels carry).
const BLITZ_FIXES: &str = r#"
input, textarea, select, button { cursor: auto !important; }
input:disabled, textarea:disabled, button:disabled { cursor: not-allowed !important; }
:root { color-scheme: dark; }
"#;

/// What the window mounts, staged like [`crate::app::stage`] and for
/// the same reason: `dioxus_native::launch_cfg` takes a bare
/// `fn() -> Element`.
struct StagedWorkstation {
    editor: Editor,
    host: Option<SharedDrumHost>,
    /// Window size, so the editor's cell can be reported before the
    /// first layout (dioxus-native fires no resize on mount).
    size: (f64, f64),
}

static STAGED: Mutex<Option<StagedWorkstation>> = Mutex::new(None);

/// Keeps the in-process daw link's acceptor alive for the window's
/// lifetime. Dropping it would silently disconnect every panel.
static DAW_BUNDLE: OnceLock<daw::standalone::bootstrap::InProcessDaw> = OnceLock::new();

/// The engine runtime, for callers that must construct something which
/// spawns tasks (the audio engine) from a plain `main` — enter it
/// first, or `architect::platform::spawn` panics with "no reactor".
static DAW_RT: OnceLock<&'static tokio::runtime::Runtime> = OnceLock::new();

/// Run `f` inside the bootstrap's tokio runtime context. Panics if
/// [`bootstrap_daw_blocking`] has not run — that ordering bug should
/// fail loudly, not silently skip audio.
pub fn in_daw_runtime<R>(f: impl FnOnce() -> R) -> R {
    let rt = DAW_RT
        .get()
        .expect("bootstrap_daw_blocking before in_daw_runtime");
    let _guard = rt.enter();
    f()
}

/// Hand the workstation its document, host and window size.
pub fn stage_workstation(editor: Editor, host: Option<SharedDrumHost>, size: (f64, f64)) {
    *STAGED.lock().unwrap() = Some(StagedWorkstation { editor, host, size });
}

/// Serve `standalone` over an in-process memory link and install the
/// global daw facade the panels read. Call once, before the window.
///
/// The runtime is leaked multi-thread with 16 MiB worker stacks —
/// vox 0.10's debug-build channel encode recurses deeply and overflows
/// tokio's default 2 MiB (the same story as the app's session engine).
pub fn bootstrap_daw_blocking(standalone: &Standalone) -> eyre::Result<()> {
    if DAW_BUNDLE.get().is_some() {
        return Ok(());
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("ee-workstation-daw")
        .thread_stack_size(16 * 1024 * 1024)
        .enable_all()
        .build()?;
    let rt: &'static tokio::runtime::Runtime = Box::leak(Box::new(rt));
    let _ = DAW_RT.set(rt);
    let bundle = rt.block_on(daw::standalone::bootstrap::build_in_process_daw(
        standalone.clone(),
    ))?;
    // A separate current-thread runtime for `daw::block_on` in sync
    // contexts, so it can never be entered from inside the engine
    // runtime's own workers.
    let block_on_rt = Arc::new(
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?,
    );
    daw::init_from_parts(bundle.daw.clone(), block_on_rt);
    DAW_BUNDLE
        .set(bundle)
        .map_err(|_| eyre::eyre!("workstation daw bootstrapped twice"))?;
    Ok(())
}

/// One song section for the ruler's region lane.
#[derive(Clone, PartialEq)]
struct Section {
    start: f64,
    end: f64,
    name: String,
    color: Option<String>,
}

/// One project marker for the ruler's marker lane.
#[derive(Clone, PartialEq)]
struct MarkerFlag {
    at: f64,
    name: String,
    color: Option<String>,
    /// REAPER's marker number — the flag's label.
    idx: u32,
}

/// Everything the native panels need, fetched in one pass. The
/// components take the facade's own types — no view-model translation.
#[derive(Default, Clone)]
struct ProjectShape {
    tracks: Vec<Track>,
    items: Vec<Item>,
    fx: HashMap<String, Vec<Fx>>,
    sections: Vec<Section>,
    markers: Vec<MarkerFlag>,
    bpm: f64,
    /// End of the last item, so the timeline spans the material.
    length_secs: f64,
}

/// The fast pass: everything the window needs to *stand* — tracks,
/// items (as colored blocks), sections, tempo. No waveforms: those are
/// the slow 90% and they stream in afterwards, item by item, through
/// [`stream_previews`]. A window that shows the arrangement in under a
/// second and fills waveforms in behind it beats a spinner every time.
async fn fetch_project() -> Option<ProjectShape> {
    let daw = daw::get()?;
    let project = daw.current_project().await.ok()?;
    let tracks = project.tracks().all().await.ok()?;
    let items = project.items().all().await.ok()?;

    let mut length = 0.0f64;
    for item in &items {
        length = length.max(item.position.as_seconds() + item.length.as_seconds());
    }

    let mut fx = HashMap::new();
    for t in &tracks {
        if let Ok(Some(handle)) = project.tracks().by_guid(&t.guid).await
            && let Ok(chain) = handle.fx_chain().all().await
            && !chain.is_empty()
        {
            fx.insert(t.guid.clone(), chain);
        }
    }

    let sections = project
        .regions()
        .all()
        .await
        .unwrap_or_default()
        .iter()
        .map(|r| Section {
            start: r.time_range.start_seconds(),
            end: r.time_range.end_seconds(),
            name: r.name.clone(),
            color: r.color.map(|c| format!("#{c:06x}")),
        })
        .collect();
    let bpm = project.transport().get_tempo().await.unwrap_or(120.0);

    // Only real markers: REAPER stores a region as a marker pair, and
    // the standalone loader keeps them apart, but an unnamed marker at
    // a region edge would be ruler noise either way.
    let markers = project
        .markers()
        .all()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|m| !m.name.is_empty())
        .enumerate()
        .map(|(i, m)| MarkerFlag {
            at: m.position.seconds().unwrap_or(0.0),
            name: m.name.clone(),
            color: m.color.map(|c| format!("#{c:06x}")),
            idx: m.id.unwrap_or(i as u32 + 1),
        })
        .collect();

    Some(ProjectShape {
        tracks,
        items,
        fx,
        sections,
        markers,
        bpm,
        length_secs: length.max(60.0),
    })
}

/// The slow pass: waveform previews, streamed into `previews` as each
/// item's peaks arrive so the arrangement fills in live. Coarse blocks
/// (2048 ≈ 23 px/s of detail at 48 k) — the arrangement is an overview;
/// the editor below holds the close view. `pending` counts down to zero
/// for the loading readout.
async fn stream_previews(
    items: Vec<Item>,
    mut previews: Signal<HashMap<String, ItemPreview>>,
    mut pending: Signal<usize>,
) {
    let Some(daw) = daw::get() else { return };
    let Ok(project) = daw.current_project().await else {
        return;
    };
    let total = items.len();
    pending.set(total);
    // Batched: a signal write re-renders the whole timeline, and four
    // hundred single-item writes would freeze the window it exists to
    // fill. A batch per ~2 dozen items is sixteen redraws for a big
    // session — invisible.
    let mut batch: HashMap<String, ItemPreview> = HashMap::new();
    let mut done = 0usize;
    for item in items {
        if let Ok(Some(handle)) = project.items().by_guid(&item.guid).await
            && let Ok(data) = handle.active_take().peaks(2048).await
        {
            let amps = waveform_from_peaks(&data);
            if !amps.is_empty() {
                batch.insert(item.guid.clone(), ItemPreview::Waveform(amps));
            }
        }
        done += 1;
        if batch.len() >= 24 || done == total {
            previews.write().extend(batch.drain());
            pending.set(total - done);
        }
    }
}

/// The workstation. Takes no props — see [`stage_workstation`].
#[component]
pub fn WorkstationApp() -> Element {
    let staged = use_hook(|| {
        Arc::new(
            STAGED
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| StagedWorkstation {
                    editor: crate::app::fallback(),
                    host: None,
                    size: (1920.0, 1080.0),
                }),
        )
    });
    let (win_w, win_h) = staged.size;
    let arrange_h = (win_h * ARRANGE_FRACTION).round();
    let left_w = (win_w - MIXER_W).max(200.0);
    // The editor's cell is everything under the arrange pane. It
    // subtracts its own chrome from what we report here.
    expression_editor_ui::available_space(left_w, win_h - arrange_h);

    let editor = use_signal(|| {
        let mut ed = staged.editor.clone();
        ed.viewport = expression_editor_ui::viewport_in(left_w, win_h - arrange_h);
        ed
    });
    let host = use_signal(|| staged.host.clone());
    let bins = use_signal(Vec::new);
    let previews_sig = use_signal(Vec::new);
    let HostCallbacks {
        error,
        on_change,
        on_apply,
        on_save,
        on_hit,
        on_undo,
        on_redo,
    } = host_callbacks(
        editor,
        host.read().clone(),
        bins,
        previews_sig,
        use_signal(Vec::new),
    );

    // One store + meter bank for every panel in the window: the TCP
    // rows, the strips and the sync/feed components must share them, so
    // they are provided here rather than self-provided per subtree.
    let store = use_track_store();
    use_daw_tracks(store);

    // The project, in two passes: the fast shape (tracks, items as
    // blocks, sections) lands in well under a second and the window
    // stands; waveforms then stream in item by item. The drum host
    // already refreshes the editor's own lanes after every write.
    let mut shape = use_signal(ProjectShape::default);
    let item_previews_sig = use_signal(HashMap::<String, ItemPreview>::new);
    let previews_pending = use_signal(|| 0usize);
    let mut previews_complete = use_signal(|| false);
    let mut arrange_zoom = use_signal(|| 1.0_f32);
    use_effect(move || {
        spawn(async move {
            if let Some(s) = fetch_project().await {
                let items = s.items.clone();
                shape.set(s);
                stream_previews(items, item_previews_sig, previews_pending).await;
                previews_complete.set(true);
            }
        });
    });

    // Transport: the native bar's signals, wired to the facade.
    let mut playing = use_signal(|| false);
    let mut playhead = use_signal(|| 0.0f64);
    use_future(move || async move {
        loop {
            let Some(daw) = daw::get() else {
                futures_timer::Delay::new(std::time::Duration::from_millis(250)).await;
                continue;
            };
            let Ok(project) = daw.current_project().await else {
                futures_timer::Delay::new(std::time::Duration::from_millis(250)).await;
                continue;
            };
            let guid = project.guid().to_string();
            let mut stream = project.transport().events();
            while let Ok(Some(ev)) = stream.recv().await {
                match ev.get() {
                    daw::service::transport::TransportStreamEvent::Position(tick)
                        if tick.project_guid == guid =>
                    {
                        if let Some(s) = tick.playhead.seconds() {
                            playhead.set(s);
                        }
                    }
                    daw::service::transport::TransportStreamEvent::State(
                        daw::service::transport::TransportEvent::PlayStateChanged {
                            project_guid,
                            play_state,
                        },
                    ) if *project_guid == guid => {
                        playing.set(matches!(
                            play_state,
                            daw::service::transport::PlayState::Playing
                                | daw::service::transport::PlayState::Recording
                        ));
                    }
                    _ => {}
                }
            }
            futures_timer::Delay::new(std::time::Duration::from_millis(500)).await;
        }
    });
    let on_play = EventHandler::new(move |_| {
        playing.set(true);
        spawn(async move {
            if let Some(daw) = daw::get()
                && let Ok(project) = daw.current_project().await
            {
                let _ = project.transport().play().await;
            }
        });
    });
    let on_stop = EventHandler::new(move |_| {
        playing.set(false);
        spawn(async move {
            if let Some(daw) = daw::get()
                && let Ok(project) = daw.current_project().await
            {
                let _ = project.transport().stop().await;
            }
        });
    });

    let folders = use_signal(daw_ui::components::folders::FolderState::default);
    // Where the shared TCP/timeline viewport is looking, in the scroll
    // container's own pixels: `(left, top)`.
    //
    // Deliberately NOT read in this body. The panes below take it as a
    // `Signal` and read it inside their own components, so a scroll
    // re-renders the rows and the items that came into view — not the
    // transport, the editor and the mixer as well.
    //
    // It is also deliberately COARSE. Blitz scrolls natively: the pane
    // moves whether or not dioxus hears about it, and the only reason to
    // hear about it is to mount whatever just came into range. Writing
    // every pixel of it made every frame of a wheel gesture a re-render
    // plus a style/layout resolve — which cost more than the whole
    // uncut document had, and made scrolling the one thing this work
    // made slower. So the window is committed in steps, and the mounted
    // overscan is what covers the drift in between: most frames of a
    // scroll now change no state at all and cost only what Blitz's own
    // scroll costs.
    // One signal per AXIS, not one for the viewport.
    //
    // They were a single `(left, top)` tuple, and that quietly cost more
    // than everything the culling saved: the timeline reads it, so a
    // VERTICAL scroll re-rendered the timeline — every item, every grid
    // line, every lane — to arrive at exactly the same horizontal
    // layout it already had. Measured, that was 10-12 ms a frame against
    // 0.5 ms for the native scroll itself. Split, a vertical scroll
    // re-renders only the track panel and a horizontal one only the
    // timeline, because a dioxus component subscribes to the signals it
    // actually reads.
    let arrange_scroll_x = use_signal(|| 0.0f32);
    let arrange_scroll_y = use_signal(|| 0.0f32);
    // The timeline's own vertical, committed far more coarsely than the
    // track panel's.
    //
    // Both panes cull vertically and both are worth culling, but they
    // pay different prices for being told. The panel mounts one row per
    // step and has to keep up with the scroll; the timeline re-renders
    // every lane, item and grid line it holds, so being told at the
    // panel's rate cost 10-12 ms a frame to arrive at the same
    // horizontal layout. It gets its own signal, a screenful's worth of
    // step, and the overscan to match.
    let timeline_scroll_y = use_signal(|| 0.0f32);
    let s = shape.read();
    let (tracks, depths) = folders.read().visible(&s.tracks);
    // Shared, not copied: these two go to the TCP column and the
    // timeline, both of which re-render on every scroll frame, and a
    // `Vec<Track>`/`Vec<Item>` prop is deep-copied on each one.
    let tracks = Arc::new(tracks);
    let depths = Arc::new(depths);
    let items = Arc::new(s.items.clone());
    let fx = s.fx.clone();
    let sections = s.sections.clone();
    let marker_flags = s.markers.clone();
    let bpm = s.bpm;
    let seconds = s.length_secs;
    drop(s);

    let t = daw_theme::Theme::default();
    let ground = t.chrome.surface.css();
    let bar_bg = t.chrome.surface_sunken.shade(-0.05).css();
    let rule = t.chrome.surface_sunken.shade(-0.25).css();

    // The arrangement zooms to readable bars — 40 px a bar keeps the
    // ruler's numbers apart at any tempo — and scrolls horizontally.
    // The region and marker lanes live INSIDE that scroll at the same
    // pixels-per-second, the way REAPER's ruler carries them: one time
    // axis, panned together. (The old whole-song strip drew sections
    // at a second scale over a zoomed timeline — a map stapled to a
    // window.)
    let arrange_w = (left_w - ROW_W as f64).max(100.0) as f32;
    let pps = (40.0 * bpm as f32 / 240.0).max(4.0) * arrange_zoom();
    let content_w = (seconds as f32 * pps).max(arrange_w);
    // The lanes' full content height: region lane + marker lane +
    // ruler + one pitch per row, the pitch both columns share. The
    // pane scrolls; the columns cannot drift because they scroll
    // together.
    let env_lanes: HashMap<String, Vec<daw_ui::components::arrangement_view::EnvelopeLaneView>> =
        HashMap::new();
    let rows = plan_rows(&tracks, &env_lanes, ROW_H);
    let ruler_block_h = REGION_H + MARKER_H;
    let arr_h = ARR_RULER_H + 1.0 + rows.iter().map(|(_, _, h)| h + 1.0).sum::<f32>();
    let content_h = ruler_block_h as f32 + arr_h;
    let lanes_h = (arrange_h - TRANSPORT_H).max(60.0);

    // The mixer column: an FX band when any strip has a chain, strips
    // below it, one horizontal scroll for both.
    let fx_band = if fx.is_empty() { 0.0 } else { FX_BAND_H };
    let strip_h = (win_h - fx_band).max(200.0) as f32;

    rsx! {
        style {
            "html, body {{ width: 100%; height: 100%; margin: 0; padding: 0; \
              overflow: hidden; background: {ground}; }}"
        }
        document::Style { {daw_ui::TAILWIND_CSS} }
        document::Style { {BLITZ_FIXES} }
        // The engine sync pair, once for the whole window: drafts flush
        // to the facade at 30 Hz, meter frames feed every strip.
        ControlSync {}
        MeterFeed {}
        if previews_complete() && !tracks.is_empty() {
            span { "data-testid": "workstation-ready", style: "display:none", "{tracks.len()} tracks, {items.len()} items" }
        }
        div {
            style: "display: flex; flex-direction: row; width: 100vw; height: 100vh; \
                    min-height: 0; min-width: 0;",
            // The window's transport keys, REAPER's: Space plays and
            // pauses, Home returns to zero. On the root and focused at
            // mount, so they work before anything is clicked. (The full
            // keymap layer — crates/input's profiles — is the follow-up;
            // these two are the ones hands expect on minute one.)
            tabindex: 0,
            autofocus: true,
            onkeydown: move |e: KeyboardEvent| {
                use dioxus::prelude::Key;
                match e.key() {
                    Key::Character(c) if c == " " => {
                        e.prevent_default();
                        playing.set(!playing());
                        spawn(async move {
                            if let Some(daw) = daw::get()
                                && let Ok(p) = daw.current_project().await
                            {
                                let _ = p.transport().play_pause().await;
                            }
                        });
                    }
                    Key::Home => {
                        playhead.set(0.0);
                        spawn(async move {
                            if let Some(daw) = daw::get()
                                && let Ok(p) = daw.current_project().await
                            {
                                let _ = p.transport().set_position(0.0).await;
                            }
                        });
                    }
                    _ => {}
                }
            },
            // ── Left column: transport, sections, TCP | arrangement,
            // and the editor under them. ──
            div {
                style: "flex: 1 1 auto; min-width: 0; display: flex; \
                        flex-direction: column; min-height: 0;",
                div {
                    style: "height: {TRANSPORT_H}px; flex: 0 0 auto; display: flex; \
                            align-items: center; padding-left: 8px; \
                            background: {bar_bg}; border-bottom: 1px solid {rule};",
                    NativeTransportBar { playing, bpm, position: playhead, on_play, on_stop }
                    StreamBadge { pending: previews_pending }
                }
                // TCP | arrangement, one shared vertical scroll.
                div {
                    style: "height: {lanes_h}px; flex: 0 0 auto; overflow-y: scroll; \
                            overflow-x: hidden;",
                    "data-testid": "workstation-arrange",
                    onscroll: {
                        let mut arrange_scroll_y = arrange_scroll_y;
                        let mut timeline_scroll_y = timeline_scroll_y;
                        move |event: ScrollEvent| {
                            let y = event.scroll_top() as f32;
                            if (*arrange_scroll_y.peek() - y).abs() >= COMMIT_Y {
                                arrange_scroll_y.set(y);
                            }
                            if (*timeline_scroll_y.peek() - y).abs() >= COMMIT_TIMELINE_Y {
                                timeline_scroll_y.set(y);
                            }
                        }
                    },
                    div {
                        style: "position: relative; display: flex; \
                                width: {left_w}px; height: {content_h}px;",
                        div {
                            style: "width: {ROW_W}px; flex: 0 0 auto; overflow: hidden;",
                            // The full ruler block's height in empty
                            // panel — region lane, marker lane and the
                            // bar ruler — so row one starts where lane
                            // one does.
                            div { style: "height: {ruler_block_h + ARR_RULER_H as f64 + 1.0}px;" }
                            TcpColumn {
                                tracks: tracks.clone(),
                                depths: depths.clone(),
                                folders,
                                scroll: arrange_scroll_y,
                                rows_top: ruler_block_h as f32 + ARR_RULER_H + 1.0,
                                view_h: lanes_h as f32,
                            }
                        }
                        if tracks.is_empty() {
                            // The fast shape lands in well under a
                            // second; this is a state, not a splash.
                            div {
                                style: "padding: 16px; color: #a0a0a0; font-size: 12px;",
                                "data-testid": "workstation-loading",
                                "Opening the project — tracks and items land first, \
                                 waveforms stream in behind them…"
                            }
                        } else {
                            // The lanes' own horizontal scroll — the
                            // TCP column stays put, the timeline pans.
                            div {
                                "data-testid": "workstation-timeline",
                                style: "width: {arrange_w}px; flex: 0 0 auto; \
                                        overflow-x: scroll; overflow-y: hidden;",
                                onscroll: {
                                    let mut arrange_scroll_x = arrange_scroll_x;
                                    move |event: ScrollEvent| {
                                        let x = event.scroll_left() as f32;
                                        if (*arrange_scroll_x.peek() - x).abs() >= COMMIT_X {
                                            arrange_scroll_x.set(x);
                                        }
                                    }
                                },
                                onwheel: move |event: WheelEvent| {
                                    if event.modifiers().contains(Modifiers::CONTROL) {
                                        let (_, dy) = expression_editor_ui::scroll::notches(&event.delta());
                                        let next = (arrange_zoom() * (-dy as f32 * 0.12).exp()).clamp(0.25, 16.0);
                                        arrange_zoom.set(next);
                                        event.prevent_default();
                                        event.stop_propagation();
                                    }
                                },
                                div {
                                    style: "position: relative; width: {content_w}px; \
                                            height: {content_h}px;",
                                    // ── The ruler block, REAPER's way:
                                    // region bands, then marker flags,
                                    // then the bar ruler — all at the
                                    // timeline's own pixels-per-second,
                                    // panning with it. ──
                                    div {
                                        style: "position: relative; height: {REGION_H}px; \
                                                background: {bar_bg}; \
                                                border-bottom: 1px solid {rule}; overflow: hidden;",
                                        "data-testid": "workstation-sections",
                                        for sec in sections.iter() {
                                            {
                                                let x = sec.start * pps as f64;
                                                let w = ((sec.end - sec.start) * pps as f64).max(1.0);
                                                let bg = sec.color.clone().unwrap_or_else(|| rule.clone());
                                                let name = sec.name.clone();
                                                let to = sec.start;
                                                rsx! {
                                                    div {
                                                        style: "position: absolute; left: {x}px; top: 1px; \
                                                                width: {w}px; height: {REGION_H - 2.0}px; \
                                                                background: {bg}; border-radius: 3px; \
                                                                border-right: 1px solid {ground}; \
                                                                font-size: 9px; color: #101014; \
                                                                line-height: {REGION_H - 2.0}px; \
                                                                padding-left: 4px; overflow: hidden;",
                                                        onclick: move |_| {
                                                            playhead.set(to);
                                                            spawn(async move {
                                                                if let Some(daw) = daw::get()
                                                                    && let Ok(p) = daw.current_project().await
                                                                {
                                                                    let _ = p.transport().set_position(to).await;
                                                                }
                                                            });
                                                        },
                                                        "{name}"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    div {
                                        style: "position: relative; height: {MARKER_H}px; \
                                                background: {bar_bg}; \
                                                border-bottom: 1px solid {rule}; overflow: hidden;",
                                        "data-testid": "workstation-markers",
                                        for m in marker_flags.iter() {
                                            {
                                                let x = m.at * pps as f64;
                                                let bg = m.color.clone().unwrap_or_else(|| rule.clone());
                                                let label = format!("{} {}", m.idx, m.name);
                                                let to = m.at;
                                                rsx! {
                                                    // The flag: a colored tab at the
                                                    // marker, its number and name
                                                    // running right, REAPER-style.
                                                    div {
                                                        style: "position: absolute; left: {x}px; top: 0; \
                                                                width: 2px; height: 100%; background: {bg};",
                                                    }
                                                    div {
                                                        style: "position: absolute; left: {x + 2.0}px; top: 1px; \
                                                                height: {MARKER_H - 2.0}px; \
                                                                background: {bg}; border-radius: 0 3px 3px 0; \
                                                                font-size: 8px; color: #101014; \
                                                                line-height: {MARKER_H - 2.0}px; \
                                                                padding: 0 4px; overflow: hidden; \
                                                                white-space: nowrap;",
                                                        onclick: move |_| {
                                                            playhead.set(to);
                                                            spawn(async move {
                                                                if let Some(daw) = daw::get()
                                                                    && let Ok(p) = daw.current_project().await
                                                                {
                                                                    let _ = p.transport().set_position(to).await;
                                                                }
                                                            });
                                                        },
                                                        "{label}"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    TimelineItems {
                                        tracks: tracks.clone(),
                                        items,
                                        previews: item_previews_sig,
                                        scroll: arrange_scroll_x,
                                        scroll_y: timeline_scroll_y,
                                        canvas_top: ruler_block_h as f32 + ARR_RULER_H,
                                        // The canvas' origin sits below
                                        // the region/marker lanes and
                                        // the arrange ruler, so that is
                                        // what the scroll offset has to
                                        // lose to land in canvas
                                        // coordinates.
                                        view_w: arrange_w,
                                        view_h: lanes_h as f32,
                                        width: content_w,
                                        height: arr_h,
                                        pixels_per_second: pps,
                                        bpm,
                                    }
                                    PlayheadLine {
                                        playhead,
                                        pps: pps as f64,
                                        height: content_h as f64,
                                    }
                                }
                            }
                        }
                    }
                }
                div {
                    style: "flex: 1 1 auto; min-height: 0; border-top: 1px solid {rule};",
                    "data-testid": "workstation-editor",
                    ExpressionEditor {
                        editor,
                        quantize_bins: bins(),
                        quantize_previews: previews_sig(),
                        on_quantize_change: on_change,
                        on_quantize_apply: on_apply,
                        on_hit,
                        on_save,
                        on_undo,
                        on_redo,
                        host_error: error.and_then(|error| error()),
                        playhead_secs: playhead,
                    }
                }
            }
            // ── Right column: the mixer, full height. One horizontal
            // scroll carries each track's FX slots and strip together,
            // so a chain never drifts off its channel. ──
            mixer::WorkstationMixer {
                tracks: tracks.as_ref().clone(), depths: depths.as_ref().clone(), fx, folders,
                height: strip_h, rule: rule.clone(), background: bar_bg.clone(),
            }
        }
    }
}

/// The timeline's items, isolated so a preview batch re-renders this
/// subtree and nothing else. The whole app reading the previews signal
/// was the freeze: every arriving waveform re-laid-out the editor and
/// the mixer too.
///
/// It is also where the song is cut down to what the window can see.
/// That has to happen HERE, above the props boundary, because the props
/// boundary is the expensive part: dioxus hands a component its props by
/// value on every render, so an `items: Vec<Item>` of 877 takes and a
/// `previews` map holding a peak vector each were being deep-copied —
/// twice, once into this component and once into `ArrangePreview` — for
/// every frame of a scroll. Filtering first means a scroll copies the
/// forty items on screen, not the whole record.
#[component]
fn TimelineItems(
    /// Behind an `Arc` for the same reason: the track list is re-read on
    /// every scroll frame and nothing here mutates it.
    tracks: Arc<Vec<Track>>,
    items: Arc<Vec<Item>>,
    previews: Signal<HashMap<String, ItemPreview>>,
    /// How far the timeline has been panned, read HERE and nowhere
    /// above: this is the component a horizontal scroll is allowed to
    /// re-render.
    ///
    /// Horizontal ONLY, on purpose. The lanes are a few screens tall and
    /// many screens wide, so culling across time is what pays; culling
    /// down the tracks saved a handful of nodes and cost a re-render of
    /// the whole timeline on every vertical scroll. Reading only the
    /// axis that matters is what keeps the two panes independent.
    scroll: Signal<f32>,
    /// The timeline's own vertical, on its coarse step — see
    /// `COMMIT_TIMELINE_Y`.
    scroll_y: Signal<f32>,
    /// How far the canvas' origin sits below the scroll container's.
    canvas_top: f32,
    view_w: f32,
    view_h: f32,
    width: f32,
    height: f32,
    pixels_per_second: f32,
    bpm: f64,
) -> Element {
    // Static preview — no live transport, so the cursors never move.
    let play_position = use_signal(|| 0.0f32);
    let edit_position = use_signal(|| 0.0f32);
    let left = scroll();
    let window = (left, scroll_y() - canvas_top, view_w, view_h);
    // The same margin the canvas culls lanes and grid lines by, so an
    // item and its lane come into view together.
    const OVERSCAN: f32 = 320.0;
    let (x0, x1) = (window.0 - OVERSCAN, window.0 + view_w + OVERSCAN);
    let on_screen: Vec<Item> = items
        .iter()
        .filter(|item| {
            let l = item.position.as_seconds() as f32 * pixels_per_second;
            let r = l + item.length.as_seconds() as f32 * pixels_per_second;
            r >= x0 && l <= x1
        })
        .cloned()
        .collect();
    // Read the map, copy the handful of peak vectors that are on
    // screen, and drop the borrow — never clone the whole cache.
    let seen = previews.read();
    let previews: HashMap<String, ItemPreview> = on_screen
        .iter()
        .filter_map(|item| {
            seen.get(&item.guid)
                .map(|preview| (item.guid.clone(), preview.clone()))
        })
        .collect();
    drop(seen);
    let tracks = tracks.as_ref().clone();
    rsx! {
        ArrangePreview {
            scrollable: false,
            viewport: Some(window),
            tracks,
            items: on_screen,
            previews,
            width,
            height,
            pixels_per_second,
            bpm,
            play_position,
            edit_position,
        }
    }
}

/// A track row plus the one-pixel divider under it — the pitch the
/// arrangement's lanes use, which is what the column has to match.
const ROW_PITCH: f32 = ROW_H + 1.0;

/// Which rows a column `view_h` tall, scrolled to `top`, has to mount.
///
/// Six rows of overscan either side: twice [`COMMIT_Y`]'s step, so a
/// scroll that has moved but not yet committed a new window is still
/// looking at rows that are mounted.
fn row_window(top: f32, view_h: f32, count: usize) -> std::ops::Range<usize> {
    const OVERSCAN_ROWS: usize = 6;
    let first = ((top.max(0.0) / ROW_PITCH).floor() as usize).saturating_sub(OVERSCAN_ROWS);
    let last = (((top.max(0.0) + view_h.max(0.0)) / ROW_PITCH).ceil() as usize + OVERSCAN_ROWS)
        .min(count);
    first.min(last)..last
}

/// The TCP column, mounting only the rows the shared viewport can see.
///
/// A track panel row is not cheap — REAPER's is a tint, a gutter, a
/// name field, two knobs, a meter, arm/mute/solo and an FX slot, and
/// this one traces all of it — so sixty-five of them are thousands of
/// nodes that a scroll pays style and layout for whether or not they
/// are on screen. Rows above and below the window are replaced by two
/// spacers of exactly their height, so the rows that ARE mounted keep
/// their document position and stay level with their lanes: the
/// column's alignment contract with the arrangement is what makes this
/// safe to do at all, and what would break first if it were wrong.
///
/// Its own component because it reads `scroll`: a vertical scroll must
/// re-render this column and the timeline, and nothing else.
#[component]
fn TcpColumn(
    tracks: Arc<Vec<Track>>,
    depths: Arc<Vec<u32>>,
    mut folders: Signal<daw_ui::components::folders::FolderState>,
    /// How far the shared viewport has scrolled down. Vertical only —
    /// see `TimelineItems` for why each pane reads one axis.
    scroll: Signal<f32>,
    /// Where row zero starts, below the ruler block.
    rows_top: f32,
    view_h: f32,
) -> Element {
    let rows = row_window((scroll() - rows_top).max(0.0), view_h, tracks.len());
    let (first, last) = (rows.start, rows.end);
    let before = first as f32 * ROW_PITCH;
    let after = (tracks.len() - last) as f32 * ROW_PITCH;
    rsx! {
        div { style: "height: {before}px;" }
        for i in first..last {
            TrackRow {
                key: "{tracks[i].guid}",
                track: tracks[i].clone(),
                index: tracks[i].index,
                depth: depths[i],
                collapsed: folders.read().is_collapsed(&tracks[i].guid),
                onfoldertoggle: { let guid = tracks[i].guid.clone(); move |_| folders.write().toggle(&guid) },
            }
        }
        div { style: "height: {after}px;" }
    }
}

/// The playhead line, isolated for the same reason: a position tick
/// arrives many times a second while playing, and it must move one
/// 1px div, not re-render the window.
#[component]
fn PlayheadLine(playhead: Signal<f64>, pps: f64, height: f64) -> Element {
    let x = playhead() * pps;
    rsx! {
        div {
            style: "position: absolute; left: {x}px; top: 0; width: 1px; \
                    height: {height}px; background: #f8fafc; opacity: 0.8;",
        }
    }
}

/// The waveform-streaming readout. Its own component so the countdown
/// re-renders a span, not the app.
#[component]
fn StreamBadge(pending: Signal<usize>) -> Element {
    if pending() == 0 {
        return rsx! {};
    }
    rsx! {
        span {
            style: "margin-left: 12px; font-size: 10px; color: #7b7b7b;",
            "waveforms… {pending()} left"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tcp_window_is_bounded_at_both_ends_and_covers_the_commit_step() {
        // Nothing to show, and a column shorter than its overscan.
        assert_eq!(row_window(0.0, 400.0, 0), 0..0);
        assert_eq!(row_window(0.0, 400.0, 4), 0..4);
        // At the top, the window starts at the first row rather than
        // underflowing past it.
        let top = row_window(0.0, 400.0, 65);
        assert_eq!(top.start, 0);
        assert!(top.end > (400.0 / ROW_PITCH) as usize);
        // Scrolled to the end, the window stops at the last row.
        assert_eq!(row_window(65.0 * ROW_PITCH, 400.0, 65).end, 65);
        // A viewport that has drifted by a whole commit step without
        // committing must still be inside the mounted rows: the window
        // for the OLD offset has to cover the NEW one's visible span.
        let committed = row_window(20.0 * ROW_PITCH, 400.0, 65);
        let drifted = row_window(20.0 * ROW_PITCH + COMMIT_Y, 400.0, 65);
        let visible_end = ((20.0 * ROW_PITCH + COMMIT_Y + 400.0) / ROW_PITCH).ceil() as usize;
        assert!(committed.end >= visible_end.min(65), "{committed:?} vs {drifted:?}");
    }
}
