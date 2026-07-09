use keyflow::sections::SectionType;

// ═══════════════════════════════════════════════════════════════════════════
// Section-Scoped Chord Memory Tests
// ═══════════════════════════════════════════════════════════════════════════
//
// Memory Architecture:
// 1. Global memory - populated from:
//    - Explicit metadata assignments (e.g., `Cm = Cm7b5`)
//    - First section chord definitions
// 2. Section memory - cleared at the start of each new section
//
// Lookup order: Section-local → Global → Key inference
//
// Key behaviors:
// - First section definitions become global (available in all sections)
// - Subsequent sections start fresh (section-scoped)
// - Basic chords (C, Cm) can RECALL but don't STORE
// - Extended chords (Cmaj7) STORE to memory
// - `!` prefix bypasses ALL memory

/// Test 1: First section definitions become global
/// Tests that:
/// - Extended chords in the first section are stored to global memory
/// - Subsequent sections can recall from global memory
/// - Template recall works independently of chord memory
#[test]
fn test_first_section_becomes_global() {
    let input = r#"First Section Global - Demo
120bpm 4/4 #G

vs 4
Gmaj13 C9 Gmaj13 C9

ch 4
G C G C

vs
"#;

    let chart = keyflow::parse(input).unwrap();

    // Test metadata
    assert_eq!(
        chart.metadata.title,
        Some("First Section Global".to_string())
    );
    assert_eq!(chart.metadata.artist, Some("Demo".to_string()));

    // Test sections
    assert_eq!(chart.sections.len(), 3);

    // Verse 1 (FIRST section): Gmaj13 C9 - extended chords stored to global
    let verse1_section = &chart.sections[0];
    assert_eq!(verse1_section.section.section_type, SectionType::Verse);
    assert_eq!(verse1_section.measures().len(), 4);
    assert_eq!(verse1_section.measures()[0].chords[0].full_symbol, "Gmaj13");
    assert_eq!(verse1_section.measures()[1].chords[0].full_symbol, "C9");

    // Chorus (second section): G C - basic chords recall from global family memory
    let chorus_section = &chart.sections[1];
    assert_eq!(chorus_section.section.section_type, SectionType::Chorus);
    assert_eq!(chorus_section.measures().len(), 4);
    // Basic G recalls Gmaj13 from global major family memory
    assert_eq!(chorus_section.measures()[0].chords[0].full_symbol, "Gmaj13");
    // Basic C recalls C9 from global major family memory
    assert_eq!(chorus_section.measures()[1].chords[0].full_symbol, "C9");

    // Verse 2: template recall from Verse 1
    let verse2_section = &chart.sections[2];
    assert_eq!(verse2_section.section.section_type, SectionType::Verse);
    assert_eq!(verse2_section.measures().len(), 4);
    assert_eq!(verse2_section.measures()[0].chords[0].full_symbol, "Gmaj13");
}

/// Test 2: Section-scoped memory (non-first sections don't affect global)
/// Tests that:
/// - Extended chords in non-first sections only store to section-local memory
/// - Subsequent sections don't inherit from non-first sections
#[test]
fn test_section_scoped_memory() {
    let input = r#"Section Scoped - Demo
120bpm 4/4 #G

intro 2
G C

vs 4
Gmaj13 C9 Gmaj13 C9

ch 4
G C G C
"#;

    let chart = keyflow::parse(input).unwrap();

    // Test sections
    assert_eq!(chart.sections.len(), 3);

    // Intro (FIRST section): G C - basic chords, no extended stored to global
    let intro_section = &chart.sections[0];
    assert_eq!(intro_section.measures()[0].chords[0].full_symbol, "G");
    assert_eq!(intro_section.measures()[1].chords[0].full_symbol, "C");

    // Verse 1 (second section): Gmaj13 C9 - extended, but NOT first section
    // Stores to section-local only, NOT global
    let verse1_section = &chart.sections[1];
    assert_eq!(verse1_section.measures()[0].chords[0].full_symbol, "Gmaj13");
    assert_eq!(verse1_section.measures()[1].chords[0].full_symbol, "C9");

    // Chorus (third section): G C - basic chords
    // Global memory is empty (intro had basic chords only)
    // Outputs basic chords
    let chorus_section = &chart.sections[2];
    assert_eq!(chorus_section.measures()[0].chords[0].full_symbol, "G");
    assert_eq!(chorus_section.measures()[1].chords[0].full_symbol, "C");
}

