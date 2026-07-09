//! Built-in example charts for the chart editor.

/// Empty chart template.
pub const EMPTY_CHART: &str = r#"New Song
120bpm 4/4 #A

VS
A D F#m // E // D


"#;

/// Thriller - Dirty Loops, Cory Wong cover arrangement.
/// Demonstrates push/pull triplets and complex rhythm notation.
pub const EXAMPLE_THRILLER: &str = r#"Thriller - Dirty Loops, Cory Wong
Transcribed By: Cody Wright
120bpm 4/4 #Ab
/push = triplet

COUNT 2

HITS
r8t >Ab9_8t r8t r8t r8t >F9_8t r2 | s1

IN
>'Cm . . .

VS
>'F/C . Cm . 'F/C . Cm . 'F/C . Cm . 'F/C . Cm Cm9


CH
>Cm/Eb / 'Eb /// | 'Eb / 'F/C / 'Cm // | 'F/A //// | 'Fm9  ////
>Cm/Eb / 'Eb /// | 'Eb / 'F/C / 'Cm // | 'F/A | r8t >Ab9_8t r8t r8t >'F9_8t r8t r4 >Fm/Ab_4

INST 4
Cm . F6 // Abdim7 'Csus2 // 'C5 //

VS
F/C . Cm . 'F/C . Cm . 'F/C . 'Cm . 'F/C . Cm // Gm7 // 'Abmaj7 / Abmaj7#5 / 'Db7#11/G //

CH
>Cm/Eb / 'Eb /// | 'Eb / 'F/C / 'Cm // | 'F/A //// | 'Fm9  ////
>Cm/Eb / 'Eb /// | 'Eb / 'F/C / 'Cm // | 'F/A | r8t >Ab9_8t r8t r8t >'F9_8t r8t r4 >Fm/Ab_4 | s1


BR
>'_4F7 | . |  Abmaj9 //// | // r8t >Abmaj9_8t r8t >Bb_8t r8t >Cm7_8t | Cm7 | Ebmaj7/Bb | Am7b5 | Abmaj7 | G7sus4 | 'G7

VS
>'F/C . Cm . 'F/C . Cm Db7#11 // 'Cmaj9 'F7 //// F7 // 'Fmaj7/C 'Cm7 .
'F 'Am // 'Dbmaj7 // 'Gmaj7 // 'Fm11 // 'Eb9 / Bbm/F / 'Gb7b9 //

CH
>'Cm/Eb / 'Eb /// | 'Eb / 'F/C / 'Cm // | 'F/A //// | 'Fm9  ////
>Cm/Eb / 'Eb /// | 'Eb / 'F/C / 'Cm // | 'F/A |
r8t >Ab9_8t r8t r8t >'F9_8t r8t r4 r8t >Bb9_8t r8t
r8t r8t C7#11_8t r4 r8t Dbmaj7_8t r8t r4
'Bb11

CH
>Cm/Eb / 'Eb /// | 'Eb / 'F/C / 'Cm // | 'F/A //// | 'Fm9  ////
>Cm/Eb / 'Eb /// | 'Eb / 'F/C / 'Cm // | 'F/A | r8t >Ab9_8t r8t r8t >'F9_8t r8t r4 >Cm

Interlude 8
Cm . . . . . . .

Interlude 8 "HORNS"
/push 4
'Cm . 'Cm7b5 . 'Cm Cm/maj7  'B/C .

Interlude 8 "WINDS"
C C+ // C // Cm7b5 Cmaj7
'Cmaj7 . Fm/C Cdim7

Interlude 8 "TRUMPETS"
Fm6 . 'Dbmaj7/F .  D/F . B7/F .

Outro 8
Em7b5/D 'Dmaj9 x3
Gm7/D 'D11

Outro 8
'Gm7/D 'Dadd9 'Em7b5 'Dadd9
'Em7b5 'Dadd9 'Gm9/Bb 'Fmaj9/C

Hits 4
'C#/G . . .

"#;

/// Messengers of Hope - Evan Human.
/// Demonstrates slash notation with half-beat chords and chromatic passing chords.
pub const EXAMPLE_MESSAGES_OF_HOPE: &str = r#"Messengers of Hope - Evan Human
4/4 66bpm #D
IN
D /// A/C# /

IN
Bm7  F#m7 Gmaj7
F#m7 Gmaj7 Asus // !A // D //// ////

VS "English"
D // A/C# // Bm // A //
G // A // D ////
Bm // A/C# // Gmaj7 // F#m / Fm /
Em // Gm / A /  D ////

VS "Farsi"
D // A/C# // Bm // A //
G // A // D // C#m7b5 / F#7 /
Bm // A // Gmaj7 // F#m / Fm /
Em // Gm / A / D ////

CH "English"
Gmaj7 Asus // !A //
Gmaj7 F#m // !F#m / Fm / Em7 ////
Bm7 C Asus // !A //

VS "Farsi"
D // A/C# // Bm // A //
G // A // D // C#m7b5 / F#7 /
Bm // A // Gmaj7 // F#m / Fm /
Em // Gm / A / !D ////

CH "English"

CH "English"

INST
Bm7 F#m7 Bm7 F#m7 x3
Gmaj7 F#m7 Gmaj7 A

VS "Farsi"
D // A/C# // Bm7 // Am7 / D7 /
G // Gm7 // !D ////
Bm // A // Gmaj7 // F#m / Fm /
Em // Gm / A / !D ////

CH "Farsi"

CH "Farsi"

CH "English"

CH "English"

Outro "Verse"
@vocals "English"
D // A/C# // Bm // A //
G // Gm // D ////
@vocals "Farsi"
D // A/C# // Bm // A //
G // Gm // D ////

"#;

/// Default chart content for the editor.
pub const DEFAULT_CHART: &str = EXAMPLE_MESSAGES_OF_HOPE;

/// Named example chart entry for the dropdown.
pub struct ExampleChart {
    pub name: &'static str,
    pub source: &'static str,
}

/// All available example charts.
pub const EXAMPLES: &[ExampleChart] = &[
    ExampleChart {
        name: "New (Empty)",
        source: EMPTY_CHART,
    },
    ExampleChart {
        name: "Thriller - Dirty Loops, Cory Wong",
        source: EXAMPLE_THRILLER,
    },
    ExampleChart {
        name: "Messengers of Hope - Evan Human",
        source: EXAMPLE_MESSAGES_OF_HOPE,
    },
];
