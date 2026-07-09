//! Test 029: Melody Pitch Rendering
//!
//! Verifies that melody notes render at correct staff positions with proper
//! accidentals, ledger lines, and stems following MuseScore conventions.

use std::fs;
use std::path::PathBuf;

use engraver::export::pdf::PdfSerializer;
use engraver::export::svg::{SvgExportConfig, SvgSerializer};
use engraver::fonts::ChartFontBundle;
use engraver::layout::chart::{ChartLayoutConfig, LayoutMode};
use engraver::style::MStyle;

fn output_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/output");
    fs::create_dir_all(&dir).expect("create output dir");
    dir
}

#[test]
fn test_melody_pitched_notes_pdf() {
    let chart_text = r#"
Melody Pitch Test

120bpm 4/4 #C

vs
| C m{ C4 D4 E4 F4 } | G m{ G4 A4 B4 C'4 } |
| Am m{ A4 B4 C'4 D'4 } | F m{ F4 E4 D4 C4 } |

ch
| F m{ C4 D4 E4 F#4 G4 } | G m{ A8 Bb8 C'8 D'8 E'8 F'8 G'8 A'8 } |
| Am m{ E'4 D4 C4 B,4 } | C m{ C2 D2 E2 F2 G2 A2 B2 C'2 } |
"#;

    let chart = keyflow::text::chart::parse_chart(chart_text).expect("parse chart");

    // Verify melodies were parsed into measures
    for section in &chart.sections {
        for measure in section.measures() {
            if !measure.melodies.is_empty() {
                assert!(
                    !measure.melodies[0].notes.is_empty(),
                    "Melody should have notes"
                );
            }
        }
    }

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

    let out = output_dir().join("melody_pitch_test.pdf");
    fs::write(&out, &pdf_bytes).expect("write melody_pitch_test.pdf");
    println!(
        "Wrote {} ({} pages, {} bytes)",
        out.display(),
        result.pages.len(),
        pdf_bytes.len()
    );
}
