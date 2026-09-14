//! The styx schemas describe the files the Facet types read: the golden
//! fixtures validate against them, and a document the types would
//! reject is rejected by the schema too (the negative control).
//!
//! r[verify flow.patch-list.plan]
//! r[verify flow.patch-list.studio-profiles]

use facet_styx::SchemaFile;

const ALBUM: &str = include_str!("../fixtures/album/patch-list.styx");
const ALBUM_SCHEMA: &str = include_str!("../schema/patch-list.schema.styx");
const ROOM: &str = include_str!("../fixtures/studios/golden-room.styx");
const ROOM_SCHEMA: &str = include_str!("../schema/studio.schema.styx");

fn schema(text: &str) -> SchemaFile {
    facet_styx::from_str(text).expect("the schema file parses as a schema")
}

fn errors(schema: &SchemaFile, doc: &str) -> Vec<String> {
    let doc = styx_tree::parse(doc).expect("the document parses");
    facet_styx::validate(&doc, schema)
        .errors
        .iter()
        .map(|e| format!("{:?} at {}", e.kind, e.path))
        .collect()
}

#[test]
fn the_fixture_album_validates_against_the_patch_list_schema() {
    let schema = schema(ALBUM_SCHEMA);
    let errors = errors(&schema, ALBUM);
    assert!(errors.is_empty(), "schema errors: {errors:#?}");
}

#[test]
fn a_bus_without_an_output_fails_the_patch_list_schema() {
    let schema = schema(ALBUM_SCHEMA);
    let errors = errors(&schema, "headphones {\n    cody {for (cody)}\n}");
    assert!(
        errors.iter().any(|e| e.contains("MissingField")),
        "expected a missing-field error, got {errors:#?}"
    );
}

#[test]
fn the_fixture_profile_validates_against_the_studio_schema() {
    let schema = schema(ROOM_SCHEMA);
    let errors = errors(&schema, ROOM);
    assert!(errors.is_empty(), "schema errors: {errors:#?}");
}

#[test]
fn an_input_with_no_channel_fails_the_studio_schema() {
    let schema = schema(ROOM_SCHEMA);
    let errors = errors(&schema, "inputs {\n    \"DI 1\" @audio{}\n}");
    assert!(
        errors.iter().any(|e| e.contains("MissingField")),
        "expected a missing-field error, got {errors:#?}"
    );
}

#[test]
fn an_output_with_one_side_fails_the_studio_schema() {
    let schema = schema(ROOM_SCHEMA);
    let errors = errors(&schema, "outputs {\n    \"HP 1\" {left 0}\n}");
    assert!(
        errors.iter().any(|e| e.contains("MissingField")),
        "expected a missing-field error, got {errors:#?}"
    );
}

#[test]
fn an_unknown_top_level_key_fails_the_patch_list_schema() {
    let schema = schema(ALBUM_SCHEMA);
    let errors = errors(&schema, "performers {}\nkits {}");
    assert!(
        errors.iter().any(|e| e.contains("UnknownField")),
        "expected an unknown-field error, got {errors:#?}"
    );
}