/// Test 3: One-time overrides with ! prefix
/// Tests that:
/// - !chord uses the quality but doesn't update memory
/// - Subsequent chords recall the original memory
#[test]
fn test_chord_memory_one_time_overrides() {
    let input = r#"Chord Override Test - Demo
120bpm 4/4 #G

vs 2
Gmaj7 Gmaj7

ch 2
!G7 G
"#;

    let chart = keyflow::parse(input).unwrap();

    // Test metadata
    assert_eq!(
        chart.metadata.title,
        Some("Chord Override Test".to_string())
    );
    assert_eq!(chart.metadata.artist, Some("Demo".to_string()));

    // Test sections
    assert_eq!(chart.sections.len(), 2);

    // Verse (first section): sets Gmaj7 in global memory
    let verse_section = &chart.sections[0];
    assert_eq!(verse_section.section.section_type, SectionType::Verse);
    assert_eq!(verse_section.measures()[0].chords[0].full_symbol, "Gmaj7");

    // Chorus (second section):
    // First chord: !G7 - uses G7 but doesn't update memory
    // Second chord: G - basic major recalls from global family memory (Gmaj7)
    let chorus_section = &chart.sections[1];
    assert_eq!(chorus_section.section.section_type, SectionType::Chorus);
    assert_eq!(chorus_section.measures()[0].chords[0].full_symbol, "G7");
    assert_eq!(chorus_section.measures()[1].chords[0].full_symbol, "Gmaj7");
}

/// Test 4: Global chord assignments in metadata
/// Tests that:
/// - `Cm = Cm7b5` syntax works in metadata area
/// - Global assignments take precedence over section definitions
#[test]
fn test_global_chord_assignments() {
    let input = r#"Global Assignment Test - Demo
120bpm 4/4 #G
Cm = Cm7b5
G = Gmaj13

vs 4
C G Cm Em

ch 4
C G Cm Em
"#;

    let chart = keyflow::parse(input).unwrap();

    // Test sections
    assert_eq!(chart.sections.len(), 2);

    // Verse (first section):
    // C - basic major, no global assignment for C major
    // G - basic major, recalls Gmaj13 from global assignment
    // Cm - basic minor, recalls Cm7b5 from global assignment
    // Em - basic minor, no global assignment for Em
    let verse_section = &chart.sections[0];
    assert_eq!(verse_section.measures()[0].chords[0].full_symbol, "C");
    assert_eq!(verse_section.measures()[1].chords[0].full_symbol, "Gmaj13");
    assert_eq!(verse_section.measures()[2].chords[0].full_symbol, "Cm7b5");
    assert_eq!(verse_section.measures()[3].chords[0].full_symbol, "Em");

    // Chorus (second section): same behavior (global assignments persist)
    let chorus_section = &chart.sections[1];
    assert_eq!(chorus_section.measures()[0].chords[0].full_symbol, "C");
    assert_eq!(chorus_section.measures()[1].chords[0].full_symbol, "Gmaj13");
    assert_eq!(chorus_section.measures()[2].chords[0].full_symbol, "Cm7b5");
    assert_eq!(chorus_section.measures()[3].chords[0].full_symbol, "Em");
}

/// Test 5: Override global assignment with !
/// Tests that:
/// - `!` prefix bypasses global assignments
/// - `!Cm` outputs plain `Cm` even with `Cm = Cm7b5` global assignment
#[test]
fn test_override_global_assignment() {
    let input = r#"Override Global - Demo
120bpm 4/4 #G
Cm = Cm7b5

vs 4
Cm !Cm Cm !Cm
"#;

    let chart = keyflow::parse(input).unwrap();

    let verse_section = &chart.sections[0];
    // Cm - recalls Cm7b5 from global assignment
    assert_eq!(verse_section.measures()[0].chords[0].full_symbol, "Cm7b5");
    // !Cm - bypasses all memory, outputs plain Cm
    assert_eq!(verse_section.measures()[1].chords[0].full_symbol, "Cm");
    // Cm - still recalls Cm7b5 (! didn't change memory)
    assert_eq!(verse_section.measures()[2].chords[0].full_symbol, "Cm7b5");
    // !Cm - bypasses again
    assert_eq!(verse_section.measures()[3].chords[0].full_symbol, "Cm");
}

/// Test 6: Template recall for repeated sections
/// Tests that:
/// - Sections without content recall templates from previous definitions
/// - Templates preserve chord progressions independently of memory
#[test]
fn test_template_recall() {
    let input = r#"Template Test - Demo
120bpm 4/4 #G

vs 4
Gmaj13 C9 Em7 D7
vs
"#;

    let chart = keyflow::parse(input).unwrap();

    assert_eq!(chart.sections.len(), 2);

    // Verse 1: sets the template
    let verse1_section = &chart.sections[0];
    assert_eq!(verse1_section.measures().len(), 4);

    // Verse 2: recalls template from Verse 1
    let verse2_section = &chart.sections[1];
    assert_eq!(verse2_section.measures().len(), 4);

    // Verify template was recalled
    assert_eq!(verse2_section.measures()[0].chords[0].full_symbol, "Gmaj13");
    assert_eq!(verse2_section.measures()[1].chords[0].full_symbol, "C9");
}

