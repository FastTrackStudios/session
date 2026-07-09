use std::fs;
use std::path::PathBuf;
use std::time::Instant;

#[cfg(feature = "desktop-panels")]
use anyrender::ImageRenderer;
#[cfg(feature = "desktop-panels")]
use anyrender_vello::VelloImageRenderer;
#[cfg(feature = "desktop-panels")]
use keyflow_ui::ChartLayoutManager;
#[cfg(feature = "desktop-panels")]
use keyflow_ui::examples::{EMPTY_CHART, EXAMPLE_THRILLER};
#[cfg(feature = "desktop-panels")]
use kurbo::Affine;

#[derive(Debug, Clone)]
struct Config {
    chart_path: Option<PathBuf>,
    example: String,
    width: u32,
    height: u32,
    dpr: f64,
    zoom: f64,
    zoom_delta: f64,
    scroll_x: f64,
    scroll_y: f64,
    pan_dx: f64,
    pan_dy: f64,
    snippet_mode: bool,
    frames: usize,
    warmup: usize,
    overlay: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            chart_path: None,
            example: "thriller".to_string(),
            width: 2560,
            height: 1381,
            dpr: 1.0,
            zoom: 1.0,
            zoom_delta: 0.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            pan_dx: 0.0,
            pan_dy: 0.0,
            snippet_mode: false,
            frames: 300,
            warmup: 60,
            overlay: false,
        }
    }
}

fn print_usage() {
    println!(
        "Usage:
  cargo run -p keyflow-ui --example headless_perf --features desktop-panels -- [options]

Options:
  --chart <path>       Chart source file path
  --example <name>     Built-in example: thriller|empty (default: thriller)
  --width <px>         Surface width (default: 2560)
  --height <px>        Surface height (default: 1381)
  --dpr <scale>        Device pixel ratio (default: 1.0)
  --zoom <scale>       View zoom multiplier (default: 1.0)
  --zoom-delta <v>     Per-frame zoom delta (additive, default: 0.0)
  --scroll-x <px>      Scene scroll X in CSS px (default: 0)
  --scroll-y <px>      Scene scroll Y in CSS px (default: 0)
  --pan-dx <px>        Per-frame pan X in CSS px (default: 0)
  --pan-dy <px>        Per-frame pan Y in CSS px (default: 0)
  --snippet            Use snippet mode instead of paginated mode
  --frames <n>         Measured frames (default: 300)
  --warmup <n>         Warmup frames (default: 60)
  --overlay            Include overlay pass (cursor/hover disabled)
  -h, --help           Show help

Notes:
  - Uses offscreen Vello rendering (no Dioxus window/WebView).
  - Honors env tuning such as KEYFLOW_AA and KEYFLOW_PAGE_LOD_SCALE."
    );
}

fn parse_args() -> Result<Config, String> {
    let mut cfg = Config::default();
    let mut args = std::env::args().skip(1).peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            "--chart" => {
                let v = args.next().ok_or("--chart requires a path")?;
                cfg.chart_path = Some(PathBuf::from(v));
            }
            "--example" => cfg.example = args.next().ok_or("--example requires a value")?,
            "--width" => {
                cfg.width = args
                    .next()
                    .ok_or("--width requires a value")?
                    .parse()
                    .map_err(|_| "invalid --width")?
            }
            "--height" => {
                cfg.height = args
                    .next()
                    .ok_or("--height requires a value")?
                    .parse()
                    .map_err(|_| "invalid --height")?
            }
            "--dpr" => {
                cfg.dpr = args
                    .next()
                    .ok_or("--dpr requires a value")?
                    .parse()
                    .map_err(|_| "invalid --dpr")?
            }
            "--zoom" => {
                cfg.zoom = args
                    .next()
                    .ok_or("--zoom requires a value")?
                    .parse()
                    .map_err(|_| "invalid --zoom")?
            }
            "--zoom-delta" => {
                cfg.zoom_delta = args
                    .next()
                    .ok_or("--zoom-delta requires a value")?
                    .parse()
                    .map_err(|_| "invalid --zoom-delta")?
            }
            "--scroll-x" => {
                cfg.scroll_x = args
                    .next()
                    .ok_or("--scroll-x requires a value")?
                    .parse()
                    .map_err(|_| "invalid --scroll-x")?
            }
            "--scroll-y" => {
                cfg.scroll_y = args
                    .next()
                    .ok_or("--scroll-y requires a value")?
                    .parse()
                    .map_err(|_| "invalid --scroll-y")?
            }
            "--pan-dx" => {
                cfg.pan_dx = args
                    .next()
                    .ok_or("--pan-dx requires a value")?
                    .parse()
                    .map_err(|_| "invalid --pan-dx")?
            }
            "--pan-dy" => {
                cfg.pan_dy = args
                    .next()
                    .ok_or("--pan-dy requires a value")?
                    .parse()
                    .map_err(|_| "invalid --pan-dy")?
            }
            "--frames" => {
                cfg.frames = args
                    .next()
                    .ok_or("--frames requires a value")?
                    .parse()
                    .map_err(|_| "invalid --frames")?
            }
            "--warmup" => {
                cfg.warmup = args
                    .next()
                    .ok_or("--warmup requires a value")?
                    .parse()
                    .map_err(|_| "invalid --warmup")?
            }
            "--snippet" => cfg.snippet_mode = true,
            "--overlay" => cfg.overlay = true,
            x => return Err(format!("unknown argument: {}", x)),
        }
    }
    Ok(cfg)
}

