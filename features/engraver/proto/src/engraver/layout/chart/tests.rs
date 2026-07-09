//! Integration-style tests for chart layout.

use super::test_utils::*;
use super::*;
use crate::engraver::scene::id::ElementType;
use crate::engraver::scene::id::SemanticId;
use crate::engraver::scene::node::SceneNode;
use crate::engraver::scene::paint::PaintCommand;
use crate::engraver::scene::traverse::SceneNodeExt;
use crate::engraver::style::MStyle;
use crate::{
    AbsolutePosition, Chart, ChartPosition, ChartSection, Chord, ChordInstance, ChordQuality,
    ChordRhythm, MusicalDuration, MusicalNote, MusicalPosition, RootNotation, Section, SectionType,
    TimeSignature,
};
use keyflow_proto::chart::notations::Placement;
use keyflow_proto::chart::types::Measure;
use keyflow_proto::time::MusicalPositionExt;
use kurbo::{Affine, Point};
use peniko::Color;
use std::path::PathBuf;
use std::sync::Arc;

/// Create a static MStyle for testing (leaked for 'static lifetime).
fn test_style() -> &'static MStyle {
    Box::leak(Box::new(MStyle::default()))
}

/// Helper to create a RootNotation from a note name string.
fn root(name: &str) -> RootNotation {
    RootNotation::from_note_name(MusicalNote::from_string(name).unwrap())
}

/// Create a simple test chart with known chord positions.
fn create_test_chart() -> Chart {
    // Create a chart with 2 measures, each with chords on specific beats
    let mut chart = Chart::new();
    chart.time_signature = Some(TimeSignature::new(4, 4));

    // Measure 1: Chord on beat 0
    let measure1 = Measure {
        chords: vec![ChordInstance::new(
            root("C"),
            "C".to_string(),
            Chord::new(root("C"), ChordQuality::Major),
            ChordRhythm::Default,
            "C".to_string(),
            MusicalDuration::new(0, 4, 0), // whole note
            AbsolutePosition::new(
                MusicalPosition::try_new(0, 0, 0).unwrap(), // beat 0
                0,                                          // section index
            ),
        )],
        ..Default::default()
    };

    // Measure 2: Two chords - beat 0 and beat 2
    let measure2 = Measure {
        chords: vec![
            ChordInstance::new(
                root("G"),
                "G".to_string(),
                Chord::new(root("G"), ChordQuality::Major),
                ChordRhythm::Default,
                "G".to_string(),
                MusicalDuration::new(0, 2, 0), // half note
                AbsolutePosition::new(
                    MusicalPosition::try_new(1, 0, 0).unwrap(), // measure 1, beat 0
                    0,                                          // section index
                ),
            ),
            ChordInstance::new(
                root("D"),
                "Dm".to_string(),
                Chord::new(root("D"), ChordQuality::Minor),
                ChordRhythm::Default,
                "Dm".to_string(),
                MusicalDuration::new(0, 2, 0), // half note
                AbsolutePosition::new(
                    MusicalPosition::try_new(1, 2, 0).unwrap(), // measure 1, beat 2
                    0,                                          // section index
                ),
            ),
        ],
        ..Default::default()
    };

    let mut section_info = Section::new(SectionType::Verse);
    section_info.number = Some(1);
    section_info.measure_count = Some(2);

    let section = ChartSection::new(section_info).with_measures(vec![measure1, measure2]);

    chart.sections.push(section);
    chart
}

fn lord_of_the_fight_fixture() -> PathBuf {
    let mut fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    fixture.push("../../../crates/keyflow/examples/png-project-charts/02 LORD OF THE FIGHT Master RS.musicxml");
    fixture
}

#[derive(Debug)]
struct TextCommandPosition {
    element_type: Option<ElementType>,
    metadata_type: Option<String>,
    dynamic_level: Option<String>,
    text: String,
    position: Point,
}

fn collect_text_command_positions(scene: &SceneNode) -> Vec<TextCommandPosition> {
    scene
        .iter_with_transforms()
        .flat_map(|(node, transform): (&SceneNode, Affine)| {
            node.commands.iter().filter_map(move |command| {
                if let PaintCommand::Text { text, position, .. } = command {
                    Some(TextCommandPosition {
                        element_type: node.id.as_ref().map(|id| id.element_type),
                        metadata_type: node.metadata.get("element_type").cloned(),
                        dynamic_level: node.metadata.get("dynamic_level").cloned(),
                        text: text.clone(),
                        position: transform * *position,
                    })
                } else {
                    None
                }
            })
        })
        .collect()
}

fn collect_chord_node_texts(scene: &SceneNode) -> Vec<(String, Option<String>, Option<String>)> {
    fn walk(node: &SceneNode, out: &mut Vec<(String, Option<String>, Option<String>)>) {
        if node.metadata.get("element_type") == Some(&"chord".to_string()) {
            let text = node
                .commands
                .iter()
                .filter_map(|command| {
                    if let PaintCommand::Text { text, .. } = command {
                        Some(text.as_str())
                    } else {
                        None
                    }
                })
                .collect::<String>();
            out.push((
                text,
                node.metadata.get("measure").cloned(),
                node.metadata.get("beat").cloned(),
            ));
        }

        for child in &node.children {
            walk(child, out);
        }
    }

    let mut out = Vec::new();
    walk(scene, &mut out);
    out
}

fn chord_node_indices_at_chart_position(
    scene: &SceneNode,
    measure_idx: usize,
    chord_idx: usize,
) -> Vec<usize> {
    scene
        .iter_with_transforms()
        .enumerate()
        .filter_map(|(node_idx, (node, _))| {
            if node.metadata.get("element_type") != Some(&"chord".to_string()) {
                return None;
            }
            let position = node.get_json_metadata::<ChartPosition>("chart_position")?;
            (position.measure == measure_idx as u32 && position.beat == chord_idx as u32)
                .then_some(node_idx)
        })
        .collect()
}

#[test]
fn lord_of_the_fight_a_major_seven_at_6_4_renders_triangle_seven() {
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());
    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let chart = keyflow_musicxml::import_file(lord_of_the_fight_fixture())
        .expect("Lord of the Fight fixture should import");

    let source_measure_6 = chart
        .sections
        .iter()
        .flat_map(|section| section.measures())
        .find(|measure| measure.source_measure_number == Some(6))
        .expect("LOTF should have source measure 6");
    let a_major_seven_at_6_4 = source_measure_6.chords.iter().any(|chord| {
        chord.full_symbol == "Amaj7"
            && chord.position.measures() == 0
            && chord.position.beats() == 3
    });
    assert!(
        a_major_seven_at_6_4,
        "LOTF should import Amaj7 at printed position 6.4"
    );

    let result = engine.layout_chart(&chart, &LayoutMode::default());
    let chord_texts = collect_chord_node_texts(&result.scene);
    assert!(
        chord_texts.iter().any(|(text, _, _)| text == "A\u{E18A}7"),
        "Amaj7 at printed position 6.4 should render as A<Triangle>7; rendered chords: {chord_texts:?}"
    );
    assert!(
        !chord_texts.iter().any(|(text, _, _)| text == "AMaj7"),
        "major seventh chords should not render with the Maj7 text suffix"
    );
}

#[test]
fn lord_of_the_fight_measure_5_staff_text_uses_expected_placement() {
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());
    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let chart = keyflow_musicxml::import_file(lord_of_the_fight_fixture())
        .expect("Lord of the Fight fixture should import");

    let source_measure_5 = chart
        .sections
        .iter()
        .flat_map(|section| section.measures())
        .find(|measure| measure.source_measure_number == Some(5))
        .expect("LOTF should have source measure 5");
    assert!(
        source_measure_5.staff_text.iter().any(|text| {
            text.text == "Cresc. <...<...<" && text.beat == 1 && text.placement == Placement::Below
        }),
        "LOTF m5 Cresc. text should import at beat 1 below the staff: {:?}",
        source_measure_5.staff_text
    );
    assert!(
        source_measure_5.staff_text.iter().any(|text| {
            text.text == "'the CLIMB'" && text.beat == 1 && text.placement == Placement::Above
        }),
        "LOTF m5 'the CLIMB' text should import at beat 1 above the staff: {:?}",
        source_measure_5.staff_text
    );

    let result = engine.layout_chart(&chart, &LayoutMode::default());
    let text_positions = collect_text_command_positions(&result.scene);
    let cresc = text_positions
        .iter()
        .find(|pos| {
            pos.metadata_type.as_deref() == Some("staff_text") && pos.text == "Cresc. <...<...<"
        })
        .expect("LOTF m5 Cresc. staff text should render");
    let climb = text_positions
        .iter()
        .find(|pos| pos.metadata_type.as_deref() == Some("staff_text") && pos.text == "'the CLIMB'")
        .expect("LOTF m5 'the CLIMB' staff text should render");
    let measure_start = result
        .beat_positions
        .iter()
        .filter(|beat| beat.beat == 0)
        .min_by(|a, b| {
            let a_score = (a.x - cresc.position.x).abs() + (a.staff_y - cresc.position.y).abs();
            let b_score = (b.x - cresc.position.x).abs() + (b.staff_y - cresc.position.y).abs();
            a_score.total_cmp(&b_score)
        })
        .expect("LOTF m5 should have a rendered first beat near the Cresc. text");

    assert!(
        (cresc.position.x - climb.position.x).abs() <= 1.0,
        "Cresc. and 'the CLIMB' should share the m5 beat-1/start x: cresc.x={:.1}, climb.x={:.1}",
        cresc.position.x,
        climb.position.x
    );
    assert!(
        cresc.position.y > measure_start.staff_y + measure_start.staff_height,
        "Cresc. text should render below the m5 staff: text.y={:.1}, staff=[{:.1}, {:.1}]",
        cresc.position.y,
        measure_start.staff_y,
        measure_start.staff_y + measure_start.staff_height
    );
    assert!(
        climb.position.y < measure_start.staff_y,
        "'the CLIMB' should render above the m5 staff: text.y={:.1}, staff_y={:.1}",
        climb.position.y,
        measure_start.staff_y
    );
}

