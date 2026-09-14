//! The arrangement, drawn rather than built.
//!
//! Every DAW draws its timeline; none of them builds one element per
//! take. This does the same: one canvas, the items as `fillRect`s, and
//! hit testing we own.
//!
//! # Why, precisely
//!
//! Not for the element count — that was measured and it is not the
//! problem. A framework-free page holds 62 fps with 910 positioned divs
//! on this exact surface, and the window holds 62 with the whole track
//! panel mounted. What cost the frames was the grid: two stacked
//! `repeating-linear-gradient`s across a 24,000 px-wide element, which
//! WebKit repaints on every scroll frame (8.7 fps with them, 61.7
//! without).
//!
//! The reason is the interaction model. This window needs modifier-aware
//! behaviour that differs across an item's body, its edges, its fades
//! and the lane behind it — hit testing we write ourselves either way.
//! The DOM was charging us for hit testing we are going to replace, and
//! the grid comes back here as two `fillRect` loops over the bars
//! actually on screen.
//!
//! # Who owns what
//!
//! Rust owns the scene and publishes it when it CHANGES — a project
//! loading, a folder opening. The page owns the scroll offset, the zoom
//! and the drawing, which are per-frame concerns. Nothing about a pan or
//! a zoom reaches Rust. That is the same split the playhead clock uses,
//! for the same reason.
//!
//! # Where it sits
//!
//! Outside the scroller, as an overlay positioned from the scroller's
//! own geometry, and sized to the VIEWPORT. A canvas the size of the
//! content would be 24,000 px wide and mostly unseen. The lanes element
//! stays in the DOM at full size so the scrollbars keep telling the
//! truth, but it draws nothing.
//!
//! # Blitz
//!
//! The architecture ports; the element may not. Blitz's `<canvas>`
//! support is limited, so the native path is a custom-painted widget
//! drawing this same scene — which is exactly what
//! `expression_editor_ui::stack::paint` already does. Scene data, a draw
//! pass and our own hit testing is the portable part.

use crate::prelude::*;

use super::{ProjectRef, RowsRef};

/// Item geometry, flattened for the page.
///
/// Flat arrays rather than a list of objects: this crosses as JSON text
/// inside a script, and eight hundred `{"start":…,"len":…}` objects is
/// several times the bytes of four arrays of numbers — for data whose
/// only consumer is a loop.
struct Scene {
    starts: Vec<f64>,
    lengths: Vec<f64>,
    rows: Vec<usize>,
    colors: Vec<String>,
    muted: Vec<bool>,
}

impl Scene {
    fn build(theme: &crate::theming::Theme, project: &ProjectRef, rows: &RowsRef) -> Self {
        let mut scene = Self {
            starts: Vec::new(),
            lengths: Vec::new(),
            rows: Vec::new(),
            colors: Vec::new(),
            muted: Vec::new(),
        };
        for (row, (track, _)) in rows.iter().enumerate() {
            let fallback = super::theme::track_color(theme, track.color);
            for item in project.lane(&track.guid) {
                scene.starts.push(item.position.as_seconds());
                scene.lengths.push(item.length.as_seconds());
                scene.rows.push(row);
                scene.colors.push(
                    item.color
                        .map_or_else(|| fallback.clone(), |c| format!("#{c:06x}")),
                );
                scene.muted.push(item.muted);
            }
        }
        scene
    }

    /// The scene as a JSON literal, ready to paste into a script.
    fn to_json(&self) -> String {
        let nums = |v: &[f64]| {
            v.iter()
                .map(|n| format!("{n:.4}"))
                .collect::<Vec<_>>()
                .join(",")
        };
        format!(
            "{{\"starts\":[{}],\"lengths\":[{}],\"rows\":[{}],\"colors\":[{}],\"muted\":[{}]}}",
            nums(&self.starts),
            nums(&self.lengths),
            self.rows
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(","),
            self.colors
                .iter()
                .map(|s| format!("\"{s}\""))
                .collect::<Vec<_>>()
                .join(","),
            self.muted
                .iter()
                .map(|b| i32::from(*b).to_string())
                .collect::<Vec<_>>()
                .join(","),
        )
    }
}

/// The canvas, and the scene it draws.
#[component]
pub fn LaneCanvas(project: ProjectRef, rows: RowsRef) -> Element {
    let theme = use_theme_for_scene();
    // Installed once; the page keeps drawing whatever scene it last had.
    use_hook(|| document::eval(RENDERER));

    use_effect(use_reactive((&project, &rows), move |(project, rows)| {
        let json = Scene::build(&theme, &project, &rows).to_json();
        let count = rows.len();
        document::eval(&format!("window.__ftsLanes?.scene({json},{count});"));
    }));

    rsx! { canvas { class: "studio-canvas", "data-testid": "studio-canvas" } }
}

/// The theme the scene's fallback colours come from, resolved once.
fn use_theme_for_scene() -> crate::theming::Theme {
    crate::theming::use_theme().theme
}

/// The page-side renderer.
///
/// Positions itself from the scroller's own geometry and reads the
/// scroll offset directly, so a pan costs one `requestAnimationFrame`
/// and no round trip. Redraws only when the view actually moved — a
/// canvas repainting a still picture sixty times a second is the same
/// waste as a component re-rendering one.
const RENDERER: &str = r"
const st = { scene: null, rows: 0, key: '', geom: null, stale: true };

window.__ftsLanes = {
  scene(data, rows) { st.scene = data; st.rows = rows; st.key = ''; st.stale = true; },
};

