//! Test 016: Push/Pull Triplet and Tuplet Syntax
//!
//! Tests the extended push/pull syntax including:
//! - 't triplet shortcut: 'tC = triplet eighth push
//! - ':N tuplet syntax: ':5C = quintuplet push
//! - /push setting for default push mode

use keyflow::chord::{PushPullAmount, PushPullBase};

#[test]
fn test_triplet_push_shortcut() {
    let input = r#"
Triplet Push Test - Artist
120bpm 4/4 #C

VS 4
'tC D 'tEm F
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    let section = &chart.sections[0];

    // When first chord has push, a space is inserted before it
    // So measure 0 has [space, C] instead of just [C]
    let measure_0 = &section.measures()[0];
    println!(
        "Measure 0 chords: {:?}",
        measure_0
            .chords
            .iter()
            .map(|c| &c.full_symbol)
            .collect::<Vec<_>>()
    );

    // Find the C chord (might be at index 0 or 1 depending on space insertion)
    let chord_c = measure_0
        .chords
        .iter()
        .find(|c| c.full_symbol == "C")
        .expect("Should find C chord in measure 0");

    if let Some((is_push, amount)) = &chord_c.push_pull {
        assert!(*is_push, "Expected push notation");
        assert_eq!(amount.level, 1, "Expected single apostrophe level");
        assert_eq!(
            amount.base,
            PushPullBase::Triplet,
            "Expected triplet base, got {:?}",
            amount.base
        );
    } else {
        panic!("Expected push_pull for 'tC, got None");
    }

    // 'tEm should be triplet eighth push
    let chord_em = &section.measures()[2].chords[0];
    assert_eq!(chord_em.full_symbol, "Em");
    if let Some((is_push, amount)) = &chord_em.push_pull {
        assert!(*is_push, "Expected push notation");
        assert_eq!(amount.level, 1);
        assert_eq!(
            amount.base,
            PushPullBase::Triplet,
            "Expected triplet base, got {:?}",
            amount.base
        );
    } else {
        panic!("Expected push_pull for 'tEm, got None");
    }
}

#[test]
fn test_triplet_pull_shortcut() {
    let input = r#"
Triplet Pull Test - Artist
120bpm 4/4 #C

VS 4
C't D Em't F
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    let section = &chart.sections[0];

    // C't should be triplet eighth pull
    let chord_c = &section.measures()[0].chords[0];
    assert_eq!(chord_c.full_symbol, "C");
    if let Some((is_push, amount)) = &chord_c.push_pull {
        assert!(!*is_push, "Expected pull notation");
        assert_eq!(amount.level, 1);
        assert_eq!(amount.base, PushPullBase::Triplet);
    } else {
        panic!("Expected push_pull for C't, got None");
    }
}

#[test]
fn test_double_triplet_push() {
    let input = r#"
Double Triplet Push Test - Artist
120bpm 4/4 #C

VS 4
''tC D ''tEm F
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    let section = &chart.sections[0];

    // When first chord has push, a space is inserted before it
    let chord_c = section.measures()[0]
        .chords
        .iter()
        .find(|c| c.full_symbol == "C")
        .expect("Should find C chord in measure 0");

    // ''tC should be triplet sixteenth push
    if let Some((is_push, amount)) = &chord_c.push_pull {
        assert!(*is_push, "Expected push notation");
        assert_eq!(amount.level, 2, "Expected double apostrophe level");
        assert_eq!(amount.base, PushPullBase::Triplet, "Expected triplet base");
    } else {
        panic!("Expected push_pull for ''tC, got None");
    }
}

#[test]
fn test_quintuplet_push() {
    let input = r#"
Quintuplet Push Test - Artist
120bpm 4/4 #C

VS 4
':5C D ':5Em F
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    let section = &chart.sections[0];

    // When first chord has push, a space is inserted before it
    let chord_c = section.measures()[0]
        .chords
        .iter()
        .find(|c| c.full_symbol == "C")
        .expect("Should find C chord in measure 0");

    // ':5C should be quintuplet eighth push
    if let Some((is_push, amount)) = &chord_c.push_pull {
        assert!(*is_push, "Expected push notation");
        assert_eq!(amount.level, 1);
        assert_eq!(
            amount.base,
            PushPullBase::Tuplet(5),
            "Expected quintuplet base"
        );
    } else {
        panic!("Expected push_pull for ':5C, got None");
    }
}