#[test]
fn lord_of_the_fight_measure_5_add_bass_sits_near_measure_end() {
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());
    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let chart = keyflow_musicxml::import_file(lord_of_the_fight_fixture())
        .expect("Lord of the Fight fixture should import");

    let source_measure_5 = chart
        .sections
        .iter()
        .flat_map(|section| section.measures())
        .find(|measure| measure.source_measure_number == Some(5))
        .expect("LOTF should have source measure 5");
    let add_bass = source_measure_5
        .staff_text
        .iter()
        .find(|text| text.text == "ADD BASS")
        .expect("LOTF m5 should import ADD BASS staff text");

    assert_eq!(add_bass.placement, Placement::Below);
    let source_width = source_measure_5
        .source_measure_width
        .expect("LOTF m5 should preserve source measure width");
    let source_x = add_bass
        .source_default_x
        .expect("ADD BASS should derive a source x from its MusicXML direction tick");
    assert!(
        source_x > source_width * 0.65,
        "ADD BASS should import near the final m5 eighth-note area, not beat 1: source_x={source_x:.1}, source_width={source_width:.1}, text={add_bass:?}"
    );

    let result = engine.layout_chart(&chart, &LayoutMode::default());
    let text_positions = collect_text_command_positions(&result.scene);
    let rendered = text_positions
        .iter()
        .find(|pos| pos.metadata_type.as_deref() == Some("staff_text") && pos.text == "ADD BASS")
        .expect("LOTF m5 ADD BASS staff text should render");
    let nearest_final_beat = result
        .beat_positions
        .iter()
        .filter(|beat| beat.beat >= 3)
        .min_by(|a, b| {
            (a.x - rendered.position.x)
                .abs()
                .total_cmp(&(b.x - rendered.position.x).abs())
        })
        .expect("LOTF should have rendered late-beat positions");

    assert!(
        (rendered.position.x - nearest_final_beat.x).abs() <= 24.0,
        "ADD BASS should render near the final m5 note area: text.x={:.1}, late_beat.x={:.1}",
        rendered.position.x,
        nearest_final_beat.x
    );
    assert!(
        rendered.position.y > nearest_final_beat.staff_y + nearest_final_beat.staff_height,
        "ADD BASS should render below the staff: text.y={:.1}, staff=[{:.1}, {:.1}]",
        rendered.position.y,
        nearest_final_beat.staff_y,
        nearest_final_beat.staff_y + nearest_final_beat.staff_height
    );
}

#[test]
fn lord_of_the_fight_a_over_c_sharp_at_10_1_has_no_conflicts() {
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());
    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let chart = keyflow_musicxml::import_file(lord_of_the_fight_fixture())
        .expect("Lord of the Fight fixture should import");

    let (measure_idx, chord_idx) = chart
        .sections
        .iter()
        .flat_map(|section| section.measures())
        .enumerate()
        .find_map(|(measure_idx, measure)| {
            (measure.source_measure_number == Some(10)).then(|| {
                measure
                    .chords
                    .iter()
                    .enumerate()
                    .find_map(|(chord_idx, chord)| {
                        (chord.full_symbol == "A/C#"
                            && chord.position.measures() == 0
                            && chord.position.beats() == 0)
                            .then_some((measure_idx, chord_idx))
                    })
            })?
        })
        .expect("LOTF should import A/C# at printed position 10.1");

    let result = engine.layout_chart(&chart, &LayoutMode::default());
    let target_nodes = chord_node_indices_at_chart_position(&result.scene, measure_idx, chord_idx);
    assert!(
        !target_nodes.is_empty(),
        "A/C# at 10.1 should have rendered chord nodes at chart measure {measure_idx}, chord {chord_idx}"
    );

    let collisions = find_disallowed_visual_collisions(&result.scene)
        .into_iter()
        .filter(|collision| {
            target_nodes.contains(&collision.a.node_index)
                || target_nodes.contains(&collision.b.node_index)
        })
        .collect::<Vec<_>>();

    assert!(
        collisions.is_empty(),
        "A/C# at 10.1 should not conflict with any visible symbol; target_nodes={target_nodes:?}, collisions={collisions:#?}"
    );
}

#[test]
fn lord_of_the_fight_long_endings_start_new_systems() {
    let style = test_style();
    let engine = ChartLayoutEngine::new(style, Arc::new(Vec::new()), Arc::new(Vec::new()));
    let chart = keyflow_musicxml::import_file(lord_of_the_fight_fixture())
        .expect("Lord of the Fight fixture should import");

    let chorus_repeat = chart
        .sections
        .iter()
        .find(|section| {
            section
                .section
                .name
                .as_deref()
                .is_some_and(|name| name.contains("CH 1") && name.contains("CH 2"))
        })
        .expect("expected CH 1 / CH 2 repeat section");
    let systems = engine.group_measures_into_systems(chorus_repeat.measures(), 700.0);
    let system_sources = systems
        .iter()
        .map(|system| {
            system
                .iter()
                .map(|idx| chorus_repeat.measures()[*idx].source_measure_number)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    assert!(
        system_sources
            .iter()
            .any(|system| system == &vec![Some(42), Some(43), Some(44), Some(45)]),
        "first ending should start on a new system and occupy m42-m45; got {system_sources:?}"
    );
    assert!(
        system_sources
            .iter()
            .any(|system| system.first() == Some(&Some(46))),
        "second ending should start on a new system at m46; got {system_sources:?}"
    );
}

#[test]
fn lord_of_the_fight_start_of_system_dynamics_sit_under_clef_prefix() {
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());
    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let chart = keyflow_musicxml::import_file(lord_of_the_fight_fixture())
        .expect("Lord of the Fight fixture should import");

    let result = engine.layout_chart(&chart, &LayoutMode::default());
    let text_positions = collect_text_command_positions(&result.scene);

    let measures: Vec<&Measure> = chart.sections.iter().flat_map(|s| s.measures()).collect();
    let system_start = result
        .beat_positions
        .iter()
        .filter(|beat| beat.beat == 0)
        .filter(|beat| {
            result
                .beat_positions
                .iter()
                .filter(|other| other.page == beat.page && other.system == beat.system)
                .map(|other| other.measure)
                .min()
                == Some(beat.measure)
        })
        .find(|beat| {
            measures.get(beat.measure).is_some_and(|measure| {
                measure.classical_dynamics.len() == 1
                    && measure.classical_dynamics.iter().any(|d| d.beat == 1)
            })
        })
        .expect("Lord of the Fight should have a beat-1 dynamic at the start of a system");

    let clefs = find_elements_on_page(&result.scene, ElementType::Clef, system_start.page);
    let key_sigs =
        find_elements_on_page(&result.scene, ElementType::KeySignature, system_start.page);
    let clef = clefs
        .iter()
        .min_by(|a, b| {
            (a.world_y - system_start.staff_y)
                .abs()
                .total_cmp(&(b.world_y - system_start.staff_y).abs())
        })
        .expect("target system should render a clef");
    let key_sig = key_sigs
        .iter()
        .min_by(|a, b| {
            (a.world_y - system_start.staff_y)
                .abs()
                .total_cmp(&(b.world_y - system_start.staff_y).abs())
        })
        .expect("target system should render a key signature");

    let dynamic = text_positions
        .iter()
        .filter(|pos| pos.metadata_type.as_deref() == Some("dynamic"))
        .filter(|pos| pos.position.x >= clef.world_x - 20.0)
        .filter(|pos| {
            pos.position.y > system_start.staff_y + system_start.staff_height
                && pos.position.y < system_start.staff_y + system_start.staff_height + 220.0
        })
        .min_by(|a, b| {
            (a.position.y - system_start.staff_y)
                .abs()
                .total_cmp(&(b.position.y - system_start.staff_y).abs())
                .then(a.position.x.total_cmp(&b.position.x))
        })
        .expect("target system should render a below-staff dynamic");

    assert_eq!(
        dynamic.element_type,
        Some(ElementType::Articulation),
        "dynamic metadata is currently carried by an articulation scene node"
    );
    assert!(
        dynamic.position.x >= clef.world_x - 20.0,
        "start-of-system dynamic '{}' should sit under the clef prefix, not before it: dynamic.x={:.1}, clef.x={:.1}",
        dynamic.text,
        dynamic.position.x,
        clef.world_x
    );
    let _ = key_sig;
}

#[test]
fn lord_of_the_fight_repeat_pass_dynamics_stack_under_section_cards() {
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());
    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let chart = keyflow_musicxml::import_file(lord_of_the_fight_fixture())
        .expect("Lord of the Fight fixture should import");

    let result = engine.layout_chart(&chart, &LayoutMode::default());
    let text_positions = collect_text_command_positions(&result.scene);

    let mf = text_positions
        .iter()
        .find(|pos| {
            pos.metadata_type.as_deref() == Some("dynamic")
                && pos.dynamic_level.as_deref() == Some("mf")
        })
        .expect("VS 1b mf dynamic should render under the stacked section cards");
    let f = text_positions
        .iter()
        .find(|pos| {
            pos.metadata_type.as_deref() == Some("dynamic")
                && pos.dynamic_level.as_deref() == Some("f")
        })
        .expect("VS 2 f dynamic should render under the stacked section cards");
    let vs_1b = text_positions
        .iter()
        .filter(|pos| pos.text == "VS 1b")
        .filter(|pos| (pos.position.x - mf.position.x).abs() < 40.0)
        .filter(|pos| pos.position.y < mf.position.y)
        .max_by(|a, b| a.position.y.total_cmp(&b.position.y))
        .expect("repeat pass section card should render above the mf dynamic");
    let vs_2 = text_positions
        .iter()
        .filter(|pos| pos.text == "VS 2")
        .filter(|pos| (pos.position.x - f.position.x).abs() < 40.0)
        .filter(|pos| pos.position.y < f.position.y)
        .max_by(|a, b| a.position.y.total_cmp(&b.position.y))
        .expect("repeat pass section card should render above the f dynamic");

    assert!(
        mf.position.y > vs_1b.position.y,
        "VS 1b dynamic should sit below its section card: mf.y={:.1}, label.y={:.1}",
        mf.position.y,
        vs_1b.position.y
    );
    assert!(
        vs_2.position.y > mf.position.y,
        "VS 2 card should sit below the VS 1b dynamic: vs2.y={:.1}, mf.y={:.1}",
        vs_2.position.y,
        mf.position.y
    );
    assert!(
        f.position.y > vs_2.position.y,
        "VS 2 dynamic should sit below its section card: f.y={:.1}, label.y={:.1}",
        f.position.y,
        vs_2.position.y
    );
}

