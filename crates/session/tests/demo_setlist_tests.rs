//! REAPER integration tests for the demo setlist stamping.
//!
//! Verifies that `stamp_demo_into_project` creates persistent markers and regions
//! in a REAPER project, and that SongBuilder can parse them into valid Songs.
//!
//! The demo setlist has 3 songs with lane-organized markers and regions:
//!   Song 1: "Great Is Thy Faithfulness" (0–120s)
//!   Song 2: "Build My Life" (130–270s)
//!   Song 3: "Way Maker" (280–430s)
//!
//! Run with:
//!   cargo xtask reaper-test -- demo_setlist

use daw::Project;
use reaper_test::reaper_test;
use session::{SongBuilder, stamp_demo_into_project};
use std::time::Duration;

/// Remove all markers and regions from a project so each test starts clean.
async fn clear_project(project: &Project) -> eyre::Result<()> {
    // Remove all markers
    let markers = project.markers().all().await?;
    for m in &markers {
        if let Some(id) = m.id {
            project.markers().remove(id).await?;
        }
    }
    // Remove all regions
    let regions = project.regions().all().await?;
    for r in &regions {
        if let Some(id) = r.id {
            project.regions().remove(id).await?;
        }
    }
    Ok(())
}

/// Verify that stamping creates markers in the project that persist.
/// 3 songs × 4 markers each (COUNT-IN, SONGSTART, SONGEND, =END) = 12 markers.
#[reaper_test]
async fn demo_setlist_creates_markers(ctx: &reaper_test::ReaperTestContext) -> eyre::Result<()> {
    clear_project(&ctx.project).await?;
    stamp_demo_into_project(&ctx.project).await?;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let markers = ctx.project.markers().all().await?;

    println!("Found {} markers after stamping:", markers.len());
    for m in &markers {
        println!("  - {} @ {:.2}s", m.name, m.position_seconds());
    }

    assert_eq!(
        markers.len(),
        12,
        "Expected 12 markers (4 per song × 3), got {}",
        markers.len()
    );

    // Each marker type should appear 3 times (once per song)
    let count_ins = markers.iter().filter(|m| m.name == "COUNT-IN").count();
    let song_starts = markers.iter().filter(|m| m.name == "SONGSTART").count();
    let song_ends = markers.iter().filter(|m| m.name == "SONGEND").count();
    let abs_ends = markers.iter().filter(|m| m.name == "=END").count();

    assert_eq!(count_ins, 3, "Expected 3 COUNT-IN markers, got {count_ins}");
    assert_eq!(
        song_starts, 3,
        "Expected 3 SONGSTART markers, got {song_starts}"
    );
    assert_eq!(song_ends, 3, "Expected 3 SONGEND markers, got {song_ends}");
    assert_eq!(abs_ends, 3, "Expected 3 =END markers, got {abs_ends}");

    Ok(())
}

/// Verify that stamping creates regions in the project that persist.
/// 3 parent SONG regions + 6+6+8 section regions = 23 total.
#[reaper_test]
async fn demo_setlist_creates_regions(ctx: &reaper_test::ReaperTestContext) -> eyre::Result<()> {
    clear_project(&ctx.project).await?;
    stamp_demo_into_project(&ctx.project).await?;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let regions = ctx.project.regions().all().await?;

    println!("Found {} regions after stamping:", regions.len());
    for r in &regions {
        println!(
            "  - {} ({:.2}s - {:.2}s)",
            r.name,
            r.start_seconds(),
            r.end_seconds()
        );
    }

    assert_eq!(
        regions.len(),
        23,
        "Expected 23 regions (3 parent + 20 section), got {}",
        regions.len()
    );

    let names: Vec<&str> = regions.iter().map(|r| r.name.as_str()).collect();

    // Parent song regions
    assert!(
        names.contains(&"Great Is Thy Faithfulness"),
        "Missing Song 1 parent region"
    );
    assert!(
        names.contains(&"Build My Life"),
        "Missing Song 2 parent region"
    );
    assert!(names.contains(&"Way Maker"), "Missing Song 3 parent region");

    // Section regions (present across multiple songs)
    assert!(names.contains(&"Intro"), "Missing Intro");
    assert!(names.contains(&"Verse 1"), "Missing Verse 1");
    assert!(names.contains(&"Chorus"), "Missing Chorus");
    assert!(names.contains(&"Verse 2"), "Missing Verse 2");
    assert!(names.contains(&"Bridge"), "Missing Bridge");
    assert!(names.contains(&"Outro"), "Missing Outro");
    assert!(names.contains(&"Pre-Chorus"), "Missing Pre-Chorus");
    assert!(names.contains(&"Tag"), "Missing Tag");

    Ok(())
}

