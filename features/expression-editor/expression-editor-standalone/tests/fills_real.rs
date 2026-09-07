//! Fill detection against the real drum session.
//!
//! The unit tests in `expression_editor_core::fills` prove the scoring
//! behaves on shapes built to have an obvious answer. They cannot say
//! whether the thing fires sensibly on a drummer, which is the only
//! question that matters, so this runs it over the album and checks the
//! result against something the detector never sees.
//!
//! That check is the song's own section markers. A drummer fills *into*
//! a section — the bar before the chorus is the classic place for one —
//! so if the detector is finding real fills, they should land next to
//! section boundaries far more often than chance would put them there.
//! It is deliberately not an assertion about a fixed count: the point is
//! that the fills are musically placed, not that there are eleven.

use expression_editor_core::fills::FillConfig;
use expression_editor_core::Viewport;
use expression_editor_standalone::{Loaded, Runner, Source, Target};

const BASE: &str =
    "/run/media/AudioHaven/Project/Crescendum-Rockstars-SESSION-BACKUP-2026-09-06/Crescendum";

/// The drum-session songs, all of which were tracked in the same pass.
const SONGS: [&str; 6] = [
    "set in stone",
    "heavify",
    "unbreakable",
    "Kornesque ",
    "Chained expectations",
    "The ballad",
];

struct Found {
    song: String,
    bars: usize,
    fills: usize,
    /// Fills whose start is within a bar of a section boundary.
    near_section: usize,
    sections: usize,
    /// Share of *all* bars that sit near a section — the rate a
    /// detector firing at random would achieve.
    chance: f64,
    /// Share of detected fills that do.
    observed: f64,
}

fn analyse(song: &str) -> Option<Found> {
    let path = format!("{BASE}/{song}/{song}.organized.RPP");
    if !std::path::Path::new(&path).exists() {
        return None;
    }
    let runner = Runner::open(
        &Source::Rpp(path.clone().into()),
        &Target {
            drums: Some(None),
            ..Target::default()
        },
        Viewport {
            w: 1600.0,
            h: 900.0,
        },
        None,
    )
    .ok()?;
    let Loaded::DrumWorkspace(ed) = &runner.loaded else {
        return None;
    };
    let host = runner.host.as_ref()?;
    let host = host.as_ref();

    let panel = expression_editor_ui::quantize_panel::QuantizePanel::default();
    let fills = host.fills(&panel, &FillConfig::default());
    let bars = host.bar_count();

    // Section boundaries the detector never sees: markers and regions,
    // in seconds.
    let mut sections: Vec<f64> = ed.doc.markers.iter().map(|m| m.t).collect();
    sections.extend(ed.doc.regions.iter().map(|r| r.start));
    // Docs are in frames; the detector works in seconds.
    let ups = ed.doc.time_base.units_per_second(120.0);
    if ups > 0.0 {
        for s in &mut sections {
            *s /= ups;
        }
    }

    // "Near" is generous on purpose — a fill leads *into* the section,
    // so it starts before the marker, and a bar is the natural unit of
    // that lead-in.
    let bar_secs = 2.5;
    let near_section = fills
        .iter()
        .filter(|f| {
            sections
                .iter()
                .any(|s| (f.end - s).abs() <= bar_secs || (f.start - s).abs() <= bar_secs)
        })
        .count();

    // The control. Fills landing near sections means nothing until we
    // know how often *any* bar is near a section: with 16 markers in a
    // five-minute song and a bar of tolerance either side, a large
    // share of the song is "near a section" and a detector firing at
    // random would score well. This is the rate to beat.
    let grid = host.bar_grid_secs();
    let bars_near = grid
        .windows(2)
        .filter(|w| {
            sections
                .iter()
                .any(|s| (w[1] - s).abs() <= bar_secs || (w[0] - s).abs() <= bar_secs)
        })
        .count();
    let chance = bars_near as f64 / grid.len().saturating_sub(1).max(1) as f64;
    let observed = near_section as f64 / fills.len().max(1) as f64;

    println!(
        "{:<24} bars={:<4} fills={:<3} near a section={:<3} (of {} sections)",
        song,
        bars,
        fills.len(),
        near_section,
        sections.len()
    );
    println!(
        "        near-section rate: fills {:.0}%  vs any bar {:.0}%  (lift {:.2}x)",
        observed * 100.0,
        chance * 100.0,
        if chance > 0.0 { observed / chance } else { 0.0 }
    );
    for f in fills.iter().take(6) {
        println!(
            "        bars {:>3}-{:<3}  {:>7.2}s  score {:.1}",
            f.bars.0, f.bars.1, f.start, f.score
        );
    }
    Some(Found {
        song: song.to_string(),
        bars,
        fills: fills.len(),
        near_section,
        sections: sections.len(),
        chance,
        observed,
    })
}

// r[verify drums.fills.detect]
#[test]
fn fills_land_where_a_drummer_puts_them() {
    if !std::path::Path::new(BASE).exists() {
        eprintln!("skipping: {BASE} not mounted");
        return;
    }
    let found: Vec<Found> = SONGS.iter().filter_map(|s| analyse(s)).collect();
    assert!(!found.is_empty(), "no album project loaded as a drum kit");

    for f in &found {
        assert!(
            f.bars > 8,
            "{}: only {} bars — the tempo map did not place a grid",
            f.song,
            f.bars
        );
        // A song with no fills at all means the detector is dead; one
        // where most bars are a fill means it is not discriminating.
        assert!(
            f.fills > 0,
            "{}: found no fills in {} bars",
            f.song,
            f.bars
        );
        assert!(
            f.fills * 4 < f.bars,
            "{}: {} of {} bars called a fill — that is the groove, not a fill",
            f.song,
            f.fills,
            f.bars
        );
    }

    // The real check: fills concentrate at section boundaries *more
    // than bars in general do*. The raw "how many fills are near a
    // section" number cannot say this on its own — roughly a third of
    // every one of these songs is within a bar of some marker, so a
    // detector firing at random would already score about 30%.
    for f in &found {
        assert!(
            f.observed >= f.chance,
            "{}: fills land near a section {:.0}% of the time against \
             {:.0}% for any bar at all — no better than firing at random",
            f.song,
            f.observed * 100.0,
            f.chance * 100.0
        );
    }
    let lift: f64 = found.iter().map(|f| f.observed / f.chance.max(1e-9)).sum::<f64>()
        / found.len() as f64;
    assert!(
        lift >= 1.4,
        "fills are only {lift:.2}x more likely than chance to sit at a \
         section boundary — that is not a drummer's phrasing, it is noise"
    );
}