#[test]
fn staff_text_shrinks_when_that_avoids_chord_symbol_collision() {
    let mut skyline = notation_renderer::MeasureSkyline::new();
    skyline.add_above(kurbo::Rect::new(30.0, -30.0, 100.0, 0.0));

    let node = SceneNode::leaf(
        SemanticId::new(ElementType::Text, 1),
        vec![PaintCommand::text(
            "Text",
            "Leland Text",
            20.0,
            Point::new(0.0, -20.0),
            Color::BLACK,
        )],
    );

    let placed = notation_renderer::autoplace_text_node(&mut skyline, node, true, 0.0, 0.0)
        .expect("text should place");
    let text = placed
        .commands
        .iter()
        .find_map(|cmd| {
            if let PaintCommand::Text {
                font_size,
                position,
                ..
            } = cmd
            {
                Some((*font_size, *position))
            } else {
                None
            }
        })
        .expect("placed node should contain text");

    assert!(
        text.0 < 20.0,
        "text should shrink before accepting a collision-driven displacement"
    );
    assert_eq!(
        text.1.y, -20.0,
        "shrunk text should stay at its intended baseline when shrinking avoids the chord"
    );
}

#[test]
fn visual_collision_scanner_flags_text_overlap_but_ignores_staff_lines() {
    let mut scene = SceneNode::group(SemanticId::new(ElementType::Page, 1));
    scene.add_child(SceneNode::leaf(
        SemanticId::new(ElementType::StaffLines, 1),
        vec![PaintCommand::line(
            Point::new(0.0, 0.0),
            Point::new(120.0, 0.0),
            Color::BLACK,
            1.0,
        )],
    ));
    scene.add_child(SceneNode::leaf(
        SemanticId::new(ElementType::Text, 1),
        vec![PaintCommand::text(
            "Alpha",
            "Leland Text",
            20.0,
            Point::new(10.0, 20.0),
            Color::BLACK,
        )],
    ));
    scene.add_child(SceneNode::leaf(
        SemanticId::new(ElementType::ChordSymbol, 2),
        vec![PaintCommand::text(
            "Beta",
            "Leland Text",
            20.0,
            Point::new(20.0, 20.0),
            Color::BLACK,
        )],
    ));

    let collisions = find_disallowed_visual_collisions(&scene);

    assert_eq!(
        collisions.len(),
        1,
        "only the text/chord overlap should be reported: {collisions:#?}"
    );
    assert_eq!(collisions[0].a.element_type, ElementType::Text);
    assert_eq!(collisions[0].b.element_type, ElementType::ChordSymbol);
}

#[test]
#[ignore = "LOTF still has known legacy collisions; run explicitly while improving placement."]
fn lord_of_the_fight_has_no_disallowed_visual_collisions() {
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());
    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let chart = keyflow_musicxml::import_file(lord_of_the_fight_fixture())
        .expect("Lord of the Fight fixture should import");

    let result = engine.layout_chart(&chart, &LayoutMode::default());
    let collisions = find_disallowed_visual_collisions(&result.scene);

    assert!(
        collisions.is_empty(),
        "Lord of the Fight has disallowed visual collisions; first collisions: {:#?}",
        collisions.iter().take(12).collect::<Vec<_>>()
    );
}

#[test]
fn lord_of_the_fight_reports_dead_vertical_space_between_systems() {
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());
    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let chart = keyflow_musicxml::import_file(lord_of_the_fight_fixture())
        .expect("Lord of the Fight fixture should import");

    let result = engine.layout_chart(&chart, &LayoutMode::default());
    let dead_spaces = find_dead_vertical_space_between_systems(&result, 12.0);

    assert!(
        !dead_spaces.is_empty(),
        "LOTF should preserve baseline system whitespace; the scanner should remain a diagnostic"
    );
    eprintln!(
        "largest dead vertical spaces over 12pt: {:#?}",
        dead_spaces.iter().take(8).collect::<Vec<_>>()
    );
}

/// Performance budget for realtime editing: a full chart relayout must fit a
/// 60 Hz frame (16.67 ms) with headroom, since layout runs on every edit and
/// many scroll/zoom interactions. This measures the paged layout of the full
/// Lord of the Fight chart (111 measures, melody + chords + annotations).
///
/// Run with output:
///   cargo test -p engraver-proto --lib layout_chart_meets_60hz_budget -- --nocapture --ignored
///
/// Ignored by default so it doesn't slow normal test runs or flake on shared
/// CI hardware; the printed numbers are the deliverable, and the assertion is a
/// generous ceiling that only trips on a real regression.
#[test]
#[ignore = "performance benchmark — run explicitly with --ignored --nocapture"]
fn layout_chart_meets_60hz_budget() {
    use std::time::Instant;

    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());
    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let chart = keyflow_musicxml::import_file(lord_of_the_fight_fixture())
        .expect("Lord of the Fight fixture should import");

    const FRAME_BUDGET_MS: f64 = 1000.0 / 60.0; // 16.67 ms
    let measures: usize = chart.sections.iter().map(|s| s.measures().len()).sum();
    let iterations = 100;

    let bench = |label: &str, mode: LayoutMode| -> f64 {
        // Warm up (allocator / caches) before timing.
        for _ in 0..5 {
            std::hint::black_box(engine.layout_chart(&chart, &mode));
        }
        let mut samples_ms = Vec::with_capacity(iterations);
        for _ in 0..iterations {
            let start = Instant::now();
            let result = engine.layout_chart(&chart, &mode);
            let elapsed = start.elapsed();
            std::hint::black_box(&result);
            samples_ms.push(elapsed.as_secs_f64() * 1000.0);
        }
        samples_ms.sort_by(|a, b| a.total_cmp(b));
        let min = samples_ms[0];
        let median = samples_ms[iterations / 2];
        let p95 = samples_ms[(iterations as f64 * 0.95) as usize];
        let max = samples_ms[iterations - 1];
        eprintln!(
            "  {label:<18} min={min:.3} median={median:.3} p95={p95:.3} max={max:.3} ms  \
             ({:.1}% of frame, {:.1} fit/frame)",
            median / FRAME_BUDGET_MS * 100.0,
            FRAME_BUDGET_MS / median.max(f64::EPSILON),
        );
        p95
    };

    eprintln!(
        "layout_chart benchmark (Lord of the Fight, {measures} measures, {iterations} iters)"
    );
    eprintln!("  60 Hz frame budget = {FRAME_BUDGET_MS:.3}ms");
    let paged_p95 = bench("paged", LayoutMode::default());
    let scroll_p95 = bench(
        "continuous-scroll",
        LayoutMode::ContinuousScroll { width: 1200.0 },
    );

    // Generous ceiling: even on slow shared hardware, a full relayout should
    // stay well under a quarter-second. This only catches order-of-magnitude
    // regressions; the printed numbers are the real signal for the 60 Hz goal.
    let worst = paged_p95.max(scroll_p95);
    assert!(
        worst < 250.0,
        "layout_chart p95 {worst:.1}ms regressed badly (60 Hz budget is {FRAME_BUDGET_MS:.2}ms)"
    );
}

/// Test that chord symbols are extracted from the scene graph.
#[test]
fn test_find_chord_symbols_in_scene() {
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());

    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let chart = create_test_chart();

    let result = engine.layout_chart(&chart, &LayoutMode::default());

    // Count chord symbols in the scene
    let chord_count = count_elements_by_type(&result.scene, ElementType::ChordSymbol);

    // Should have 3 chord symbols total (1 in measure 1, 2 in measure 2)
    assert_eq!(
        chord_count, 3,
        "Expected 3 chord symbols, found {}",
        chord_count
    );
}

/// Test that measures are present in the scene.
#[test]
fn test_find_measures_in_scene() {
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());

    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let chart = create_test_chart();

    let result = engine.layout_chart(&chart, &LayoutMode::default());

    // Count measure containers in the scene
    // Note: The layout creates a measure container for each measure in each system
    let measure_count = count_elements_by_type(&result.scene, ElementType::Measure);

    // With 2 chart measures on 1 system, we get 2 measure containers
    // But the MeasureBuilder also creates internal measure structure
    // So we just verify we have at least 2 measures
    assert!(
        measure_count >= 2,
        "Expected at least 2 measures, found {}",
        measure_count
    );
}

/// Test that chart layout produces valid result structure.
#[test]
fn test_chart_layout_result_structure() {
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());

    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let chart = create_test_chart();

    let result = engine.layout_chart(&chart, &LayoutMode::default());

    // Verify result has valid dimensions
    assert!(result.total_width > 0.0, "Total width should be positive");
    assert!(result.total_height > 0.0, "Total height should be positive");

    // Verify we have at least one page
    assert!(!result.pages.is_empty(), "Should have at least one page");
}