#[test]
fn test_septuplet_pull() {
    let input = r#"
Septuplet Pull Test - Artist
120bpm 4/4 #C

VS 4
C':7 D Em':7 F
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    let section = &chart.sections[0];

    // C':7 should be septuplet eighth pull
    let chord_c = &section.measures()[0].chords[0];
    assert_eq!(chord_c.full_symbol, "C");
    if let Some((is_push, amount)) = &chord_c.push_pull {
        assert!(!*is_push, "Expected pull notation");
        assert_eq!(amount.level, 1);
        assert_eq!(
            amount.base,
            PushPullBase::Tuplet(7),
            "Expected septuplet base"
        );
    } else {
        panic!("Expected push_pull for C':7, got None");
    }
}

#[test]
fn test_push_setting_triplet() {
    let input = r#"
Default Triplet Push Test - Artist
120bpm 4/4 #C
/push = triplet

VS 4
'C D 'Em F
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    let section = &chart.sections[0];

    // When first chord has push, a space is inserted before it
    let chord_c = section.measures()[0]
        .chords
        .iter()
        .find(|c| c.full_symbol == "C")
        .expect("Should find C chord in measure 0");

    // With /push = triplet, 'C should be triplet eighth push (not standard)
    if let Some((is_push, amount)) = &chord_c.push_pull {
        assert!(*is_push, "Expected push notation");
        assert_eq!(amount.level, 1);
        assert_eq!(
            amount.base,
            PushPullBase::Triplet,
            "Expected triplet base from setting"
        );
    } else {
        panic!("Expected push_pull for 'C with /push=triplet, got None");
    }
}

#[test]
fn test_push_setting_quintuplet() {
    let input = r#"
Default Quintuplet Push Test - Artist
120bpm 4/4 #C
/push = 5

VS 4
'C D ''Em F
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    let section = &chart.sections[0];

    // When first chord has push, a space is inserted before it
    let chord_c = section.measures()[0]
        .chords
        .iter()
        .find(|c| c.full_symbol == "C")
        .expect("Should find C chord in measure 0");

    // With /push = 5, 'C should be quintuplet eighth push
    if let Some((is_push, amount)) = &chord_c.push_pull {
        assert!(*is_push, "Expected push notation");
        assert_eq!(amount.level, 1);
        assert_eq!(
            amount.base,
            PushPullBase::Tuplet(5),
            "Expected quintuplet base from setting"
        );
    } else {
        panic!("Expected push_pull for 'C with /push=5, got None");
    }

    // ''Em should be quintuplet sixteenth push
    let chord_em = &section.measures()[2].chords[0];
    assert_eq!(chord_em.full_symbol, "Em");
    if let Some((is_push, amount)) = &chord_em.push_pull {
        assert!(*is_push, "Expected push notation");
        assert_eq!(amount.level, 2);
        assert_eq!(
            amount.base,
            PushPullBase::Tuplet(5),
            "Expected quintuplet base from setting"
        );
    } else {
        panic!("Expected push_pull for ''Em with /push=5, got None");
    }
}

#[test]
fn test_push_amount_beats_standard() {
    // Standard eighth
    let amount = PushPullAmount::eighth();
    assert!(
        (amount.to_beats() - 0.5).abs() < 0.001,
        "Eighth note should be 0.5 beats"
    );

    // Standard sixteenth
    let amount = PushPullAmount::sixteenth();
    assert!(
        (amount.to_beats() - 0.25).abs() < 0.001,
        "Sixteenth note should be 0.25 beats"
    );

    // Standard thirty-second
    let amount = PushPullAmount::thirty_second();
    assert!(
        (amount.to_beats() - 0.125).abs() < 0.001,
        "Thirty-second note should be 0.125 beats"
    );
}

