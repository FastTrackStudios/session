//! The applied copy and the session override, in project ext-state
//! (decision #29: section `fts.patch-list`, keys `applied` and
//! `override`), and staleness — the album or the profile moving on
//! since the last apply.
//!
//! r[verify flow.patch-list.apply]
//! r[verify flow.patch-list.session-override]

use daw_proto::{ProjectContext, ProjectInfo};
use daw_standalone::sync::Standalone;
use patch_list::{PatchList, apply, stale};

type Result = std::result::Result<(), Box<dyn std::error::Error>>;

fn seeded() -> (Standalone, ProjectContext) {
    let daw = Standalone::new();
    let guid = daw.seed_project(ProjectInfo {
        guid: "test-proj".into(),
        name: "test".into(),
        path: String::new(),
    });
    (daw, ProjectContext::Project(guid))
}

#[test]
fn nothing_applied_yet_is_not_stale() -> Result {
    let (daw, ctx) = seeded();
    assert_eq!(stale(&daw, ctx, "performers {}", "golden-room")?, None);
    Ok(())
}

#[test]
fn the_applied_copy_round_trips_with_its_profile_and_a_timestamp() -> Result {
    let (daw, ctx) = seeded();
    let text = "performers {\n    cody {guitar {di \"DI 3\"}}\n}";
    apply::store_applied(&daw, ctx.clone(), text, "golden-room")?;

    let record = apply::applied(&daw, ctx)?.ok_or("an applied copy")?;
    assert_eq!(record.text, text);
    assert_eq!(record.profile, "golden-room");
    // RFC 3339 — not empty, not a placeholder.
    assert!(record.at.contains('T'), "{}", record.at);
    Ok(())
}

#[test]
fn a_changed_album_makes_the_session_stale_with_a_diff() -> Result {
    let (daw, ctx) = seeded();
    let applied_text = "performers {\n    cody {guitar {di \"DI 3\"}}\n}";
    apply::store_applied(&daw, ctx.clone(), applied_text, "golden-room")?;

    let current_text = "performers {\n    cody {guitar {di \"DI 9\"}}\n}";
    let status = stale(&daw, ctx, current_text, "golden-room")?.ok_or("stale")?;
    assert_eq!(status.applied.text, applied_text);
    assert!(status.diff.removed.iter().any(|l| l.contains("DI 3")));
    assert!(status.diff.added.iter().any(|l| l.contains("DI 9")));
    Ok(())
}

#[test]
fn a_changed_profile_makes_the_session_stale_even_with_the_same_text() -> Result {
    let (daw, ctx) = seeded();
    let text = "performers {\n    cody {guitar {di \"DI 3\"}}\n}";
    apply::store_applied(&daw, ctx.clone(), text, "golden-room")?;

    let status = stale(&daw, ctx, text, "other-room")?.ok_or("stale on a profile change")?;
    assert!(status.diff.is_empty(), "the text did not move");
    Ok(())
}

#[test]
fn the_same_text_and_profile_is_not_stale() -> Result {
    // The negative control: staleness is not "an applied copy exists".
    let (daw, ctx) = seeded();
    let text = "performers {\n    cody {guitar {di \"DI 3\"}}\n}";
    apply::store_applied(&daw, ctx.clone(), text, "golden-room")?;
    assert_eq!(stale(&daw, ctx, text, "golden-room")?, None);
    Ok(())
}

#[test]
fn the_override_round_trips_and_removing_it_returns_the_session_to_the_list() -> Result {
    let (daw, ctx) = seeded();
    assert_eq!(apply::get_override(&daw, ctx.clone())?, None);

    let over = PatchList::from_styx("performers {\n    cody {guitar {di \"DI 9\"}}\n}")?;
    apply::set_override(&daw, ctx.clone(), &over)?;
    assert_eq!(apply::get_override(&daw, ctx.clone())?, Some(over));

    apply::clear_override(&daw, ctx.clone())?;
    assert_eq!(apply::get_override(&daw, ctx)?, None);
    Ok(())
}