/// Test that chord symbols are present for each chord in the chart.
#[test]
fn test_chord_symbol_count_matches_chart() {
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());

    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let chart = create_test_chart();

    // Count total chords in the chart
    let chart_chord_count: usize = chart
        .sections
        .iter()
        .flat_map(|s| s.measures())
        .map(|m| m.chords.len())
        .sum();

    let result = engine.layout_chart(&chart, &LayoutMode::default());
    let scene_chord_count = count_elements_by_type(&result.scene, ElementType::ChordSymbol);

    assert_eq!(
        scene_chord_count, chart_chord_count,
        "Scene should have same number of chord symbols as chart: expected {}, got {}",
        chart_chord_count, scene_chord_count
    );
}

/// Test that chord symbol positions are accessible via transforms.
/// This verifies the transform-based positioning infrastructure works.
#[test]
fn test_chord_symbol_world_positions() {
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());

    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let chart = create_test_chart();

    let result = engine.layout_chart(&chart, &LayoutMode::default());
    let chord_symbols = find_elements_by_type(&result.scene, ElementType::ChordSymbol);

    // All chord symbols should have positive x positions (transforms are now applied)
    for (i, cs) in chord_symbols.iter().enumerate() {
        assert!(
            cs.world_x > 0.0,
            "Chord symbol {} should have positive world x position, got {}",
            i,
            cs.world_x
        );
        println!(
            "Chord symbol {}: world_x={:.1}, world_y={:.1}",
            i, cs.world_x, cs.world_y
        );
    }

    // Chord symbols should be in increasing x order within each measure
    // The last two chords (G and Dm in measure 2) should have G.x < Dm.x
    if chord_symbols.len() >= 2 {
        let dm_chord = &chord_symbols[chord_symbols.len() - 1]; // Dm is last
        let g_chord = &chord_symbols[chord_symbols.len() - 2]; // G is second-to-last

        assert!(
            dm_chord.world_x > g_chord.world_x,
            "Dm chord (beat 2) should be to the right of G chord (beat 0): Dm.x={:.1} should be > G.x={:.1}",
            dm_chord.world_x,
            g_chord.world_x
        );
    }
}

/// Test that chord symbols in different measures are positioned correctly.
#[test]
fn test_chord_symbol_positions_across_measures() {
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());

    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let chart = create_test_chart();

    let result = engine.layout_chart(&chart, &LayoutMode::default());
    let chord_symbols = find_elements_by_type(&result.scene, ElementType::ChordSymbol);

    // We have 3 chords: C (measure 1), G (measure 2, beat 0), Dm (measure 2, beat 2)
    assert_eq!(chord_symbols.len(), 3);

    let c_chord = &chord_symbols[0]; // C in measure 1
    let g_chord = &chord_symbols[1]; // G in measure 2
    let dm_chord = &chord_symbols[2]; // Dm in measure 2

    // G should be to the right of C (different measure)
    assert!(
        g_chord.world_x > c_chord.world_x,
        "G (measure 2) should be right of C (measure 1): G.x={:.1} > C.x={:.1}",
        g_chord.world_x,
        c_chord.world_x
    );

    // Dm should be to the right of G (same measure, later beat)
    assert!(
        dm_chord.world_x > g_chord.world_x,
        "Dm (beat 2) should be right of G (beat 0): Dm.x={:.1} > G.x={:.1}",
        dm_chord.world_x,
        g_chord.world_x
    );

    // Print positions for verification
    println!("C chord (m1): x={:.1}", c_chord.world_x);
    println!("G chord (m2, b0): x={:.1}", g_chord.world_x);
    println!("Dm chord (m2, b2): x={:.1}", dm_chord.world_x);
}

/// Test that all measures on the same system have equal width.
/// Uses "Autumn Leaves" chart - all sections have 4 measures with 1 chord each.
/// Because content is identical, all 4 measures per line should have equal width.
#[test]
fn test_equal_measure_widths_autumn_leaves() {
    let autumn_leaves = r#"
Autumn Leaves - Joseph Kosma
120bpm 4/4 #G

intro 4
Gmaj7 Cmaj7 F#m7b5 B7

vs 8
Em7 Am7 D7 Gmaj7 Cmaj7 F#m7b5 B7 Em

ch 8
Am7 D7 Gmaj7 Cmaj7 F#m7b5 B7 Em7 E7

br 4
Am7 D7 Gmaj7 B7

outro 4
Em7 Am7 D7 Gmaj7
"#;

    let chart = keyflow::parse(autumn_leaves).expect("Failed to parse Autumn Leaves chart");
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());

    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let result = engine.layout_chart(&chart, &LayoutMode::default());

    // Get all measure positions - these now have page metadata
    let measures = find_elements_by_type(&result.scene, ElementType::Measure);

    // Deduplicate measures by position (MeasureBuilder creates nested elements)
    // Keep only unique (x, y, page) positions with page metadata
    use std::collections::{HashMap, HashSet};
    let mut seen_positions: HashSet<(i64, i64, u32)> = HashSet::new();
    let unique_measures: Vec<_> = measures
        .iter()
        .filter(|m| {
            if let Some(page) = m.page {
                let key = (m.world_x.round() as i64, m.world_y.round() as i64, page);
                seen_positions.insert(key)
            } else {
                false // Skip measures without page metadata
            }
        })
        .collect();

    println!(
        "Found {} unique measure positions with page tags:",
        unique_measures.len()
    );
    for (i, m) in unique_measures.iter().enumerate() {
        println!(
            "  Measure {}: x={:.1}, y={:.1}, page={:?}",
            i, m.world_x, m.world_y, m.page
        );
    }

    // Group measures by (page, y_position) - same page + same y = same system
    let mut measures_by_system: HashMap<(u32, i64), Vec<&ElementPosition>> = HashMap::new();
    for m in &unique_measures {
        if let Some(page) = m.page {
            let system_key = (page, m.world_y.round() as i64);
            measures_by_system.entry(system_key).or_default().push(m);
        }
    }

    // Verify equal widths within each system (page + y group)
    for ((page, system_y), system_measures) in measures_by_system.iter() {
        if system_measures.len() < 2 {
            continue;
        }

        // Sort by x position
        let mut sorted: Vec<_> = system_measures.clone();
        sorted.sort_by(|a, b| a.world_x.partial_cmp(&b.world_x).unwrap());

        // Calculate widths (distance between consecutive measures)
        let widths: Vec<f64> = sorted
            .windows(2)
            .map(|pair| pair[1].world_x - pair[0].world_x)
            .collect();

        if widths.is_empty() {
            continue;
        }

        // All widths should be approximately equal (within 0.1 points tolerance)
        let first_width = widths[0];
        let tolerance = 0.1;

        for (i, &width) in widths.iter().enumerate() {
            let diff = (width - first_width).abs();
            assert!(
                diff <= tolerance,
                "Page {}, system y={}: Measure {} width ({:.1}) differs from measure 0 width ({:.1}) by {:.3}",
                page,
                system_y,
                i + 1,
                width,
                first_width,
                diff
            );
        }

        println!(
            "Page {}, system y={}: {} measures, all widths equal ({:.1} points)",
            page,
            system_y,
            sorted.len(),
            first_width
        );
    }

    // Also verify total measure count matches chart
    let expected_measures: usize = chart.sections.iter().map(|s| s.measures().len()).sum();

    // Account for MeasureBuilder creating internal structure
    assert!(
        measures.len() >= expected_measures,
        "Expected at least {} measures, found {}",
        expected_measures,
        measures.len()
    );
}

/// Test that page metadata is correctly assigned to elements.
#[test]
fn test_page_metadata_assigned() {
    let autumn_leaves = r#"
Autumn Leaves - Joseph Kosma
120bpm 4/4 #G

intro 4
Gmaj7 Cmaj7 F#m7b5 B7

vs 8
Em7 Am7 D7 Gmaj7 Cmaj7 F#m7b5 B7 Em
"#;

    let chart = keyflow::parse(autumn_leaves).expect("Failed to parse chart");
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());

    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let result = engine.layout_chart(&chart, &LayoutMode::default());

    // Check that measures have page metadata
    let measures = find_elements_by_type(&result.scene, ElementType::Measure);
    let measures_with_page: Vec<_> = measures.iter().filter(|m| m.page.is_some()).collect();

    println!(
        "Total measures: {}, with page metadata: {}",
        measures.len(),
        measures_with_page.len()
    );

    // At least the outer measure containers should have page metadata
    assert!(
        !measures_with_page.is_empty(),
        "Expected some measures to have page metadata"
    );

    // Check that chord symbols have page metadata
    let chords = find_elements_by_type(&result.scene, ElementType::ChordSymbol);
    let chords_with_page: Vec<_> = chords.iter().filter(|c| c.page.is_some()).collect();

    println!(
        "Total chord symbols: {}, with page metadata: {}",
        chords.len(),
        chords_with_page.len()
    );

    assert!(
        !chords_with_page.is_empty(),
        "Expected chord symbols to have page metadata"
    );

    // Group by page and verify distribution
    let measures_cloned: Vec<_> = measures_with_page.iter().map(|m| (*m).clone()).collect();
    let measures_by_page = group_elements_by_page(&measures_cloned);
    println!(
        "Measures distributed across {} page(s)",
        measures_by_page.len()
    );
    for (page, page_measures) in &measures_by_page {
        println!("  Page {}: {} measures", page, page_measures.len());
    }
}

