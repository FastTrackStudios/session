//! Chart Editor Layout
//!
//! Split-view editor with a keyflow text editor on the left and a live
//! WGPU chart preview on the right. The preview area is transparent so
//! the WGPU surface (rendered by the app's ChartGraphics) shows through.

use crate::catalog;
use crate::components::HighlightedEditor;
use crate::examples::{self, EXAMPLES};
use crate::prelude::*;
use crate::signals::*;

/// Navigate the viewport to show a specific page.
///
/// Pages are laid out **side by side** horizontally. Computes the scroll_x
/// that places the target page's left edge at the viewport left edge.
fn navigate_to_page(page_number: u32) {
    let info = CHART_PAGE_INFO.read();
    let Some(meta) = info.page_metadata.iter().find(|m| m.number == page_number) else {
        return;
    };
    let x_offset = meta.x_offset;
    drop(info);
    let bounds = *CHART_EDITOR_BOUNDS.read();
    let viewport = *CHART_VIEWPORT.read();
    let base_scale = *CHART_BASE_SCALE.read();
    if base_scale <= 0.0 || bounds.dpr <= 0.0 {
        return;
    }
    // scroll_x is in CSS pixels. Transform: screen_x = pad - scroll_x*dpr + scene_x*base_scale*zoom
    // To place page left at viewport left: scroll_x = x_offset * base_scale * zoom / dpr
    let scroll_x = x_offset * base_scale * viewport.zoom / bounds.dpr;
    tracing::info!(
        "navigate_to_page({}): x_offset={:.1} base_scale={:.4} zoom={:.3} dpr={:.2} → scroll_x={:.1}",
        page_number,
        x_offset,
        base_scale,
        viewport.zoom,
        bounds.dpr,
        scroll_x
    );
    let mut vp = CHART_VIEWPORT.write();
    vp.scroll_x = scroll_x;
    vp.scroll_y = 0.0; // Reset vertical scroll when turning pages
}

/// Apply a semantic zoom preset to the viewport.
///
/// Computes the zoom factor that fits the requested vertical content amount
/// in the viewport. Also scrolls horizontally to the current page.
fn apply_semantic_zoom(level: SemanticZoomLevel) {
    let info = CHART_PAGE_INFO.read();
    let bounds = *CHART_EDITOR_BOUNDS.read();
    let base_scale = *CHART_BASE_SCALE.read();
    if base_scale <= 0.0 || bounds.dpr <= 0.0 || bounds.height <= 0.0 {
        return;
    }

    let current_page = info.current_page;
    let Some(page_meta) = info.page_metadata.iter().find(|m| m.number == current_page) else {
        return;
    };

    let (zoom, scroll_y) =
        compute_zoom_for_level(level, page_meta, base_scale, bounds.height, bounds.dpr);
    // Also compute scroll_x to keep current page centered
    let scroll_x = page_meta.x_offset * base_scale * zoom / bounds.dpr;
    tracing::info!(
        "apply_semantic_zoom({:?}): page={} base_scale={:.4} viewport_h={:.0} dpr={:.2} → zoom={:.3} scroll_x={:.1} scroll_y={:.1}",
        level,
        current_page,
        base_scale,
        bounds.height,
        bounds.dpr,
        zoom,
        scroll_x,
        scroll_y
    );
    let mut vp = CHART_VIEWPORT.write();
    vp.zoom = zoom.clamp(0.1, 8.0);
    vp.scroll_x = scroll_x;
    vp.scroll_y = scroll_y;
    vp.zoom_level = level;
}

/// Pure function: compute (zoom, scroll_y) for a semantic zoom level.
///
/// The zoom controls vertical fit — how many systems are visible.
/// The returned scroll_y positions the viewport at the page's y_offset
/// (which is typically a constant 20.0 since pages are side-by-side).
fn compute_zoom_for_level(
    level: SemanticZoomLevel,
    page: &PageMeta,
    base_scale: f64,
    viewport_height: f64,
    dpr: f64,
) -> (f64, f64) {
    let zoom = match level {
        SemanticZoomLevel::FullPage => viewport_height / (page.height * base_scale),
        SemanticZoomLevel::HalfPage => viewport_height / (page.height * 0.5 * base_scale),
        SemanticZoomLevel::SystemView => {
            if page.systems.is_empty() {
                return (1.0, 0.0);
            }
            let n = 3.min(page.systems.len());
            let first = &page.systems[0];
            let last = &page.systems[n - 1];
            let h = (last.y + last.height) - first.y + 40.0; // 40pt breathing room
            viewport_height / (h * base_scale)
        }
        SemanticZoomLevel::LineView => {
            if page.systems.is_empty() {
                return (1.0, 0.0);
            }
            let n = 2.min(page.systems.len());
            let first = &page.systems[0];
            let last = &page.systems[n - 1];
            let h = (last.y + last.height) - first.y + 20.0; // 20pt breathing room
            viewport_height / (h * base_scale)
        }
        SemanticZoomLevel::Custom => return (1.0, 0.0),
    };
    // scroll_y positions at the page's y_offset (constant for horizontal layout)
    let scroll_y = page.y_offset * base_scale * zoom / dpr;
    (zoom, scroll_y)
}

