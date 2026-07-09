//! Test 027: Stop Signs — !STOP and !STOPGROOVE rendering
//!
//! Verifies that stop sign tokens are parsed correctly and renders a PDF
//! showing both stop sign types (octagonal STOP and circular STOP GROOVE).
//!
//! Position semantics:
//! - `!STOP C`  → stop BEFORE the chord ("stop, then hit C")
//! - `C !STOP`  → stop AFTER the chord ("hit C, then stop")

use std::fs;
use std::path::PathBuf;

use engraver::export::pdf::PdfSerializer;
use engraver::export::svg::{SvgExportConfig, SvgSerializer};
use engraver::fonts::ChartFontBundle;
use engraver::layout::chart::{ChartLayoutConfig, LayoutMode};
use engraver::style::MStyle;
use keyflow::chart::commands::Command;

fn output_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/output");
    fs::create_dir_all(&dir).expect("create output dir");
    dir
}

#[test]
fn test_stop_before_chord() {
    // !STOP before a chord → Stop (before) command
    let chart_text = "120bpm 4/4 #C\nvs\n| !STOP C | G |";
    let chart = keyflow::text::chart::parse_chart(chart_text).expect("parse chart");
    let measures = chart.sections[0].measures();

    assert_eq!(measures[0].chords[0].full_symbol, "C");
    assert!(
        measures[0].chords[0]
            .commands
            .iter()
            .any(|c| matches!(c, Command::Stop)),
        "!STOP C should produce Stop (before). Got: {:?}",
        measures[0].chords[0].commands
    );
    assert!(
        !measures[0].chords[0]
            .commands
            .iter()
            .any(|c| c.is_stop_after()),
        "!STOP C should NOT be stop-after"
    );
}

#[test]
fn test_stop_after_chord() {
    // !STOP after a chord → StopAfter command on that chord
    let chart_text = "120bpm 4/4 #C\nvs\n| C !STOP | G |";
    let chart = keyflow::text::chart::parse_chart(chart_text).expect("parse chart");
    let measures = chart.sections[0].measures();

    assert_eq!(measures[0].chords[0].full_symbol, "C");
    assert!(
        measures[0].chords[0]
            .commands
            .iter()
            .any(|c| matches!(c, Command::StopAfter)),
        "C !STOP should produce StopAfter. Got: {:?}",
        measures[0].chords[0].commands
    );
}

#[test]
fn test_stop_groove_before_and_after() {
    let chart_text = "120bpm 4/4 #C\nvs\n| !STOPGROOVE C | G !STOPGROOVE |";
    let chart = keyflow::text::chart::parse_chart(chart_text).expect("parse chart");
    let measures = chart.sections[0].measures();

    // Measure 0: !STOPGROOVE C → StopGroove (before)
    assert!(
        measures[0].chords[0]
            .commands
            .iter()
            .any(|c| matches!(c, Command::StopGroove)),
        "!STOPGROOVE C should produce StopGroove (before). Got: {:?}",
        measures[0].chords[0].commands
    );

    // Measure 1: G !STOPGROOVE → StopGrooveAfter
    assert!(
        measures[1].chords[0]
            .commands
            .iter()
            .any(|c| matches!(c, Command::StopGrooveAfter)),
        "G !STOPGROOVE should produce StopGrooveAfter. Got: {:?}",
        measures[1].chords[0].commands
    );
}

#[test]
fn test_stop_sign_case_insensitive() {
    let chart_text = "120bpm 4/4 #C\nvs\n| !stop C | !StopGroove G |";
    let chart = keyflow::text::chart::parse_chart(chart_text).expect("parse chart");
    let measures = chart.sections[0].measures();

    assert!(
        measures[0].chords[0]
            .commands
            .iter()
            .any(|c| matches!(c, Command::Stop)),
        "lowercase !stop should parse"
    );
    assert!(
        measures[1].chords[0]
            .commands
            .iter()
            .any(|c| matches!(c, Command::StopGroove)),
        "mixed case !StopGroove should parse"
    );
}

#[test]
fn test_stop_no_bleed_to_other_chords() {
    // Stop signs should only attach to the adjacent chord, not bleed
    let chart_text = "120bpm 4/4 #C\nvs\n| !STOP C G | Am |";
    let chart = keyflow::text::chart::parse_chart(chart_text).expect("parse chart");
    let measures = chart.sections[0].measures();

    assert!(
        measures[0].chords[0].commands.iter().any(|c| c.is_stop()),
        "C should have stop"
    );
    assert!(
        !measures[0].chords[1].commands.iter().any(|c| c.is_stop()),
        "G should NOT have stop"
    );
    assert!(
        !measures[1].chords[0].commands.iter().any(|c| c.is_stop()),
        "Am should NOT have stop"
    );
}

#[test]
fn test_stop_sign_pdf_render() {
    let chart_text = r#"
Stop Signs Demo

120bpm 4/4 #C

in
| !STOP C | G |

vs
| Am | F !STOP | !STOPGROOVE C | G !STOPGROOVE |

ch
| !STOP Fmaj7 | G7 !STOP | Am | !STOPGROOVE Em7 |

br
| Dm | !STOP Gsus4 | C !STOPGROOVE | Am |

ou
| C !STOP |
"#;

    let chart = keyflow::text::chart::parse_chart(chart_text).expect("parse chart");

    // Layout engine
    let font_bundle = ChartFontBundle::new().expect("load fonts");
    let style: &'static MStyle = Box::leak(Box::new(MStyle::new()));
    let engine = font_bundle.create_layout_engine(style);

    let layout_config = ChartLayoutConfig::master_rhythm().with_page_offsets(true);
    let mode = LayoutMode::paginated_a4();
    let result = engine.layout_chart_with_config(&chart, &mode, &layout_config);

    // Export SVG pages
    let mut svg_pages = Vec::with_capacity(result.pages.len());
    for page in &result.pages {
        let svg_config =
            SvgExportConfig::for_page(page.x_offset, page.y_offset, page.width, page.height)
                .with_embedded_font("Bravura", font_bundle.symbol_font_data().as_ref().clone())
                .with_embedded_font(
                    "MuseJazzText",
                    font_bundle.text_font_data().as_ref().clone(),
                )
                .with_embedded_font("FreeSans", font_bundle.aux_font_data().as_ref().clone());
        let mut serializer = SvgSerializer::new(svg_config);
        svg_pages.push(serializer.serialize(&result.scene));
    }

    // SVG → PDF
    let pdf_bytes = PdfSerializer::serialize_from_svg(
        &svg_pages,
        &[
            ("Bravura", font_bundle.symbol_font_data().as_slice()),
            ("MuseJazzText", font_bundle.text_font_data().as_slice()),
            ("FreeSans", font_bundle.aux_font_data().as_slice()),
        ],
    )
    .expect("serialize PDF");

    let out = output_dir().join("stop_signs_demo.pdf");
    fs::write(&out, &pdf_bytes).expect("write stop_signs_demo.pdf");
    println!(
        "Wrote {} ({} pages, {} bytes)",
        out.display(),
        result.pages.len(),
        pdf_bytes.len()
    );
}
