//! Test 028: Feature Showcase
//!
//! A comprehensive chart that demonstrates all keyflow rendering features:
//! - Chord symbols (major, minor, 7th, extended, slash chords)
//! - Stop signs (!STOP before/after, !STOPGROOVE before/after)
//! - Text cues (@keys, @drums, @bass, @all)
//! - Explicit rhythm notation (durations, rests, triplets)
//! - Push/pull notation ('C, C')
//! - Accents and staccato (>C, .C)
//! - Dynamic markings (<Build>, <Down>)
//! - Section types (Intro, Verse, Pre-Chorus, Chorus, Bridge, Solo, Outro)
//! - Slash rhythm notation (C //, C ////)
//! - Repeat syntax (x4)
//! - Measure separators (|)
//! - Tempo changes (->140bpm)

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
fn test_feature_showcase_pdf() {
    let chart_text = r#"
Feature Showcase - Keyflow Demo

120bpm 4/4 #C

IN 4
@drums "click count"
| !STOP >C | Am7 | F | G !STOP |

VS 8
@keys "rhodes"
| C | Am | F | G |
| C 'Em | Am | .F .G .Am .Bm | >C |

PRE-CH 4
<Build>
| Dm7 | G7 | Em7 | Am7 !STOPGROOVE |

CH 8
->140bpm
@all "full band"
| !STOP F | G | Am | Em |
| F | G | C !STOP | C |

BR 4
@keys "pad"
<Down>
| Dm | Fm | !STOPGROOVE C/E | F |

SOLO 4
@guitar "lead"
| Am | F | C | G !STOPGROOVE |

OUT 4
| C | Am | !STOP F | G !STOP |
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

    let out = output_dir().join("feature_showcase.pdf");
    fs::write(&out, &pdf_bytes).expect("write feature_showcase.pdf");
    println!(
        "Wrote {} ({} pages, {} bytes)",
        out.display(),
        result.pages.len(),
        pdf_bytes.len()
    );
}
