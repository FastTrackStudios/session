//! Inventory a MusicXML file (or every .musicxml/.mxl in a directory):
//! parts, staves, note/rest counts, voices, clefs, keys, meters.
//!
//! cargo run -p engraver-score --example inventory -- <file-or-dir> [...]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: inventory <file-or-dir> [...]");
        std::process::exit(2);
    }
    let mut files = Vec::new();
    for arg in &args {
        let path = std::path::PathBuf::from(arg);
        if path.is_dir() {
            let mut entries: Vec<_> = std::fs::read_dir(&path)
                .expect("readable dir")
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                        e.eq_ignore_ascii_case("musicxml") || e.eq_ignore_ascii_case("mxl")
                    })
                })
                .collect();
            entries.sort();
            files.extend(entries);
        } else {
            files.push(path);
        }
    }

    let mut ok = 0usize;
    let mut failed = 0usize;
    for file in &files {
        match engraver_score::import_file(file) {
            Ok(score) => {
                ok += 1;
                let inv = engraver_score::inventory(&score);
                let notes: usize = inv.iter().map(|i| i.notes).sum();
                let rests: usize = inv.iter().map(|i| i.rests).sum();
                let measures = inv.first().map(|i| i.measures).unwrap_or(0);
                println!(
                    "OK   {:60} parts={:2} measures={:4} notes={:6} rests={:5} title={:?}",
                    file.file_name().unwrap_or_default().to_string_lossy(),
                    inv.len(),
                    measures,
                    notes,
                    rests,
                    score
                        .work_title
                        .or(score.movement_title)
                        .unwrap_or_default(),
                );
                if std::env::var("INVENTORY_VERBOSE").is_ok() {
                    for i in &inv {
                        println!(
                            "     - {:32} staves={} chords={:5} notes={:6} rests={:5} voices={:?} clefs={:?} keys={:?} times={:?}",
                            i.name,
                            i.staves,
                            i.chords,
                            i.notes,
                            i.rests,
                            i.voices,
                            i.clefs,
                            i.keys,
                            i.times
                        );
                    }
                }
            }
            Err(e) => {
                failed += 1;
                println!(
                    "FAIL {:60} {e}",
                    file.file_name().unwrap_or_default().to_string_lossy()
                );
            }
        }
    }
    println!("\n{ok} ok, {failed} failed of {} files", files.len());
    if failed > 0 {
        std::process::exit(1);
    }
}