/// Test 7: Template length and inline chord notation
/// Tests that:
/// - Sections can have inline chords after measure count
/// - Templates preserve the correct length
#[test]
fn test_template_length_and_inline_chords() {
    let input = r#"Template Length Test - Demo
120bpm 4/4 #G

vs 4
Gmaj7 Cmaj7 Dmin7 G7
vs
"#;

    let chart = keyflow::parse(input).unwrap();

    assert_eq!(chart.sections.len(), 2);

    // Verse 1: 4 measures
    let verse1_section = &chart.sections[0];
    assert_eq!(verse1_section.measures().len(), 4);

    // Verse 2: recalls template with same length (4 measures)
    let verse2_section = &chart.sections[1];
    assert_eq!(verse2_section.measures().len(), 4);

    // Verify chords were recalled correctly
    assert_eq!(verse2_section.measures()[0].chords[0].full_symbol, "Gmaj7");
}

/// Test 8: Explicit chord qualities
/// Tests that:
/// - Chords are parsed with explicit quality
/// - Scale degrees and Roman numerals are preserved as-is
#[test]
fn test_explicit_chord_qualities() {
    let input = r#"Explicit Quality Test - Demo
120bpm 4/4 #G

intro 4
G C Em D
vs 4
1 4 6 5
ch 4
I IV vi V
"#;

    let chart = keyflow::parse(input).unwrap();

    assert_eq!(chart.sections.len(), 3);

    // Intro: explicit chord symbols
    let intro_section = &chart.sections[0];
    assert_eq!(intro_section.measures()[0].chords[0].full_symbol, "G");
    assert_eq!(intro_section.measures()[1].chords[0].full_symbol, "C");
    assert_eq!(intro_section.measures()[2].chords[0].full_symbol, "Em");
    assert_eq!(intro_section.measures()[3].chords[0].full_symbol, "D");

    // Verse: scale degrees (preserved as-is)
    let verse_section = &chart.sections[1];
    assert_eq!(verse_section.measures()[0].chords[0].full_symbol, "1");
    assert_eq!(verse_section.measures()[1].chords[0].full_symbol, "4");
    assert_eq!(verse_section.measures()[2].chords[0].full_symbol, "6");
    assert_eq!(verse_section.measures()[3].chords[0].full_symbol, "5");

    // Chorus: Roman numerals (lowercase vi gets 'm' for minor)
    let chorus_section = &chart.sections[2];
    assert_eq!(chorus_section.measures()[0].chords[0].full_symbol, "I");
    assert_eq!(chorus_section.measures()[1].chords[0].full_symbol, "IV");
    assert_eq!(chorus_section.measures()[2].chords[0].full_symbol, "vim"); // lowercase = minor
    assert_eq!(chorus_section.measures()[3].chords[0].full_symbol, "V");
}

/// Test 9: Section-local memory within a section
/// Tests that:
/// - Extended chords within a section can be recalled by basic chords later in the same section
/// - This works regardless of whether it's the first section
#[test]
fn test_section_local_recall() {
    let input = r#"Section Local - Demo
120bpm 4/4 #G

intro 2
G C

vs 4
Gmaj13 C9 G C
"#;

    let chart = keyflow::parse(input).unwrap();

    // Verse: Gmaj13 and C9 are stored to section-local memory
    // G and C (basic) recall from section-local memory within the same section
    let verse_section = &chart.sections[1];
    assert_eq!(verse_section.measures()[0].chords[0].full_symbol, "Gmaj13");
    assert_eq!(verse_section.measures()[1].chords[0].full_symbol, "C9");
    // G recalls Gmaj13 from section-local memory
    assert_eq!(verse_section.measures()[2].chords[0].full_symbol, "Gmaj13");
    // C recalls C9 from section-local memory
    assert_eq!(verse_section.measures()[3].chords[0].full_symbol, "C9");
}

/// Test 10: Split family memory (major vs minor)
/// Tests that:
/// - Major family chords (C, Cmaj7) and minor family chords (Cm, Cm7) have separate memory
/// - Basic C recalls from major family, Cm recalls from minor family
#[test]
fn test_split_family_memory() {
    let input = r#"Split Family - Demo
120bpm 4/4 #G

vs 4
Cmaj7 Cm7 C Cm
"#;

    let chart = keyflow::parse(input).unwrap();

    let verse_section = &chart.sections[0];
    // Cmaj7 stores to major family
    assert_eq!(verse_section.measures()[0].chords[0].full_symbol, "Cmaj7");
    // Cm7 stores to minor family
    assert_eq!(verse_section.measures()[1].chords[0].full_symbol, "Cm7");
    // C (basic major) recalls Cmaj7 from major family
    assert_eq!(verse_section.measures()[2].chords[0].full_symbol, "Cmaj7");
    // Cm (basic minor) recalls Cm7 from minor family
    assert_eq!(verse_section.measures()[3].chords[0].full_symbol, "Cm7");
}
