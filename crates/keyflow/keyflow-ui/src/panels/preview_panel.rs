//! Live chart preview panel — standalone WGPU-rendered chart with DAW cursor following.
//!
//! Renders the active song's chart with auto-follow, click-to-seek,
//! zoom/pan, and smooth cursor interpolation. Can be placed anywhere
//! in the dock layout independently.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dioxus::prelude::*;
use dioxus_core::Task;
use fts_ui::prelude::{Button, ButtonSize, ButtonVariant};
use kurbo::Affine;

use crate::chart_graphics::ChartGraphics;
use crate::{
    CHART_CURSOR_TICK, CHART_CURSOR_VISIBLE, CHART_SOURCE, ChartLayoutManager, SESSION_CHART_SOURCE,
};
use keyflow::engraver::layout::chart::{ChartLayoutConfig, ChartLayoutEngine, LayoutMode};
use keyflow::engraver::style::MStyle;

use dock_dioxus::DOCK_WORKSPACE;
use dock_proto::PanelId;
use session_ui::{
    ACTIVE_INDICES, ACTIVE_PLAYBACK_IS_PLAYING, ACTIVE_PLAYBACK_MUSICAL, CHART_AREA_BOUNDS,
    ChartAreaBounds, PERF_CHART_BASE_SCALE, PERF_CHART_CLICK, PERF_CHART_HOVER,
    PERF_CHART_VIEWPORT, PerfChartViewport, Session,
};
use tracing::info;

use super::render_stats::{PerfCursorMotionState, PerfRenderAccumulator, PerfStaticSceneKey};