/// Chart editor split-view layout.
///
/// Left panel: syntax-highlighted text editor with examples dropdown.
/// Right panel: transparent area for WGPU chart rendering + mode toggle.
///
/// The consuming app is responsible for:
/// 1. Watching `CHART_SOURCE` signal changes
/// 2. Parsing, laying out, and rendering the chart via `ChartLayoutManager`
/// 3. Rendering the resulting `vello::Scene` to `ChartGraphics` at `CHART_EDITOR_BOUNDS`
#[component]
pub fn ChartEditorLayout() -> Element {
    // Wrap the whole editor in `ThemeProvider` so theme tokens
    // (`--destructive`, `--warning`, etc.) are defined for child CSS,
    // and so `fts-ui` components pick up the active preset.
    // `ToastProvider` lets descendants call `use_toast()`.
    // Default to Dark to match the host app's dark `:root` theme — the bare
    // `default_theme_state` is Light, which renders the whole editor white.
    let theme_state = use_signal(|| ThemeState::new(default_theme_preset(), ThemeMode::Dark));
    rsx! {
        // `h-full` so the editor fills the host pane: ThemeProvider's own div is
        // only `min-h-full`, which doesn't give `ChartEditorLayoutInner`'s
        // `h-full` a definite parent height (preview collapses to ~29px otherwise).
        ThemeProvider {
            state: theme_state,
            class: "h-full",
            toast::ToastProvider { ChartEditorLayoutInner {} }
        }
    }
}