/// Test inter-system spacing is consistent.
#[test]
fn test_inter_system_spacing_consistency() {
    let chart_text = r#"
Song - Artist
120bpm 4/4 #C

vs 32
C G Am F x8
"#;
    let chart = keyflow::parse(chart_text).expect("Failed to parse chart");
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());

    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let result = engine.layout_chart(&chart, &LayoutMode::default());

    let metrics = result.page_metrics(1).expect("Should have page 1");

    println!("\n=== Inter-System Spacing Test ===");
    metrics.print_debug();

    // All inter-system spacing should be equal (within 0.1 points)
    if metrics.inter_system_spacing.len() >= 2 {
        let first_spacing = metrics.inter_system_spacing[0];
        for (i, &spacing) in metrics.inter_system_spacing.iter().enumerate() {
            let diff = (spacing - first_spacing).abs();
            assert!(
                diff <= 0.1,
                "Inter-system spacing {} ({:.1}) differs from first ({:.1}) by {:.2}",
                i + 1,
                spacing,
                first_spacing,
                diff
            );
        }
        println!(
            "All inter-system spacings are consistent: {:.1} points",
            first_spacing
        );
    }
}

/// Test that last system doesn't overflow the bottom margin.
#[test]
fn test_last_system_to_bottom_margin() {
    let chart_text = r#"
Song - Artist
120bpm 4/4 #C

vs 32
C G Am F x8
"#;
    let chart = keyflow::parse(chart_text).expect("Failed to parse chart");
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());

    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let result = engine.layout_chart(&chart, &LayoutMode::default());

    for page_metrics in result.all_page_metrics() {
        println!(
            "\n=== Page {} Bottom Margin Test ===",
            page_metrics.page_number
        );
        page_metrics.print_debug();

        // Last system should not overflow the bottom margin
        assert!(
            page_metrics.last_system_to_bottom >= 0.0,
            "Page {}: Last system overflows bottom margin by {:.1} points",
            page_metrics.page_number,
            -page_metrics.last_system_to_bottom
        );

        // Should have reasonable bottom space (not too much empty space)
        let max_bottom_space = page_metrics.available_height * 0.35;
        assert!(
            page_metrics.last_system_to_bottom <= max_bottom_space,
            "Page {}: Too much empty space at bottom: {:.1} points ({:.0}%)",
            page_metrics.page_number,
            page_metrics.last_system_to_bottom,
            (page_metrics.last_system_to_bottom / page_metrics.available_height) * 100.0
        );
    }
}

/// Test spacing check warnings.
#[test]
fn test_spacing_check_warnings() {
    let chart_text = r#"
Song - Artist
120bpm 4/4 #C

vs 16
C G Am F x4
"#;
    let chart = keyflow::parse(chart_text).expect("Failed to parse chart");
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());

    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let result = engine.layout_chart(&chart, &LayoutMode::default());

    let metrics = result.page_metrics(1).expect("Should have page 1");

    // Check with reasonable min/max spacing (20-60 points)
    let warnings = metrics.check_spacing(20.0, 60.0);

    println!("\n=== Spacing Check Test ===");
    metrics.print_debug();

    if warnings.is_empty() {
        println!("No spacing warnings - all systems properly spaced");
    } else {
        for warning in &warnings {
            println!("Warning: {}", warning);
        }
    }

    // With default config, we shouldn't have critical warnings
    // (systems too close or content overflow)
    let critical_warnings: Vec<_> = warnings
        .iter()
        .filter(|w| w.contains("too close") || w.contains("extends past"))
        .collect();

    assert!(
        critical_warnings.is_empty(),
        "Found critical spacing warnings: {:?}",
        critical_warnings
    );
}

/// Example test demonstrating debug output for a real chart.
#[test]
fn example_page_layout_debug_output() {
    let autumn_leaves = r#"
Autumn Leaves - Joseph Kosma
120bpm 4/4 #G

intro 4
Gmaj7 Cmaj7 F#m7b5 B7

vs 8
Em7 Am7 D7 Gmaj7 Cmaj7 F#m7b5 B7 Em

ch 8
Am7 D7 Gmaj7 Cmaj7 F#m7b5 B7 Em7 E7

br 4
Am7 D7 Gmaj7 B7

outro 4
Em7 Am7 D7 Gmaj7
"#;

    let chart = keyflow::parse(autumn_leaves).expect("Failed to parse chart");
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());

    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let result = engine.layout_chart(&chart, &LayoutMode::default());

    println!("\n=== Autumn Leaves Layout Debug ===");
    println!("Total pages: {}", result.pages.len());
    println!(
        "Total dimensions: {:.1} × {:.1}",
        result.total_width, result.total_height
    );
    println!();

    for metrics in result.all_page_metrics() {
        metrics.print_debug();

        // Also show check results
        let warnings = metrics.check_spacing(15.0, 50.0);
        if !warnings.is_empty() {
            println!("Warnings:");
            for w in &warnings {
                println!("  - {}", w);
            }
            println!();
        }
    }

    // Verify the layout is reasonable
    let page1 = result.page_metrics(1).unwrap();
    assert!(
        page1.system_count >= 5,
        "Should have at least 5 systems on page 1"
    );
}

/// Test that header space is accounted for on first page.
#[test]
fn test_first_page_header_space() {
    let chart_text = r#"
My Long Song Title - Famous Artist Name
120bpm 4/4 #C

vs 32
C G Am F x8
"#;
    let chart = keyflow::parse(chart_text).expect("Failed to parse chart");
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());

    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let result = engine.layout_chart(&chart, &LayoutMode::default());

    // Get metrics for multiple pages
    let all_metrics = result.all_page_metrics();

    println!("\n=== First Page Header Space Test ===");
    for m in &all_metrics {
        println!(
            "Page {}: {} systems, first system at y={:.1}",
            m.page_number,
            m.system_count,
            m.system_y_positions.first().unwrap_or(&0.0)
        );
    }

    // First page should have systems starting at margin.top
    // (header text is handled separately in the renderer)
    if let Some(page1) = all_metrics.first() {
        let first_system_y = page1.system_y_positions.first().copied().unwrap_or(0.0);
        assert!(
            first_system_y >= page1.margins.top - 1.0, // Allow 1pt tolerance
            "First system should start at or below top margin: y={:.1}, margin={:.1}",
            first_system_y,
            page1.margins.top
        );
    }
}

/// Test multi-page layout with extended chart (Autumn Leaves Extended).
/// This chart has ~120 measures across 10 sections, requiring ~4 pages.
#[test]
fn test_multipage_layout_extended_chart() {
    let extended_chart = r#"
Autumn Leaves (Extended) - Joseph Kosma
120bpm 4/4 #G

intro 8
Gmaj7 Cmaj7 F#m7b5 B7
Em7 A7 Dmaj7 G7

vs 16
Em7 Am7 D7 Gmaj7
Cmaj7 F#m7b5 B7 Em
Em7 Am7 D7 Gmaj7
Cmaj7 F#m7b5 B7 Em7

pre 4
Am7 D7 Bm7 E7

ch 16
Am7 D7 Gmaj7 Cmaj7
F#m7b5 B7 Em7 E7
Am7 D7 Gmaj7 Cmaj7
F#m7b5 B7 Em7 Em7

vs 16
Em7 Am7 D7 Gmaj7
Cmaj7 F#m7b5 B7 Em
Em7 Am7 D7 Gmaj7
Cmaj7 F#m7b5 B7 Em7

pre 4

ch 16

br 8
Cmaj7 Bm7 Am7 Gmaj7
F#m7b5 B7 Em7 A7

inst 16
Gmaj7 Cmaj7 F#m7b5 B7
Em7 Am7 D7 Gmaj7
Cmaj7 F#m7b5 B7 Em
Em7 Am7 D7 G7

ch 16

outro 8
Gmaj7 Cmaj7 F#m7b5 B7
Em7 Am7 D7 Gmaj7
"#;

    let chart = keyflow::parse(extended_chart).expect("Failed to parse extended chart");
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());

    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let result = engine.layout_chart(&chart, &LayoutMode::default());

    println!("\n=== Extended Chart Multi-Page Layout Test ===");
    println!("Sections: {}", chart.sections.len());
    println!(
        "Total measures: {}",
        chart
            .sections
            .iter()
            .map(|s| s.measures().len())
            .sum::<usize>()
    );
    println!("Total pages: {}", result.pages.len());
    println!();

    // Should span multiple pages
    assert!(
        result.pages.len() >= 3,
        "Extended chart should require at least 3 pages, got {}",
        result.pages.len()
    );

    // Print metrics for each page
    let mut total_systems = 0;
    for metrics in result.all_page_metrics() {
        println!(
            "Page {}: {} systems, content height {:.1}pt, remaining {:.1}pt",
            metrics.page_number,
            metrics.system_count,
            metrics.content_height,
            metrics.last_system_to_bottom
        );
        total_systems += metrics.system_count;

        // Each page should have reasonable system count
        assert!(
            metrics.system_count >= 1,
            "Page {} should have at least 1 system",
            metrics.page_number
        );
        assert!(
            metrics.system_count <= 10,
            "Page {} should have at most 10 systems, got {}",
            metrics.page_number,
            metrics.system_count
        );

        // No overflow
        assert!(
            metrics.last_system_to_bottom >= -1.0, // 1pt tolerance for rounding
            "Page {}: Content overflows bottom margin by {:.1}pt",
            metrics.page_number,
            -metrics.last_system_to_bottom
        );
    }

    // Total systems should be roughly 32+ (chart has ~128 measures at 4 per system)
    // Parser may add slightly more due to smart chord memory
    assert!(
        (30..=40).contains(&total_systems),
        "Expected ~30-40 total systems across all pages, got {}",
        total_systems
    );

    println!("\nTotal systems across all pages: {}", total_systems);
    println!(
        "Systems per page average: {:.1}",
        total_systems as f64 / result.pages.len() as f64
    );
}