/// Verify marker positions for Song 1.
#[reaper_test]
async fn demo_setlist_marker_positions(ctx: &reaper_test::ReaperTestContext) -> eyre::Result<()> {
    clear_project(&ctx.project).await?;
    stamp_demo_into_project(&ctx.project).await?;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let markers = ctx.project.markers().all().await?;
    let eps = 0.01;

    // Song 1 markers (first occurrence of each)
    let first = |name: &str| {
        markers
            .iter()
            .filter(|m| m.name == name)
            .map(|m| m.position_seconds())
            .next()
            .unwrap()
    };

    // Song 1: COUNT-IN@0, SONGSTART@4, SONGEND@116, =END@120
    assert!((first("COUNT-IN") - 0.0).abs() < eps, "First COUNT-IN @ 0s");
    assert!(
        (first("SONGSTART") - 4.0).abs() < eps,
        "First SONGSTART @ 4s"
    );
    assert!(
        (first("SONGEND") - 116.0).abs() < eps,
        "First SONGEND @ 116s"
    );
    assert!((first("=END") - 120.0).abs() < eps, "First =END @ 120s");

    // Song 2 markers (second occurrence)
    let mut song_starts: Vec<f64> = markers
        .iter()
        .filter(|m| m.name == "SONGSTART")
        .map(|m| m.position_seconds())
        .collect();
    song_starts.sort_by(|a, b| a.partial_cmp(b).unwrap());

    assert!(song_starts.len() == 3, "Should have 3 SONGSTART markers");
    assert!(
        (song_starts[1] - 134.0).abs() < eps,
        "Song 2 SONGSTART @ 134s"
    );
    assert!(
        (song_starts[2] - 284.0).abs() < eps,
        "Song 3 SONGSTART @ 284s"
    );

    Ok(())
}

/// Verify section region boundaries are contiguous within Song 1.
#[reaper_test]
async fn demo_setlist_regions_contiguous(ctx: &reaper_test::ReaperTestContext) -> eyre::Result<()> {
    clear_project(&ctx.project).await?;
    stamp_demo_into_project(&ctx.project).await?;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let all_regions = ctx.project.regions().all().await?;

    // Filter to section regions within Song 1 bounds (4s–116s)
    // Exclude the parent SONG region itself
    let mut song1_sections: Vec<_> = all_regions
        .iter()
        .filter(|r| {
            r.start_seconds() >= 4.0
                && r.end_seconds() <= 116.0
                && r.name != "Great Is Thy Faithfulness"
        })
        .collect();
    song1_sections.sort_by(|a, b| a.start_seconds().partial_cmp(&b.start_seconds()).unwrap());

    println!("Song 1 section regions ({}):", song1_sections.len());
    for r in &song1_sections {
        println!(
            "  - {} ({:.2}s - {:.2}s)",
            r.name,
            r.start_seconds(),
            r.end_seconds()
        );
    }

    assert_eq!(
        song1_sections.len(),
        6,
        "Song 1 should have 6 section regions"
    );

    // Each region's end == next region's start
    for w in song1_sections.windows(2) {
        let gap = (w[1].start_seconds() - w[0].end_seconds()).abs();
        assert!(
            gap < 0.01,
            "Gap between '{}' and '{}': {:.4}s",
            w[0].name,
            w[1].name,
            gap
        );
    }

    // First section starts at SONGSTART, last ends at SONGEND
    assert!(
        (song1_sections[0].start_seconds() - 4.0).abs() < 0.01,
        "First section starts at 4s"
    );
    assert!(
        (song1_sections.last().unwrap().end_seconds() - 116.0).abs() < 0.01,
        "Last section ends at 116s"
    );

    Ok(())
}

