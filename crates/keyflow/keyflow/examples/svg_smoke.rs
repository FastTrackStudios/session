//! Smoke test: `.kf` text → SVG via the `svg` feature (no GPU).
//! Run: cargo run -p keyflow --no-default-features --features svg,text --example svg_smoke

fn main() {
    let src = "Smoke Test - Keyflow\n4/4 120bpm #C\n\nVS\nG C Em D";

    let mode = keyflow::engraver::layout::chart::LayoutMode::paginated_a4();
    let result = keyflow::engraver::api::chart::layout_text(src, &mode).expect("layout text");

    let mut serializer = keyflow::engraver::export::svg::SvgSerializer::new(Default::default());
    let svg = serializer.serialize(&result.scene);

    println!("layout: {}x{}", result.total_width, result.total_height);
    println!("svg bytes: {}", svg.len());
    assert!(svg.contains("<svg"), "output is not SVG");
    println!("OK — produced SVG from .kf text with no GPU deps");
}