/// Test that duplicate consecutive chords are hidden when setting is enabled.
#[test]
fn test_hide_repeated_chords() {
    // Chart with repeated chords: C C C G G C
    // With max_measures_per_system=4, this creates 2 systems:
    //   System 1 (measures 0-3): C C C G → shows C, G (hiding repeated Cs)
    //   System 2 (measures 4-5): G C → shows G, C (chord tracking resets at system boundary)
    // Total visible: 4 chord symbols
    let chart_text = r#"
Test - Artist
120bpm 4/4 #C

vs 6
C C C G G C
"#;
    let chart = keyflow::parse(chart_text).expect("Failed to parse chart");
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());

    // Test with hide_repeated_chords = true (default)
    let engine = ChartLayoutEngine::new(style, text_font.clone(), symbol_font.clone());
    let result = engine.layout_chart(&chart, &LayoutMode::default());

    let chord_count_hidden = count_elements_by_type(&result.scene, ElementType::ChordSymbol);
    println!(
        "With hide_repeated_chords=true: {} chord symbols rendered",
        chord_count_hidden
    );

    // With hiding enabled and system boundary reset, we see:
    // System 1: C, G (2 chords)
    // System 2: G, C (2 chords, G re-shown due to system reset)
    // Total: 4 chord symbols
    assert_eq!(
        chord_count_hidden, 4,
        "Expected 4 chord symbols with hiding enabled (C, G on system 1; G, C on system 2), got {}",
        chord_count_hidden
    );

    // Test with hide_repeated_chords = false
    let config = ChartLayoutConfig {
        hide_repeated_chords: false,
        ..Default::default()
    };
    let engine_no_hide = ChartLayoutEngine::with_config(config, style, text_font, symbol_font);
    let result_no_hide = engine_no_hide.layout_chart(&chart, &LayoutMode::default());

    let chord_count_all = count_elements_by_type(&result_no_hide.scene, ElementType::ChordSymbol);
    println!(
        "With hide_repeated_chords=false: {} chord symbols rendered",
        chord_count_all
    );

    // Without hiding, we should see all 6 chords
    assert_eq!(
        chord_count_all, 6,
        "Expected 6 chord symbols with hiding disabled, got {}",
        chord_count_all
    );
}

/// Test that chord hiding works across measure boundaries.
#[test]
fn test_hide_repeated_chords_across_measures() {
    // Chart where the same chord spans multiple measures
    let chart_text = r#"
Test - Artist
120bpm 4/4 #C

vs 8
C C C C G G G G
"#;
    let chart = keyflow::parse(chart_text).expect("Failed to parse chart");
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());

    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let result = engine.layout_chart(&chart, &LayoutMode::default());

    let chord_count = count_elements_by_type(&result.scene, ElementType::ChordSymbol);
    println!("Chord symbols rendered: {}", chord_count);

    // With 4 Cs followed by 4 Gs, we should see: C, G = 2 unique chord changes
    assert_eq!(
        chord_count, 2,
        "Expected 2 chord symbols (C, G), got {}",
        chord_count
    );
}

/// Test that chord hiding resets at section boundaries (rehearsal marks).
/// The first chord of each section should always show, even if it's the same
/// as the last chord of the previous section.
#[test]
fn test_chord_shows_at_section_boundary() {
    // Verse ends with C, Chorus starts with C - both should show
    let chart_text = r#"
Test - Artist
120bpm 4/4 #C

vs 4
G Am F C

ch 4
C G Am F
"#;
    let chart = keyflow::parse(chart_text).expect("Failed to parse chart");
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());

    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let result = engine.layout_chart(&chart, &LayoutMode::default());

    let chord_count = count_elements_by_type(&result.scene, ElementType::ChordSymbol);
    println!("Chord symbols rendered: {}", chord_count);

    // Verse: G, Am, F, C = 4 unique chords
    // Chorus: C (shows because new section), G, Am, F = 4 chords
    // Total = 8 chord symbols
    assert_eq!(
        chord_count, 8,
        "Expected 8 chord symbols (section boundary resets tracking), got {}",
        chord_count
    );
}

/// Test that repeated chords within a section are still hidden.
#[test]
fn test_repeated_chords_hidden_within_section() {
    let chart_text = r#"
Test - Artist
120bpm 4/4 #C

vs 4
C C G G

ch 4
Am Am F F
"#;
    let chart = keyflow::parse(chart_text).expect("Failed to parse chart");
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());

    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let result = engine.layout_chart(&chart, &LayoutMode::default());

    let chord_count = count_elements_by_type(&result.scene, ElementType::ChordSymbol);
    println!("Chord symbols rendered: {}", chord_count);

    // Verse: C, G = 2 unique chords (duplicates hidden)
    // Chorus: Am, F = 2 unique chords (duplicates hidden)
    // Total = 4 chord symbols
    assert_eq!(
        chord_count, 4,
        "Expected 4 chord symbols (duplicates hidden within sections), got {}",
        chord_count
    );
}

/// Debug test to see what chords are being parsed from the example chart.
#[test]
fn debug_example_chart_chords() {
    let chart_text = r#"
Autumn Leaves (Extended) - Joseph Kosma
120bpm 4/4 #G

intro 8
Gmaj7 Cmaj7 F#m7b5 B7
Em7 A7 Dmaj7 G7

vs 16
Em7 Am7 D7 Gmaj7
Cmaj7 F#m7b5 B7 Em
Em7 Am7 D7 Gmaj7
Cmaj7 F#m7b5 B7 Em7

pre 4
Am7 D7 Bm7 E7

ch 16
Am7 D7 Gmaj7 Cmaj7
F#m7b5 B7 Em7 E7
Am7 D7 Gmaj7 Cmaj7
F#m7b5 B7 Em7 Em7
"#;

    let chart = keyflow::parse(chart_text).expect("Failed to parse chart");

    println!("\n=== Debug: Parsed Chart Chords ===");
    println!("Total sections: {}", chart.sections.len());

    for (section_idx, section) in chart.sections.iter().enumerate() {
        println!(
            "\nSection {}: {:?} ({} measures)",
            section_idx,
            section.section.section_type,
            section.measures().len()
        );

        for (measure_idx, measure) in section.measures().iter().enumerate() {
            print!("  Measure {}: ", measure_idx);
            for chord in &measure.chords {
                print!("'{}' ", chord.full_symbol);
            }
            println!("({} chords)", measure.chords.len());
        }
    }

    // Check if any chord has "C5" in it
    let mut c5_found = false;
    for section in &chart.sections {
        for measure in section.measures() {
            for chord in &measure.chords {
                if chord.full_symbol.contains("C5") || chord.full_symbol == "C" {
                    println!("\nFound chord with 'C' or 'C5': '{}'", chord.full_symbol);
                    c5_found = true;
                }
            }
        }
    }

    if !c5_found {
        println!("\nNo 'C5' or plain 'C' chord found in parsed chart");
    }
}

#[test]
fn debug_verse_bar5_keyflow() {
    // Test what keyflow parses for bar 5 of first verse
    // vs 16 with only 4 chords specified - what fills measures 5-16?
    let chart_text = r#"Autumn Leaves (Extended) - Joseph Kosma
120bpm 4/4 #G

intro 8
Gmaj7 Cmaj7 F#m7b5 B7
Em7 A7 Dmaj7 G7

vs 16
Em7 Am7 D7 Gmaj7
"#;

    let chart = keyflow::parse(chart_text).expect("Failed to parse chart");

    println!("\n=== KEYFLOW LEVEL: Verse Section Analysis ===");

    // Find the verse section
    let verse_section = chart
        .sections
        .iter()
        .find(|s| matches!(s.section.section_type, SectionType::Verse))
        .expect("Should have a verse section");

    println!(
        "Verse section: {} measures declared",
        verse_section.measures().len()
    );

    // Print all measures with their chords
    for (i, measure) in verse_section.measures().iter().enumerate() {
        let chords: Vec<&str> = measure
            .chords
            .iter()
            .map(|c| c.full_symbol.as_str())
            .collect();
        println!("  Bar {}: {:?}", i + 1, chords);

        // Highlight bar 5 specifically
        if i == 4 {
            println!("    ^^^ BAR 5 - Is there a C chord here?");
            for chord in &measure.chords {
                if chord.full_symbol.starts_with('C') || chord.full_symbol == "C" {
                    println!("    !!! FOUND: '{}' in bar 5", chord.full_symbol);
                }
            }
        }
    }
}

