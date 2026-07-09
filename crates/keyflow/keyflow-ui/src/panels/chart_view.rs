//! Chart editor panel — split editor with live WGPU chart preview.
//!
//! Uses `ChartEditorLayout` for the Dioxus UI, and renders the chart
//! via `ChartLayoutManager` → `ChartGraphics` for WGPU.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dioxus::prelude::*;
use dioxus_core::Task;
use kurbo::Affine;

use crate::chart_graphics::ChartGraphics;
use crate::signals::{ChartEditorBounds, PreviewMode};
use crate::{
    CHART_BASE_SCALE, CHART_CURSOR_POSITION, CHART_CURSOR_SCENE_CLICK, CHART_CURSOR_TICK,
    CHART_CURSOR_VISIBLE, CHART_EDITOR_BOUNDS, CHART_HOVER_SCENE_POINT, CHART_PAGE_INFO,
    CHART_PREVIEW_MODE, CHART_RENDER_STATS, CHART_SOURCE, CHART_VIEWPORT, ChartEditorLayout,
    ChartLayoutManager, SemanticZoomLevel,
};
use keyflow::engraver::layout::chart::ChartLayoutEngine;
use keyflow::engraver::style::MStyle;

use dock_dioxus::DOCK_WORKSPACE;
use dock_proto::PanelId;
use session_ui::{ACTIVE_INDICES, ACTIVE_PLAYBACK_IS_PLAYING, ACTIVE_PLAYBACK_MUSICAL, Session};

use super::render_stats::{FpsTracker, PerfCursorMotionState};

struct ChartViewPerfWindow {
    started: Instant,
    frame_ms: Vec<f64>,
    prep_ms_total: f64,
    lock_wait_ms_total: f64,
    render_ms_total: f64,
    static_ms_total: f64,
    overlay_ms_total: f64,
    resize_count: u64,
    lock_contention_frames: u64,
}

impl ChartViewPerfWindow {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            frame_ms: Vec::with_capacity(1024),
            prep_ms_total: 0.0,
            lock_wait_ms_total: 0.0,
            render_ms_total: 0.0,
            static_ms_total: 0.0,
            overlay_ms_total: 0.0,
            resize_count: 0,
            lock_contention_frames: 0,
        }
    }

    fn record(
        &mut self,
        frame_ms: f64,
        prep_ms: f64,
        lock_wait_ms: f64,
        render_ms: f64,
        static_ms: f64,
        overlay_ms: f64,
        did_resize: bool,
    ) {
        self.frame_ms.push(frame_ms);
        self.prep_ms_total += prep_ms;
        self.lock_wait_ms_total += lock_wait_ms;
        self.render_ms_total += render_ms;
        self.static_ms_total += static_ms;
        self.overlay_ms_total += overlay_ms;
        if did_resize {
            self.resize_count += 1;
        }
        if lock_wait_ms > 1.0 {
            self.lock_contention_frames += 1;
        }
    }

    fn maybe_flush_log(&mut self) {
        if self.started.elapsed() < Duration::from_secs(5) || self.frame_ms.is_empty() {
            return;
        }

        let frames = self.frame_ms.len() as f64;
        let avg_frame_ms = self.frame_ms.iter().sum::<f64>() / frames;
        let fps = if avg_frame_ms > 0.0 {
            1000.0 / avg_frame_ms
        } else {
            0.0
        };

        let mut sorted = self.frame_ms.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let p95_idx = ((sorted.len() as f64 * 0.95).floor() as usize).min(sorted.len() - 1);
        let p95_ms = sorted[p95_idx];

        tracing::info!(
            "ChartView perf (5s): fps={:.1}, avg={:.2}ms, p95={:.2}ms, prep={:.2}ms, lock_wait={:.2}ms, render={:.2}ms, static={:.2}ms, overlay={:.2}ms, resize={}, lock_contention_frames={}",
            fps,
            avg_frame_ms,
            p95_ms,
            self.prep_ms_total / frames,
            self.lock_wait_ms_total / frames,
            self.render_ms_total / frames,
            self.static_ms_total / frames,
            self.overlay_ms_total / frames,
            self.resize_count,
            self.lock_contention_frames,
        );

        *self = Self::new();
    }
}

