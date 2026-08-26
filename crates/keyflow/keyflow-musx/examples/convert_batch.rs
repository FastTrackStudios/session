//! Convert MANY `.musx` files to MusicXML in parallel (rayon).
//!
//! Usage: `cargo run --release -p keyflow-musx --example convert_batch -- <file-or-dir>…`
//!
//! Directories are walked recursively for `*.musx`; each file converts to a
//! sibling `<name>.musicxml`. Existing outputs are skipped (pass `--force` to
//! reconvert). Large full-orchestra scores can take minutes each — the whole
//! point of running the batch across every core.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use rayon::prelude::*;

fn collect(path: &std::path::Path, out: &mut Vec<PathBuf>) {
    if path.is_dir() {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for e in entries.flatten() {
            collect(&e.path(), out);
        }
    } else if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("musx"))
    {
        out.push(path.to_path_buf());
    }
}

fn main() {
    let mut force = false;
    let mut inputs = Vec::new();
    for arg in std::env::args().skip(1) {
        if arg == "--force" {
            force = true;
        } else {
            collect(std::path::Path::new(&arg), &mut inputs);
        }
    }
    if inputs.is_empty() {
        eprintln!("usage: convert_batch [--force] <file-or-dir>…");
        std::process::exit(2);
    }
    inputs.sort();

    let (done, skipped, failed) = (
        AtomicUsize::new(0),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
    );
    let started = Instant::now();
    inputs.par_iter().for_each(|input| {
        let out = input.with_extension("musicxml");
        if !force && out.exists() {
            skipped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let t = Instant::now();
        let result = std::fs::read(input)
            .map_err(|e| format!("read: {e}"))
            .and_then(|musx| {
                keyflow_musx::musx_to_musicxml(&musx).map_err(|e| format!("convert: {e}"))
            })
            .and_then(|xml| std::fs::write(&out, xml).map_err(|e| format!("write: {e}")));
        match result {
            Ok(()) => {
                done.fetch_add(1, Ordering::Relaxed);
                eprintln!(
                    "ok   {:>7.1}s  {}",
                    t.elapsed().as_secs_f32(),
                    input.display()
                );
            }
            Err(e) => {
                failed.fetch_add(1, Ordering::Relaxed);
                let _ = std::fs::remove_file(&out);
                eprintln!(
                    "FAIL {:>7.1}s  {} — {e}",
                    t.elapsed().as_secs_f32(),
                    input.display()
                );
            }
        }
    });
    eprintln!(
        "batch: {} converted, {} skipped, {} failed in {:.1}s",
        done.load(Ordering::Relaxed),
        skipped.load(Ordering::Relaxed),
        failed.load(Ordering::Relaxed),
        started.elapsed().as_secs_f32(),
    );
    if failed.load(Ordering::Relaxed) > 0 {
        std::process::exit(1);
    }
}