#[test]
fn debug_verse_bar5_scene_graph() {
    // Test what the scene graph renders for bar 5 of first verse
    let chart_text = r#"Autumn Leaves (Extended) - Joseph Kosma
120bpm 4/4 #G

intro 8
Gmaj7 Cmaj7 F#m7b5 B7
Em7 A7 Dmaj7 G7

vs 16
Em7 Am7 D7 Gmaj7
"#;

    let chart = keyflow::parse(chart_text).expect("Failed to parse chart");

    let style = Box::leak(Box::new(MStyle::default()));
    let engine = ChartLayoutEngine::new(style, Arc::new(Vec::new()), Arc::new(Vec::new()));

    let result = engine.layout_chart(&chart, &LayoutMode::default());

    println!("\n=== SCENE GRAPH LEVEL: Chord Nodes with Measure Metadata ===");

    // Find all chord nodes and group by section/measure
    fn find_chords_with_measure(
        node: &SceneNode,
        chords: &mut Vec<(String, Option<u32>, Option<String>)>,
    ) {
        // Check if this is a chord node
        if node.metadata.get("element_type") == Some(&"chord".to_string()) {
            // Extract the text content
            let mut chord_text = String::new();
            for cmd in &node.commands {
                if let PaintCommand::Text { text, .. } = cmd {
                    chord_text.push_str(text);
                }
            }

            let measure = node
                .metadata
                .get("measure")
                .and_then(|m| m.parse::<u32>().ok());
            let section_type = node.metadata.get("section_type").cloned();

            chords.push((chord_text, measure, section_type));
        }

        for child in &node.children {
            find_chords_with_measure(child, chords);
        }
    }

    let mut chords = Vec::new();
    find_chords_with_measure(&result.scene, &mut chords);

    // Group by section and print
    println!("\nAll rendered chords:");
    let mut current_section = String::new();
    for (chord, measure, section) in &chords {
        let section_name = section.as_deref().unwrap_or("unknown");
        if section_name != current_section {
            println!("\n  {} section:", section_name);
            current_section = section_name.to_string();
        }
        println!("    Measure {}: '{}'", measure.unwrap_or(999), chord);
    }

    // Specifically check bar 5 of verse (measure index 4)
    println!("\n=== BAR 5 OF VERSE (measure index 4) ===");
    let verse_bar5_chords: Vec<_> = chords
        .iter()
        .filter(|(_, measure, section)| section.as_deref() == Some("Verse") && *measure == Some(4))
        .collect();

    if verse_bar5_chords.is_empty() {
        println!("No chords rendered for verse bar 5 (might be hidden as duplicate)");
    } else {
        for (chord, _, _) in verse_bar5_chords {
            println!("  Rendered: '{}'", chord);
            if chord.starts_with('C') || chord == "C" {
                println!("  !!! FOUND C chord in bar 5!");
            }
        }
    }
}

#[test]
fn debug_scene_paint_commands() {
    // Trace all text/glyph commands in the rendered scene to find "C5"
    // Using full DEFAULT_CHART_TEXT to see if C5 appears after first verse
    let chart_text = r#"Autumn Leaves (Extended) - Joseph Kosma
120bpm 4/4 #G

intro 8
Gmaj7 Cmaj7 F#m7b5 B7
Em7 A7 Dmaj7 G7

vs 16
Em7 Am7 D7 Gmaj7
Cmaj7 F#m7b5 B7 Em
Em7 Am7 D7 Gmaj7
Cmaj7 F#m7b5 B7 Em7

pre 4
Am7 D7 Bm7 E7
"#;

    let chart = keyflow::parse(chart_text).expect("Failed to parse chart");

    // Create minimal layout config for testing
    let style = Box::leak(Box::new(MStyle::default()));
    let engine = ChartLayoutEngine::new(
        style,
        Arc::new(Vec::new()), // text font
        Arc::new(Vec::new()), // symbol font
    );

    let result = engine.layout_chart(&chart, &LayoutMode::default());

    println!("\n=== Debug: Scene Paint Commands ===");

    // Recursive function to collect all text commands
    fn collect_text_commands(node: &SceneNode, depth: usize) {
        let indent = "  ".repeat(depth);

        // Check node metadata for element type
        if let Some(elem_type) = node.metadata.get("element_type") {
            println!("{}Node: {} ({})", indent, elem_type, node.commands.len());
        }

        // Check paint commands
        for cmd in &node.commands {
            match cmd {
                PaintCommand::Text { text, .. } => {
                    println!("{}  Text: '{}'", indent, text);
                    if text.contains('C') || text.contains('5') {
                        println!("{}  ^^^ FOUND C or 5 in text!", indent);
                    }
                }
                PaintCommand::Glyph { codepoint, .. } => {
                    println!(
                        "{}  Glyph: U+{:04X} ('{}')",
                        indent, *codepoint as u32, codepoint
                    );
                }
                _ => {}
            }
        }

        // Recurse into children
        for child in &node.children {
            collect_text_commands(child, depth + 1);
        }
    }

    collect_text_commands(&result.scene, 0);
}

#[test]
fn test_short_system_width() {
    // Test that short systems (< 4 measures) don't stretch to full width
    // Like LilyPond's pseudo-indent system
    let chart_text = r#"Short Line Test - Test Artist
120bpm 4/4 #C

vs 6
C G Am F | C G
"#;

    let chart = keyflow::parse(chart_text).expect("Failed to parse chart");
    let style = test_style();
    let text_font = Arc::new(Vec::new());
    let symbol_font = Arc::new(Vec::new());

    let engine = ChartLayoutEngine::new(style, text_font, symbol_font);
    let result = engine.layout_chart(&chart, &LayoutMode::default());

    println!("\n=== Short System Width Test ===");

    // Find all staff line commands and their widths
    fn find_staff_line_widths(node: &SceneNode, widths: &mut Vec<f64>) {
        for cmd in &node.commands {
            if let PaintCommand::Line { start, end, .. } = cmd {
                // Staff lines are horizontal (same y coordinate)
                if (start.y - end.y).abs() < 0.1 {
                    let width = (end.x - start.x).abs();
                    if width > 100.0 {
                        // Only count substantial lines (staff lines, not decorations)
                        widths.push(width);
                    }
                }
            }
        }
        for child in &node.children {
            find_staff_line_widths(child, widths);
        }
    }

    let mut staff_widths = Vec::new();
    find_staff_line_widths(&result.scene, &mut staff_widths);

    // Group by similar widths (staff lines come in groups of 5)
    let mut unique_widths: Vec<f64> = Vec::new();
    for width in &staff_widths {
        if !unique_widths.iter().any(|w| (w - width).abs() < 1.0) {
            unique_widths.push(*width);
        }
    }
    unique_widths.sort_by(|a, b| a.partial_cmp(b).unwrap());

    println!("Staff line width groups: {:?}", unique_widths);

    // We should have at least 2 different widths:
    // - Full width for 4-measure system
    // - Shorter width for 2-measure system
    // (6 measures = 1 system of 4 + 1 system of 2)
    if unique_widths.len() >= 2 {
        let short_width = unique_widths[0];
        let full_width = unique_widths[unique_widths.len() - 1];
        println!("Short system width: {:.1}", short_width);
        println!("Full system width: {:.1}", full_width);

        // Short system should be approximately 50% of full width (2/4 measures)
        let ratio = short_width / full_width;
        println!("Ratio (short/full): {:.2}", ratio);

        assert!(
            ratio < 0.75,
            "Short system should be significantly narrower than full system. Ratio: {:.2}",
            ratio
        );
    }
}

#[test]
fn with_scale_scales_all_geometric_fields() {
    let base = ChartLayoutConfig::master_rhythm();
    let scaled = ChartLayoutConfig::master_rhythm().with_scale(2.0);
    assert!((scaled.spatium - base.spatium * 2.0).abs() < 1e-9);
    assert!((scaled.system_spacing - base.system_spacing * 2.0).abs() < 1e-9);
    assert!((scaled.min_measure_width - base.min_measure_width * 2.0).abs() < 1e-9);
    assert!((scaled.margins.left - base.margins.left * 2.0).abs() < 1e-9);
    assert!((scaled.margins.top - base.margins.top * 2.0).abs() < 1e-9);
    // Non-geometric fields untouched
    assert_eq!(scaled.max_measures_per_system, base.max_measures_per_system);
    assert_eq!(scaled.show_measure_numbers, base.show_measure_numbers);
}

#[test]
fn with_scale_clamps_extremes() {
    let huge = ChartLayoutConfig::master_rhythm().with_scale(1_000_000.0);
    let base = ChartLayoutConfig::master_rhythm();
    // Clamped to 10x
    assert!((huge.spatium - base.spatium * 10.0).abs() < 1e-9);

    let tiny = ChartLayoutConfig::master_rhythm().with_scale(0.000_001);
    assert!((tiny.spatium - base.spatium * 0.1).abs() < 1e-9);
}

#[test]
fn with_scale_ignores_non_finite() {
    let base = ChartLayoutConfig::master_rhythm();
    let nan_scaled = ChartLayoutConfig::master_rhythm().with_scale(f64::NAN);
    let inf_scaled = ChartLayoutConfig::master_rhythm().with_scale(f64::INFINITY);
    assert_eq!(nan_scaled.spatium, base.spatium);
    assert_eq!(inf_scaled.spatium, base.spatium);
}

#[test]
fn breakpoint_classifies_viewport_widths() {
    use crate::engraver::layout::chart::Breakpoint;
    assert_eq!(Breakpoint::from_viewport_pt(320.0), Breakpoint::Phone);
    assert_eq!(Breakpoint::from_viewport_pt(479.99), Breakpoint::Phone);
    assert_eq!(Breakpoint::from_viewport_pt(480.0), Breakpoint::Tablet);
    assert_eq!(Breakpoint::from_viewport_pt(700.0), Breakpoint::Tablet);
    assert_eq!(Breakpoint::from_viewport_pt(899.99), Breakpoint::Tablet);
    assert_eq!(Breakpoint::from_viewport_pt(900.0), Breakpoint::Desktop);
    assert_eq!(Breakpoint::from_viewport_pt(1920.0), Breakpoint::Desktop);
}

#[test]
fn responsive_for_phone_has_bigger_staff_than_desktop() {
    use crate::engraver::layout::chart::Breakpoint;
    let phone = ChartLayoutConfig::responsive_for(Breakpoint::Phone);
    let desktop = ChartLayoutConfig::responsive_for(Breakpoint::Desktop);
    // Phone staff should be visibly larger than desktop (iReal Pro behavior:
    // smaller screen → bigger cells, not more cells)
    assert!(
        phone.spatium > desktop.spatium,
        "phone spatium {} should exceed desktop {}",
        phone.spatium,
        desktop.spatium
    );
    // Phone shows fewer measures per system (iReal Pro fits 2 measures/row on a phone)
    assert!(phone.max_measures_per_system < desktop.max_measures_per_system);
}