#[component]
fn ChartEditorLayoutInner() -> Element {
    let source = CHART_SOURCE.read().clone();
    let preview_mode = *CHART_PREVIEW_MODE.read();

    // Dropdown open state
    let mut examples_open = use_signal(|| false);
    let mut catalog_open = use_signal(|| false);

    // Currently selected example index
    let mut selected_example = use_signal(|| 1usize); // Default: Thriller
    let catalog_entries = use_memo(catalog::local_musicxml_catalog);

    // Parse error state
    let parse_error = use_memo(move || {
        let source = CHART_SOURCE.read().clone();
        keyflow::text::chart::parse_chart(&source).err()
    });

    rsx! {
        div {
            class: "flex h-full w-full text-foreground",
            style: "background: transparent !important;",

            // Click anywhere to close dropdown
            onclick: move |_| {
                if *examples_open.read() {
                    examples_open.set(false);
                }
                if *catalog_open.read() {
                    catalog_open.set(false);
                }
            },

            // Left panel: Editor
            div {
                class: "flex flex-col w-1/2 border-r border-border bg-card",

                // Header bar
                div {
                    class: "flex items-center justify-between px-3 py-2 border-b border-border bg-card",

                    div {
                        class: "flex items-center gap-2",
                        span { class: "text-sm font-medium text-foreground", "Keyflow Source" }
                    }

                    div {
                        class: "flex items-center gap-2",

                        // Examples dropdown — fts-ui primitive-backed
                        // (handles open/close + keyboard nav + focus
                        // management; we just feed in items).
                        Dropdown {
                            open: *examples_open.read(),
                            on_open_change: Callback::new(move |open: bool| {
                                examples_open.set(open);
                            }),
                            DropdownTrigger {
                                Button {
                                    variant: ButtonVariant::Secondary,
                                    size: ButtonSize::Small,
                                    span { "{EXAMPLES[*selected_example.read()].name}" }
                                    fts_ui::lucide_dioxus::ChevronDown { size: 12 }
                                }
                            }
                            DropdownContent {
                                width: "w-48".to_string(),
                                for (i, example) in EXAMPLES.iter().enumerate() {
                                    DropdownItem {
                                        key: "{i}",
                                        value: example.name.to_string(),
                                        index: i,
                                        on_select: Callback::new(move |_: String| {
                                            selected_example.set(i);
                                            if let Some(example) = EXAMPLES.get(i) {
                                                *CHART_SOURCE.write() =
                                                    example.source.to_string();
                                            }
                                            examples_open.set(false);
                                        }),
                                        "{example.name}"
                                    }
                                }
                            }
                        }

                        Dropdown {
                            open: *catalog_open.read(),
                            on_open_change: Callback::new(move |open: bool| {
                                catalog_open.set(open);
                            }),
                            DropdownTrigger {
                                Button {
                                    variant: ButtonVariant::Secondary,
                                    size: ButtonSize::Small,
                                    span { "Catalog" }
                                    fts_ui::lucide_dioxus::ChevronDown { size: 12 }
                                }
                            }
                            DropdownContent {
                                width: "w-72".to_string(),
                                if catalog_entries.read().is_empty() {
                                    DropdownItem {
                                        value: "empty".to_string(),
                                        index: 0usize,
                                        on_select: Callback::new(move |_: String| {
                                            catalog_open.set(false);
                                        }),
                                        "No local MusicXML catalog"
                                    }
                                } else {
                                    for (i, entry) in catalog_entries.read().iter().take(80).enumerate() {
                                        DropdownItem {
                                            key: "{entry.path}",
                                            value: entry.path.clone(),
                                            index: i,
                                            on_select: Callback::new({
                                                let path = entry.path.clone();
                                                move |_: String| {
                                                    match catalog::load_musicxml_catalog_chart(&path) {
                                                        Ok(source) => {
                                                            *CHART_SOURCE.write() = source;
                                                            *CHART_PREVIEW_MODE.write() = PreviewMode::Responsive;
                                                        }
                                                        Err(err) => {
                                                            *CHART_SOURCE.write() = format!(
                                                                "Catalog Import Error\n120bpm 4/4 #C\n\nERR\n// {}\n",
                                                                err.replace('\n', " ")
                                                            );
                                                        }
                                                    }
                                                    catalog_open.set(false);
                                                }
                                            }),
                                            if let Some(composer) = &entry.composer {
                                                "{entry.title} - {composer}"
                                            } else {
                                                "{entry.title}"
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Reset button
                        Button {
                            variant: ButtonVariant::Ghost,
                            size: ButtonSize::Small,
                            on_click: Callback::new(move |_| {
                                *CHART_SOURCE.write() = examples::DEFAULT_CHART.to_string();
                                selected_example.set(1);
                            }),
                            "Reset"
                        }
                    }
                }

                // Editor area
                HighlightedEditor {
                    source: source.clone(),
                    on_change: move |new_source: String| {
                        *CHART_SOURCE.write() = new_source;
                    }
                }
            }

            // Right panel: Preview (transparent for WGPU to show through)
            div {
                class: "flex flex-col w-1/2 chart-transparent-area",
                style: "background: transparent !important; background-color: transparent !important;",

                // Header bar
                div {
                    class: "flex items-center justify-between px-3 py-2 border-b border-border bg-card",

                    // Left group: label + FPS + error + page nav
                    div {
                        class: "flex items-center gap-2",
                        span { class: "text-sm font-medium text-foreground", "Live Preview" }

                        // FPS counter
                        {
                            let stats = *CHART_RENDER_STATS.read();
                            // Theme-token colors: success/warning/destructive
                            // resolve through ThemeProvider so light/dark
                            // and custom themes Just Work.
                            let fps_color = if stats.fps >= 55.0 {
                                "text-success"
                            } else if stats.fps >= 25.0 {
                                "text-warning"
                            } else {
                                "text-destructive"
                            };
                            rsx! {
                                span {
                                    class: "text-xs font-mono {fps_color}",
                                    "{stats.fps:.0} fps  {stats.frame_time_ms:.1}ms"
                                }
                            }
                        }

                        // Musical position display
                        {
                            let cursor_pos = CHART_CURSOR_POSITION.read();
                            rsx! {
                                span {
                                    class: "text-xs font-mono text-info select-none",
                                    title: "Musical position (Measure.Beat.Ticks)",
                                    "{cursor_pos}"
                                }
                            }
                        }

                        // Parse error indicator
                        if let Some(error) = &*parse_error.read() {
                            span {
                                class: "flex items-center gap-1 text-xs text-destructive truncate max-w-xs",
                                title: "{error}",
                                // Warning icon
                                svg {
                                    width: "14",
                                    height: "14",
                                    view_box: "0 0 24 24",
                                    fill: "none",
                                    stroke: "currentColor",
                                    stroke_width: "2",
                                    stroke_linecap: "round",
                                    stroke_linejoin: "round",
                                    path { d: "m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3" }
                                    path { d: "M12 9v4" }
                                    path { d: "M12 17h.01" }
                                }
                                "Parse error"
                            }
                        }

                        // Page navigation (Page mode only)
                        if preview_mode == PreviewMode::Page {
                            {
                                let page_info = CHART_PAGE_INFO.read();
                                let current = page_info.current_page;
                                let total = page_info.total_pages;
                                rsx! {
                                    div {
                                        class: "flex items-center gap-0.5 ml-1",

                                        // Previous page
                                        Button {
                                            variant: ButtonVariant::Ghost,
                                            size: ButtonSize::Small,
                                            disabled: current <= 1,
                                            on_click: Callback::new(move |_| {
                                                let cp = CHART_PAGE_INFO.read().current_page;
                                                if cp > 1 {
                                                    navigate_to_page(cp - 1);
                                                }
                                            }),
                                            fts_ui::lucide_dioxus::ChevronLeft { size: 14 }
                                        }

                                        // Page indicator
                                        span {
                                            class: "text-xs font-mono text-muted-foreground select-none",
                                            "{current}/{total}"
                                        }

                                        // Next page
                                        Button {
                                            variant: ButtonVariant::Ghost,
                                            size: ButtonSize::Small,
                                            disabled: current >= total,
                                            on_click: Callback::new(move |_| {
                                                let info = CHART_PAGE_INFO.read();
                                                if info.current_page < info.total_pages {
                                                    let next = info.current_page + 1;
                                                    drop(info);
                                                    navigate_to_page(next);
                                                }
                                            }),
                                            fts_ui::lucide_dioxus::ChevronRight { size: 14 }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Right group: zoom presets + mode toggle
                    div {
                        class: "flex items-center gap-2",

                        // Semantic zoom presets (Page mode only)
                        if preview_mode == PreviewMode::Page {
                            {
                                let page_info = CHART_PAGE_INFO.read();
                                let current_level = page_info.zoom_level;
                                let zoom_value = match current_level {
                                    SemanticZoomLevel::FullPage => "page",
                                    SemanticZoomLevel::HalfPage => "half",
                                    SemanticZoomLevel::SystemView => "3line",
                                    SemanticZoomLevel::LineView => "2line",
                                    _ => "page",
                                }
                                .to_string();
                                rsx! {
                                    SegmentedControl {
                                        size: SegmentedControlSize::Small,
                                        value: zoom_value,
                                        options: vec![
                                            ("page".into(), "Page".into()),
                                            ("half".into(), "Half".into()),
                                            ("3line".into(), "3-Line".into()),
                                            ("2line".into(), "2-Line".into()),
                                        ],
                                        on_change: Callback::new(move |v: String| {
                                            let level = match v.as_str() {
                                                "page" => SemanticZoomLevel::FullPage,
                                                "half" => SemanticZoomLevel::HalfPage,
                                                "3line" => SemanticZoomLevel::SystemView,
                                                "2line" => SemanticZoomLevel::LineView,
                                                _ => SemanticZoomLevel::FullPage,
                                            };
                                            apply_semantic_zoom(level);
                                        }),
                                    }
                                }
                            }
                        }

                        // Preview mode toggle (segmented control)
                        SegmentedControl {
                            value: match preview_mode {
                                PreviewMode::Snippet => "snippet".to_string(),
                                PreviewMode::Page => "page".to_string(),
                                PreviewMode::Responsive => "responsive".to_string(),
                            },
                            options: vec![
                                ("snippet".into(), "Snippet".into()),
                                ("page".into(), "Letter".into()),
                                ("responsive".into(), "iReal".into()),
                            ],
                            on_change: Callback::new(move |v: String| {
                                *CHART_PREVIEW_MODE.write() = match v.as_str() {
                                    "page" => PreviewMode::Page,
                                    "responsive" => PreviewMode::Responsive,
                                    _ => PreviewMode::Snippet,
                                };
                            }),
                        }
                    }
                }

                // Transparent preview area — WGPU renders behind this
                // Captures mouse events and forwards them as viewport transforms
                {
                    let mut dragging = use_signal(|| false);
                    let mut last_mouse = use_signal(|| (0.0f64, 0.0f64));
                    // Track mouse-down position to distinguish click from drag
                    let mut mouse_down_pos = use_signal(|| (0.0f64, 0.0f64));

                    rsx! {
                        div {
                            id: "chart-editor-preview",
                            class: "flex-1 cursor-grab",
                            style: "background: transparent !important;",

                            // Wheel → zoom (marks as Custom to deselect presets)
                            onwheel: move |evt| {
                                let delta_y = evt.delta().strip_units().y;
                                let mut vp = CHART_VIEWPORT.write();
                                let zoom_factor = if delta_y < 0.0 { 1.05 } else { 0.95 };
                                vp.zoom = (vp.zoom * zoom_factor).clamp(0.1, 8.0);
                                vp.zoom_level = SemanticZoomLevel::Custom;
                            },

                            // Mouse drag → pan, click → set cursor
                            onmousedown: move |evt| {
                                dragging.set(true);
                                let coords = evt.client_coordinates();
                                last_mouse.set((coords.x, coords.y));
                                mouse_down_pos.set((coords.x, coords.y));
                            },

                            onmousemove: move |evt| {
                                let coords = evt.client_coordinates();

                                if *dragging.read() {
                                    let (lx, ly) = *last_mouse.read();
                                    let dx = coords.x - lx;
                                    let dy = coords.y - ly;

                                    let mut vp = CHART_VIEWPORT.write();
                                    vp.scroll_x -= dx;
                                    vp.scroll_y -= dy;

                                    last_mouse.set((coords.x, coords.y));
                                    // Clear hover during drag
                                    *CHART_HOVER_SCENE_POINT.write() = None;
                                } else {
                                    // Not dragging — compute hover point in scene coordinates
                                    let bounds = *CHART_EDITOR_BOUNDS.peek();
                                    let viewport = *CHART_VIEWPORT.peek();
                                    let base_scale = *CHART_BASE_SCALE.peek();

                                    if base_scale > 0.0 && bounds.dpr > 0.0 {
                                        let scale = base_scale * viewport.zoom;
                                        let pad = 20.0 * bounds.dpr;
                                        let px_x = coords.x * bounds.dpr - bounds.x;
                                        let px_y = coords.y * bounds.dpr - bounds.y;
                                        let scene_x = (px_x - pad + viewport.scroll_x * bounds.dpr) / scale;
                                        let scene_y = (px_y - pad + viewport.scroll_y * bounds.dpr) / scale;
                                        *CHART_HOVER_SCENE_POINT.write() = Some((scene_x, scene_y));
                                    }
                                }
                            },

                            onmouseup: move |evt| {
                                if *dragging.read() {
                                    dragging.set(false);

                                    // Distinguish click from drag: if mouse barely moved, treat as click
                                    let coords = evt.client_coordinates();
                                    let (dx, dy) = *mouse_down_pos.read();
                                    let distance = ((coords.x - dx).powi(2) + (coords.y - dy).powi(2)).sqrt();

                                    if distance < 5.0 {
                                        // Click → set cursor position
                                        // Convert CSS click coords to scene coordinates
                                        let bounds = *CHART_EDITOR_BOUNDS.peek();
                                        let viewport = *CHART_VIEWPORT.peek();
                                        let base_scale = *CHART_BASE_SCALE.peek();

                                        if base_scale > 0.0 && bounds.dpr > 0.0 {
                                            let scale = base_scale * viewport.zoom;
                                            let pad = 20.0 * bounds.dpr;

                                            // CSS click coords → physical pixel coords relative to preview area
                                            let px_x = coords.x * bounds.dpr - bounds.x;
                                            let px_y = coords.y * bounds.dpr - bounds.y;

                                            // Invert the render transform: screen = pad - scroll*dpr + scene*scale
                                            // scene = (screen - pad + scroll*dpr) / scale
                                            let scene_x = (px_x - pad + viewport.scroll_x * bounds.dpr) / scale;
                                            let scene_y = (px_y - pad + viewport.scroll_y * bounds.dpr) / scale;

                                            tracing::info!(
                                                "Click-to-position: css=({:.0},{:.0}) scene=({:.1},{:.1})",
                                                coords.x, coords.y, scene_x, scene_y
                                            );

                                            // Write to cursor tick signal — render effect will pick it up
                                            // We defer the actual tick lookup to the render pipeline
                                            // via a separate signal for the scene point
                                            *CHART_CURSOR_SCENE_CLICK.write() = Some((scene_x, scene_y));
                                        }
                                    }
                                }
                            },

                            onmouseleave: move |_| {
                                dragging.set(false);
                                *CHART_HOVER_SCENE_POINT.write() = None;
                            },
                        }
                    }
                }
            }
        }
    }
}