/// End-to-end: stamp demo data → SongBuilder → verify Song structure.
///
/// This is the full pipeline test: create markers/regions in REAPER,
/// then run SongBuilder to parse them, and confirm the resulting Songs
/// have the expected sections, timing, and structure.
#[reaper_test]
async fn demo_setlist_end_to_end(ctx: &reaper_test::ReaperTestContext) -> eyre::Result<()> {
    // ── Step 1: Clear and stamp demo markers and regions ─────────
    clear_project(&ctx.project).await?;
    stamp_demo_into_project(&ctx.project).await?;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // ── Step 2: Run SongBuilder on the project ───────────────────
    let songs = SongBuilder::build(&ctx.project).await?;

    println!("\nSongBuilder produced {} song(s):", songs.len());
    assert_eq!(songs.len(), 3, "SongBuilder should produce 3 songs");

    for (si, song) in songs.iter().enumerate() {
        println!("\n  Song {si}: {}", song.name);
        println!("  Start: {:.2}s", song.start_seconds);
        println!("  End: {:.2}s", song.end_seconds);
        println!("  Count-in: {:?}", song.count_in_seconds);
        println!("  Sections ({}):", song.sections.len());
        for (i, s) in song.sections.iter().enumerate() {
            println!(
                "    {}: {} ({:?}) {:.2}s - {:.2}s",
                i, s.name, s.section_type, s.start_seconds, s.end_seconds
            );
        }
    }

    // ── Step 3: Verify Song 1 ("Great Is Thy Faithfulness") ──────
    let song1 = &songs[0];
    assert_eq!(song1.name, "Great Is Thy Faithfulness");

    let eps = 2.0;
    assert!(
        song1.start_seconds < 1.0,
        "Song 1 start should be ~0s (includes count-in), got {:.2}",
        song1.start_seconds
    );
    assert!(
        (song1.end_seconds - 120.0).abs() < eps,
        "Song 1 end should be ~120s, got {:.2}",
        song1.end_seconds
    );

    // Count-in should be detected from MARKS-lane COUNT-IN → SONGSTART markers
    let ci = song1
        .count_in_seconds
        .expect("Song 1 should have count-in detected");
    println!("  Count-in detected: {ci:.2}s");
    assert!(
        (ci - 4.0).abs() < eps,
        "Count-in should be ~4s, got {:.2}",
        ci
    );

    // Verify Song 1 section names
    let section_names: Vec<&str> = song1.sections.iter().map(|s| s.name.as_str()).collect();
    println!("\n  Song 1 section names: {:?}", section_names);

    assert!(section_names.contains(&"Intro"), "Missing Intro section");
    assert!(
        section_names.contains(&"Verse 1"),
        "Missing Verse 1 section"
    );
    assert!(section_names.contains(&"Chorus"), "Missing Chorus section");
    assert!(
        section_names.contains(&"Verse 2"),
        "Missing Verse 2 section"
    );
    assert!(section_names.contains(&"Outro"), "Missing Outro section");

    // Verify section types (including Count-In and End from structural markers)
    use session::SectionType;
    let has_count_in = song1
        .sections
        .iter()
        .any(|s| s.section_type == SectionType::CountIn);
    let has_intro = song1
        .sections
        .iter()
        .any(|s| s.section_type == SectionType::Intro);
    let has_verse = song1
        .sections
        .iter()
        .any(|s| s.section_type == SectionType::Verse);
    let has_chorus = song1
        .sections
        .iter()
        .any(|s| s.section_type == SectionType::Chorus);
    let has_outro = song1
        .sections
        .iter()
        .any(|s| s.section_type == SectionType::Outro);
    let has_end = song1
        .sections
        .iter()
        .any(|s| s.section_type == SectionType::End);

    assert!(has_count_in, "Should detect Count-In section type");
    assert!(has_intro, "Should detect Intro section type");
    assert!(has_verse, "Should detect Verse section type");
    assert!(has_chorus, "Should detect Chorus section type");
    assert!(has_outro, "Should detect Outro section type");
    assert!(has_end, "Should detect End section type");

    // ── Step 4: Verify sections are contiguous within each song ──
    for song in &songs {
        for w in song.sections.windows(2) {
            let gap = (w[1].start_seconds - w[0].end_seconds).abs();
            assert!(
                gap < 0.5,
                "Gap in '{}' between '{}' (end {:.2}s) and '{}' (start {:.2}s): {:.4}s",
                song.name,
                w[0].name,
                w[0].end_seconds,
                w[1].name,
                w[1].start_seconds,
                gap
            );
        }
    }

    // ── Step 5: Verify Song 2 has Bridge, Pre-Chorus, and count-in ──
    let song2 = &songs[1];
    assert_eq!(song2.name, "Build My Life");
    assert!(
        song2.count_in_seconds.is_some(),
        "Song 2 should have count-in"
    );
    let song2_names: Vec<&str> = song2.sections.iter().map(|s| s.name.as_str()).collect();
    assert!(song2_names.contains(&"Count-In"), "Song 2 missing Count-In");
    assert!(
        song2_names.contains(&"Pre-Chorus"),
        "Song 2 missing Pre-Chorus"
    );
    assert!(song2_names.contains(&"Bridge"), "Song 2 missing Bridge");

    // ── Step 6: Verify Song 3 has Tag and count-in ──────────────
    let song3 = &songs[2];
    assert_eq!(song3.name, "Way Maker");
    assert!(
        song3.count_in_seconds.is_some(),
        "Song 3 should have count-in"
    );
    let song3_names: Vec<&str> = song3.sections.iter().map(|s| s.name.as_str()).collect();
    assert!(song3_names.contains(&"Count-In"), "Song 3 missing Count-In");
    assert!(song3_names.contains(&"Tag"), "Song 3 missing Tag");
    assert!(song3_names.contains(&"Bridge"), "Song 3 missing Bridge");

    println!("\nEnd-to-end test passed: 3-song demo data → SongBuilder → valid Songs");
    Ok(())
}
