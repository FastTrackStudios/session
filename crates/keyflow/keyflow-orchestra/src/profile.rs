//! Instrument family profiles + track-name detection.
//!
//! The Cinematic Studio Series shares CC58/CC1 and the common articulation
//! bands across CSS/CSW/CSB, but the libraries DIFFER on the 11–15 band
//! (Spiccato vs Repetitions), 56–60 (Tremolo vs nothing vs Muted-long),
//! Pizzicato (CSS only), and CC2 behaviour (x-fade / on-off switch / absent).
//! `generic` and `harp` are the safe profiles for non-CSS tracks.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProfileKind {
    Strings,
    Woodwinds,
    Brass,
    BrassTrumpet,
    Generic,
    Harp,
}

impl ProfileKind {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Strings => "strings",
            Self::Woodwinds => "woodwinds",
            Self::Brass => "brass",
            Self::BrassTrumpet => "brass_trumpet",
            Self::Generic => "generic",
            Self::Harp => "harp",
        }
    }
    pub fn profile(&self) -> &'static Profile {
        profile(*self)
    }
}

/// How CC2 (vibrato) behaves for a family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VibratoMode {
    /// Continuous x-fade following CC1 (CSS strings).
    Xfade,
    /// On/off switch (CSW).
    Switch,
    /// No CC2 at all.
    None,
}

/// Per-family sampled legato-transition delays (ms, from each manual).
/// `modes` = the patch has the Expressive/Low-Latency toggle (CSS/CSW);
/// brass has only two velocity zones and no mode toggle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LegatoDelays {
    pub modes: bool,
    /// Expressive mode: (slow, medium, fast) — or `two` when `modes=false`.
    pub expr_slow: f64,
    pub expr_medium: f64,
    pub expr_fast: f64,
    /// Low-latency mode: (medium, fast). Unused when `modes=false`.
    pub ll_medium: f64,
    pub ll_fast: f64,
}

pub const LEGATO_STRINGS: LegatoDelays = LegatoDelays {
    modes: true,
    expr_slow: 333.0,
    expr_medium: 250.0,
    expr_fast: 100.0,
    ll_medium: 150.0,
    ll_fast: 65.0,
};
pub const LEGATO_WOODWINDS: LegatoDelays = LegatoDelays {
    modes: true,
    expr_slow: 220.0,
    expr_medium: 130.0,
    expr_fast: 90.0,
    ll_medium: 90.0,
    ll_fast: 70.0,
};
/// Trombone/horn/tuba: two velocity zones only (medium=expr_slow+expr_medium, fast).
pub const LEGATO_BRASS: LegatoDelays = LegatoDelays {
    modes: false,
    expr_slow: 230.0,
    expr_medium: 230.0,
    expr_fast: 100.0,
    ll_medium: 230.0,
    ll_fast: 100.0,
};
/// Trumpets respond faster (180ms).
pub const LEGATO_BRASS_TRUMPET: LegatoDelays = LegatoDelays {
    modes: false,
    expr_slow: 180.0,
    expr_medium: 180.0,
    expr_fast: 100.0,
    ll_medium: 180.0,
    ll_fast: 100.0,
};

/// Capabilities of an instrument family — port of `M.PROFILES`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Profile {
    pub kind: ProfileKind,
    pub spiccato: bool,
    pub tremolo: bool,
    pub pizz: bool,
    pub vib: VibratoMode,
    pub con_sord: bool,
    pub porta: bool,
    pub legato: Option<LegatoDelays>,
    /// A "solo" passage routes to a separate solo-channel block only for
    /// SECTION instruments; woodwinds are already solo instruments.
    pub solo_separate: bool,
    /// Notes + channel split + CC1 only — no CC58, no CC2, no timing pull.
    pub no_keyswitch: bool,
    /// Harp: glissandos expand into note sweeps through the notated chord.
    pub gliss_sweep: bool,
    /// Harp: notes ring at written length, no mono legato per channel.
    pub polyphonic: bool,
    /// Harp: dynamics ride VELOCITY (pluck strength), not the mod wheel.
    pub vel_dynamics: bool,
    pub no_cc_dynamics: bool,
}

const fn base(kind: ProfileKind) -> Profile {
    Profile {
        kind,
        spiccato: false,
        tremolo: false,
        pizz: false,
        vib: VibratoMode::None,
        con_sord: false,
        porta: false,
        legato: None,
        solo_separate: false,
        no_keyswitch: false,
        gliss_sweep: false,
        polyphonic: false,
        vel_dynamics: false,
        no_cc_dynamics: false,
    }
}

pub static STRINGS: Profile = Profile {
    spiccato: true,
    tremolo: true,
    pizz: true,
    vib: VibratoMode::Xfade,
    con_sord: true,
    porta: true,
    legato: Some(LEGATO_STRINGS),
    solo_separate: true,
    ..base(ProfileKind::Strings)
};
pub static WOODWINDS: Profile = Profile {
    vib: VibratoMode::Switch,
    legato: Some(LEGATO_WOODWINDS),
    ..base(ProfileKind::Woodwinds)
};
pub static BRASS: Profile = Profile {
    legato: Some(LEGATO_BRASS),
    solo_separate: true,
    ..base(ProfileKind::Brass)
};
pub static BRASS_TRUMPET: Profile = Profile {
    legato: Some(LEGATO_BRASS_TRUMPET),
    solo_separate: true,
    ..base(ProfileKind::BrassTrumpet)
};
pub static GENERIC: Profile = Profile {
    no_keyswitch: true,
    ..base(ProfileKind::Generic)
};
pub static HARP: Profile = Profile {
    no_keyswitch: true,
    gliss_sweep: true,
    polyphonic: true,
    vel_dynamics: true,
    no_cc_dynamics: true,
    ..base(ProfileKind::Harp)
};

