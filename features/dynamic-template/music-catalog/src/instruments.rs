//! Instrument group and sub-group color definitions.
//!
//! Canonical colors for every instrument family used in music production.
//! These are referenced by the auto-color system and the track classification engine.

use color_palette::palette;
use color_palette::Color;

/// Top-level instrument group colors
pub mod groups {
    use super::*;

    pub const DRUMS: Color = palette::red::S500;
    pub const PERCUSSION: Color = palette::orange::S500;
    pub const BASS: Color = palette::yellow::S400;
    /// Defaults to electric guitar color since most "Guitars" folders contain electric
    pub const GUITARS: Color = palette::sky::S700;
    pub const KEYS: Color = palette::green::S500;
    pub const SYNTHS: Color = palette::violet::S500;
    pub const HORNS: Color = palette::amber::S400;
    pub const HARMONICA: Color = palette::stone::S500;
    pub const FIDDLE: Color = palette::rose::S600;
    pub const VOCALS: Color = palette::pink::S500;
    pub const CHOIR: Color = palette::purple::S300;
    pub const ORCHESTRA: Color = palette::purple::S600;
    pub const SFX: Color = palette::teal::S500;
    pub const GUIDE: Color = palette::slate::S400;
    pub const REFERENCE: Color = palette::slate::S500;
    /// Stem-separation outputs (Demucs, LALAL.ai, …) — a neutral utility color.
    pub const STEM_SPLIT: Color = palette::zinc::S500;
}

/// Guitar sub-group colors
pub mod guitars {
    use super::*;

    pub const ELECTRIC: Color = palette::sky::S700;
    pub const ACOUSTIC: Color = palette::cyan::S400;
    pub const STEEL: Color = palette::sky::S400;
    pub const BANJO: Color = palette::amber::S600;
}

/// Vocal sub-group colors
pub mod vocals {
    use super::*;

    pub const LEAD: Color = palette::pink::S500;
    pub const BACKGROUND: Color = palette::pink::S300;
}

/// Drum sub-group colors
pub mod drums {
    use super::*;

    pub const DRUM_KIT: Color = palette::red::S500;
    pub const ELECTRONIC: Color = palette::red::S600;
    pub const KICK: Color = palette::red::S700;
    pub const SNARE: Color = palette::orange::S600;
    pub const TOM: Color = palette::red::S500;
    pub const CYMBALS: Color = palette::amber::S400;
    pub const HI_HAT: Color = palette::yellow::S300;
    pub const ROOM: Color = palette::red::S400;
}

/// Bass sub-group colors
pub mod bass {
    use super::*;

    pub const GUITAR: Color = palette::yellow::S400;
    pub const SYNTH: Color = palette::yellow::S500;
    pub const UPRIGHT: Color = palette::amber::S700;
}

/// Keys sub-group colors
pub mod keys {
    use super::*;

    pub const PIANO: Color = palette::green::S500;
    pub const ELECTRIC: Color = palette::green::S600;
    pub const ORGAN: Color = palette::green::S700;
    pub const HARPSICHORD: Color = palette::green::S300;
    pub const CLAVICHORD: Color = palette::green::S400;
}

/// Synth sub-group colors
pub mod synths {
    use super::*;

    pub const LEAD: Color = palette::violet::S500;
    pub const PAD: Color = palette::violet::S400;
    pub const ARP: Color = palette::violet::S600;
    pub const CHORD: Color = palette::violet::S300;
    pub const KEYS: Color = palette::violet::S700;
    pub const FX: Color = palette::violet::S800;
}

/// Orchestra sub-group colors
pub mod orchestra {
    use super::*;

    pub const STRINGS: Color = palette::amber::S800;
    pub const WOODWINDS: Color = palette::sky::S300;
    pub const BRASS: Color = palette::amber::S500;
    pub const HARP: Color = palette::purple::S400;
    pub const PERCUSSION: Color = palette::orange::S600;

    pub mod strings {
        use super::*;

        pub const VIOLINS: Color = palette::amber::S700;
        pub const VIOLA: Color = palette::amber::S800;
        pub const CELLO: Color = palette::amber::S900;
        pub const CONTRABASS: Color = palette::amber::S950;
    }

    pub mod woodwinds {
        use super::*;

        pub const FLUTES: Color = palette::sky::S200;
        pub const OBOES: Color = palette::sky::S300;
        pub const CLARINETS: Color = palette::sky::S400;
        pub const BASSOONS: Color = palette::sky::S500;
        pub const PICCOLO: Color = palette::sky::S100;
    }

    pub mod brass {
        use super::*;

        pub const TRUMPETS: Color = palette::amber::S400;
        pub const HORNS: Color = palette::amber::S500;
        pub const TROMBONES: Color = palette::amber::S600;
        pub const TUBA: Color = palette::amber::S700;
    }
}

/// Percussion sub-group colors
pub mod percussion {
    use super::*;

    pub const SHAKER: Color = palette::orange::S500;
    pub const TAMBOURINE: Color = palette::orange::S600;
    pub const CABASA: Color = palette::orange::S500;
    pub const CONGA: Color = palette::orange::S700;
    pub const BONGO: Color = palette::orange::S600;
    pub const COWBELL: Color = palette::amber::S400;
    pub const CLAP: Color = palette::orange::S400;
    pub const TRIANGLE: Color = palette::yellow::S300;
}