/// Chart Editor view — split editor with live WGPU chart preview.
///
/// Uses `ChartEditorLayout` from keyflow-ui for the Dioxus UI, and
/// renders the chart via `ChartLayoutManager` → `ChartGraphics` for WGPU.
#[component]
pub fn ChartView() -> Element {
    let graphics = consume_context::<Arc<Mutex<ChartGraphics>>>();

    // Enable transparent mode on mount
    use_effect(move || {
        document::eval(
            r#"
            document.documentElement.classList.add('transparent-mode');

            if (!window.__keyflowBoundsInit) {
                window.__keyflowBoundsInit = true;
                window.__keyflowBoundsData = null;

                const updateBounds = () => {
                    const el = document.getElementById('chart-editor-preview');
                    if (!el) {
                        window.__keyflowBoundsData = null;
                        return;
                    }
                    const rect = el.getBoundingClientRect();
                    const dpr = window.devicePixelRatio || 1;
                    window.__keyflowBoundsData = {
                        x: rect.x * dpr,
                        y: rect.y * dpr,
                        width: rect.width * dpr,
                        height: rect.height * dpr,
                        dpr: dpr
                    };
                };

                window.__keyflowUpdateBounds = updateBounds;
                window.addEventListener('resize', updateBounds, { passive: true });
                window.addEventListener('scroll', updateBounds, { passive: true });

                const observeWhenReady = () => {
                    const el = document.getElementById('chart-editor-preview');
                    if (!el) {
                        return false;
                    }
                    if (window.__keyflowBoundsObserver) {
                        window.__keyflowBoundsObserver.disconnect();
                    }
                    const observer = new ResizeObserver(updateBounds);
                    observer.observe(el);
                    window.__keyflowBoundsObserver = observer;
                    updateBounds();
                    return true;
                };

                if (!observeWhenReady()) {
                    const retry = setInterval(() => {
                        if (observeWhenReady()) {
                            clearInterval(retry);
                        }
                    }, 250);
                    window.__keyflowBoundsRetry = retry;
                }
            } else if (window.__keyflowUpdateBounds) {
                window.__keyflowUpdateBounds();
            }
            "#,
        );
    });

    // Cleanup: remove transparent mode when component unmounts (if no other chart visible)
    use_drop(move || {
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

    // Layout manager — created once, persists across renders.
    let layout_manager: Signal<Option<std::rc::Rc<std::cell::RefCell<ChartLayoutManager>>>> =
        use_signal(|| {
            ChartLayoutManager::new()
                .ok()
                .map(|m| std::rc::Rc::new(std::cell::RefCell::new(m)))
        });

    // Generation counter: bumped each time layout changes, so render effect re-fires.
    let mut layout_generation = use_signal(|| 0u64);
    let cursor_frame_clock = use_signal(|| 0u64);
    let cursor_motion =
        use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(PerfCursorMotionState::default())));
    let interaction_until = use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(Instant::now())));
    let last_viewport_state =
        use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(None::<(f64, f64, f64)>)));

    // 120Hz local ticker for smooth cursor interpolation while transport is playing.
    {
        let mut frame_clock = cursor_frame_clock;
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

    // Layout effect: re-layout when source or preview mode changes.
    // Parse and layout run on a background thread via spawn_blocking to
    // keep the UI thread (and the text editor) responsive.
    {
        let mut layout_task: Signal<Option<Task>> = use_signal(|| None);

        use_effect(move || {
            let source = CHART_SOURCE.read().clone();
            let preview_mode = *CHART_PREVIEW_MODE.read();
            let bounds = *CHART_EDITOR_BOUNDS.read();
            let viewport = *CHART_VIEWPORT.read();
            let layout_zoom = if preview_mode == PreviewMode::Responsive {
                viewport.zoom
            } else {
                1.0
            };

            if !bounds.is_valid() {
                return;
            }

            // Quick hash check on main thread — avoid spawning if nothing changed
            let (text_font, symbol_font) = if let Some(ref manager_rc) = *layout_manager.read() {
                let manager = manager_rc.borrow();
                if !manager.needs_layout_for_preview_mode(
                    &source,
                    preview_mode,
                    bounds.width,
                    layout_zoom,
                ) {
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

            // Spawn async task that runs parse+layout on a background thread
            let task = spawn(async move {
                let source_clone = source.clone();
                let viewport_width = bounds.width;

                let result = tokio::task::spawn_blocking(move || {
                    // Static MStyle — created once, reused across all background layouts
                    use std::sync::OnceLock;
                    static BG_STYLE: OnceLock<&'static MStyle> = OnceLock::new();
                    let style = *BG_STYLE.get_or_init(|| Box::leak(Box::new(MStyle::new())));

                    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);

                    let chart = keyflow::parse(&source_clone).map_err(|e| format!("{}", e))?;

                    let (mode, config) = crate::chart_renderer::layout_mode_for_preview(
                        preview_mode,
                        viewport_width,
                        layout_zoom,
                    );

                    let layout_result = engine.layout_chart_with_config(&chart, &mode, &config);
                    Ok::<_, String>((chart, layout_result))
                })
                .await;

                // Back on main thread — apply the result to the manager
                match result {
                    Ok(Ok((chart, layout_result))) => {
                        if let Some(ref manager_rc) = *layout_manager.read() {
                            let mut manager = manager_rc.borrow_mut();
                            manager.apply_precomputed_layout_with_preview_mode(
                                chart,
                                layout_result,
                                &source,
                                preview_mode,
                                bounds.width,
                                layout_zoom,
                            );

                            layout_generation.set(layout_generation() + 1);
                            tracing::info!(
                                "Chart layout updated (gen {}) [background]",
                                layout_generation()
                            );

                            let base_scale = manager.fit_to_width_scale(bounds.width, bounds.dpr);
                            *CHART_BASE_SCALE.write() = base_scale;

                            if let Some(metadata) = manager.page_metadata() {
                                let total = metadata.len() as u32;
                                let mut info = CHART_PAGE_INFO.write();
                                info.total_pages = total.max(1);
                                info.page_metadata = metadata;
                                if info.current_page > total {
                                    info.current_page = total.max(1);
                                }
                            }

                            // Apply FullPage zoom on initial layout
                            if preview_mode == PreviewMode::Page {
                                let vp = *CHART_VIEWPORT.peek();
                                if vp.zoom_level == SemanticZoomLevel::FullPage
                                    && (vp.zoom - 1.0).abs() < 0.01
                                    && vp.scroll_y.abs() < 0.01
                                {
                                    if let Some(page) =
                                        manager.layout_result().and_then(|r| r.pages.first())
                                    {
                                        if page.height > 0.0 && base_scale > 0.0 {
                                            let zoom = bounds.height / (page.height * base_scale);
                                            let mut vp = CHART_VIEWPORT.write();
                                            vp.zoom = zoom.clamp(0.1, 8.0);
                                            vp.zoom_level = SemanticZoomLevel::FullPage;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        tracing::info!("Chart parse error: {}", e);
                    }
                    Err(e) => {
                        tracing::warn!("Background layout task panicked: {}", e);
                    }
                }
            });

            layout_task.set(Some(task));
        });
    }

    // FPS tracking: sliding window of frame times.
    let fps_state = use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(FpsTracker::new())));
    let perf_window =
        use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(ChartViewPerfWindow::new())));

    // Render effect: re-renders when layout, viewport, or bounds change.
    {
        let graphics_clone = graphics.clone();
        let fps_tracker = fps_state.clone();
        let cursor_motion = cursor_motion.clone();
        let perf_window = perf_window.clone();
        let interaction_until = interaction_until.clone();
        let last_viewport_state = last_viewport_state.clone();
        use_effect(move || {
            let viewport = *CHART_VIEWPORT.read();
            let _gen = layout_generation();
            let _frame_clock = cursor_frame_clock();
            let active_song_index = ACTIVE_INDICES.peek().song_index;
            let playback_musical = *ACTIVE_PLAYBACK_MUSICAL.read();
            let playback_is_playing = *ACTIVE_PLAYBACK_IS_PLAYING.read();
            let current_cursor_tick = *CHART_CURSOR_TICK.read();

            let bounds = *CHART_EDITOR_BOUNDS.peek();

            if !bounds.is_valid() {
                return;
            }

            if let Some(ref manager_rc) = *layout_manager.read() {
                let mut manager = manager_rc.borrow_mut();
                if manager.layout_result().is_none() {
                    return;
                }

                let frame_start = std::time::Instant::now();
                let now = Instant::now();
                {
                    let mut last = last_viewport_state.borrow_mut();
                    let changed = match *last {
                        Some((sx, sy, zoom)) => {
                            (sx - viewport.scroll_x).abs() > 0.001
                                || (sy - viewport.scroll_y).abs() > 0.001
                                || (zoom - viewport.zoom).abs() > 0.0005
                        }
                        None => true,
                    };
                    if changed {
                        *interaction_until.borrow_mut() = now + Duration::from_millis(250);
                        *last = Some((viewport.scroll_x, viewport.scroll_y, viewport.zoom));
                    }
                }
                let interaction_active = now < *interaction_until.borrow();

                let base_scale = manager.fit_to_width_scale(bounds.width, bounds.dpr);
                let pad = 20.0 * bounds.dpr;
                let transform = Affine::translate((
                    pad - viewport.scroll_x * bounds.dpr,
                    pad - viewport.scroll_y * bounds.dpr,
                )) * Affine::scale(base_scale * viewport.zoom);

                if (*CHART_BASE_SCALE.peek() - base_scale).abs() > 0.001 {
                    *CHART_BASE_SCALE.write() = base_scale;
                }

                let current_page = manager.current_page_for_scroll(
                    viewport.scroll_x,
                    base_scale,
                    viewport.zoom,
                    bounds.dpr,
                );
                {
                    let info = CHART_PAGE_INFO.peek();
                    if info.current_page != current_page || info.zoom_level != viewport.zoom_level {
                        drop(info);
                        let mut info = CHART_PAGE_INFO.write();
                        info.current_page = current_page;
                        info.zoom_level = viewport.zoom_level;
                    }
                }

                let mut cursor_tick = current_cursor_tick;

                // Follow DAW transport while playback is running.
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

                    // Extrapolate cursor between transport packets for smooth motion.
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

                    if cursor_tick != current_cursor_tick {
                        *CHART_CURSOR_TICK.write() = cursor_tick;
                    }
                } else {
                    let mut motion = cursor_motion.borrow_mut();
                    motion.last_sample_tick = None;
                    motion.last_sample_time = None;
                    motion.velocity_ticks_per_sec = 0.0;
                }

                // Process click-to-position
                let pending_click = *CHART_CURSOR_SCENE_CLICK.peek();
                if let Some((scene_x, scene_y)) = pending_click {
                    *CHART_CURSOR_SCENE_CLICK.write() = None;
                    if let Some(tick) = manager.tick_at_scene_point(scene_x, scene_y) {
                        tracing::info!(
                            "Click-to-position: scene=({:.1},{:.1}) → tick={}",
                            scene_x,
                            scene_y,
                            tick
                        );
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

                let cursor_tick = if *CHART_CURSOR_VISIBLE.peek() {
                    Some(cursor_tick)
                } else {
                    let _ = *CHART_CURSOR_TICK.read();
                    None
                };

                let hover_point = *CHART_HOVER_SCENE_POINT.read();

                if let Some(tick) = cursor_tick {
                    let pos = manager.musical_position_for_tick(tick);
                    if *CHART_CURSOR_POSITION.peek() != pos {
                        *CHART_CURSOR_POSITION.write() = pos;
                    }
                }

                let prep_ms = frame_start.elapsed().as_secs_f64() * 1000.0;
                let lock_wait_start = std::time::Instant::now();
                let mut lock_wait_ms = 0.0;
                let mut render_ms = 0.0;
                let mut static_ms = 0.0;
                let mut overlay_ms = 0.0;
                let mut did_resize = false;

                if let Ok(mut gfx) = graphics_clone.lock() {
                    lock_wait_ms = lock_wait_start.elapsed().as_secs_f64() * 1000.0;
                    gfx.set_interaction_active(interaction_active);
                    let win_size = dioxus::desktop::window().window.inner_size();
                    let (sw, sh) = gfx.size();
                    if sw != win_size.width || sh != win_size.height {
                        tracing::debug!(
                            "Surface resize: {}x{} -> {}x{}",
                            sw,
                            sh,
                            win_size.width,
                            win_size.height
                        );
                        gfx.resize(win_size.width, win_size.height);
                        did_resize = true;
                    }

                    let dock_offset = Affine::translate((bounds.x, bounds.y));
                    let render_start = std::time::Instant::now();
                    gfx.render_chart(|painter| {
                        let static_start = std::time::Instant::now();
                        manager.render_static_layer_to_scene(
                            painter,
                            bounds.width,
                            bounds.height,
                            dock_offset,
                            transform,
                        );
                        static_ms = static_start.elapsed().as_secs_f64() * 1000.0;

                        let overlay_start = std::time::Instant::now();
                        manager.render_overlay_layer_to_scene(
                            painter,
                            bounds.width,
                            bounds.height,
                            dock_offset,
                            transform,
                            cursor_tick,
                            hover_point,
                        );
                        overlay_ms = overlay_start.elapsed().as_secs_f64() * 1000.0;
                    });
                    render_ms = render_start.elapsed().as_secs_f64() * 1000.0;
                }
                dioxus::desktop::window().window.request_redraw();

                let frame_time_us = frame_start.elapsed().as_micros() as u64;
                fps_tracker.borrow_mut().add_sample(frame_time_us);

                let frame_ms = frame_start.elapsed().as_secs_f64() * 1000.0;
                let mut perf = perf_window.borrow_mut();
                perf.record(
                    frame_ms,
                    prep_ms,
                    lock_wait_ms,
                    render_ms,
                    static_ms,
                    overlay_ms,
                    did_resize,
                );
                perf.maybe_flush_log();
            }
        });
    }

    // FPS display update: periodic, decoupled from render.
    {
        let fps_tracker_for_display = fps_state.clone();
        use_future(move || {
            let tracker = fps_tracker_for_display.clone();
            async move {
                loop {
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                    let stats = tracker.borrow().snapshot();
                    *CHART_RENDER_STATS.write() = stats;
                }
            }
        });
    }

    // Bounds polling: continuously query chart-editor-preview position
    {
        use_future(move || async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

                let result = document::eval(
                    r#"
                        return JSON.stringify(window.__keyflowBoundsData || null);
                    "#,
                );

                match result.await {
                    Ok(value) => {
                        let json_str = value
                            .as_str()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| value.to_string());

                        if json_str != "null" && json_str != "\"null\"" {
                            match serde_json::from_str::<serde_json::Value>(&json_str) {
                                Ok(parsed) => {
                                    let x = parsed["x"].as_f64().unwrap_or(0.0);
                                    let y = parsed["y"].as_f64().unwrap_or(0.0);
                                    let width = parsed["width"].as_f64().unwrap_or(0.0);
                                    let height = parsed["height"].as_f64().unwrap_or(0.0);
                                    let dpr = parsed["dpr"].as_f64().unwrap_or(1.0);

                                    if width > 0.0 && height > 0.0 {
                                        let current = *CHART_EDITOR_BOUNDS.read();
                                        if (current.x - x).abs() > 1.0
                                            || (current.y - y).abs() > 1.0
                                            || (current.width - width).abs() > 1.0
                                            || (current.height - height).abs() > 1.0
                                        {
                                            tracing::info!(
                                                "Chart editor bounds updated: ({:.0}, {:.0}, {:.0}x{:.0}), dpr={:.2}",
                                                x,
                                                y,
                                                width,
                                                height,
                                                dpr
                                            );
                                            *CHART_EDITOR_BOUNDS.write() =
                                                ChartEditorBounds::new(x, y, width, height, dpr);
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("Failed to parse bounds JSON: {}", e);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        if !format!("{:?}", e).contains("Finished") {
                            tracing::debug!("Chart editor bounds eval: {:?}", e);
                        }
                    }
                }
            }
        });
    }

    rsx! {
        div {
            class: "w-full h-full",
            style: "background: transparent !important;",
            ChartEditorLayout {}
        }
    }
}