// Anything that changes the LAYOUT invalidates the cached geometry. A
// scroll does not — which is the whole point.
addEventListener('resize', () => { st.stale = true; });

const cssNum = (css, name, fallback) => {
  const v = parseFloat(css.getPropertyValue(name));
  return Number.isNaN(v) ? fallback : v;
};

// Reading computed style or a bounding rect forces the engine to resolve
// style and layout. Doing that once a frame — which the first version of
// this did, before its own dirty check — costs more than everything it
// was drawing: 31 fps against the 62 the DOM managed once its gradient
// was gone. So the geometry is CACHED and refreshed only when something
// that is not a scroll has changed.
const geometry = (root, pane, lanes, canvas) => {
  const css = getComputedStyle(root);
  const paneRect = pane.getBoundingClientRect();
  const laneRect = lanes.getBoundingClientRect();
  const rootRect = root.getBoundingClientRect();
  const left = Math.max(paneRect.left, laneRect.left) - rootRect.left;
  const top = Math.max(paneRect.top, laneRect.top) - rootRect.top;
  const g = {
    left, top,
    w: Math.max(0, paneRect.right - rootRect.left - left),
    h: Math.max(0, paneRect.bottom - rootRect.top - top),
    // Where the lanes' origin sits relative to the overlay, before the
    // scroll offset is applied.
    ox: (laneRect.left - rootRect.left - left) + pane.scrollLeft,
    oy: (laneRect.top - rootRect.top - top) + pane.scrollTop,
    pitch: cssNum(css, '--row-pitch', 71),
    scale: cssNum(css, '--pps', 40),
    secsPerBar: cssNum(css, '--secs-per-bar', 2),
    dpr: window.devicePixelRatio || 1,
    rowA: css.getPropertyValue('--row-bg-a').trim() || '#1b1b1f',
    rowB: css.getPropertyValue('--row-bg-b').trim() || '#1e1e23',
    divider: css.getPropertyValue('--row-divider').trim() || '#2e2e35',
    measure: css.getPropertyValue('--grid-measure').trim() || '#2e2e35',
  };
  canvas.style.left = g.left + 'px';
  canvas.style.top = g.top + 'px';
  canvas.style.width = g.w + 'px';
  canvas.style.height = g.h + 'px';
  const bw = Math.round(g.w * g.dpr), bh = Math.round(g.h * g.dpr);
  if (canvas.width !== bw) canvas.width = bw;
  if (canvas.height !== bh) canvas.height = bh;
  return g;
};

const draw = () => {
  requestAnimationFrame(draw);
  const canvas = document.querySelector('.studio-canvas');
  const pane = document.querySelector('.studio-scroll');
  const root = document.querySelector('.studio');
  const lanes = document.querySelector('.studio-lanes');
  if (!canvas || !pane || !root || !lanes || !st.scene) return;

  // The only per-frame DOM reads. Both are cheap in a steady state, and
  // neither forces a style resolve.
  const sx = pane.scrollLeft, sy = pane.scrollTop;

  if (st.stale || !st.geom) { st.geom = geometry(root, pane, lanes, canvas); st.stale = false; st.key = ''; }
  const g = st.geom;

  const key = sx + ':' + sy;
  if (key === st.key) return;
  st.key = key;

  const ctx = canvas.getContext('2d');
  ctx.setTransform(g.dpr, 0, 0, g.dpr, 0, 0);
  ctx.clearRect(0, 0, g.w, g.h);

  const ox = g.ox - sx, oy = g.oy - sy;

  // Lane stripes, for the rows in view only.
  const first = Math.max(0, Math.floor(-oy / g.pitch));
  const last = Math.min(st.rows, Math.ceil((-oy + g.h) / g.pitch));
  for (let r = first; r < last; r++) {
    const y = oy + r * g.pitch;
    ctx.fillStyle = (r % 2 === 0) ? g.rowA : g.rowB;
    ctx.fillRect(0, y, g.w, g.pitch - 1);
    ctx.fillStyle = g.divider;
    ctx.fillRect(0, y + g.pitch - 1, g.w, 1);
  }

  // The grid: only the bars on screen. This is what a CSS gradient was
  // repainting across 24,000px every frame.
  const barPx = g.secsPerBar * g.scale;
  if (barPx > 3) {
    ctx.fillStyle = g.measure;
    for (let b = Math.max(0, Math.floor(-ox / barPx)); b * barPx + ox < g.w; b++) {
      ctx.fillRect(Math.round(ox + b * barPx), 0, 1, g.h);
    }
  }

  // Items, culled to the viewport before anything is drawn.
  const s = st.scene;
  for (let i = 0; i < s.starts.length; i++) {
    const y = oy + s.rows[i] * g.pitch;
    if (y + g.pitch < 0 || y > g.h) continue;
    const x = ox + s.starts[i] * g.scale;
    const iw = Math.max(1, s.lengths[i] * g.scale);
    if (x + iw < 0 || x > g.w) continue;
    ctx.globalAlpha = s.muted[i] ? 0.4 : 1;
    ctx.fillStyle = s.colors[i];
    ctx.fillRect(x, y + 2, iw, g.pitch - 5);
    ctx.globalAlpha = 1;
    ctx.strokeStyle = 'rgba(0,0,0,0.45)';
    ctx.strokeRect(Math.round(x) + 0.5, Math.round(y + 2) + 0.5,
                   Math.max(1, Math.round(iw) - 1), g.pitch - 6);
  }
};
requestAnimationFrame(draw);
";