/// Live chart preview panel with auto-follow cursor, 120Hz ticker, and click-to-seek.
#[component]
pub fn ChartPreviewPanel() -> Element {
    let graphics = consume_context::<Arc<Mutex<ChartGraphics>>>();

    // --- Chart layout manager (created once, persists across renders) ---
    let perf_layout_manager: Signal<Option<std::rc::Rc<std::cell::RefCell<ChartLayoutManager>>>> =
        use_signal(|| match ChartLayoutManager::new() {
            Ok(m) => {
                tracing::debug!("ChartPreviewPanel: layout manager created");
                Some(std::rc::Rc::new(std::cell::RefCell::new(m)))
            }
            Err(e) => {
                tracing::error!("Failed to create ChartPreviewPanel layout manager: {}", e);
                None
            }
        });

    // Layout generation counter — bumped when layout changes, triggers re-render
    let mut perf_layout_gen = use_signal(|| 0u64);
    let perf_static_scene_cache =
        use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(None::<PerfStaticSceneKey>)));
    let perf_cursor_frame_clock = use_signal(|| 0u64);
    let perf_cursor_motion =
        use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(PerfCursorMotionState::default())));
    let perf_render_accumulator =
        use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(PerfRenderAccumulator::new())));

    // Enable transparency on mount, disable on unmount
    use_effect(|| {
        document::eval(r#"document.documentElement.classList.add('transparent-mode');"#);
    });
    use_drop(|| {
        let workspace = DOCK_WORKSPACE.peek();
        let chart_editor_visible = workspace
            .windows
            .values()
            .any(|w| w.layout.panel_is_visible(PanelId::ChartEditor));
        let chart_preview_visible = workspace
            .windows
            .values()
            .any(|w| w.layout.panel_is_visible(PanelId::ChartPreview));
        if !chart_editor_visible && !chart_preview_visible {
            document::eval(r#"document.documentElement.classList.remove('transparent-mode');"#);
        }
    });

    // --- Bounds polling: continuously query chart-preview-panel position ---
    {
        use_future(move || async move {
            loop {
                tokio::time::sleep(Duration::from_millis(200)).await;

                let result = document::eval(
                    r#"
                    const el = document.getElementById('chart-preview-panel');
                    if (el) {
                        const rect = el.getBoundingClientRect();
                        const dpr = window.devicePixelRatio || 1;
                        return JSON.stringify({
                            x: rect.x * dpr,
                            y: rect.y * dpr,
                            width: rect.width * dpr,
                            height: rect.height * dpr,
                            dpr: dpr
                        });
                    }
                    return "null";
                "#,
                );

                if let Ok(value) = result.await {
                    let json_str = value
                        .as_str()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| value.to_string());

                    if json_str != "null" && json_str != "\"null\"" {
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&json_str) {
                            let x = parsed["x"].as_f64().unwrap_or(0.0);
                            let y = parsed["y"].as_f64().unwrap_or(0.0);
                            let width = parsed["width"].as_f64().unwrap_or(0.0);
                            let height = parsed["height"].as_f64().unwrap_or(0.0);
                            let dpr = parsed["dpr"].as_f64().unwrap_or(1.0);

                            if width > 0.0 && height > 0.0 {
                                let current = *CHART_AREA_BOUNDS.peek();
                                if (current.x - x).abs() > 1.0
                                    || (current.y - y).abs() > 1.0
                                    || (current.width - width).abs() > 1.0
                                    || (current.height - height).abs() > 1.0
                                {
                                    *CHART_AREA_BOUNDS.write() =
                                        ChartAreaBounds::new(x, y, width, height, dpr);
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    // --- 120Hz local cursor ticker ---
    {
        let mut frame_clock = perf_cursor_frame_clock;
        use_future(move || async move {
            let mut interval = tokio::time::interval(Duration::from_millis(8));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if *ACTIVE_PLAYBACK_IS_PLAYING.peek() {
                    frame_clock.set(frame_clock() + 1);
                }
            }
        });
    }

    // --- Layout effect: parse + layout chart when source changes ---
    // Runs parse+layout on a background thread to keep the UI responsive.
    {
        let mut layout_task: Signal<Option<Task>> = use_signal(|| None);

        use_effect(move || {
            let source = SESSION_CHART_SOURCE
                .read()
                .clone()
                .unwrap_or_else(|| CHART_SOURCE.read().clone());
            let bounds = *CHART_AREA_BOUNDS.read();

            if !bounds.is_valid() {
                return;
            }

            let snippet_mode = false;

            let (text_font, symbol_font) = if let Some(ref manager_rc) = *perf_layout_manager.read()
            {
                let manager = manager_rc.borrow();
                if !manager.needs_layout(&source, snippet_mode) {
                    return;
                }
                manager.font_data()
            } else {
                return;
            };

            // Cancel any in-flight layout task
            if let Some(prev) = *layout_task.peek() {
                prev.cancel();
            }

            let task = spawn(async move {
                let source_clone = source.clone();
                let result = tokio::task::spawn_blocking(move || {
                    use std::sync::OnceLock;
                    static BG_STYLE: OnceLock<&'static MStyle> = OnceLock::new();
                    let style = *BG_STYLE.get_or_init(|| Box::leak(Box::new(MStyle::new())));

                    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
                    let chart = keyflow::parse(&source_clone).map_err(|e| format!("{}", e))?;

                    let config = ChartLayoutConfig::master_rhythm().with_page_offsets(true);
                    let mode = LayoutMode::paginated_a4();
                    let layout_result = engine.layout_chart_with_config(&chart, &mode, &config);
                    Ok::<_, String>((chart, layout_result))
                })
                .await;

                match result {
                    Ok(Ok((chart, layout_result))) => {
                        if let Some(ref manager_rc) = *perf_layout_manager.read() {
                            let mut manager = manager_rc.borrow_mut();
                            manager.apply_precomputed_layout(
                                chart,
                                layout_result,
                                &source,
                                snippet_mode,
                            );
                            perf_layout_gen.set(perf_layout_gen() + 1);
                            tracing::debug!(
                                "ChartPreview layout done (gen {}), pages={} [background]",
                                perf_layout_gen(),
                                manager.total_pages()
                            );
                        }
                    }
                    Ok(Err(e)) => {
                        tracing::warn!("ChartPreview parse error: {}", e);
                    }
                    Err(e) => {
                        tracing::warn!("ChartPreview background layout panicked: {}", e);
                    }
                }
            });

            layout_task.set(Some(task));
        });
    }

    // --- Render effect: auto-follow cursor OR manual viewport, render scene ---
    {
        let graphics_clone = graphics.clone();
        let static_scene_cache = perf_static_scene_cache.clone();
        let cursor_motion = perf_cursor_motion.clone();
        let render_accumulator = perf_render_accumulator.clone();
        use_effect(move || {
            let frame_started = Instant::now();
            let current_cursor_tick = *CHART_CURSOR_TICK.read();
            let layout_generation = perf_layout_gen();
            let _frame_clock = perf_cursor_frame_clock();
            let bounds = *CHART_AREA_BOUNDS.read();
            let perf_vp = *PERF_CHART_VIEWPORT.read();
            let hover_point = *PERF_CHART_HOVER.read();
            let pending_click = *PERF_CHART_CLICK.read();
            let playback_musical = *ACTIVE_PLAYBACK_MUSICAL.read();
            let playback_is_playing = *ACTIVE_PLAYBACK_IS_PLAYING.read();
            let active_song_index = ACTIVE_INDICES.peek().song_index;

            if !bounds.is_valid() {
                return;
            }

            if let Some(ref manager_rc) = *perf_layout_manager.read() {
                let mut manager = manager_rc.borrow_mut();
                if manager.layout_result().is_none() {
                    return;
                }

                let mut cursor_tick = current_cursor_tick;

                // Follow live DAW playback
                if playback_is_playing {
                    if let Some(musical) = playback_musical {
                        if let Some(playback_tick) = manager.tick_for_musical_position(
                            musical.measure - 1,
                            musical.beat - 1,
                            musical.subdivision,
                        ) {
                            let now = Instant::now();
                            {
                                let mut motion = cursor_motion.borrow_mut();
                                if let (Some(prev_tick), Some(prev_time)) =
                                    (motion.last_sample_tick, motion.last_sample_time)
                                {
                                    let dt = now.duration_since(prev_time).as_secs_f64();
                                    if dt > 0.0 {
                                        motion.velocity_ticks_per_sec =
                                            (playback_tick - prev_tick) as f64 / dt;
                                    }
                                }
                                motion.last_sample_tick = Some(playback_tick);
                                motion.last_sample_time = Some(now);
                            }
                        }
                    }

                    // Extrapolate cursor forward between transport packets
                    {
                        let motion = cursor_motion.borrow();
                        if let (Some(sample_tick), Some(sample_time)) =
                            (motion.last_sample_tick, motion.last_sample_time)
                        {
                            let elapsed = Instant::now().duration_since(sample_time).as_secs_f64();
                            let max_ahead = 480.0;
                            let ahead = (motion.velocity_ticks_per_sec * elapsed)
                                .clamp(-max_ahead, max_ahead);
                            cursor_tick = (sample_tick as f64 + ahead).round() as i64;
                        }
                    }
                } else {
                    let mut motion = cursor_motion.borrow_mut();
                    motion.last_sample_tick = None;
                    motion.last_sample_time = None;
                    motion.velocity_ticks_per_sec = 0.0;
                }

                // Click-to-seek
                if let Some((scene_x, scene_y)) = pending_click {
                    *PERF_CHART_CLICK.write() = None;
                    if let Some(tick) = manager.tick_at_scene_point(scene_x, scene_y) {
                        cursor_tick = tick;
                        if tick != current_cursor_tick {
                            *CHART_CURSOR_TICK.write() = tick;
                        }

                        if let Some(song_index) = active_song_index {
                            if let Some((measure, beat, subdivision)) =
                                manager.musical_position_at_tick(tick)
                            {
                                spawn(async move {
                                    let _ = Session::get()
                                        .setlist()
                                        .seek_to_musical_position(
                                            song_index,
                                            daw_proto::MusicalPosition::new(
                                                measure + 1,
                                                beat + 1,
                                                subdivision,
                                            ),
                                        )
                                        .await;
                                });
                            }
                        }
                    }
                }

                // Compute viewport transform
                let (page_num, sys_idx) = manager.system_for_tick(cursor_tick).unwrap_or((1, 0));

                let page_width = manager
                    .layout_result()
                    .and_then(|r| r.pages.iter().find(|p| p.number == page_num))
                    .map(|p| p.width)
                    .unwrap_or(595.0);

                let pad_physical = 20.0 * bounds.dpr;
                let available_width = bounds.width - pad_physical * 2.0;
                let base_scale = if available_width > 0.0 && page_width > 0.0 {
                    available_width / page_width
                } else {
                    bounds.dpr
                };

                if (*PERF_CHART_BASE_SCALE.peek() - base_scale).abs() > 0.001 {
                    *PERF_CHART_BASE_SCALE.write() = base_scale;
                }

                let scale = base_scale * perf_vp.zoom;

                let (scroll_x, scroll_y) = if perf_vp.auto_follow {
                    let sy = manager
                        .scroll_y_for_system(page_num, sys_idx, scale, 1.0, bounds.dpr)
                        .unwrap_or(0.0);

                    let page_x_offset = manager
                        .layout_result()
                        .and_then(|r| {
                            r.pages
                                .iter()
                                .find(|p| p.number == page_num)
                                .map(|p| p.x_offset)
                        })
                        .unwrap_or(0.0);
                    let sx = page_x_offset * scale / bounds.dpr;

                    (sx, sy)
                } else {
                    (perf_vp.scroll_x, perf_vp.scroll_y)
                };

                let transform = Affine::translate((
                    pad_physical - scroll_x * bounds.dpr,
                    pad_physical - scroll_y * bounds.dpr,
                )) * Affine::scale(scale);

                let cursor = if *CHART_CURSOR_VISIBLE.peek() {
                    Some(cursor_tick)
                } else {
                    None
                };

                let current_key = PerfStaticSceneKey {
                    generation: layout_generation,
                    width: bounds.width,
                    height: bounds.height,
                    tx: transform.translation().x,
                    ty: transform.translation().y,
                    scale,
                };

                let needs_static_rebuild = {
                    let cache = static_scene_cache.borrow();
                    match cache.as_ref() {
                        Some(cached_key) => !cached_key.approx_eq(current_key),
                        None => true,
                    }
                };

                if needs_static_rebuild {
                    *static_scene_cache.borrow_mut() = Some(current_key);
                }

                let render_start = Instant::now();
                if let Ok(mut gfx) = graphics_clone.lock() {
                    let win_size = dioxus::desktop::window().window.inner_size();
                    let (sw, sh) = gfx.size();
                    if sw != win_size.width || sh != win_size.height {
                        gfx.resize(win_size.width, win_size.height);
                    }

                    let dock_offset = Affine::translate((bounds.x, bounds.y));
                    gfx.render_chart(|painter| {
                        manager.render_static_layer_to_scene(
                            painter,
                            bounds.width,
                            bounds.height,
                            dock_offset,
                            transform,
                        );
                        manager.render_overlay_layer_to_scene(
                            painter,
                            bounds.width,
                            bounds.height,
                            dock_offset,
                            transform,
                            cursor,
                            if playback_is_playing {
                                None
                            } else {
                                hover_point
                            },
                        );
                    });
                }
                let render_ms = render_start.elapsed().as_secs_f64() * 1000.0;
                dioxus::desktop::window().window.request_redraw();

                let frame_ms = frame_started.elapsed().as_secs_f64() * 1000.0;
                let mut accum = render_accumulator.borrow_mut();
                accum.record(render_ms, 0.0, frame_ms, needs_static_rebuild);
                if let Some((
                    frames,
                    avg_frame_ms,
                    p95_ms,
                    avg_overlay_ms,
                    static_rebuilds,
                    avg_static_ms,
                )) = accum.maybe_flush_log()
                {
                    info!(
                        "ChartPreview renderer (5s): frames={}, avg={:.2}ms, p95={:.2}ms, overlay={:.2}ms, static_rebuilds={}, static={:.2}ms",
                        frames,
                        avg_frame_ms,
                        p95_ms,
                        avg_overlay_ms,
                        static_rebuilds,
                        avg_static_ms,
                    );
                }
            }
        });
    }

    // Render the transparent div with mouse handlers
    let perf_vp = *PERF_CHART_VIEWPORT.read();

    let mut dragging = use_signal(|| false);
    let mut dragged = use_signal(|| false);
    let mut last_mouse = use_signal(|| (0.0f64, 0.0f64));

    rsx! {
        div {
            id: "chart-preview-panel",
            class: "h-full w-full relative cursor-grab",
            style: "background: transparent !important; background-color: transparent !important;",

            onwheel: move |evt| {
                let delta_y = evt.delta().strip_units().y;
                let mut vp = PERF_CHART_VIEWPORT.write();
                let zoom_factor = if delta_y < 0.0 { 1.05 } else { 0.95 };
                vp.zoom = (vp.zoom * zoom_factor).clamp(0.1, 8.0);
                vp.auto_follow = false;
            },

            onmousedown: move |evt| {
                dragging.set(true);
                dragged.set(false);
                let coords = evt.client_coordinates();
                last_mouse.set((coords.x, coords.y));
            },

            onmousemove: move |evt| {
                let coords = evt.client_coordinates();

                if *dragging.read() {
                    let (lx, ly) = *last_mouse.read();
                    let dx = coords.x - lx;
                    let dy = coords.y - ly;

                    if dx.abs() > 1.0 || dy.abs() > 1.0 {
                        dragged.set(true);
                    }

                    let mut vp = PERF_CHART_VIEWPORT.write();
                    vp.scroll_x -= dx;
                    vp.scroll_y -= dy;
                    vp.auto_follow = false;

                    last_mouse.set((coords.x, coords.y));
                    *PERF_CHART_HOVER.write() = None;
                } else {
                    let bounds = *CHART_AREA_BOUNDS.peek();
                    let vp = *PERF_CHART_VIEWPORT.peek();
                    let base_scale = *PERF_CHART_BASE_SCALE.peek();

                    if base_scale > 0.0 && bounds.dpr > 0.0 {
                        let scale = base_scale * vp.zoom;
                        let pad = 20.0 * bounds.dpr;
                        let px_x = coords.x * bounds.dpr - bounds.x;
                        let px_y = coords.y * bounds.dpr - bounds.y;
                        let scene_x = (px_x - pad + vp.scroll_x * bounds.dpr) / scale;
                        let scene_y = (px_y - pad + vp.scroll_y * bounds.dpr) / scale;
                        *PERF_CHART_HOVER.write() = Some((scene_x, scene_y));
                    }
                }
            },

            onmouseup: move |evt| {
                if !*dragged.read() {
                    let coords = evt.client_coordinates();
                    let bounds = *CHART_AREA_BOUNDS.peek();
                    let vp = *PERF_CHART_VIEWPORT.peek();
                    let base_scale = *PERF_CHART_BASE_SCALE.peek();

                    if base_scale > 0.0 && bounds.dpr > 0.0 {
                        let scale = base_scale * vp.zoom;
                        let pad = 20.0 * bounds.dpr;
                        let px_x = coords.x * bounds.dpr - bounds.x;
                        let px_y = coords.y * bounds.dpr - bounds.y;
                        let scene_x = (px_x - pad + vp.scroll_x * bounds.dpr) / scale;
                        let scene_y = (px_y - pad + vp.scroll_y * bounds.dpr) / scale;
                        *PERF_CHART_CLICK.write() = Some((scene_x, scene_y));
                    }
                }

                dragging.set(false);
                dragged.set(false);
                *PERF_CHART_HOVER.write() = None;
            },

            onmouseleave: move |_| {
                dragging.set(false);
                dragged.set(false);
                *PERF_CHART_HOVER.write() = None;
            },

            if !perf_vp.auto_follow {
                div {
                    class: "absolute top-4 left-4 z-10",
                    Button {
                        variant: ButtonVariant::Ghost,
                        size: ButtonSize::Small,
                        class: "bg-card/80 backdrop-blur-sm shadow-lg".to_string(),
                        on_click: Callback::new(move |_| {
                            *PERF_CHART_VIEWPORT.write() = PerfChartViewport::default();
                        }),
                        fts_ui::lucide_dioxus::RotateCcw {
                            size: 14,
                        }
                        "Reset View"
                    }
                }
            }
        }
    }
}