#[test]
fn test_push_amount_beats_triplet() {
    // Triplet eighth = 0.5 * (2/3) = 0.333...
    let amount = PushPullAmount::eighth_triplet();
    let expected = 0.5 * (2.0 / 3.0);
    assert!(
        (amount.to_beats() - expected).abs() < 0.001,
        "Triplet eighth should be ~0.333 beats, got {}",
        amount.to_beats()
    );

    // Triplet sixteenth = 0.25 * (2/3) = 0.166...
    let amount = PushPullAmount::sixteenth_triplet();
    let expected = 0.25 * (2.0 / 3.0);
    assert!(
        (amount.to_beats() - expected).abs() < 0.001,
        "Triplet sixteenth should be ~0.166 beats, got {}",
        amount.to_beats()
    );
}

#[test]
fn test_push_amount_beats_quintuplet() {
    // Quintuplet eighth = 0.5 * (4/5) = 0.4
    let amount = PushPullAmount::from_count_tuplet(1, 5).unwrap();
    let expected = 0.5 * (4.0 / 5.0);
    assert!(
        (amount.to_beats() - expected).abs() < 0.001,
        "Quintuplet eighth should be 0.4 beats, got {}",
        amount.to_beats()
    );
}

#[test]
fn test_round_trip_triplet_push() {
    let input = r#"
Round Trip Triplet Test - Artist
120bpm 4/4 #C

VS 4
'tC D 'tEm F
"#;

    let chart = keyflow::parse(input).unwrap();
    let syntax = chart.to_syntax();
    println!("Original:\n{}", input);
    println!("Generated:\n{}", syntax);

    // The generated syntax should contain the triplet notation
    assert!(
        syntax.contains("'t"),
        "Should preserve triplet notation in output"
    );
}

#[test]
fn test_round_trip_tuplet_push() {
    let input = r#"
Round Trip Tuplet Test - Artist
120bpm 4/4 #C

VS 4
':5C D ':5Em F
"#;

    let chart = keyflow::parse(input).unwrap();
    let syntax = chart.to_syntax();
    println!("Original:\n{}", input);
    println!("Generated:\n{}", syntax);

    // The generated syntax should contain the tuplet notation
    assert!(
        syntax.contains(":5"),
        "Should preserve tuplet notation in output"
    );
}

#[test]
fn test_mixed_standard_and_triplet() {
    let input = r#"
Mixed Standard and Triplet Test - Artist
120bpm 4/4 #C

VS 4
'C 'tDm Em't F'
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    let section = &chart.sections[0];

    // 'C should be standard eighth push
    // When first chord has push, a space is inserted before it
    let chord_c = section.measures()[0]
        .chords
        .iter()
        .find(|c| c.full_symbol == "C")
        .expect("Should find C chord in measure 0");
    if let Some((is_push, amount)) = &chord_c.push_pull {
        assert!(*is_push);
        assert_eq!(amount.base, PushPullBase::Standard);
    } else {
        panic!("Expected standard push for 'C");
    }

    // 'tDm should be triplet eighth push
    // When chord has push, a space is inserted before it
    let chord_dm = section.measures()[1]
        .chords
        .iter()
        .find(|c| c.full_symbol == "Dm")
        .expect("Should find Dm chord in measure 1");
    if let Some((is_push, amount)) = &chord_dm.push_pull {
        assert!(*is_push);
        assert_eq!(amount.base, PushPullBase::Triplet);
    } else {
        panic!("Expected triplet push for 'tD");
    }

    // Em't should be triplet eighth pull
    let chord_em = section.measures()[2]
        .chords
        .iter()
        .find(|c| c.full_symbol == "Em")
        .expect("Should find Em chord in measure 2");
    if let Some((is_push, amount)) = &chord_em.push_pull {
        assert!(!*is_push);
        assert_eq!(amount.base, PushPullBase::Triplet);
    } else {
        panic!("Expected triplet pull for Em't");
    }

    // F' should be standard eighth pull
    let chord_f = section.measures()[3]
        .chords
        .iter()
        .find(|c| c.full_symbol == "F")
        .expect("Should find F chord in measure 3");
    if let Some((is_push, amount)) = &chord_f.push_pull {
        assert!(!*is_push);
        assert_eq!(amount.base, PushPullBase::Standard);
    } else {
        panic!("Expected standard pull for F'");
    }
}