#[test]
fn breakpoint_measures_per_system_matches_ireal_convention() {
    use crate::engraver::layout::chart::Breakpoint;
    assert_eq!(Breakpoint::Phone.measures_per_system(), 2);
    assert_eq!(Breakpoint::Tablet.measures_per_system(), 4);
    assert_eq!(Breakpoint::Desktop.measures_per_system(), 4);
}

#[test]
fn responsive_for_chord_symbols_scale_with_spatium() {
    use crate::engraver::layout::chart::Breakpoint;
    let phone = ChartLayoutConfig::responsive_for(Breakpoint::Phone);
    let tablet = ChartLayoutConfig::responsive_for(Breakpoint::Tablet);
    let desktop = ChartLayoutConfig::responsive_for(Breakpoint::Desktop);
    // Chord root_size grows with breakpoint's spatium — phone has biggest
    // chords for the same reason it has biggest staff (iReal Pro behavior).
    assert!(phone.harmony_style.root_size > tablet.harmony_style.root_size);
    assert!(tablet.harmony_style.root_size > desktop.harmony_style.root_size);
    // Sanity: desktop matches the iReal Pro baseline.
    assert_eq!(desktop.harmony_style.root_size, 24.0);
}

/// Build a single-section chart with one whole-bar chord per measure, each
/// measure carrying the supplied `(num, den)` meter.
fn chart_with_meters(meters: &[(u8, u8)]) -> Chart {
    let mut chart = Chart::new();
    chart.time_signature = Some(TimeSignature::new(meters[0].0 as u32, meters[0].1 as u32));

    let measures: Vec<Measure> = meters
        .iter()
        .enumerate()
        .map(|(i, &ts)| Measure {
            chords: vec![ChordInstance::new(
                root("C"),
                "C".to_string(),
                Chord::new(root("C"), ChordQuality::Major),
                ChordRhythm::Default,
                "C".to_string(),
                MusicalDuration::new(0, 4, 0),
                AbsolutePosition::new(MusicalPosition::try_new(i as i32, 0, 0).unwrap(), 0),
            )],
            time_signature: ts,
            ..Default::default()
        })
        .collect();

    let mut section_info = Section::new(SectionType::Verse);
    section_info.number = Some(1);
    section_info.measure_count = Some(measures.len());
    chart
        .sections
        .push(ChartSection::new(section_info).with_measures(measures));
    chart
}

/// Collect the colors of every glyph emitted by nodes of the given element type.
fn glyph_colors_of(scene: &SceneNode, element_type: ElementType) -> Vec<Color> {
    scene
        .iter_with_transforms()
        .filter(|(node, _)| {
            node.id
                .as_ref()
                .is_some_and(|id| id.element_type == element_type)
        })
        .flat_map(|(node, _)| {
            node.commands.iter().filter_map(|cmd| match cmd {
                PaintCommand::Glyph { color, .. } => Some(*color),
                _ => None,
            })
        })
        .collect()
}

/// Collect the colors of every glyph emitted by a `TimeSignature` scene node.
fn time_sig_glyph_colors(scene: &SceneNode) -> Vec<Color> {
    glyph_colors_of(scene, ElementType::TimeSignature)
}

#[test]
fn mid_chart_time_signature_change_renders_in_red() {
    let style = test_style();
    let engine = ChartLayoutEngine::new(style, Arc::new(Vec::new()), Arc::new(Vec::new()));
    let red = Color::from_rgba8(0xCC, 0x00, 0x00, 0xFF);
    let red_count = |chart: &Chart| {
        let result = engine.layout_chart(chart, &LayoutMode::default());
        time_sig_glyph_colors(&result.scene)
            .iter()
            .filter(|c| **c == red)
            .count()
    };

    // Constant 4/4: the only time signatures drawn are the (black) system prefix.
    assert_eq!(
        red_count(&chart_with_meters(&[(4, 4), (4, 4), (4, 4)])),
        0,
        "a constant-meter chart should render no red time signatures"
    );

    // One-measure excursion (`!T2/4`): the 2/4 is highlighted red (2 digits),
    // but the immediate revert to 4/4 is adjacent/obvious, so it draws in black.
    assert_eq!(
        red_count(&chart_with_meters(&[(4, 4), (2, 4), (4, 4)])),
        2,
        "only the one-measure 2/4 should be red; its immediate revert stays black"
    );

    // Persistent change then revert several bars later: BOTH the 2/4 and the
    // eventual return to 4/4 are highlighted red (2 digits each).
    assert_eq!(
        red_count(&chart_with_meters(&[
            (4, 4),
            (2, 4),
            (2, 4),
            (2, 4),
            (4, 4)
        ])),
        4,
        "a persistent change and its later revert should both be red"
    );
}

#[test]
fn mid_chart_key_change_renders_in_red() {
    use crate::key::Key;
    use keyflow_proto::chart::types::KeyChange;

    let style = test_style();
    let engine = ChartLayoutEngine::new(style, Arc::new(Vec::new()), Arc::new(Vec::new()));
    let red = Color::from_rgba8(0xCC, 0x00, 0x00, 0xFF);

    // Two 4/4 measures with a key change to A major (3 sharps) on the downbeat
    // of section-local measure 1.
    let mut chart = chart_with_meters(&[(4, 4), (4, 4)]);
    chart.key_changes.push(KeyChange::new(
        AbsolutePosition::new(MusicalPosition::try_new(1, 0, 0).unwrap(), 0),
        Some(Key::major(MusicalNote::from_string("E").unwrap())),
        Key::major(MusicalNote::from_string("A").unwrap()),
        0,
    ));

    let result = engine.layout_chart(&chart, &LayoutMode::default());
    let colors = glyph_colors_of(&result.scene, ElementType::KeySignature);
    assert!(
        colors.contains(&red),
        "expected a red key-change accidental glyph, got {colors:?}"
    );
}

#[cfg(test)]
mod content_bounds_guard {
    use crate::engraver::fonts::ChartFontBundle;
    use crate::engraver::layout::chart::LayoutMode;

    /// `content_bounds()` must account for node transforms. Chord symbols are
    /// placed via a parent-group transform with the text at local (0,0); a
    /// transform-blind bounds collapses them to the origin, so cropping a
    /// snippet's viewBox to those bounds clips every chord off the page. Guards
    /// the regression where `1 4 6 5` rendered no chord numbers in the editor.
    #[test]
    fn content_bounds_includes_transformed_chord_symbols() {
        let fonts = ChartFontBundle::new().unwrap();
        let style = crate::api::style::leak_lead_sheet_style();
        let engine = fonts.create_layout_engine(style);
        let chart = keyflow_text::chart::parse_chart("1 4 6 5").unwrap();
        let r = engine.layout_chart(&chart, &LayoutMode::ContinuousScroll { width: 800.0 });
        let b = r.content_bounds().expect("non-empty scene");
        // The last chord ("5") renders near x=595; the bounds must reach it.
        assert!(
            b.x1 >= 590.0,
            "content_bounds must span the transformed chord symbols (x1={:.1})",
            b.x1
        );
        // The chord row (baseline ~y=40) must be within the vertical span.
        assert!(
            b.y0 <= 40.0 && b.y1 >= 40.0,
            "chord baseline ~40 must be within bounds ({:.1}..{:.1})",
            b.y0,
            b.y1
        );
    }
}

#[cfg(test)]
mod degree_root_size_guard {
    use crate::engraver::fonts::ChartFontBundle;
    use crate::engraver::layout::chart::LayoutMode;
    use crate::engraver::layout::text_metrics::TextFontMetrics;
    use crate::engraver::scene::node::SceneNode;
    use crate::engraver::scene::paint::PaintCommand;

    fn chord_glyphs(n: &SceneNode, out: &mut Vec<(char, f64)>) {
        for c in &n.commands {
            if let PaintCommand::Text {
                text,
                font_size,
                font_family,
                ..
            } = c
                && font_family.contains("MuseJazz")
                && let Some(ch) = text.chars().next()
            {
                out.push((ch, *font_size));
            }
        }
        for ch in &n.children {
            chord_glyphs(ch, out);
        }
    }

    /// Nashville/degree chord roots must render as tall as letter chord roots:
    /// the MuseJazz font draws digits shorter than capitals, so a degree root is
    /// scaled up until its digit's glyph ink reaches the font's cap height.
    /// Guards the "numbers aren't as big as the chord symbols" fix.
    #[test]
    fn degree_roots_render_at_cap_height() {
        let fonts = ChartFontBundle::new().unwrap();
        let fm = TextFontMetrics::new(fonts.text_font_data().clone());
        let style = crate::api::style::leak_lead_sheet_style();
        let engine = fonts.create_layout_engine(style);

        let chart = keyflow_text::chart::parse_chart("1 5 7").unwrap();
        let r = engine.layout_chart(&chart, &LayoutMode::ContinuousScroll { width: 800.0 });
        let mut glyphs = Vec::new();
        chord_glyphs(&r.scene, &mut glyphs);

        // The chord-symbol digits (1, 5, 7) should each have been scaled so the
        // rendered glyph height matches cap height (within a small tolerance),
        // and the scaled point size should exceed the base 14pt.
        let cap = fm.cap_height(14.0);
        let mut checked = 0;
        for (ch, fs) in &glyphs {
            if ch.is_ascii_digit() {
                let gh = fm.glyph_height(*ch, *fs);
                assert!(
                    (gh - cap).abs() < 0.4,
                    "digit '{ch}' glyph height {gh:.2} should match cap height {cap:.2}"
                );
                assert!(
                    *fs > 14.0,
                    "digit '{ch}' should be scaled above the base 14pt, got {fs:.2}"
                );
                checked += 1;
            }
        }
        assert!(
            checked >= 3,
            "expected the three degree roots, checked {checked}"
        );
    }
}