pub fn profile(kind: ProfileKind) -> &'static Profile {
    match kind {
        ProfileKind::Strings => &STRINGS,
        ProfileKind::Woodwinds => &WOODWINDS,
        ProfileKind::Brass => &BRASS,
        ProfileKind::BrassTrumpet => &BRASS_TRUMPET,
        ProfileKind::Generic => &GENERIC,
        ProfileKind::Harp => &HARP,
    }
}

/// Substring overrides for specific track/part names (checked first,
/// lowercase). Choir → generic, harp → harp.
const PROFILE_OVERRIDES: &[(&str, ProfileKind)] = &[
    ("s-a", ProfileKind::Generic),
    ("t-b", ProfileKind::Generic),
    ("s a", ProfileKind::Generic),
    ("t b", ProfileKind::Generic),
    ("choir", ProfileKind::Generic),
    ("soprano", ProfileKind::Generic),
    // "altos" (French for violas) must win over the choir "alto" override.
    ("altos", ProfileKind::Strings),
    ("alto", ProfileKind::Generic),
    ("tenor", ProfileKind::Generic),
    ("harp", ProfileKind::Harp),
    ("harpe", ProfileKind::Harp),
];

/// Family keyword lists, scanned in order woodwinds → brass_trumpet → brass →
/// strings — woodwinds first so "bassoon" doesn't match "bass".
const FAMILY_KEYWORDS: &[(ProfileKind, &[&str])] = &[
    (
        ProfileKind::Woodwinds,
        &[
            "flute",
            "piccolo",
            "oboe",
            "cor anglais",
            "english horn",
            "clarinet",
            "bassoon",
            "contrabassoon",
            "picc",
            // French score names (also covers "contrebasson"/"clarinette")
            "basson",
            "hautbois",
            "clarinette",
        ],
    ),
    (
        ProfileKind::BrassTrumpet,
        &["trumpet", "cornet", "flugel", "tpt"],
    ),
    (
        ProfileKind::Brass,
        &["horn", "trombone", "tuba", "euphonium", "tbn", "hn"],
    ),
    (
        ProfileKind::Strings,
        &[
            "violin",
            "viola",
            "cello",
            "double bass",
            "contrabass",
            "bass",
            "vln",
            "vla",
            "vc",
            // French score names ("violoncelle" also matches via "violon")
            "violon",
            "contrebasse",
        ],
    ),
];

/// Lowercase + collapse whitespace, like the front-end's `normalize`.
fn normalize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_space = false;
    for c in name.trim().chars() {
        if c.is_whitespace() {
            if !last_space {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.extend(c.to_lowercase());
            last_space = false;
        }
    }
    out
}

/// Detect the instrument family from a track/part name — port of
/// `detectProfile`. Defaults to `generic` when nothing matches.
pub fn detect_profile(name: &str) -> ProfileKind {
    let n = normalize(name);
    for (sub, kind) in PROFILE_OVERRIDES {
        if n.contains(sub) {
            return *kind;
        }
    }
    for (kind, keywords) in FAMILY_KEYWORDS {
        for kw in *keywords {
            if n.contains(kw) {
                return *kind;
            }
        }
    }
    ProfileKind::Generic
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bassoon_is_woodwind_not_bass() {
        assert_eq!(detect_profile("Bassoon 1"), ProfileKind::Woodwinds);
        assert_eq!(detect_profile("Double Bass"), ProfileKind::Strings);
    }

    #[test]
    fn french_part_names() {
        assert_eq!(detect_profile("Violon 1"), ProfileKind::Strings);
        assert_eq!(detect_profile("Violoncelles"), ProfileKind::Strings);
        assert_eq!(detect_profile("Altos"), ProfileKind::Strings);
        assert_eq!(detect_profile("Contrebasses"), ProfileKind::Strings);
        assert_eq!(detect_profile("Contrebasson"), ProfileKind::Woodwinds);
        assert_eq!(detect_profile("Harpe"), ProfileKind::Harp);
        // choir Alto still generic
        assert_eq!(detect_profile("Alto"), ProfileKind::Generic);
    }

    #[test]
    fn families() {
        assert_eq!(detect_profile("Violin 1"), ProfileKind::Strings);
        assert_eq!(detect_profile("Trumpet in Bb 1"), ProfileKind::BrassTrumpet);
        assert_eq!(detect_profile("Horn in F 1-4"), ProfileKind::Brass);
        assert_eq!(detect_profile("Harp"), ProfileKind::Harp);
        assert_eq!(detect_profile("Choir S-A"), ProfileKind::Generic);
        assert_eq!(detect_profile("Timpani"), ProfileKind::Generic);
    }
}
