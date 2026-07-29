//! Render one part of a MusicXML score to SVG pages.
//!
//! cargo run -p engraver-score --example render_part -- <file> <part-substr> [out-base]
//!
//! Writes `<out-base>.p1.svg`, `.p2.svg`, … (default out-base: the part name
//! in the current directory).

use engraver_proto::engraver::fonts::ChartFontBundle;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: render_part <file> <part-substr> [out-base]");
        std::process::exit(2);
    };
    let part_sub = args.next().unwrap_or_default().to_lowercase();

    let score = engraver_score::import_file(&path).expect("import failed");
    let part_index = score
        .parts
        .iter()
        .position(|p| p.name.to_lowercase().contains(&part_sub))
        .unwrap_or_else(|| {
            eprintln!("part matching {part_sub:?} not found; parts:");
            for p in &score.parts {
                eprintln!("  - {}", p.name);
            }
            std::process::exit(1);
        });

    let out_base = args
        .next()
        .unwrap_or_else(|| score.parts[part_index].name.replace(['/', ' '], "_"));

    let opts = engraver_score::layout::LayoutOptions::default();
    let layout = engraver_score::layout::layout_part(&score, part_index, &opts);
    eprintln!(
        "part {:?}: {} page(s), {} dropped non-primary-voice events",
        score.parts[part_index].name,
        layout.pages.len(),
        layout.dropped_voice_events
    );

    let fonts = ChartFontBundle::new().expect("font bundle");
    let pages = engraver_score::render::svg_pages(&layout, &fonts);
    for (i, svg) in pages.iter().enumerate() {
        let out = format!("{out_base}.p{}.svg", i + 1);
        std::fs::write(&out, svg).expect("write svg");
        println!("wrote {out}");
    }
}