fn percentile_ms(samples_ms: &[f64], p: f64) -> f64 {
    if samples_ms.is_empty() {
        return 0.0;
    }
    let mut sorted = samples_ms.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((sorted.len() as f64 * p).floor() as usize).min(sorted.len() - 1);
    sorted[idx]
}

#[cfg(not(feature = "desktop-panels"))]
fn main() {
    eprintln!("This example requires feature `desktop-panels`.");
    eprintln!(
        "Run: cargo run -p keyflow-ui --example headless_perf --features desktop-panels -- --help"
    );
}

#[cfg(feature = "desktop-panels")]
fn main() -> Result<(), String> {
    let cfg = parse_args()?;

    let source = if let Some(path) = cfg.chart_path.as_ref() {
        fs::read_to_string(path).map_err(|e| format!("failed to read {}: {}", path.display(), e))?
    } else {
        match cfg.example.to_ascii_lowercase().as_str() {
            "empty" => EMPTY_CHART.to_string(),
            _ => EXAMPLE_THRILLER.to_string(),
        }
    };

    println!(
        "headless_perf: {}x{} dpr={} zoom={} snippet={} frames={} warmup={} overlay={}",
        cfg.width,
        cfg.height,
        cfg.dpr,
        cfg.zoom,
        cfg.snippet_mode,
        cfg.frames,
        cfg.warmup,
        cfg.overlay
    );

    let mut manager =
        ChartLayoutManager::new().map_err(|e| format!("manager init failed: {}", e))?;

    let layout_start = Instant::now();
    manager.parse_and_layout(&source, cfg.width as f64, cfg.snippet_mode)?;
    let layout_ms = layout_start.elapsed().as_secs_f64() * 1000.0;
    println!("layout: {:.2}ms", layout_ms);

    let base_scale = manager.fit_to_width_scale(cfg.width as f64, cfg.dpr);
    let pad = 20.0 * cfg.dpr;
    let frame_transform = |frame_idx: usize| {
        let zoom = (cfg.zoom + cfg.zoom_delta * frame_idx as f64).max(0.05);
        let scroll_x = cfg.scroll_x + cfg.pan_dx * frame_idx as f64;
        let scroll_y = cfg.scroll_y + cfg.pan_dy * frame_idx as f64;
        Affine::translate((pad - scroll_x * cfg.dpr, pad - scroll_y * cfg.dpr))
            * Affine::scale(base_scale * zoom)
    };

    let mut renderer = VelloImageRenderer::new(cfg.width, cfg.height);
    let mut pixels = Vec::new();

    for i in 0..cfg.warmup {
        let transform = frame_transform(i);
        renderer.render_to_vec(
            |scene| {
                manager.render_static_layer_to_scene(
                    scene,
                    cfg.width as f64,
                    cfg.height as f64,
                    Affine::IDENTITY,
                    transform,
                );
                if cfg.overlay {
                    manager.render_overlay_layer_to_scene(
                        scene,
                        cfg.width as f64,
                        cfg.height as f64,
                        Affine::IDENTITY,
                        transform,
                        None,
                        None,
                    );
                }
            },
            &mut pixels,
        );
    }

    let mut frame_ms = Vec::with_capacity(cfg.frames);
    let measure_start = Instant::now();
    for i in 0..cfg.frames {
        let transform = frame_transform(i + cfg.warmup);
        let frame_start = Instant::now();
        renderer.render_to_vec(
            |scene| {
                manager.render_static_layer_to_scene(
                    scene,
                    cfg.width as f64,
                    cfg.height as f64,
                    Affine::IDENTITY,
                    transform,
                );
                if cfg.overlay {
                    manager.render_overlay_layer_to_scene(
                        scene,
                        cfg.width as f64,
                        cfg.height as f64,
                        Affine::IDENTITY,
                        transform,
                        None,
                        None,
                    );
                }
            },
            &mut pixels,
        );
        frame_ms.push(frame_start.elapsed().as_secs_f64() * 1000.0);
    }

    let total_ms = measure_start.elapsed().as_secs_f64() * 1000.0;
    let avg_ms = if frame_ms.is_empty() {
        0.0
    } else {
        frame_ms.iter().sum::<f64>() / frame_ms.len() as f64
    };
    let fps = if avg_ms > 0.0 { 1000.0 / avg_ms } else { 0.0 };
    let p95_ms = percentile_ms(&frame_ms, 0.95);
    let p99_ms = percentile_ms(&frame_ms, 0.99);

    println!(
        "render: frames={} total={:.2}ms avg={:.2}ms p95={:.2}ms p99={:.2}ms fps={:.1}",
        cfg.frames, total_ms, avg_ms, p95_ms, p99_ms, fps
    );

    // Explicit 60 Hz verdict: a frame must finish within 16.67 ms to sustain
    // 60 fps. p95 is the honest bar — occasional spikes still drop frames.
    const FRAME_BUDGET_MS: f64 = 1000.0 / 60.0;
    let verdict = if p95_ms <= FRAME_BUDGET_MS {
        "UNDER budget"
    } else {
        "OVER budget"
    };
    println!(
        "60Hz budget: {:.2}ms/frame — p95 {:.2}ms ({:.0}% of frame) => {verdict}",
        FRAME_BUDGET_MS,
        p95_ms,
        p95_ms / FRAME_BUDGET_MS * 100.0,
    );

    Ok(())
}
