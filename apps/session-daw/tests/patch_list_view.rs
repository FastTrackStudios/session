//! The Patch List view over the fixture album in the fixture room.
//!
//! Two seams: the table the view is built from (rows in, rows out —
//! grouped by performer in the list's order, unresolved roles marked),
//! and the picture, compared byte for byte with the PNG the bench
//! wrote (`just daw-patch-list`), which is the standard every scene
//! fixture holds to (spec #48, Testing Decisions).
//!
//! r[verify flow.patch-list.plan]
//! r[verify flow.patch-list.studio-profiles]

use session_daw::patch_list::{Mark, PerformerRows, Row, Table};

type Result = std::result::Result<(), Box<dyn std::error::Error>>;

/// The committed picture, at the size the recipe renders.
const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/patch-list.png");
const SIZE: (u32, u32) = (1280, 1280);

fn performer<'a>(table: &'a Table, name: &str) -> Option<&'a PerformerRows> {
    table.performers.iter().find(|p| p.name == name)
}

fn row<'a>(performer: &'a PerformerRows, key: &str) -> Option<&'a Row> {
    performer.rows.iter().find(|r| r.key == key)
}

#[test]
fn the_table_groups_source_kinds_by_performer_in_list_order() -> Result {
    let table = Table::fixture()?;
    let performers: Vec<&str> = table.performers.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        performers,
        [
            "cody", "john", "ron", "belen", "aline", "drummer", "bassist", "engineer", "producer"
        ]
    );

    let cody = performer(&table, "cody").ok_or("cody")?;
    assert_eq!(cody.rows.len(), 6);
    let di = row(cody, "di").ok_or("cody's DI")?;
    assert_eq!(di.kind, "guitar");
    assert_eq!(di.role, "DI 3");
    assert_eq!(di.input, "ch 3");
    assert_eq!(di.mark, Mark::Resolved);

    // A kit reads piece then mic, in the list's order.
    let drummer = performer(&table, "drummer").ok_or("the drummer")?;
    let keys: Vec<&str> = drummer.rows.iter().map(|r| r.key.as_str()).collect();
    assert_eq!(
        &keys[..4],
        ["kick/in", "kick/out", "kick/trig", "snare/top"]
    );
    let snare = row(drummer, "snare/top").ok_or("the snare top")?;
    assert_eq!(snare.input, "ch 28");

    // A MIDI entry says so.
    let john = performer(&table, "john").ok_or("john")?;
    let synth = row(john, "synth").ok_or("john's synth")?;
    assert_eq!(synth.input, "MIDI Prophet-6 ch 1");
    assert_eq!(synth.mark, Mark::Resolved);
    Ok(())
}

#[test]
fn an_unresolved_role_is_marked_not_dropped() -> Result {
    let table = Table::fixture()?;
    let bassist = performer(&table, "bassist").ok_or("the bassist")?;
    let amp = row(bassist, "amp").ok_or("the amp mic")?;
    assert_eq!(amp.role, "Mic 11");
    assert_eq!(amp.mark, Mark::Unresolved);
    assert_eq!(amp.input, "unresolved");
    // The negative control on the same rig.
    let di = row(bassist, "di").ok_or("the DI")?;
    assert_eq!(di.mark, Mark::Resolved);

    let bus = table
        .buses
        .iter()
        .find(|b| b.name == "bassist")
        .ok_or("the bassist's bus")?;
    assert_eq!(bus.mark, Mark::Unresolved);
    assert_eq!(table.unresolved, 2);
    assert_eq!(table.buses.len(), 10);

    let cody = table.buses.first().ok_or("the first bus")?;
    assert_eq!(cody.audience, "cody");
    assert_eq!(cody.input, "out 3/4");
    Ok(())
}

#[test]
fn the_view_renders_the_committed_fixture_byte_for_byte() -> Result {
    let Some(rendered) = session_daw::patch_list::shot(&Table::fixture()?, SIZE) else {
        // No GPU on this box: the picture cannot be taken. The table
        // tests above still hold; the picture is checked wherever a
        // device exists.
        tracing::warn!("no wgpu adapter; skipping the patch list picture");
        return Ok(());
    };
    let committed = image::open(FIXTURE)
        .map_err(|e| format!("{FIXTURE} did not open ({e}); run `just daw-patch-list`"))?
        .into_rgba8();
    assert_eq!(committed.dimensions(), SIZE, "the fixture's size");
    assert!(
        committed.as_raw() == &rendered,
        "the Patch List view no longer matches {FIXTURE}; if the change is intended, \
         regenerate it with `just daw-patch-list` and commit the picture"
    );
    Ok(())
}
