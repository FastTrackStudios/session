//! The Tone rack — EQ, compressor and saturation, inside the strip.
//!
//! The mix phase after Balance is Tone, and it is three processors:
//! compression, EQ, saturation. The parallel filters and the parallel
//! compressors are a different phase and are deliberately not here.
//!
//! A channel strip that shows those three is a channel you can mix
//! without opening a plugin window, which is the whole point — a mix is
//! a comparison between tracks, and a comparison you have to make one
//! floating window at a time is not a comparison.
//!
//! # It draws the plugins' own curves
//!
//! Every response in this module comes from the function the plugin's
//! own editor draws with:
//!
//! | panel | curve from |
//! |---|---|
//! | EQ | `eq_ui::eq_graph_response::calculate_combined_response` |
//! | Compressor | `comp_ui::comp_graph_svg::compress_transfer` |
//! | Saturation | `saturate_dsp::preamp::analysis::transfer_curve` |
//!
//! None of the maths is here, and that is the point: a curve in the
//! strip that disagreed with the curve in the plugin window would be
//! worse than no curve at all, because it would be believed. This module
//! samples those functions into polylines and places them in a box.
//!
//! # What it does not do yet
//!
//! Move. The curves depend only on the parameters, so they are recorded
//! into the mixer's scene once and replayed under the transform like
//! everything else. The parts that DO move — gain reduction, the
//! spectrum behind the EQ — are metering, and metering arrives through
//! `daw_ui::widgets::trace` in a live pass over the top, so that a track
//! whose processing was rendered offline still animates.

use anyrender::{PaintScene, Scene};
use delay_dsp::engine::{DelayStyle, Family as DelayFamily};
use eq_ui::eq_graph_interaction::{self as interaction, GraphMapper, Mods};
use eq_ui::eq_graph_model::{EqBand, EqBandShape, StereoMode};
use eq_ui::eq_graph_response::calculate_combined_response;
use fts_audio_ui::axis::{DbAxis, FreqAxis};
use reverb_dsp::algorithm::{AlgorithmType, Family as RoomFamily};
use saturate_dsp::preamp::{Circuit, ClassAPreamp, SideShaper};
use vello::kurbo::{Affine, BezPath, Line, Rect, Shape, Stroke};
use vello::peniko::{Color, Fill};

use crate::arrangement::Palette;
use crate::live::Meters;
use crate::text::Font;

/// The sample rate the curves are drawn against.
///
/// A display constant, not an audio one: the EQ's response at 20 kHz
/// bends with the rate, and a strip that redrew its curve when the
/// device changed would be showing the device rather than the setting.
const DISPLAY_RATE: f64 = 48_000.0;

/// How many points a curve is sampled at.
///
/// One per ~2px of a 200-wide panel. Beyond that the polyline is finer
/// than the pixels and costs recording time for nothing — and this runs
/// once per track at project open, not per frame.
const SAMPLES: usize = 96;

/// The three processors of the Tone phase, for one track.
///
/// Deliberately the plugins' own parameter types rather than a summary
/// of them. A rack that held its own reduced idea of "an EQ" would have
/// to be widened every time a band gained a shape, and would be drawing
/// something the plugin cannot produce.
#[derive(Clone, Debug)]
pub struct Tone {
    /// Rescue.
    pub rescue_eq: Vec<EqBand>,
    pub gate: Gate,
    pub rescue_comp: Comp,
    /// Tone.
    pub eq: Vec<EqBand>,
    pub comp: Comp,
    pub sat: ClassAPreamp,
    /// Which of the plugin's profiles the stage is — an index into
    /// `saturate_profiles::PROFILES`. The selector picks one; `sat` is
    /// what `apply` made of it, plus whatever the grips moved since.
    pub sat_profile: usize,
    /// The quantiser after the stage, which only the digital profiles
    /// switch on. Kept so the curve can draw its steps.
    pub sat_digital: saturate_dsp::digital::DigitalStage,
    /// Polish.
    pub de_ess: Suppress,
    pub resonance: Suppress,
    /// Relational.
    pub space: Vec<EqBand>,
    /// Depth.
    pub delay: Echo,
    pub reverb: Room,
    /// The advanced Depth units' own EQs: ahead of the effect, and
    /// after it. Inside the one instance — a reverb return does not
    /// carry three plugins, it carries one with an EQ at each end.
    pub pre_eq: Vec<EqBand>,
    pub post_eq: Vec<EqBand>,
    /// The reverb's Decay Rate EQ: bands of decay-TIME multipliers over
    /// frequency, drawn as gain where ±12 dB is ×0.25..×4. The plugin's
    /// `decay_bands`, in the shape the EQ graph already draws.
    pub decay_eq: Vec<EqBand>,
    /// The widener: 0 is mono, 1 is as recorded, 2 is pushed past the
    /// speakers.
    pub wide: f32,
    /// The pitch shifter, in semitones, and how much of it is heard.
    pub pitch: i32,
    pub pitch_mix: f32,
    /// What this track is FOR, which decides its chain — see [`Role`].
    pub role: Role,
    /// The presets this track keeps, brighter to darker, and which of
    /// them was loaded last. A row of chips at the top of the rack.
    pub presets: Vec<Preset>,
    pub preset: Option<usize>,
    /// Which of the three are switched out.
    pub bypass: Bypass,
    /// How far the EQ graph is zoomed, as an index into the plugin's
    /// own [`eq_ui::eq_graph_model::DB_RANGE_STEPS`].
    ///
    /// An INDEX rather than a number of decibels, because the zoom is a
    /// list of stops the plugin publishes and a rack that invented its
    /// own would disagree with the editor about what "one step out"
    /// means. Per track, like everything else here: a vocal worked at
    /// ±3 and a room mic at ±18 is the normal case, not a special one.
    pub eq_range: i32,
}

/// What a track is for, which decides which units its rack shows.
///
/// Not every track carries every step. A lead vocal is a channel and
/// gets the whole chain; a delay return is ONE delay — a de-esser on
/// the way in, the delay itself, an EQ on the way out — and a reverb
/// return is one reverb with a de-esser, an EQ at each end and its own
/// decay-rate EQ. Decided from where the track sits (the folder it is
/// in) and what it is called, at seed time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// An instrument or a voice: the eleven-unit chain.
    Channel,
    /// A folder. Its level and its mute; the processing is on what it
    /// sums.
    Bus,
    /// A delay return: the advanced delay, one instance.
    Delay,
    /// A reverb return: the advanced reverb, one instance.
    Reverb,
    /// A widener.
    Wide,
    /// A pitch shifter.
    Pitch,
}

impl Role {
    /// Decide a track's role from its name, whether it is a folder,
    /// and the names of the folders above it, nearest last.
    #[must_use]
    pub fn of(name: &str, is_folder: bool, ancestors: &[String]) -> Self {
        if is_folder {
            return Self::Bus;
        }
        let lower = name.to_lowercase();
        if lower.starts_with("wide") || lower.starts_with("widen") {
            return Self::Wide;
        }
        if lower.starts_with("oct") || lower.starts_with("pitch") {
            return Self::Pitch;
        }
        for folder in ancestors.iter().rev() {
            let folder = folder.to_lowercase();
            if folder.starts_with("delay") {
                return Self::Delay;
            }
            if folder.starts_with("verb") || folder.starts_with("reverb") {
                return Self::Reverb;
            }
            if folder.starts_with("pitch") {
                return Self::Pitch;
            }
            if folder.starts_with("wide") {
                return Self::Wide;
            }
        }
        Self::Channel
    }

    /// The chain this role shows, or `None` for the channel's own.
    #[must_use]
    pub const fn panels(self) -> Option<&'static [Which]> {
        match self {
            Self::Channel => None,
            Self::Bus => Some(&BUS_CHAIN),
            Self::Delay => Some(&DELAY_CHAIN),
            Self::Reverb => Some(&REVERB_CHAIN),
            Self::Wide => Some(&WIDE_CHAIN),
            Self::Pitch => Some(&PITCH_CHAIN),
        }
    }
}

/// A delay return: the de-esser on the way in, the delay itself with
/// its machine selector and knobs, and an EQ on the way out.
pub const DELAY_CHAIN: [Which; 5] = [Which::Presets, Which::DeEssIn, Which::Delay, Which::Knobs, Which::PostEq];

/// A reverb return: de-esser, an EQ into the space, the space with its
/// selector and knobs, its decay-rate EQ, and an EQ out.
pub const REVERB_CHAIN: [Which; 7] = [
    Which::Presets,
    Which::DeEssIn,
    Which::PreEq,
    Which::Reverb,
    Which::Knobs,
    Which::DecayEq,
    Which::PostEq,
];

/// A bus: what it sums is processed elsewhere.
pub const BUS_CHAIN: [Which; 2] = [Which::Eq, Which::Comp];

pub const WIDE_CHAIN: [Which; 2] = [Which::Presets, Which::Wide];
pub const PITCH_CHAIN: [Which; 2] = [Which::Presets, Which::Pitch];

/// A named setting of the whole rack, kept on the track.
///
/// Stored as a whole `Tone` rather than a diff, so loading one is a
/// copy and nothing is left over from the setting before it. The stored
/// tone carries no presets of its own.
#[derive(Clone, Debug)]
pub struct Preset {
    pub name: String,
    pub tone: Box<Tone>,
}

/// The EQ graph's default zoom.
///
/// The plugin's own default (±6 dB), not a second opinion — see
/// `DEFAULT_DB_RANGE` there for why ±3 was too tight to work in.
pub const DEFAULT_EQ_RANGE: i32 = 1;

impl Tone {
    /// The EQ graph's dB range, top to bottom.
    #[must_use]
    pub fn eq_db_range(&self) -> f64 {
        eq_ui::eq_graph_model::db_range_for_index(self.eq_range)
    }

    /// Zoom the graph by `steps`, positive being further out.
    ///
    /// Returns whether it moved, so a caller knows whether to re-record
    /// rather than guessing.
    pub fn zoom_eq(&mut self, steps: i32) -> bool {
        let last = i32::try_from(eq_ui::eq_graph_model::DB_RANGE_STEPS.len())
            .unwrap_or(1)
            .saturating_sub(1);
        let to = self.eq_range.saturating_add(steps).clamp(0, last);
        let moved = to != self.eq_range;
        self.eq_range = to;
        moved
    }

    /// The band set a given EQ panel edits.
    ///
    /// Three panels share one drawing and one interaction model and
    /// differ only in which list they are about — so this is the one
    /// place that mapping lives, and adding a fourth EQ is one arm.
    #[must_use]
    pub fn bands(&mut self, which: Which) -> Option<&mut Vec<EqBand>> {
        match which {
            Which::RescueEq => Some(&mut self.rescue_eq),
            Which::Eq => Some(&mut self.eq),
            Which::Space => Some(&mut self.space),
            Which::PreEq => Some(&mut self.pre_eq),
            Which::PostEq => Some(&mut self.post_eq),
            Which::DecayEq => Some(&mut self.decay_eq),
            _ => None,
        }
    }

    /// The compressor a given panel edits.
    #[must_use]
    pub const fn compressor(&mut self, which: Which) -> Option<&mut Comp> {
        match which {
            Which::RescueComp => Some(&mut self.rescue_comp),
            Which::Comp => Some(&mut self.comp),
            _ => None,
        }
    }

    /// The chain this track shows, given the chain the window would
    /// show a channel.
    ///
    /// `default` is what the phase filter and the tone toggle decided
    /// for a channel — empty when the rack is off — so an FX return
    /// follows those decisions and then draws its own units instead.
    #[must_use]
    pub fn panels<'a>(&self, default: &'a [Which]) -> &'a [Which] {
        if default.is_empty() {
            return default;
        }
        self.role.panels().unwrap_or(default)
    }

    /// Whether this track wants the focused column layout — see
    /// `strip::Layout`. A channel's chain is taller than the rack's
    /// share and earns the column; a return's is short, and a focused
    /// return stays stacked.
    #[must_use]
    pub fn wants_column(&self) -> bool {
        self.role == Role::Channel
    }

    /// The band set a panel draws, to read.
    #[must_use]
    pub fn bands_ref(&self, which: Which) -> &[EqBand] {
        match which {
            Which::RescueEq => &self.rescue_eq,
            Which::Space => &self.space,
            Which::PreEq => &self.pre_eq,
            Which::PostEq => &self.post_eq,
            Which::DecayEq => &self.decay_eq,
            _ => &self.eq,
        }
    }

    /// Load one of this track's presets into every setting the rack
    /// draws, keeping the presets and the role.
    ///
    /// The whole visualiser follows, because the settings ARE the
    /// picture: a preset is not a name on a chip, it is what the chain
    /// becomes when you click it.
    pub fn load_preset(&mut self, index: usize) {
        let Some(preset) = self.presets.get(index) else {
            return;
        };
        let loaded = (*preset.tone).clone();
        let presets = std::mem::take(&mut self.presets);
        let role = self.role;
        *self = loaded;
        self.presets = presets;
        self.role = role;
        self.preset = Some(index);
    }

    /// Which machine the saturator reads as: the quantiser if a digital
    /// profile is in, else what the shapers say.
    #[must_use]
    pub fn sat_circuit(&self) -> Circuit {
        if saturate_profiles::PROFILES
            .get(self.sat_profile)
            .is_some_and(|p| p.voicing.digital)
        {
            Circuit::Steps
        } else {
            self.sat.circuit()
        }
    }

    /// Point the stage at a profile, through the plugin's own `apply`,
    /// keeping the drive knob where it was and the mix as it is.
    ///
    /// The drive is carried as a KNOB position rather than a gain: a
    /// fuzz travels further than a tape machine, and carrying the gain
    /// across would land a modest tape drive at the top of the fuzz.
    pub fn set_sat_profile(&mut self, index: usize) {
        let Some(profile) = saturate_profiles::PROFILES.get(index) else {
            return;
        };
        let was_scale = saturate_profiles::PROFILES
            .get(self.sat_profile)
            .map_or(1.0, |p| p.voicing.drive_scale)
            .max(f32::EPSILON);
        let knob = ((self.sat.drive - 1.0) / (15.0 * was_scale)).clamp(0.0, 1.0);
        let mix = self.sat.mix;
        let controls = saturate_profiles::Controls {
            drive: knob,
            mix,
            ..saturate_profiles::Controls::default()
        };
        saturate_profiles::apply(profile, &controls, &mut self.sat, &mut self.sat_digital);
        self.sat.mix = mix;
        self.sat_profile = index;
    }

    /// A click on a circuit chip: the plugin's own rail rule — the
    /// family's first profile, or the next one if you are already in it.
    pub fn choose_sat_family(&mut self, category: usize) {
        let to = saturate_profiles::rail_click_target(self.sat_profile, category);
        self.set_sat_profile(to);
    }

    /// The next profile, wrapping — what a click on the glyph does.
    pub fn cycle_sat(&mut self) {
        let next = self.sat_profile.saturating_add(1);
        let to = if next >= saturate_profiles::PROFILES.len() { 0 } else { next };
        self.set_sat_profile(to);
    }

    /// The wet/dry mix a given panel edits.
    #[must_use]
    pub const fn mix(&mut self, which: Which) -> Option<&mut f32> {
        match which {
            Which::Delay => Some(&mut self.delay.mix),
            Which::Reverb => Some(&mut self.reverb.mix),
            Which::Sat => Some(&mut self.sat.mix),
            _ => None,
        }
    }

    /// The suppressor a given panel edits.
    #[must_use]
    pub const fn suppressor(&mut self, which: Which) -> Option<&mut Suppress> {
        match which {
            Which::DeEss | Which::DeEssIn => Some(&mut self.de_ess),
            Which::Resonance => Some(&mut self.resonance),
            _ => None,
        }
    }

    /// Render every reverb tail this rack needs, now, on this thread.
    ///
    /// For a bench or a test that must be deterministic — see
    /// [`crate::live::render_tail_now`].
    pub fn prerender_tails(&self) {
        crate::live::render_tail_now(self.reverb.key());
    }

    /// Every panel with a threshold, as one number.
    ///
    /// The compressors, the gate and the two suppressors all have a red
    /// line you drag, and they all mean "where this starts acting" —
    /// which is why one grip serves them and why the ranges differ.
    #[must_use]
    pub const fn threshold(&self, which: Which) -> Option<(f32, f32, f32)> {
        match which {
            Which::RescueComp => Some((self.rescue_comp.threshold, -60.0, 0.0)),
            Which::Comp => Some((self.comp.threshold, -60.0, 0.0)),
            Which::Gate => Some((self.gate.threshold, -80.0, 0.0)),
            // A suppressor's threshold is how far ABOVE its own average
            // a peak has to stand, so its range is small and positive.
            Which::DeEss | Which::DeEssIn => Some((self.de_ess.threshold, 0.0, 24.0)),
            Which::Resonance => Some((self.resonance.threshold, 0.0, 24.0)),
            _ => None,
        }
    }

    /// And setting it, clamped to that panel's own range.
    pub const fn set_threshold(&mut self, which: Which, to: f32) {
        let Some((_, low, high)) = self.threshold(which) else {
            return;
        };
        let to = if to < low {
            low
        } else if to > high {
            high
        } else {
            to
        };
        match which {
            Which::RescueComp => self.rescue_comp.threshold = to,
            Which::Comp => self.comp.threshold = to,
            Which::Gate => self.gate.threshold = to,
            Which::DeEss | Which::DeEssIn => self.de_ess.threshold = to,
            Which::Resonance => self.resonance.threshold = to,
            _ => {}
        }
    }

    /// The next stop, wrapping — what a click on the readout does.
    ///
    /// Wrapping rather than stopping because a chip you click is a
    /// cycle: at the last stop the only useful thing left to do is go
    /// back to the first, and a click that does nothing reads as a
    /// broken control.
    pub fn cycle_eq_range(&mut self) {
        let last = i32::try_from(eq_ui::eq_graph_model::DB_RANGE_STEPS.len()).unwrap_or(1);
        self.eq_range = self.eq_range.saturating_add(1).rem_euclid(last.max(1));
    }
}

/// Which processors are bypassed.
///
/// Per PROCESSOR, not per rack: bypassing is how you check a decision,
/// and the question is almost always "what does this track sound like
/// without the compressor", not "without any of it". A whole-rack
/// switch would make the common comparison the one you cannot make.
///
/// A bitset rather than a field each, because the chain grows: it was
/// three bools, and every processor added meant three more edits in
/// three functions that could disagree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bypass {
    out: u16,
}

impl Bypass {
    const fn bit(which: Which) -> u16 {
        1_u16 << (which as u16)
    }

    #[must_use]
    pub const fn is(self, which: Which) -> bool {
        self.out & Self::bit(which) != 0
    }

    pub const fn toggle(&mut self, which: Which) {
        self.out ^= Self::bit(which);
    }

    /// Whether anything at all is switched out — for a caller that
    /// wants to say so without asking eleven times.
    #[must_use]
    pub const fn any(self) -> bool {
        self.out != 0
    }
}

/// What a suppressor is set to, in one short line.
fn suppression(set: Suppress) -> String {
    format!("{:.0}dB · {:.0}%", set.threshold, set.depth * 100.0)
}

/// A gate, as its display needs it.
///
/// The compressor's mirror: it acts BELOW its threshold rather than
/// above, which is the whole difference and the reason the two cannot
/// share a display without saying which way they work.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gate {
    /// Where it opens, in dBFS.
    pub threshold: f32,
    /// How far down it takes what stays shut. Not silence: a gate that
    /// closes completely turns a room into a series of holes, and the
    /// number you actually reach for is how much LESS of it you want.
    pub range: f32,
    /// How fast it opens.
    pub attack: f32,
    /// How long it stays open after the signal drops back.
    pub hold: f32,
    /// And how fast it closes then.
    pub release: f32,
}

impl Default for Gate {
    fn default() -> Self {
        Self {
            threshold: -40.0,
            range: -18.0,
            attack: 1.0,
            hold: 60.0,
            release: 180.0,
        }
    }
}

/// A spectral suppressor, as its display needs it.
///
/// The de-esser and the resonance suppressor are one processor with two
/// jobs: find where the spectrum stands proud of its own average, and
/// pull those places down. The de-esser looks only in the sibilance
/// range and the suppressor looks everywhere, which is a BAND and not a
/// different algorithm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Suppress {
    /// How far above its own average a peak has to stand before it is
    /// treated as a resonance, in dB.
    pub threshold: f32,
    /// How much of what it finds it takes off, 0 to 1.
    pub depth: f32,
    /// How narrow a peak has to be to count — the width of the average
    /// it is compared against, in octaves. Wide, and a whole region
    /// reads as one resonance; narrow, and nothing does.
    pub sharpness: f32,
    /// The range it listens in.
    pub low: f32,
    pub high: f32,
}

impl Suppress {
    /// The de-esser's defaults: sibilance only.
    #[must_use]
    pub const fn sibilance() -> Self {
        Self {
            threshold: 4.0,
            depth: 0.6,
            sharpness: 0.5,
            low: 4_000.0,
            high: 12_000.0,
        }
    }

    /// And the resonance suppressor's: the whole spectrum, gentler.
    #[must_use]
    pub const fn broadband() -> Self {
        Self {
            threshold: 6.0,
            depth: 0.45,
            sharpness: 0.33,
            low: 100.0,
            high: 18_000.0,
        }
    }
}

/// A delay, as its display needs it.
///
/// Time, feedback and mix — the three that decide what the repeats
/// LOOK like, which is what the panel draws. The rest of the plugin's
/// parameters (wow, flutter, drive, duck) change how a repeat sounds
/// rather than where it lands, and a strip-width picture cannot show
/// them without lying about what it is showing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Echo {
    pub time: f32,
    pub feedback: f32,
    pub mix: f32,
    /// How dark the repeats are, 0 bright .. 1 dark.
    pub tone: f32,
    /// How wide, 0 mono .. 1 full.
    pub width: f32,
    /// Which machine makes the repeats. Not a picture of a knob: the
    /// plugin's fourteen styles are fourteen different engines, and
    /// the strip draws the family the style belongs to the way the
    /// plugin's own faces do — a tape softens, a chip smears, a
    /// shimmer climbs.
    pub style: DelayStyle,
}

impl Default for Echo {
    fn default() -> Self {
        Self {
            time: 320.0,
            feedback: 0.38,
            mix: 0.22,
            tone: 0.4,
            width: 0.5,
            style: DelayStyle::Tape,
        }
    }
}

impl Echo {
    /// The family the style draws as.
    #[must_use]
    pub const fn family(self) -> DelayFamily {
        self.style.family()
    }

    /// Pick a family from the selector strip: its first style, or the
    /// next one along if the current style is already in it — so a
    /// family with several machines is walked by clicking its chip
    /// again.
    pub fn choose_family(&mut self, index: usize) {
        let Some(family) = DelayFamily::ALL.get(index).copied() else {
            return;
        };
        self.style = next_in(family.styles(), self.style);
    }

    /// The next style, wrapping — what a click on the glyph does.
    pub const fn cycle_style(&mut self) {
        let next = self.style.to_index().saturating_add(1);
        self.style = if next >= DelayStyle::COUNT {
            DelayStyle::Tape
        } else {
            DelayStyle::from_index(next)
        };
    }
}

/// A reverb, as its display needs it.
///
/// An impulse, the gap before its tail starts, and the tail — which is
/// `reverb-ui`'s own third centrepiece, "the recorded thing itself: an
/// impulse and its decay envelope". The one picture of a reverb that
/// survives being drawn a hundred and thirty pixels wide.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Room {
    /// Seconds to −60 dB.
    pub decay: f32,
    /// Milliseconds before the tail begins.
    pub predelay: f32,
    pub mix: f32,
    /// Which space. The tail the strip draws is this algorithm's own
    /// impulse response, rendered — a plate and a spring with the same
    /// decay are different pictures because they are different sounds.
    pub algorithm: AlgorithmType,
    /// The space's size, 0..1, as the algorithm takes it.
    pub size: f32,
    /// High-frequency damping, 0..1.
    pub damping: f32,
    /// How dense the reflections are, 0..1.
    pub diffusion: f32,
}

impl Default for Room {
    fn default() -> Self {
        Self {
            decay: 1.8,
            predelay: 24.0,
            mix: 0.18,
            algorithm: AlgorithmType::Room,
            size: 0.5,
            damping: 0.3,
            diffusion: 0.7,
        }
    }
}

impl Room {
    /// The family the algorithm draws as.
    #[must_use]
    pub const fn family(self) -> RoomFamily {
        self.algorithm.family()
    }

    /// What the tail is rendered from.
    #[must_use]
    pub fn key(self) -> crate::live::RoomKey {
        crate::live::RoomKey::of(self.algorithm, self.decay, self.size, self.damping)
    }

    /// Pick a family from the selector strip — see [`Echo::choose_family`].
    pub fn choose_family(&mut self, index: usize) {
        let Some(family) = RoomFamily::ALL.get(index).copied() else {
            return;
        };
        self.algorithm = next_in(family.algorithms(), self.algorithm);
    }

    /// The next algorithm, wrapping — what a click on the glyph does.
    pub fn cycle_algorithm(&mut self) {
        let next = self.algorithm.index().saturating_add(1);
        self.algorithm = if next >= AlgorithmType::ALL.len() {
            AlgorithmType::Room
        } else {
            AlgorithmType::from_index(next)
        };
    }
}

/// The member of a family a click on its chip lands on: the one after
/// `current` if `current` is in the family, else the first. `current`
/// itself if the family is empty.
fn next_in<T: Copy + PartialEq>(members: impl Iterator<Item = T>, current: T) -> T {
    let members: Vec<T> = members.collect();
    let Some(first) = members.first().copied() else {
        return current;
    };
    members
        .iter()
        .position(|m| *m == current)
        .and_then(|at| members.get(at.saturating_add(1)).copied())
        .unwrap_or(first)
}

/// A compressor, as its display needs it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Comp {
    /// Where it starts working, in dBFS. The red line across the
    /// visualiser — it is a LEVEL, and a level belongs on the axis the
    /// levels are drawn against rather than on a knob you have to read
    /// a number off.
    pub threshold: f32,
    pub ratio: f32,
    pub knee: f32,
    /// How fast it gets there, in milliseconds.
    pub attack: f32,
    /// And how fast it lets go.
    pub release: f32,
}

impl Default for Comp {
    fn default() -> Self {
        Self {
            threshold: -18.0,
            ratio: 3.0,
            knee: 6.0,
            attack: 10.0,
            release: 120.0,
        }
    }
}

/// The range each of the compressor's knobs travels.
///
/// Stated once because three places need them and they have to agree:
/// the knob's own fraction, the drag that moves it, and the reset that
/// puts it back. Attack and release are logarithmic because a
/// millisecond matters at the fast end and twenty do not at the slow.
impl Comp {
    /// Ratio as a knob fraction, and back.
    #[must_use]
    pub fn ratio_norm(self) -> f64 {
        ((f64::from(self.ratio) - 1.0) / 19.0).clamp(0.0, 1.0)
    }

    #[must_use]
    pub fn attack_norm(self) -> f64 {
        log_norm(f64::from(self.attack), 0.1, 200.0)
    }

    #[must_use]
    pub fn release_norm(self) -> f64 {
        log_norm(f64::from(self.release), 5.0, 3_000.0)
    }
}

/// A compressor knob's value as a fraction of its own range.
fn knob_norm(comp: Comp, grip: Grip) -> f64 {
    match grip {
        Grip::Ratio(_) => comp.ratio_norm(),
        Grip::Attack(_) => comp.attack_norm(),
        Grip::Release(_) => comp.release_norm(),
        _ => 0.0,
    }
}

/// And back, clamped to it.
fn set_knob(comp: &mut Comp, grip: Grip, to: f64) {
    let to = to.clamp(0.0, 1.0);
    match grip {
        Grip::Ratio(_) => comp.ratio = f64_to_f32(to.mul_add(19.0, 1.0)),
        Grip::Attack(_) => comp.attack = f64_to_f32(log_denorm(to, 0.1, 200.0)),
        Grip::Release(_) => comp.release = f64_to_f32(log_denorm(to, 5.0, 3_000.0)),
        _ => {}
    }
}

/// A time, short enough for a header.
///
/// Sub-millisecond attacks are real and a "0ms" would be a lie, so the
/// fast end keeps a decimal and everything above ten drops it — which
/// is also where a millisecond stops being a distinction anyone hears.
fn millis(ms: f32) -> String {
    if ms < 10.0 {
        format!("{ms:.1}ms")
    } else {
        format!("{ms:.0}ms")
    }
}

/// A value's position on a logarithmic range, 0..1.
fn log_norm(value: f64, low: f64, high: f64) -> f64 {
    if low <= 0.0 || high <= low {
        return 0.0;
    }
    ((value.max(low).log10() - low.log10()) / (high.log10() - low.log10())).clamp(0.0, 1.0)
}

/// And back.
fn log_denorm(t: f64, low: f64, high: f64) -> f64 {
    if low <= 0.0 || high <= low {
        return low;
    }
    let span = high.log10() - low.log10();
    10.0_f64.powf(t.clamp(0.0, 1.0).mul_add(span, low.log10()))
}

/// What fits in a rack of a given width.
///
/// The same shed-in-order rule as `Squeeze` and `Collapse`: a panel that
/// cannot be read is worse than the space it took, because it still
/// looks like information.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Rack {
    /// The plugin's whole editing surface: its grid, its labels, its
    /// own band nodes. What a FOCUSED strip gets — one track opened
    /// wide enough that the rack stops being a readout you glance at
    /// and becomes the thing you work in.
    ///
    /// The difference is not more of the same drawing. At a glance
    /// width the rack shows what a track's processing IS; at a focus
    /// width it shows you where to put your hands, which needs the
    /// plugin's own calibration rather than the strip's abbreviation
    /// of it.
    Focus,
    /// All three panels, each with its grid and its label.
    Full,
    /// The curves alone, stacked. No grid, no labels — at this width a
    /// gridline is a third of the panel and the label is most of it.
    Curves,
    /// Nothing. The strip is a fader and a name, and the rack's height
    /// goes back to the strip.
    Off,
}

impl Rack {
    /// What fits in `width`.
    ///
    /// `Full` needs room for a decade of frequency to still read as a
    /// decade — below about 150 the EQ's three gridlines land on top of
    /// each other. `Curves` needs only enough width for the curve's
    /// shape to survive, which is far less.
    #[must_use]
    pub fn at(width: f64) -> Self {
        if width >= FOCUSED {
            Self::Focus
        } else if width >= LEGIBLE {
            Self::Full
        } else if width >= SHAPE {
            Self::Curves
        } else {
            Self::Off
        }
    }

    /// Whether the rack draws at all.
    #[must_use]
    pub fn on(self) -> bool {
        self != Self::Off
    }

    /// The narrowest width still in this tier.
    ///
    /// What a strip may be shrunk to without changing what it shows —
    /// see `mcp::lending_floor`, which is the only caller and the
    /// reason this exists. Returning the tier's own lower bound rather
    /// than a fixed floor is what keeps a strip from being lent out of
    /// the rack it was drawing.
    #[must_use]
    pub const fn floor(self) -> Option<f64> {
        match self {
            Self::Focus => Some(FOCUSED),
            Self::Full => Some(LEGIBLE),
            Self::Curves => Some(SHAPE),
            Self::Off => None,
        }
    }

    /// Whether this tier hands the panel over to the plugin's own
    /// drawing — its grid, its labels, its nodes.
    #[must_use]
    pub const fn editing(self) -> bool {
        matches!(self, Self::Focus)
    }

    /// Whether the panel is detailed: headers, markers, grips.
    ///
    /// Both of the two widest tiers. Written as a predicate rather than
    /// a comparison because the enum is ordered most-capable-first, so
    /// `>= Full` reads as "at least Full" and means the opposite.
    #[must_use]
    pub const fn detailed(self) -> bool {
        matches!(self, Self::Focus | Self::Full)
    }
}

/// The narrowest rack whose EQ panel still reads as a frequency axis.
///
/// Three gridlines — 100, 1k, 10k — and below this they crowd into each
/// other, at which point the panel is a squiggle rather than a decision
/// you can check.
pub const LEGIBLE: f64 = 96.0;

/// The narrowest rack that still says anything.
///
/// Below this a curve is a few pixels of wiggle — it reads as ornament
/// rather than as a setting, and ornament in a mixer is worse than
/// space.
pub const SHAPE: f64 = 90.0;

/// The narrowest rack that gives the panel over to the plugin's own
/// editing surface.
///
/// Below this the plugin's grid crowds, its labels collide and its
/// nodes — authored for a graph eight hundred pixels wide — overlap
/// each other. Above it there is room for all three, and the rack stops
/// abbreviating.
///
/// Well under a focus width (618 on a 1440p panel), because a focused
/// strip is not the only way to get here: two of them side by side on
/// an ultrawide, or a mixer of six tracks, land in the same place and
/// should get the same panel.
pub const FOCUSED: f64 = 260.0;

/// The width a strip opens to when you go to WORK on it.
///
/// Above [`LEGIBLE`] rather than at it: the threshold is where the rack
/// stops being illegible, and a control you have deliberately opened
/// should not land on the edge of that.
///
/// One number for two jobs, and they have to be the same number: it is
/// what a selected strip expands to, AND what the template stores for a
/// track that carries its piece's processing. If the stored width were
/// larger, selecting a piece would SHRINK it; if smaller, selecting one
/// would widen it and shove every strip to its right. Either way the
/// mixer moves under you at the moment you click a track, which is
/// exactly when it must not.
///
/// It does not have to be paid for twice. An opened strip BORROWS its
/// extra width from the others rather than adding to the total — see
/// `mcp::widths` — so this is bounded by what the resting layout can
/// afford, not by what the worst selection would cost on top of it.
pub const WORKING: f64 = 133.0;

/// Where the rack goes.
#[derive(Clone, Copy, Debug)]
pub struct Panel {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Panel {
    /// From a rect in strip coordinates, moved to where the strip is.
    ///
    /// The rack's box comes from [`crate::strip::Strip::rack_rect`], in
    /// the strip's own space; the recording is in the mixer's. This is
    /// the one conversion between them, so a grip and a curve cannot
    /// disagree about where the rack starts.
    #[must_use]
    pub fn of(rect: Rect, left: f64) -> Self {
        Self {
            x: rect.x0 + left,
            y: rect.y0,
            width: rect.width(),
            height: rect.height(),
        }
    }

    /// The same box, moved up by a scroll.
    ///
    /// The rack scrolls by moving where its panels START, not by
    /// offsetting each one — see [`layout`]. One move, applied once,
    /// which is what keeps the drawing and the hit test on the same
    /// pixel.
    #[must_use]
    pub fn up(self, by: f64) -> Self {
        Self { y: self.y - by, ..self }
    }

    const fn rect(self) -> Rect {
        Rect::new(self.x, self.y, self.x + self.width, self.y + self.height)
    }

    /// Whether a point is inside.
    #[must_use]
    pub fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }

    /// The panel inset by `by` on every side.
    const fn inset(self, by: f64) -> Self {
        Self {
            x: self.x + by,
            y: self.y + by,
            width: self.width - by * 2.0,
            height: self.height - by * 2.0,
        }
    }

    /// `by` off the top, and what is left.
    ///
    /// The header is a strip of the panel rather than an overlay on it:
    /// a value printed over a curve is unreadable exactly when the
    /// curve is interesting, which is the moment you want the number.
    const fn split_top(self, by: f64) -> (Self, Self) {
        let head = Self {
            height: by,
            ..self
        };
        let body = Self {
            y: self.y + by,
            height: self.height - by,
            ..self
        };
        (head, body)
    }
}

/// How tall a panel's header is.
///
/// The type is 7pt and this is the line it sits on plus a pixel of air
/// under it. Small, because it is a readout and not a title — you look
/// at the curve and read the number to confirm what you saw.
const HEAD: f64 = 10.0;

/// The gap between two panels of the rack.
const GAP: f64 = 3.0;

/// How tall a rack of these panels comes to, plus its gaps.
///
/// What the panels ACTUALLY occupy, which is less than the rack they
/// are laid out in — the space below them is held for the processors
/// the other phases bring. A caller that wants to know where the rack's
/// contents end asks this.
#[must_use]
pub fn wanted(panels: &[Which]) -> f64 {
    if panels.is_empty() {
        return 0.0;
    }
    let gaps = GAP * crate::num::coord(panels.len().saturating_sub(1));
    panels.iter().map(|which| which.natural()).sum::<f64>() + gaps
}

/// Which phases are folded shut.
///
/// A phase is the container the chain is read in — you are working the
/// Tone pass, so the four panels above and below it are context you
/// want out of the way — and folding one is how the rack stays legible
/// as it grows past what a strip can hold.
///
/// A bitset over the canonical phase order, for the same reason
/// [`Bypass`] is one: the chain gains phases, and a field each would
/// mean an edit in three places every time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Folded {
    shut: u16,
}

impl Folded {
    fn bit(phase: session::mix_phases::MixPhase) -> u16 {
        let at = session::mix_phases::MixPhase::ALL
            .iter()
            .position(|p| *p == phase)
            .unwrap_or(0);
        1_u16 << u16::try_from(at).unwrap_or(0)
    }

    #[must_use]
    pub fn is(self, phase: session::mix_phases::MixPhase) -> bool {
        self.shut & Self::bit(phase) != 0
    }

    pub fn toggle(&mut self, phase: session::mix_phases::MixPhase) {
        self.shut ^= Self::bit(phase);
    }

    #[must_use]
    pub const fn any(self) -> bool {
        self.shut != 0
    }
}

/// Where the folds live: one answer for the mixer, or one per track.
///
/// Both, because they answer different questions. Synced is the
/// default — the chain is read ACROSS, and a mixer where each strip
/// folded on its own would put a different processor at the same height
/// on every track, which is the one thing a mixer must not do. Per
/// track is for when you are working one track rather than comparing.
#[derive(Clone, Debug, Default)]
pub struct Fold {
    /// The shared answer, used when `synced`.
    every: Folded,
    by_guid: std::collections::HashMap<String, Folded>,
    pub synced: bool,
}

impl Fold {
    /// Synced, which is the default a mixer wants.
    #[must_use]
    pub fn shared() -> Self {
        Self {
            synced: true,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn of(&self, guid: &str) -> Folded {
        if self.synced {
            self.every
        } else {
            self.by_guid.get(guid).copied().unwrap_or_default()
        }
    }

    /// Fold or unfold a phase on a track — or on all of them.
    pub fn toggle(&mut self, guid: &str, phase: session::mix_phases::MixPhase) {
        if self.synced {
            self.every.toggle(phase);
        } else {
            self.by_guid.entry(guid.to_owned()).or_default().toggle(phase);
        }
    }

    /// Switch between the two, carrying the state across.
    ///
    /// Going synced takes the track you were on as the answer for
    /// everyone, rather than resetting: the fold you just made is
    /// almost always the one you want everywhere.
    pub fn sync(&mut self, synced: bool, from: &str) {
        if synced && !self.synced {
            self.every = self.by_guid.get(from).copied().unwrap_or_default();
        }
        self.synced = synced;
    }
}

/// A row of the rack: a phase's header, or one processor under it.
///
/// The two are laid out TOGETHER because they are one column, and a
/// header whose position was worked out separately from the panels it
/// caps would drift from them the first time a phase folded. Drawing
/// and hit testing both walk this.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Row {
    /// The container's bar: its name and the chevron that folds it.
    Head(session::mix_phases::MixPhase),
    Unit(Which),
}

/// How tall a phase header is.
pub const HEAD_H: f64 = 15.0;

/// Every processor the rack can show, in signal order.
///
/// ALL of them, whatever the phase. The rack used to show a phase's own
/// subset — the EQ alone in Rescue, the compressor and saturator in
/// Polish — which meant changing phase changed which processors
/// existed, and a strip you were reading reorganised itself under you.
///
/// A chain is a chain. You scroll it, and a phase's job is to say which
/// part of it to LOOK at rather than which parts to have. The rail's
/// buttons become focus and collapse once there is a setting for it;
/// until then the whole chain is on screen and the rack scrolls.
pub const ALL_PANELS: [Which; 11] = [
    Which::RescueEq,
    Which::Gate,
    Which::RescueComp,
    Which::Eq,
    Which::Comp,
    Which::Sat,
    Which::DeEss,
    Which::Resonance,
    Which::Space,
    Which::Delay,
    Which::Reverb,
];

/// Which panels a mix phase asks for.
///
/// Kept as a function because the callers read like the rack is a
/// question about the phase, and one day it will be again — as focus,
/// not as membership.
#[must_use]
pub const fn panels_for(_phase: session::mix_phases::MixPhase) -> &'static [Which] {
    &ALL_PANELS
}

/// Record the rack into `scene`.
///
/// Returns nothing: like every other control in the mixer this is
/// recorded once into the strip's command range and replayed from there.
pub fn record(
    scene: &mut Scene,
    palette: &Palette,
    font: &Font,
    tone: &Tone,
    panels: &[Which],
    panel: Panel,
    folded: Folded,
) {
    draw(scene, palette, font, tone, &Meters::default(), panels, panel, folded, None);
}

/// The same, with one grip lit.
///
/// Only the live pass has a pointer to report, so the recording calls
/// [`record`] and this is what the overlay reaches for. A lit grip is
/// the difference between "there is a handle here" and "this is the
/// handle you will move", which on a curve with four of them is the
/// whole question.
pub fn draw(
    scene: &mut Scene,
    palette: &Palette,
    font: &Font,
    tone: &Tone,
    // `meters` is everything in the rack that moves with the audio —
    // the analyser's bins, the suppressors' reduction, the wet returns.
    // Empty when nothing is playing, which is a rack that can stay in
    // the recording.
    meters: &Meters,
    panels: &[Which],
    panel: Panel,
    folded: Folded,
    lit: Option<Grip>,
) {
    let rack = Rack::at(panel.width);
    if !rack.on() || panel.height < 24.0 || panels.is_empty() {
        return;
    }

    for (row, at) in chain(panels, panel, folded) {
        let Row::Unit(which) = row else {
            let Row::Head(phase) = row else { continue };
            container(scene, palette, font, phase, at, folded.is(phase), lit);
            continue;
        };
        if which == Which::Presets {
            presets(scene, palette, font, tone, at, lit);
            continue;
        }
        ground(scene, palette, at);
        let inner = at.inset(2.0);
        // The header is only taken at `Full`. At `Curves` the panel is
        // a shape and nothing else fits; giving up a tenth of its
        // height for a number nobody can read would cost the shape too.
        let body = body_of(at, rack);
        let head = (body.y > inner.y).then(|| inner.split_top(HEAD).0);
        if body.width > 0.0 && body.height > 0.0 {
            match which {
                // The three EQs are one drawing over three band sets.
                // What differs is what the bands are FOR, which is the
                // panel's name and not its picture.
                Which::RescueEq | Which::Eq | Which::Space | Which::PreEq | Which::PostEq => {
                    eq(scene, palette, font, tone, which, tone.bands_ref(which), &meters.spectrum, body, rack, lit, None);
                }
                // Blue, because the vertical axis is not gain: it is
                // how long each band rings, and a graph that looked
                // like the EQ above it would be read as one.
                Which::DecayEq => {
                    eq(scene, palette, font, tone, which, &tone.decay_eq, &[], body, rack, lit, Some(DECAY_INK));
                }
                Which::Knobs => knobs(scene, palette, font, tone, body, rack, lit),
                Which::Wide => wide(scene, palette, tone, body, rack, lit),
                Which::Pitch => pitch(scene, palette, font, tone, body, rack, lit),
                Which::Presets => {}
                Which::Gate => gate(scene, palette, tone.gate, body, rack, lit),
                Which::RescueComp => {
                    comp(scene, palette, font, tone.rescue_comp, body, rack, lit);
                }
                Which::Comp => comp(scene, palette, font, tone.comp, body, rack, lit),
                Which::Sat => {
                    sat(scene, palette, tone, meters, display_of(body, which, rack), rack, lit);
                }
                Which::DeEss | Which::DeEssIn => {
                    suppress(scene, palette, which, tone.de_ess, meters, body, rack, lit);
                }
                Which::Resonance => {
                    suppress(scene, palette, which, tone.resonance, meters, body, rack, lit);
                }
                Which::Delay => {
                    echo(scene, palette, tone.delay, meters, display_of(body, which, rack), rack, lit);
                }
                Which::Reverb => {
                    room(scene, palette, tone.reverb, meters, display_of(body, which, rack), rack, lit);
                }
            }
            if let Some(strip) = lane_of(body, which, rack)
                && matches!(which, Which::Sat | Which::Delay | Which::Reverb)
            {
                selector(scene, palette, font, tone, which, strip, lit);
            }
            // The bypass, over everything the panel just drew.
            //
            // A scrim rather than a badge, and over the WHOLE panel
            // rather than beside it: what you need to know at a glance
            // across a mixer is that this processing is not happening,
            // and a small mark in a corner is exactly the thing a
            // glance misses. Greying the curve out says it once, in the
            // place you are already looking.
            if tone.bypass.is(which) {
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    palette.tcp_meter_well.multiply_alpha(0.78),
                    None,
                    &at.rect(),
                );
            }
            if let Some(head) = head {
                header(
                    scene,
                    palette,
                    font,
                    which.name(),
                    which.glyph(tone),
                    &which.summary(tone, rack),
                    tone.bypass.is(which),
                    head,
                );
            }
        }
    }
}

/// Where each panel of the rack lands.
///
/// The one place the rack's vertical division is worked out. Drawing
/// reads it and so does the hit test, which is the same rule
/// [`crate::strip::Strip`] exists for: a grip that is not where its
/// curve is drawn is a grip that moves the wrong thing.
///
/// Every panel keeps its NATURAL height and they stack from `panel.y`,
/// which means the chain is as long as the chain is and runs off the
/// bottom of a box too short for it. It used to scale them down
/// together to fit; that made every processor added shrink every
/// processor already there, until the whole rack was unreadable. A
/// chain is scrolled instead.
///
/// To scroll, hand it a panel whose `y` is already moved up — the
/// scroll is a fact about where the rack is drawn, not a second
/// parameter three callers would have to pass identically. `panel`'s
/// own height is what a caller clips to and what [`scroll_span`]
/// measures against; the layout itself does not read it.
#[must_use]
pub fn layout(panels: &[Which], panel: Panel) -> Vec<(Which, Panel)> {
    units(panels, panel, Folded::default())
}

/// The same, with some phases folded shut.
#[must_use]
pub fn units(panels: &[Which], panel: Panel, folded: Folded) -> Vec<(Which, Panel)> {
    chain(panels, panel, folded)
        .into_iter()
        .filter_map(|(row, at)| match row {
            Row::Unit(which) => Some((which, at)),
            Row::Head(_) => None,
        })
        .collect()
}

/// The whole column: every phase header and every unit under it.
///
/// One walk, so a header and the panels it caps cannot disagree about
/// where they are — which is the same rule [`crate::strip::Strip`] is
/// built on, applied down instead of across.
#[must_use]
pub fn chain(panels: &[Which], panel: Panel, folded: Folded) -> Vec<(Row, Panel)> {
    let mut out = Vec::with_capacity(panels.len() + 5);
    let mut y = panel.y;
    let mut phase = None;
    let row = |y: f64, height: f64| Panel {
        x: panel.x,
        y,
        width: panel.width,
        height,
    };
    for which in panels.iter().copied() {
        // The preset row is in no phase: it caps the chain, and folds
        // with nothing.
        if which == Which::Presets {
            out.push((Row::Unit(which), row(y, which.natural())));
            y += which.natural() + GAP;
            continue;
        }
        let tier = Rack::at(panel.width);
        // A header whenever the phase changes, which is what makes the
        // chain's ORDER do the grouping: the units are already in phase
        // order, so a container is a run of them.
        if phase != Some(which.phase()) {
            phase = Some(which.phase());
            out.push((Row::Head(which.phase()), row(y, HEAD_H)));
            y += HEAD_H;
        }
        if folded.is(which.phase()) {
            continue;
        }
        let height = which.natural_at(tier);
        out.push((Row::Unit(which), row(y, height)));
        y += height + GAP;
    }
    out
}

/// How far the rack can scroll before the last panel's floor arrives.
///
/// Zero when the chain already fits its box, which is what keeps the
/// gesture inert until there is something below to reach.
#[must_use]
pub fn scroll_span(panels: &[Which], box_height: f64, folded: Folded) -> f64 {
    (tall(panels, folded) - box_height).max(0.0)
}

/// How tall the whole column comes to, headers and folds included.
#[must_use]
pub fn tall(panels: &[Which], folded: Folded) -> f64 {
    let probe = Panel {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };
    chain(panels, probe, folded)
        .last()
        .map_or(0.0, |(_, at)| at.y + at.height)
}

/// The body of a panel — what is left once its header is taken.
///
/// Shared by the drawing and the hit test for the same reason
/// [`layout`] is: the curve is drawn in the body, so a grip has to be
/// measured against the body.
#[must_use]
pub fn body_of(at: Panel, rack: Rack) -> Panel {
    let inner = at.inset(2.0);
    if rack.detailed() && inner.height > HEAD * 2.0 {
        inner.split_top(HEAD).1
    } else {
        inner
    }
}

/// One panel of the rack.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Which {
    // ── Rescue: make it usable ───────────────────────────────────────
    /// Surgical cuts. The EQ you reach for before anything else,
    /// because the rest of the chain has to work on what this leaves.
    RescueEq,
    /// What is between the notes — the gate decides how much of the
    /// room, the bleed and the noise survives into everything after it.
    Gate,
    /// The first compressor: catching what is wrong rather than
    /// shaping what is right.
    RescueComp,

    // ── Tone: make it sound like itself ──────────────────────────────
    Eq,
    Comp,
    Sat,

    // ── Polish: take the ugly out ────────────────────────────────────
    /// Sibilance, dynamically.
    DeEss,
    /// And every other resonance — the narrow peaks a room or a body
    /// puts in, found by comparing the spectrum with its own average.
    Resonance,

    // ── Relational: make it sit with the others ──────────────────────
    /// The EQ that is not about this track: it carves the room another
    /// one needs, which is why it is its own unit and not more bands
    /// on the tone EQ.
    Space,

    // ── Depth: put it somewhere ──────────────────────────────────────
    /// Delay before reverb, which is the order they go in — a repeat
    /// arriving after the tail has started is a repeat you cannot hear
    /// as a repeat.
    Delay,
    Reverb,

    // ── The advanced units' own parts ────────────────────────────────
    /// The de-esser INSIDE an effect: the same detector as the Polish
    /// unit, on the way into the delay or the reverb, so a return
    /// carries one instance and not two. Lives in Depth, because the
    /// return does.
    DeEssIn,
    /// The EQ into an effect — inside the one instance.
    PreEq,
    /// The EQ out of it.
    PostEq,
    /// The reverb's decay-rate EQ, drawn blue: gain here is TIME.
    DecayEq,
    /// A row of the effect's knobs.
    Knobs,
    /// The widener.
    Wide,
    /// The pitch shifter.
    Pitch,
    /// The track's presets, brighter to darker — a row of chips at the
    /// top of the rack, in no phase.
    Presets,
}

impl Which {
    const fn name(self) -> &'static str {
        match self {
            Self::RescueEq => "RESCUE EQ",
            Self::Gate => "GATE",
            Self::RescueComp => "RESCUE COMP",
            Self::Eq => "EQ",
            Self::Comp => "COMP",
            Self::Sat => "SAT",
            Self::DeEss | Self::DeEssIn => "DE-ESS",
            Self::Resonance => "RESONANCE",
            Self::Space => "SPACE",
            Self::Delay => "DELAY",
            Self::Reverb => "REVERB",
            Self::PreEq => "PRE EQ",
            Self::PostEq => "POST EQ",
            Self::DecayEq => "DECAY EQ",
            Self::Knobs | Self::Presets => "",
            Self::Wide => "WIDE",
            Self::Pitch => "PITCH",
        }
    }

    /// Which phase this unit belongs to.
    ///
    /// The chain is ordered BY phase, and the phase is what the rail's
    /// buttons will focus once there is a setting for it. Stated per
    /// unit rather than as a table of ranges, so adding a processor is
    /// one line in one place.
    #[must_use]
    pub const fn phase(self) -> session::mix_phases::MixPhase {
        use session::mix_phases::MixPhase as P;
        match self {
            Self::RescueEq | Self::Gate | Self::RescueComp => P::Rescue,
            Self::Eq | Self::Comp | Self::Sat => P::Tone,
            Self::DeEss | Self::Resonance => P::Polish,
            Self::Space => P::Relational,
            // Every part of an FX return is Depth — the return is where
            // a track is put, front to back — including the de-esser on
            // its way in and the widener and the shifter.
            Self::Delay
            | Self::Reverb
            | Self::DeEssIn
            | Self::PreEq
            | Self::PostEq
            | Self::DecayEq
            | Self::Knobs
            | Self::Wide
            | Self::Pitch => P::Depth,
            // No phase: the row sits above the chain. `chain` never
            // opens a container for it.
            Self::Presets => P::Overview,
        }
    }

    /// Whether this unit draws a frequency response.
    ///
    /// Four of them do, on the plugin's own graph — the two EQs, the
    /// de-esser and the resonance suppressor — and they differ in what
    /// the curve MEANS rather than in how it is drawn.
    #[must_use]
    pub const fn is_spectral(self) -> bool {
        matches!(
            self,
            Self::RescueEq
                | Self::Eq
                | Self::Space
                | Self::DeEss
                | Self::DeEssIn
                | Self::Resonance
                | Self::PreEq
                | Self::PostEq
                | Self::DecayEq
        )
    }

    /// What this panel's settings come to, in one short line.
    ///
    /// A curve says the SHAPE of a decision and a number says the
    /// decision. Both, because they answer different questions: you
    /// scan the curves across a mixer to find the track that is
    /// different, and you read the number to know what to type into the
    /// one you opened.
    fn summary(self, tone: &Tone, rack: Rack) -> String {
        let curve = |bands: &[EqBand]| {
            let live = bands.iter().filter(|b| b.enabled && b.used).count();
            let range = bands
                .iter()
                .filter(|b| b.enabled && b.used)
                .map(|b| b.gain.abs())
                .fold(0.0_f32, f32::max);
            if live == 0 {
                "flat".to_owned()
            } else {
                format!("{live} · {range:.1}dB")
            }
        };
        let squash = |comp: Comp| {
            let head = format!("{:.0}dB · {:.1}:1", comp.threshold, comp.ratio);
            // The times are the envelope's own shape, which is the
            // point of drawing it — but a shape says "fast" and not
            // "three milliseconds", and at a focus width there is room
            // to say both.
            if rack.editing() {
                format!("{head} · {}/{}", millis(comp.attack), millis(comp.release))
            } else {
                head
            }
        };
        match self {
            Self::RescueEq => curve(&tone.rescue_eq),
            Self::Eq => curve(&tone.eq),
            Self::Space => curve(&tone.space),
            Self::PreEq => curve(&tone.pre_eq),
            Self::PostEq => curve(&tone.post_eq),
            // Gain here is time: the deepest band, as a rate.
            Self::DecayEq => {
                let live = tone.decay_eq.iter().filter(|b| b.enabled && b.used).count();
                let peak = tone
                    .decay_eq
                    .iter()
                    .filter(|b| b.enabled && b.used)
                    .map(|b| b.gain)
                    .fold(0.0_f32, |a, g| if g.abs() > a.abs() { g } else { a });
                if live == 0 {
                    "flat".to_owned()
                } else {
                    format!("{live} · ×{:.2}", 10.0_f32.powf(peak / 20.0))
                }
            }
            Self::Knobs | Self::Presets => String::new(),
            Self::Wide => format!("{:.0}%", tone.wide * 100.0),
            Self::Pitch => format!("{:+}st · {:.0}%", tone.pitch, tone.pitch_mix * 100.0),
            Self::Gate => format!("{:.0}dB · {:.0}", tone.gate.threshold, tone.gate.range),
            Self::RescueComp => squash(tone.rescue_comp),
            Self::Comp => squash(tone.comp),
            // The even share is the number an engineer reads a
            // saturator by — it is what separates a valve from a rail
            // — and it is measured, not read off the knob.
            Self::Sat => {
                let even = crate::live::ladder(&tone.sat).even_share() * 100.0;
                format!("x{:.1} · 2nd {even:.0}%", tone.sat.drive)
            }
            Self::DeEss | Self::DeEssIn => suppression(tone.de_ess),
            Self::Resonance => suppression(tone.resonance),
            Self::Delay => {
                let head = format!("{} · {:.0}%", millis(tone.delay.time), tone.delay.feedback * 100.0);
                if rack.editing() {
                    format!("{head} · {}", tone.delay.style.label().to_lowercase())
                } else {
                    head
                }
            }
            Self::Reverb => {
                let head = format!("{:.1}s · {:.0}%", tone.reverb.decay, tone.reverb.mix * 100.0);
                if rack.editing() {
                    format!("{head} · {}", tone.reverb.algorithm.name().to_lowercase())
                } else {
                    head
                }
            }
        }
    }

    /// The glyph that says which MACHINE this unit is, if it has one.
    ///
    /// Three units are a family of machines rather than one: the
    /// saturator is a valve or a rail, the delay is tape or a chip, the
    /// reverb is a hall or a spring. The plugin's own faces put that
    /// first — "you should know which delay you are looking at before
    /// you read a word" — and the strip says it in eight pixels beside
    /// the name.
    #[must_use]
    pub fn glyph(self, tone: &Tone) -> Option<Glyph> {
        match self {
            Self::Sat => Some(Glyph::Circuit(tone.sat_circuit())),
            Self::Delay => Some(Glyph::Delay(tone.delay.family())),
            Self::Reverb => Some(Glyph::Room(tone.reverb.family())),
            _ => None,
        }
    }

    /// Whether this unit's glyph is a switch — clicked to cycle the
    /// machine. The three that are a family of machines.
    #[must_use]
    pub const fn glyph_switches(self) -> bool {
        matches!(self, Self::Sat | Self::Delay | Self::Reverb)
    }

    /// How tall this panel is at a given rack tier.
    ///
    /// The natural height at every tier but focus. A focused rack is a
    /// column the height of the mixer, and the pictures that are time —
    /// the delay's repeats, the reverb's tail — earn more of it: at a
    /// hundred pixels tall and four hundred wide they read as a strip
    /// under a selector rather than as the visualiser they are.
    #[must_use]
    pub const fn natural_at(self, rack: Rack) -> f64 {
        let base = self.natural();
        if !rack.editing() {
            return base;
        }
        match self {
            Self::Delay | Self::Reverb => base * 2.2,
            Self::Sat => base * 1.4,
            Self::Wide | Self::Pitch => base * 1.6,
            _ => base,
        }
    }

    /// How tall this panel wants to be, in pixels.
    ///
    /// A HEIGHT, not a share. A share of the panel means a rack that
    /// grows with the window, and these do not need to: a frequency
    /// response is readable at a hundred and seventy pixels and no more
    /// readable at four hundred — it is the same curve with more air
    /// around it. What the extra height is worth something to is the
    /// FADER, which is a ruler and gets more precise the longer it is.
    ///
    /// So the rack asks for what it needs and the strip keeps the rest.
    const fn natural(self) -> f64 {
        match self {
            // Two axes to read, and the panels where extra height buys
            // resolution rather than air: a 3 dB decision and a 12 dB
            // one have to look different.
            Self::RescueEq | Self::Eq | Self::Space | Self::PreEq | Self::PostEq => 175.0,
            // Time over frequency: the same graph, read as a rate.
            Self::DecayEq => 150.0,
            // A row of five knobs and their legends.
            Self::Knobs => 52.0,
            // A field and an interval: one figure each.
            Self::Wide => 90.0,
            Self::Pitch => 74.0,
            // One row of chips.
            Self::Presets => PRESETS_H,
            // The suppressors are a spectrum and a cut hanging off it —
            // shorter than an EQ, because there is one curve to read
            // rather than a curve against a grid of decisions.
            Self::DeEss | Self::DeEssIn | Self::Resonance => 130.0,
            // One display, with the envelope drawn into it. Taller than
            // the saturator because the levels in it are read against a
            // threshold, and a threshold you cannot place precisely is
            // a threshold you set by ear twice.
            Self::Comp => 170.0,
            // The rescue compressor catches what is wrong rather than
            // shaping what is right: a coarser decision, a shorter
            // display.
            Self::RescueComp => 120.0,
            // The gate is the same display without the envelope: a line
            // and what falls under it.
            Self::Gate => 120.0,
            // A bent line through a square, and the ladder of what it
            // adds beside it. The ladder wants the height the curve
            // has, no more.
            Self::Sat => 130.0 + SELECTOR + 2.0,
            // Time pictures, both. A delay needs width for its taps and
            // no height beyond telling them apart; a reverb's tail is a
            // single falling line.
            // Time pictures, both, with the machine selector under them.
            Self::Delay | Self::Reverb => 100.0 + SELECTOR + 2.0,
        }
    }
}

/// A phase's own colour.
///
/// The same hues the toolbar icons are built with — see
/// `daw/features/reaper/fts-icons/examples/mix.toml`, where each phase
/// states its own. Taken from there rather than chosen again so the
/// container, the rail button and the REAPER toolbar all agree about
/// what colour Tone is.
fn phase_tint(phase: session::mix_phases::MixPhase) -> Color {
    use session::mix_phases::MixPhase as P;
    hex(match phase {
        P::Rescue => "#EF4444",
        P::Balance => "#22C55E",
        P::Tone => "#FACC15",
        P::Polish => "#06B6D4",
        P::Relational => "#3B82F6",
        P::Depth => "#8B5CF6",
        P::Creative => "#EC4899",
        P::Overview => "#6366F1",
    })
}

/// A phase's container bar: its name, and the chevron that folds it.
///
/// The chain is eleven units long and a strip shows about half of it,
/// so the thing that makes it legible is knowing WHICH PASS you are
/// looking at without counting panels. The bar says it, and folding one
/// puts the passes you are not working out of the way without taking
/// them off the chain.
fn container(
    scene: &mut Scene,
    palette: &Palette,
    font: &Font,
    phase: session::mix_phases::MixPhase,
    at: Panel,
    shut: bool,
    lit: Option<Grip>,
) {
    let held = lit == Some(Grip::Phase(phase));
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        if held { palette.tcp_field } else { palette.tcp_meter_well },
        None,
        &at.rect(),
    );
    // The phase's own colour down the left edge — the same hue its
    // button wears in the rail, so the container and the button that
    // will focus it are obviously the same thing.
    let tint = phase_tint(phase);
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        tint,
        None,
        &Rect::new(at.x, at.y, at.x + 2.0, at.y + at.height),
    );
    const SIZE: f32 = 8.0;
    crate::tcp::glyphs(
        scene,
        font,
        if held { palette.text } else { palette.text_dim },
        phase.display_name(),
        at.x + 6.0,
        at.y + at.height - 4.0,
        SIZE,
    );
    // The chevron: up when the container is open, down when it is shut,
    // which is the direction a click will move its contents.
    let right = at.x + at.width;
    let (cx, cy) = (right - 9.0, at.y + at.height / 2.0);
    let arm = 3.2;
    let ink = if held { palette.text } else { palette.text_faint };
    let tip = if shut { cy + arm * 0.6 } else { cy - arm * 0.6 };
    let base = if shut { cy - arm * 0.6 } else { cy + arm * 0.6 };
    rule_wide(scene, ink, Line::new((cx - arm, base), (cx, tip)), 1.4);
    rule_wide(scene, ink, Line::new((cx, tip), (cx + arm, base)), 1.4);
}

/// A panel's well — darker than the strip, so the rack reads as inset
/// rather than as three more controls on the strip's own surface.
fn ground(scene: &mut Scene, palette: &Palette, at: Panel) {
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        palette.tcp_meter_well,
        None,
        &at.rect(),
    );
}

/// The plugin's own EQ graph, painted into the rack.
///
/// Not a redraw of it — `eq_ui::eq_graph_painter` records into an
/// `anyrender::Scene` under a transform, which is exactly what this
/// rack does, so the strip can hand it a sub-rectangle and get the
/// editor's own grid, curves, fills and per-shape nodes.
///
/// That is the whole point of the plugin's UI being renderer-agnostic:
/// a curve in the strip that disagreed with the curve in the plugin
/// window would be worse than no curve at all, because it would be
/// believed — and the only way to guarantee they agree is for them to
/// be the same code.
///
/// The labels come off, because a strip panel is a hundred and thirty
/// pixels wide and the editor's are authored for eight hundred.
fn eq_from_plugin(
    scene: &mut Scene,
    tone: &Tone,
    bands: &[EqBand],
    spectrum: &[f32],
    at: Panel,
    rack: Rack,
) -> bool {
    let state = eq_ui::eq_graph_model::EqGraphRenderState::new();
    state.bands.write().clone_from(&bands.to_vec());
    // The analyser, behind the curves. The painter draws it when the
    // state carries it and skips it when it does not, so a track with
    // no signal gets the same graph it had before this existed.
    if spectrum.len() >= 2 {
        state.spectrum_db.write().extend_from_slice(spectrum);
    }
    {
        let mut config = state.config.write();
        config.db_range = tone.eq_db_range();
        config.min_freq = 20.0;
        config.max_freq = 20_000.0;
        config.sample_rate = DISPLAY_RATE;
        // Two looks, from one painter.
        //
        // At a glance width the rack has already drawn its own ground,
        // its dB ladder and its decades at a size that fits a strip,
        // and the editor's own would cover or crowd them — so the
        // plugin draws only what the rack cannot: the per-band curves,
        // their colours, the filled response. Its nodes come off too,
        // because four of them at their authored size overlap across a
        // hundred and thirty pixels and hide the curve they sit on; the
        // rack draws its own markers instead, sized for a strip.
        //
        // At a focus width there is room for the real thing, so the
        // plugin draws all of it and the rack draws none.
        let editing = rack.editing();
        config.fill_background = false;
        config.show_freq_labels = editing;
        config.show_db_labels = editing;
        config.show_grid = editing;
        config.fill_curve = true;
        config.node_scale = if editing { 1.0 } else { 0.0 };
        config.scale = 1.0;
    }
    let Ok(width) = u32::try_from(at.width.max(0.0).round() as i64) else {
        return false;
    };
    let Ok(height) = u32::try_from(at.height.max(0.0).round() as i64) else {
        return false;
    };
    if width < 2 || height < 2 {
        return false;
    }
    eq_ui::eq_graph_painter::paint_eq_graph_scene(
        scene,
        &state,
        Affine::translate((at.x, at.y)),
        width,
        height,
    );
    true
}

/// The EQ's response across the audible band.
#[expect(clippy::too_many_arguments, reason = "a drawing and everything it needs")]
fn eq(
    scene: &mut Scene,
    palette: &Palette,
    font: &Font,
    tone: &Tone,
    which: Which,
    bands: &[EqBand],
    spectrum: &[f32],
    at: Panel,
    rack: Rack,
    lit: Option<Grip>,
    // A colour for the whole graph, for a graph whose vertical axis is
    // not gain. `None` is the EQ's own look: the plugin's painter and
    // its per-band hues.
    tint: Option<Color>,
) {
    let freq = FreqAxis::audible();
    let db = DbAxis::symmetric(tone.eq_db_range());
    let right = at.x + at.width;
    let bottom = at.y + at.height;
    if let Some(tint) = tint {
        scene.fill(Fill::NonZero, Affine::IDENTITY, tint.multiply_alpha(0.08), None, &at.rect());
    }

    // Only at Full: at Focus the plugin draws its own grid and labels,
    // calibrated for a graph you are working in rather than glancing at.
    if rack == Rack::Full {
        for (hz, name) in DECADES {
            let x = freq.freq_to_x(hz, at.x, right);
            rule(scene, palette.grid_beat, Line::new((x, at.y), (x, bottom)));
            // Labelled along the floor, so a bump can be described to
            // someone else without opening the plugin.
            const SIZE: f32 = 6.0;
            let w = font.width(name, SIZE);
            if x - w / 2.0 >= at.x && x + w / 2.0 <= right {
                crate::tcp::glyphs(
                    scene,
                    font,
                    palette.text_faint,
                    name,
                    x - w / 2.0,
                    bottom - 2.0,
                    SIZE,
                );
            }
        }
        // The dB ladder. A strip panel is tall and a gentle EQ uses
        // little of it, so most of the box is empty — and empty space
        // with no marks in it makes a 3 dB decision look the same as a
        // 12 dB one. These are what turn the height into a scale.
        //
        // Stepped by the range the graph is drawn to, by the painter's
        // own law — a ladder that stayed at 6 and 12 would be two lines
        // off the top of a ±3 graph and a smear on a ±30 one.
        let range = tone.eq_db_range();
        let step = ladder_step(range);
        let mut gain = step;
        while gain <= range {
            for at_db in [gain, -gain] {
                let y = db.db_to_y(at_db, at.y, bottom);
                rule(scene, palette.grid_beat, Line::new((at.x, y), (right, y)));
            }
            gain += step;
        }
    }
    // Unity, always: without it a boost and a cut look the same.
    let zero = db.db_to_y(0.0, at.y, bottom);
    rule(
        scene,
        palette.grid,
        Line::new((at.x, zero), (right, zero)),
    );

    if bands.is_empty() {
        return;
    }
    // The plugin's own graph, where there is room for it. Its curve, its
    // fill, its per-band colours — the same code the editor paints with,
    // so the strip and the plugin window cannot disagree about what the
    // EQ is doing.
    //
    // The handles are still drawn below, because the editor's nodes are
    // authored for a graph eight hundred pixels wide and this one is a
    // hundred and thirty: at that size a labelled node with a shape
    // glyph is a smudge, where a dot is a position.
    let painted = tint.is_none() && rack.detailed() && eq_from_plugin(scene, tone, bands, spectrum, at, rack);
    if !painted {
        // The fallback: the same response function the plugin's painter
        // uses, as one polyline. What the narrow tier gets, what a
        // graph too small for the plugin's own drawing falls back to,
        // and what a tinted graph always is.
        let points: Vec<(f64, f64)> = (0..SAMPLES)
            .map(|i| {
                let t = crate::num::coord(i) / crate::num::coord(SAMPLES.saturating_sub(1).max(1));
                let hz = freq.norm_to_freq(t);
                let gain = calculate_combined_response(bands, hz, DISPLAY_RATE);
                (
                    freq.freq_to_x(hz, at.x, right),
                    db.db_to_y(gain, at.y, bottom).clamp(at.y, bottom),
                )
            })
            .collect();
        if let Some(tint) = tint {
            // Filled to unity, so a longer band and a shorter one read
            // as areas above and below the line rather than as one
            // wiggle.
            let mut area = BezPath::new();
            area.move_to((at.x, zero));
            for point in points.iter().copied() {
                area.line_to(point);
            }
            area.line_to((right, zero));
            area.close_path();
            scene.fill(Fill::NonZero, Affine::IDENTITY, tint.multiply_alpha(0.22), None, &area);
        }
        curve(scene, tint.unwrap_or(palette.accent), points.into_iter(), 1.5);
    }

    // The bands themselves, as handles on the curve.
    //
    // Without them the panel is a line: you can see that something was
    // done and not how many decisions it took or where they sit. A
    // four-band cut and one wide shelf can draw the same curve, and
    // they are not the same setting.
    //
    // Only at `Full`. At `Curves` a handle is three pixels of dot on a
    // curve two pixels wide, which reads as a kink in the line — and at
    // `Focus` the plugin has already drawn its own nodes, which are the
    // ones you came to the focus width for.
    if rack != Rack::Full {
        return;
    }
    for (index, band) in bands.iter().enumerate() {
        if !(band.enabled && band.used) {
            continue;
        }
        let x = freq.freq_to_x(f64::from(band.frequency), at.x, right);
        let y = db.db_to_y(f64::from(band.gain), at.y, bottom);
        if x < at.x || x > right || y < at.y || y > bottom {
            continue;
        }
        // The band's OWN colour, from the plugin's frequency map — the
        // same hue its node takes in the editor and at the focus tier.
        //
        // One accent for every band said "here is a band" and nothing
        // else; a hue that sweeps red to violet across the spectrum
        // says WHICH band, which is how you tell the low shelf from the
        // air band without reading a number. The two tiers now differ
        // in the size of the marker, not in what it means.
        let grown = lit == Some(Grip::Band(which, index));
        let r = if grown { HANDLE + 1.6 } else { HANDLE };
        dot(scene, palette.tcp_meter_well, (x, y), r + 1.0);
        dot(scene, tint.unwrap_or_else(|| band_color(f64::from(band.frequency))), (x, y), r);
    }

    // The zoom, last, so nothing draws over the one thing in the panel
    // that says what the rest of it means.
    scale(scene, palette, font, tone, which, at, lit);
}

/// The ladder's spacing for a given range.
///
/// The painter's own law (`eq_graph_painter`), so the rack's grid and
/// the editor's land on the same decibels.
fn ladder_step(range: f64) -> f64 {
    if range <= 6.0 {
        3.0
    } else if range <= 12.0 {
        6.0
    } else {
        12.0
    }
}

/// The EQ graph's zoom, as a chip at the top of it.
///
/// A curve without its scale is a shape, not a measurement: the same
/// wobble is a surgical half-decibel at ±3 and an inaudible nothing at
/// ±30. This is the label that makes the panel readable at a glance,
/// and the control that changes it — clicked to step out, wheeled
/// either way, double-clicked back to the default.
fn scale(
    scene: &mut Scene,
    palette: &Palette,
    font: &Font,
    tone: &Tone,
    which: Which,
    at: Panel,
    lit: Option<Grip>,
) {
    let chip = scale_chip(at);
    if chip.width < 20.0 || chip.height < 8.0 {
        return;
    }
    let held = lit == Some(Grip::Scale(which));
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        if held { palette.tcp_field } else { palette.tcp_meter_well },
        None,
        &vello::kurbo::Rect::new(
            chip.x,
            chip.y,
            chip.x + chip.width,
            chip.y + chip.height,
        ),
    );
    const SIZE: f32 = 8.0;
    let label = format!("±{:.0}", tone.eq_db_range());
    let width = font.width(&label, SIZE);
    crate::tcp::glyphs(
        scene,
        font,
        if held { palette.text } else { palette.text_faint },
        &label,
        chip.x + (chip.width - width) / 2.0,
        chip.y + chip.height - 3.0,
        SIZE,
    );
}

/// How big an EQ band's handle is.
///
/// Small enough that four of them on one curve do not merge, large
/// enough to be a target: this is the radius a pointer will have to
/// find once bands are draggable.
const HANDLE: f64 = 2.6;

/// The decades the frequency axis is labelled at.
///
/// Three, not five: at a strip's width the labels are 6pt and four of
/// them collide. 100, 1k and 10k are the ones a tone decision is
/// described in — "take out some 300", "lift the 3k" — and they bracket
/// the two that are not.
const DECADES: [(f64, &str); 3] = [(100.0, "100"), (1_000.0, "1k"), (10_000.0, "10k")];

// The transfer curve is gone from the strip. It said the same thing
// the threshold line and the ratio arrow now say between them — where
// it starts and how hard — in a form you had to read rather than
// reach for. `comp_graph_svg::transfer_curve_path` is still the
// plugin's, and still what a focused editor would draw.

/// The compressor: its ramps, its levels and what it is doing to them.
fn comp(
    scene: &mut Scene,
    palette: &Palette,
    font: &Font,
    comp: Comp,
    at: Panel,
    rack: Rack,
    lit: Option<Grip>,
) {
    let at = comp_split(at, rack);
    let right = at.x + at.width;
    // The comp editor's own axis: 0 dB at the top, −60 at the floor.
    // Taken from the plugin so the threshold line, the ladder and
    // whatever level is drawn over them land on the same numbers it
    // uses.
    let to_y = |db: f64| at.y + comp_ui::comp_graph_svg::db_to_y(db, at.height);

    // The ladder the levels are read against. Without it the threshold
    // is a line at a height rather than a line at a level.
    if rack.detailed() {
        for db in [-12.0, -24.0, -36.0, -48.0] {
            let y = to_y(db);
            rule(scene, palette.grid_beat, Line::new((at.x, y), (right, y)));
        }
    }

    // The threshold: a line ACROSS the display at its own level, which
    // is where a threshold belongs. It was a knob, and a knob makes you
    // read a number and compare it to a meter somewhere else; a line
    // over the levels is the comparison.
    let y = threshold_y(comp, at);
    let held = matches!(lit, Some(Grip::Threshold(_)));
    let red = hex(comp_ui::comp_graph_svg::colors::THRESHOLD);
    rule_wide(
        scene,
        red,
        Line::new((at.x, y), (right, y)),
        if held { 2.5 } else { 1.5 },
    );
    // A grab tab at the right end, so there is something to aim at on a
    // line that is otherwise one pixel tall.
    if rack.detailed() {
        let r = if held { HANDLE + 1.6 } else { HANDLE };
        dot(scene, red, (right - r - 1.0, y), r);
        // And the settings themselves, as the reduction they produce —
        // outlined over the live one, in the same axes.
        envelope(scene, comp, at, y, lit);
    }
    let _ = font;
}

/// Where the threshold line sits in a compressor display.
///
/// One function because three things have to agree about it: the line
/// itself, the envelope glyph that meets it, and the hit test that
/// decides which of the two a click belongs to.
fn threshold_y(comp: Comp, at: Panel) -> f64 {
    let y = at.y + comp_ui::comp_graph_svg::db_to_y(f64::from(comp.threshold), at.height);
    y.clamp(at.y, at.y + at.height)
}

/// The compressor's settings, drawn on the threshold they act at.
///
/// One glyph, in the same axes as the live gain reduction above it:
/// a ramp DOWN to the threshold line, a run along it, a ramp back UP —
/// and an arrow hanging off that line for the ratio. This is what one
/// hit looks like with these settings, outlined, over what is actually
/// happening, filled. Model and measurement in one picture, which is
/// the only arrangement where you can see whether the setting is the
/// right one.
///
/// Four parts, three of them grips, each dragged along its own axis:
///
/// - the lead-in ramp is the ATTACK, dragged sideways, because time is
///   the horizontal axis here and always has been;
/// - the flat middle IS the threshold line, which the ramps meet
///   rather than duplicate;
/// - the arrow is the RATIO, dragged down, because it points down and
///   that is the direction the signal goes;
/// - the lead-out ramp is the RELEASE, dragged sideways.
///
/// The times are widths rather than positions, so a slow attack is a
/// long lead-in — which is what a slow attack IS, and what the live
/// trace beside it will show the moment something plays.
fn envelope(scene: &mut Scene, comp: Comp, at: Panel, level: f64, lit: Option<Grip>) {
    let Some(shape) = Envelope::of(comp, at, level) else {
        return;
    };
    // The threshold's own pink rather than the reduction's red: this
    // glyph is a SETTING, like the line it hangs from, and the filled
    // red under it is the measurement. Drawn in the measurement's
    // colour it disappeared into it — which is exactly the distinction
    // the two-in-one-picture arrangement exists to make.
    let red = hex(comp_ui::comp_graph_svg::colors::THRESHOLD);
    let held = |grip| if lit == Some(grip) { 2.4 } else { 1.4 };

    // Drawn as separate strokes rather than one path, so the part under
    // the pointer can thicken on its own — a glyph that lit up whole
    // would not say which of its parts you are about to move.
    curve(scene, red, shape.fall().into_iter(), held(Grip::Attack(Which::Comp)));
    curve(scene, red, shape.rise().into_iter(), held(Grip::Release(Which::Comp)));

    // The ratio, as an arrow off the threshold line. Its length is the
    // reduction a full-scale signal takes — what the ratio DOES rather
    // than a length chosen to look proportional — so it grows as the
    // ratio hardens and vanishes at unity, where the compressor is
    // taking nothing off.
    if let Some((shaft, head)) = shape.arrow() {
        curve(scene, red, shaft.into_iter(), held(Grip::Ratio(Which::Comp)));
        for side in head {
            curve(scene, red, side.into_iter(), held(Grip::Ratio(Which::Comp)));
        }
    }

    // A mark where each ramp leaves the line, because a slope is a line
    // and a line is not something you aim at.
    let r = |grip| if lit == Some(grip) { HANDLE + 1.4 } else { HANDLE * 0.8 };
    dot(scene, red, shape.knee, r(Grip::Attack(Which::Comp)));
    dot(scene, red, shape.foot, r(Grip::Release(Which::Comp)));
}

/// Where the envelope glyph's corners land.
///
/// Shared by the drawing and the hit test, for the reason everything in
/// this module is: a grip that is not on the part it moves is a grip
/// that moves the wrong thing.
#[derive(Clone, Copy, Debug)]
struct Envelope {
    /// Where the reduction begins, at the ceiling.
    start: (f64, f64),
    /// Where the lead-in ramp meets the threshold line.
    knee: (f64, f64),
    /// And where the lead-out ramp leaves it.
    foot: (f64, f64),
    /// Where the release has recovered, back at the ceiling.
    back: (f64, f64),
    /// How far the ratio's arrow hangs below the line, in pixels.
    drop: f64,
    /// The x the arrow hangs on — its own rail at the left edge.
    rail: f64,
}

/// The column the ratio's arrow lives in, at the display's left edge.
const RAIL: f64 = 11.0;

/// How far the arrowhead's barbs reach back up the shaft.
const BARB: f64 = 4.0;

impl Envelope {
    /// The glyph for these settings in this box, or `None` when there
    /// is no room to draw one that means anything.
    fn of(comp: Comp, at: Panel, level: f64) -> Option<Self> {
        if at.width < 30.0 || at.height < 30.0 {
            return None;
        }
        // The times as widths, on their own log scales — the same ones
        // the parameters are stored on, so the drag that moves a ramp
        // is the drag that moves the number.
        //
        // Two fifths of the panel each at their longest, which leaves
        // the run along the line a fifth at the extreme and keeps the
        // glyph inside its box whatever the settings.
        let span = at.width * 0.4;
        let lead = log_norm(f64::from(comp.attack), 0.1, 200.0) * span;
        let tail = log_norm(f64::from(comp.release), 5.0, 3_000.0) * span;
        // The ramps start clear of the rail, so the arrow has a column
        // of its own to hang in rather than crossing the lead-in on its
        // way down.
        let left = at.x + RAIL;
        let top = at.y;
        let line = level.clamp(at.y + 2.0, at.y + at.height - 2.0);
        // A run long enough to hang an arrow from even when both times
        // are at their longest.
        let hold = (at.width - lead - tail - 4.0).max(8.0);
        Some(Self {
            start: (left, top),
            knee: (left + lead, line),
            foot: (left + lead + hold, line),
            back: ((left + lead + hold + tail).min(at.x + at.width - 1.0), top),
            // Clipped to the box: past the floor the arrow would be
            // drawing reduction the display cannot show.
            drop: ratio_drop(comp, at.height).min(at.y + at.height - line),
            rail: at.x + RAIL / 2.0,
        })
    }

    fn fall(self) -> [(f64, f64); 2] {
        [self.start, self.knee]
    }

    fn rise(self) -> [(f64, f64); 2] {
        [self.foot, self.back]
    }

    /// Where the arrow hangs from: a rail down the display's left edge.
    ///
    /// Out at the edge rather than anywhere in the middle, because the
    /// middle is where the live reduction and the waveform are busiest
    /// and an arrow there sat on top of the thing it is meant to be
    /// read against. On its own rail it stays out of the way and still
    /// measures down the same dB axis as everything else here.
    fn stem(self) -> (f64, f64) {
        (self.rail, self.knee.1)
    }

    /// The arrow's shaft and the two barbs of its head, or `None` when
    /// the ratio is taking too little off to point at.
    fn arrow(self) -> Option<([(f64, f64); 2], [[(f64, f64); 2]; 2])> {
        if self.drop < BARB + 1.0 {
            return None;
        }
        let (x, y) = self.stem();
        let tip = (x, y + self.drop);
        Some((
            [(x, y), tip],
            [
                [tip, (x - BARB, tip.1 - BARB)],
                [tip, (x + BARB, tip.1 - BARB)],
            ],
        ))
    }

    /// Which of its parts a point is nearest, if any.
    fn grip_at(self, x: f64, y: f64) -> Option<Grip> {
        let near = |(ax, ay): (f64, f64), (bx, by): (f64, f64)| {
            // Distance to the segment, so a ramp is grabbable along its
            // whole length rather than only at its ends.
            let (dx, dy) = (bx - ax, by - ay);
            let len = dx.hypot(dy);
            if len < f64::EPSILON {
                return (x - ax).hypot(y - ay);
            }
            let t = (((x - ax) * dx + (y - ay) * dy) / (len * len)).clamp(0.0, 1.0);
            (x - t.mul_add(dx, ax)).hypot(y - t.mul_add(dy, ay))
        };
        let stem = self.stem();
        [
            (Grip::Attack(Which::Comp), near(self.start, self.knee)),
            (Grip::Release(Which::Comp), near(self.foot, self.back)),
            // The arrow wins ties with the ramps, because it is the one
            // that lies BETWEEN them — a point on the shaft is never
            // also on a ramp, but a point near where it leaves the line
            // is near everything.
            (Grip::Ratio(Which::Comp), near(stem, (stem.0, stem.1 + self.drop)) - 0.01),
        ]
        .into_iter()
        .filter(|(_, away)| *away <= GRAB)
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(grip, _)| grip)
    }
}

// The knobs are gone. Ratio, attack and release were three numbers you
// read and then imagined the effect of on a display two inches away;
// they are one shape on that display now, in its own axes. See
// `envelope`.

/// The gate: its threshold across the level display, the range it
/// takes off below it, and its times as a glyph between the two.
///
/// The compressor's axis, deliberately — same red line you drag, same
/// dB ladder — because the pair is read together down a chain and a
/// level should sit at the same height on both. What is NOT shared is
/// the story. A compressor's is how much; a gate's is open or shut, and
/// its glyph says so: where the compressor's is a dip, the gate's is a
/// TABLE — attack up, hold flat, release down — standing in the band
/// it acts in. The door lane under the display (drawn in the live
/// pass, see [`levels`]) is when it was open.
fn gate(scene: &mut Scene, palette: &Palette, gate: Gate, at: Panel, rack: Rack, lit: Option<Grip>) {
    let at = display_of(at, Which::Gate, rack);
    let right = at.x + at.width;
    let bottom = at.y + at.height;
    let to_y = |db: f64| at.y + comp_ui::comp_graph_svg::db_to_y(db, at.height);

    if rack.detailed() {
        for db in [-12.0, -24.0, -36.0, -48.0] {
            let y = to_y(db);
            rule(scene, palette.grid_beat, Line::new((at.x, y), (right, y)));
        }
    }

    let line = to_y(f64::from(gate.threshold)).clamp(at.y, bottom);
    let floor = to_y(f64::from(gate.threshold + gate.range)).clamp(at.y, bottom);
    // What the gate takes off is what lives BELOW the line, down to the
    // range — so that is the band that gets shaded. A compressor shades
    // above.
    // In the gate's own green: the compressor's red is a reduction,
    // and a gate is a door — its lane says "open" in this colour, and
    // so does everything else on the panel.
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        GATE_INK.multiply_alpha(0.18),
        None,
        &Rect::new(at.x, line, right, floor),
    );
    let held = matches!(lit, Some(Grip::Threshold(_)));
    let red = GATE_INK;
    rule_wide(
        scene,
        red,
        Line::new((at.x, line), (right, line)),
        if held { 2.5 } else { 1.5 },
    );
    if rack.detailed() {
        let r = if held { HANDLE + 1.6 } else { HANDLE };
        dot(scene, red, (right - r - 1.0, line), r);
        // The range, as a second line: how far down what stays shut
        // goes. A gate that closed completely would put that line on
        // the floor, and the gap between the two IS the setting.
        let range_held = lit == Some(Grip::Range);
        rule_wide(
            scene,
            red.multiply_alpha(0.55),
            Line::new((at.x, floor), (right, floor)),
            if range_held { 2.0 } else { 1.0 },
        );
        if let Some(shape) = GateGlyph::of(gate, at, line, floor) {
            let width = |grip| if lit == Some(grip) { 2.4 } else { 1.4 };
            curve(scene, red, shape.rise().into_iter(), width(Grip::Attack(Which::Gate)));
            curve(scene, red, shape.run().into_iter(), width(Grip::Hold(Which::Gate)));
            curve(scene, red, shape.fall().into_iter(), width(Grip::Release(Which::Gate)));
            let r = |grip| if lit == Some(grip) { HANDLE + 1.4 } else { HANDLE * 0.8 };
            dot(scene, red, shape.open, r(Grip::Attack(Which::Gate)));
            dot(scene, red, shape.close, r(Grip::Release(Which::Gate)));
        }
    }
}

/// The gate's ink: green, the door lane's "open" — so a gate is never
/// mistaken for the compressor it sits beside.
const GATE_INK: Color = Color::from_rgba8(0x34, 0xd3, 0x99, 0xff);

/// The gate's times, as a table standing between its two lines.
///
/// The mirror of the compressor's [`Envelope`]: that one hangs from the
/// ceiling down to the threshold, this one stands up from the range
/// floor to the threshold — because a gate OPENS, and up is open. Three
/// grips, each dragged sideways along the time axis they are widths on:
/// the rising edge is the attack, the flat top is the hold, and the
/// falling edge is the release.
#[derive(Clone, Copy, Debug)]
struct GateGlyph {
    /// On the floor, where the signal arrives.
    start: (f64, f64),
    /// Where the attack reaches the threshold line — the door is open.
    open: (f64, f64),
    /// Where the hold ends and the release begins.
    close: (f64, f64),
    /// Back on the floor — shut.
    end: (f64, f64),
}

impl GateGlyph {
    fn of(gate: Gate, at: Panel, line: f64, floor: f64) -> Option<Self> {
        if at.width < 30.0 || at.height < 30.0 {
            return None;
        }
        // Three widths on their own log scales, each up to a quarter
        // of the display, so the glyph stays inside the box at every
        // setting and the run has room to be a run.
        let span = at.width * 0.25;
        let attack = log_norm(f64::from(gate.attack), 0.1, 200.0) * span;
        let hold = log_norm(f64::from(gate.hold), 1.0, 2_000.0).mul_add(span, 6.0);
        let release = log_norm(f64::from(gate.release), 5.0, 3_000.0) * span;
        let left = at.x + RAIL;
        // A table needs a leg: when the range is so small the two
        // lines nearly touch, the glyph stands a little below the
        // threshold anyway, or it would be a line on a line.
        let floor = floor.max(line + 8.0).min(at.y + at.height - 1.0);
        Some(Self {
            start: (left, floor),
            open: (left + attack, line),
            close: (left + attack + hold, line),
            end: ((left + attack + hold + release).min(at.x + at.width - 1.0), floor),
        })
    }

    const fn rise(self) -> [(f64, f64); 2] {
        [self.start, self.open]
    }

    const fn run(self) -> [(f64, f64); 2] {
        [self.open, self.close]
    }

    const fn fall(self) -> [(f64, f64); 2] {
        [self.close, self.end]
    }

    /// Which of its edges a point is nearest, if any.
    fn grip_at(self, x: f64, y: f64) -> Option<Grip> {
        [
            (Grip::Attack(Which::Gate), near_segment((x, y), self.start, self.open)),
            (Grip::Hold(Which::Gate), near_segment((x, y), self.open, self.close)),
            (Grip::Release(Which::Gate), near_segment((x, y), self.close, self.end)),
        ]
        .into_iter()
        .filter(|(_, away)| *away <= GRAB)
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(grip, _)| grip)
    }
}

/// Distance from a point to a segment — so a ramp is grabbable along
/// its whole length rather than only at its ends.
fn near_segment(point: (f64, f64), from: (f64, f64), to: (f64, f64)) -> f64 {
    let (px, py) = point;
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let len = dx.hypot(dy);
    if len < f64::EPSILON {
        return (px - from.0).hypot(py - from.1);
    }
    let along = ((px - from.0).mul_add(dx, (py - from.1) * dy) / (len * len)).clamp(0.0, 1.0);
    (px - along.mul_add(dx, from.0)).hypot(py - along.mul_add(dy, from.1))
}

/// How tall the lane under a gate or a de-esser is.
///
/// The door lane and the fire lane: a strip of time along the bottom
/// of the display, drawn in the live pass from the level history. Nine
/// pixels is enough to see a segment and its ramp, and little enough
/// that the display above keeps its ladder.
pub const LANE: f64 = 9.0;

/// How tall the machine selector under a delay or a reverb is.
///
/// One chip per family, the current one lit, and the machine's name
/// beside them. A row rather than a dropdown: across a mixer the
/// question is "which of these is a plate", and a row answers it
/// without a click.
pub const SELECTOR: f64 = 16.0;

/// How wide one chip of the selector is.
pub const CHIP: f64 = 14.0;

/// How tall the comb under a resonance suppressor is.
///
/// The teeth hang below the spectrum's floor by their settled depth;
/// this is the depth a six-decibel notch reaches.
pub const TEETH: f64 = 20.0;

/// The part of a body a unit's main display occupies.
///
/// Three units keep a strip along the bottom for a second picture —
/// the gate's door lane, the de-esser's fire lane, the resonance
/// suppressor's comb — and only at a detailed width, because at
/// `Curves` the whole body is the shape and nothing else fits. Stated
/// once because the drawing, the live pass and the hit test all have
/// to agree where the display ends.
#[must_use]
pub const fn display_of(body: Panel, which: Which, rack: Rack) -> Panel {
    if !rack.detailed() {
        return body;
    }
    let keep = match which {
        Which::Gate | Which::DeEss | Which::DeEssIn => LANE + 2.0,
        Which::Sat | Which::Delay | Which::Reverb => SELECTOR + 2.0,
        Which::Resonance => TEETH + 2.0,
        _ => 0.0,
    };
    Panel {
        height: body.height - keep,
        ..body
    }
}

/// The lane under a unit's display, if it has one at this width.
#[must_use]
pub const fn lane_of(body: Panel, which: Which, rack: Rack) -> Option<Panel> {
    let display = display_of(body, which, rack);
    if display.height >= body.height {
        return None;
    }
    Some(Panel {
        y: display.y + display.height + 2.0,
        height: body.height - display.height - 2.0,
        ..body
    })
}

/// A spectral suppressor: what is there, and what it is taking out of
/// it — in place, at the frequency it happens.
///
/// Two units, one engine, two questions. The de-esser is an event
/// detector — it fires on an "s" and lets go — so its identity is the
/// fire lane under the display, a strip of time (see [`levels`]). The
/// resonance suppressor finds what is ALWAYS there, so its identity is
/// the comb below the floor: the engine's settled reduction, one tooth
/// per notch, at the frequency it lives. Same spectrum, same ribbon,
/// and you cannot mistake one panel for the other.
///
/// The reduction is the engine's own per-bin gain, arriving in
/// `meters`, not something worked out here from the spectrum: the
/// density, tilt and gate rules live in the plugin, and a second copy
/// of them would drift. What IS worked out here is the reference line
/// — the neighbourhood average the threshold stands above — because
/// that is the picture of a SETTING, and dragging it is how the
/// threshold is set.
#[expect(clippy::too_many_arguments, reason = "a drawing and everything it needs")]
fn suppress(
    scene: &mut Scene,
    palette: &Palette,
    which: Which,
    set: Suppress,
    meters: &Meters,
    at: Panel,
    rack: Rack,
    lit: Option<Grip>,
) {
    let full_body = at;
    let at = display_of(at, which, rack);
    let bottom = at.y + at.height;
    let zoom = SuppressZoom::of(set);

    // The band it actually acts in, marked on the axis rather than as a
    // wash over the panel: the shoulders are context, the band is the
    // subject. Drawn before the spectrum so they can be seen through
    // it, and drawn even with nothing playing, because they are the
    // setting.
    if rack.detailed() {
        for (hz, side) in [(f64::from(set.low), Side::Low), (f64::from(set.high), Side::High)] {
            let x = zoom.x_of(hz, at);
            let held = lit == Some(Grip::Edge(which, side));
            rule_wide(
                scene,
                if held { palette.text } else { palette.grid_beat },
                Line::new((x, at.y), (x, bottom)),
                if held { 1.8 } else { 1.0 },
            );
        }
    }

    let spectrum = &meters.spectrum;
    let reduction = match which {
        Which::DeEss | Which::DeEssIn => &meters.deess_db,
        _ => &meters.resonance_db,
    };
    if spectrum.len() < 4 || reduction.len() != spectrum.len() {
        return;
    }
    let level_at = |hz: f64| bin_at(spectrum, hz);
    let cut_at = |hz: f64| bin_at(reduction, hz);

    // The baseline the peaks are judged against: the spectrum's own
    // average over a span set by the sharpness. What stands proud of it
    // by more than the threshold is what gets cut, so the threshold IS
    // the height of the dashed line above the average — and that is
    // the line you drag.
    let octaves = f64::from(set.sharpness).mul_add(-0.9, 1.2);
    let span = (octaves / 2.0).exp2();
    // The neighbourhood without the bin itself — the shoulders either
    // side of the centre third — so a peak is judged against what
    // surrounds it and not against half of itself. The same rule the
    // reduction is computed by.
    let average_at = |hz: f64| {
        const STEPS: usize = 11;
        let (sum, count) = (0..=STEPS)
            .filter(|k| !(4..=6).contains(k))
            .map(|k| {
                let t = crate::num::coord(k) / crate::num::coord(STEPS);
                level_at(hz / span * (span * span).powf(t))
            })
            .fold((0.0_f64, 0.0_f64), |(sum, n), v| (sum + v, n + 1.0));
        sum / count.max(1.0)
    };

    // One sample per bin, no finer. The analyser has ninety-six of them
    // across the audible range and a zoomed band holds a fraction of
    // that, so sampling past the data invents detail that is not there
    // and pays for it in path length.
    let across = spectrum.len().clamp(24, 96);
    let sample = |i: usize| crate::num::coord(i) / crate::num::coord(across.saturating_sub(1).max(1));
    let to_y = |db: f64| {
        let t = ((db - SUPPRESS_FLOOR_DB) / (SUPPRESS_CEIL_DB - SUPPRESS_FLOOR_DB)).clamp(0.0, 1.0);
        bottom - t * (at.height - 2.0) - 1.0
    };
    let x_at = |i: usize| at.x + sample(i) * at.width;

    // What arrived, as an area — it is the material, and an outline
    // reads as another curve competing with the two that matter. The
    // resonance panel draws it dimmer: there the spectrum is context
    // and the teeth are the subject.
    let dim = if which == Which::Resonance { 0.6 } else { 1.0 };
    // The de-esser's is yellow — the colour of the top end it is there
    // to take the edge off.
    let ink = if which == Which::Resonance { palette.accent } else { DEESS_INK };
    let input: Vec<(f64, f64)> = (0..across)
        .map(|i| (x_at(i), to_y(level_at(zoom.hz_at(sample(i))))))
        .collect();
    area_under(
        scene,
        ink.multiply_alpha(crate::mcp::f64_to_f32(0.16 * dim)),
        &input,
        bottom,
    );

    // The reference, dashed: the average plus the threshold. Where the
    // spectrum crosses above it is where the engine acts.
    if rack.detailed() {
        let held = matches!(lit, Some(Grip::Threshold(_) | Grip::Sharpness(_)));
        let reference = (0..across).map(|i| {
            (x_at(i), to_y(average_at(zoom.hz_at(sample(i))) + f64::from(set.threshold)))
        });
        dashed(scene, palette.text.multiply_alpha(0.75), reference, if held { 1.6 } else { 0.8 });
    }

    // And what leaves. The gap between the two is the reduction, at the
    // frequency it is happening, over the peak that caused it. NOT
    // clamped at zero: these are decibels.
    let output: Vec<(f64, f64)> = (0..across)
        .map(|i| {
            let hz = zoom.hz_at(sample(i));
            (x_at(i), to_y(level_at(hz) - cut_at(hz)))
        })
        .collect();
    let red = hex(comp_ui::comp_graph_svg::colors::REDUCTION_EDGE);
    // Only when there is something to show. With nothing being cut the
    // two curves coincide, and the ribbon between them is a degenerate
    // polygon — two hundred vertices tracing out and back along the
    // same line, which the rasteriser pays for in full and which draws
    // nothing. That cost half a millisecond a frame across a mixer.
    let deepest = reduction.iter().copied().fold(0.0_f32, f32::max);
    if deepest > 0.15 {
        let held = matches!(lit, Some(Grip::Depth(_)));
        ribbon(scene, red.multiply_alpha(if held { 0.8 } else { 0.55 }), &input, &output);
    }
    curve(
        scene,
        ink.multiply_alpha(crate::mcp::f64_to_f32(0.7 * dim)),
        input.into_iter(),
        1.0,
    );
    let held = matches!(lit, Some(Grip::Depth(_)));
    let ribbon = if which == Which::Resonance { 0.9 } else { 1.1 };
    curve(scene, red, output.into_iter(), if held { 1.8 } else { ribbon });

    if rack.detailed() {
        marks(scene, palette, zoom, at);
    }

    if which == Which::Resonance
        && let Some(comb_at) = lane_of(full_body, which, rack)
    {
        comb(scene, palette, &meters.resonance_settled_db, zoom, comb_at);
    }
}

/// Where you are on a zoomed axis, since it is no longer the familiar
/// one: a tick at each of [`SUPPRESS_MARKS`] that falls in the window.
fn marks(scene: &mut Scene, palette: &Palette, zoom: SuppressZoom, at: Panel) {
    let bottom = at.y + at.height;
    for (hz, _) in SUPPRESS_MARKS {
        if hz < zoom.low || hz > zoom.high {
            continue;
        }
        let x = zoom.x_of(hz, at);
        if x - 3.0 < at.x || x + 3.0 > at.x + at.width {
            continue;
        }
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette.grid,
            None,
            &Rect::new(x - 0.5, bottom - 3.0, x + 0.5, bottom),
        );
    }
}

/// The comb: the settled reduction, one tooth per notch, hanging below
/// the floor at the frequency it lives.
///
/// Slow by construction — the engine averages it over seconds — so
/// across a mixer a snare with three teeth is a snare you will go and
/// look at.
fn comb(scene: &mut Scene, palette: &Palette, settled: &[f32], zoom: SuppressZoom, at: Panel) {
    rule(scene, palette.grid_beat, Line::new((at.x, at.y), (at.x + at.width, at.y)));
    let tint = phase_tint(session::mix_phases::MixPhase::Polish);
    for tooth in teeth(settled, &zoom) {
        let x = tooth.place.mul_add(at.width, at.x);
        // Six decibels is the whole comb: a suppressor that has found a
        // six-decibel resonance has found what it exists for, and the
        // decibel it usually takes off a room mode still has to read as
        // a tooth rather than a smudge — hence the floor.
        let depth = ((f64::from(tooth.depth_db) / 6.0).clamp(0.0, 1.0) * (at.height - 2.0)).max(3.0);
        let half = f64::from(tooth.depth_db).min(6.0).mul_add(0.4, 2.0);
        let mut tri = BezPath::new();
        tri.move_to((x - half, at.y));
        tri.line_to((x + half, at.y));
        tri.line_to((x, at.y + depth));
        tri.close_path();
        scene.fill(Fill::NonZero, Affine::IDENTITY, tint.multiply_alpha(0.8), None, &tri);
    }
}

/// The ribbon between two polylines of the same length.
fn ribbon(scene: &mut Scene, color: Color, upper: &[(f64, f64)], lower: &[(f64, f64)]) {
    let Some((x, y)) = upper.first().copied() else {
        return;
    };
    let mut taken = BezPath::new();
    taken.move_to((x, y));
    for point in upper.iter().skip(1).copied() {
        taken.line_to(point);
    }
    for point in lower.iter().rev().copied() {
        taken.line_to(point);
    }
    taken.close_path();
    scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &taken);
}

/// The area under a polyline, down to `floor`.
fn area_under(scene: &mut Scene, color: Color, points: &[(f64, f64)], floor: f64) {
    let (Some((x0, y0)), Some((x1, _))) = (points.first().copied(), points.last().copied()) else {
        return;
    };
    let mut area = BezPath::new();
    area.move_to((x0, floor));
    area.line_to((x0, y0));
    for point in points.iter().skip(1).copied() {
        area.line_to(point);
    }
    area.line_to((x1, floor));
    area.close_path();
    scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &area);
}

/// A notch the settled reduction has found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tooth {
    /// Where across the zoomed band, 0..1.
    pub place: f64,
    /// How deep, in dB.
    pub depth_db: f32,
}

/// The local maxima of a settled reduction curve that are worth a
/// tooth: deeper than half a decibel, and the deepest bin of their
/// neighbourhood.
#[must_use]
pub fn teeth(settled: &[f32], zoom: &SuppressZoom) -> Vec<Tooth> {
    let bins = crate::num::coord(settled.len().saturating_sub(1).max(1));
    let full = (20_000.0_f64 / 20.0).log10();
    settled
        .iter()
        .enumerate()
        .filter(|(i, depth)| {
            **depth > 0.5
                && settled.get(i.wrapping_sub(1)).is_none_or(|left| left <= *depth)
                && settled.get(i.saturating_add(1)).is_none_or(|right| right < *depth)
        })
        .filter_map(|(i, depth)| {
            let hz = 20.0 * 10.0_f64.powf(crate::num::coord(i) / bins * full);
            let place = zoom.place_of(hz);
            (0.0..=1.0).contains(&place).then_some(Tooth {
                place,
                depth_db: *depth,
            })
        })
        .collect()
}

/// A value read out of the analyser's log-spaced bins at a frequency.
///
/// Between two bins rather than snapped to one, or a zoomed view turns
/// into a staircase.
fn bin_at(bins: &[f32], hz: f64) -> f64 {
    let last = crate::num::coord(bins.len().saturating_sub(1).max(1));
    let full = (20_000.0_f64 / 20.0).log10();
    let place = ((hz / 20.0).log10() / full).clamp(0.0, 1.0) * last;
    let low = place.floor();
    let i = crate::num::index(low);
    let a = bins.get(i).copied().unwrap_or(0.0);
    let b = bins.get(i.saturating_add(1)).copied().unwrap_or(a);
    let t = place - low;
    f64::from(a).mul_add(1.0 - t, f64::from(b) * t)
}

/// The window a suppressor's display shows: its band, opened out a
/// third of an octave each side so the shoulders are visible.
///
/// A cut you can see starting is a cut you can tell is in the right
/// place. Shared by the drawing and the hit test, because an edge you
/// grab has to be where the edge is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SuppressZoom {
    pub low: f64,
    pub high: f64,
}

impl SuppressZoom {
    #[must_use]
    pub fn of(set: Suppress) -> Self {
        Self {
            low: f64::from(set.low) / SUPPRESS_SHOULDER,
            high: f64::from(set.high) * SUPPRESS_SHOULDER,
        }
    }

    fn decade(self) -> f64 {
        (self.high / self.low).log10().max(f64::EPSILON)
    }

    /// The frequency at `t` across the window.
    #[must_use]
    pub fn hz_at(self, t: f64) -> f64 {
        self.low * 10.0_f64.powf(t * self.decade())
    }

    /// Where a frequency falls across the window, 0..1 inside it.
    #[must_use]
    pub fn place_of(self, hz: f64) -> f64 {
        (hz / self.low).log10() / self.decade()
    }

    /// The same, in a panel's pixels.
    #[must_use]
    pub fn x_of(self, hz: f64, at: Panel) -> f64 {
        self.place_of(hz).clamp(0.0, 1.0).mul_add(at.width, at.x)
    }
}

/// How far past its band a suppressor's display opens out.
const SUPPRESS_SHOULDER: f64 = 1.26;

/// The frequencies a zoomed suppressor ticks, where they fall inside
/// the band it is showing.
const SUPPRESS_MARKS: [(f64, &str); 6] = [
    (200.0, "200"),
    (1_000.0, "1k"),
    (2_000.0, "2k"),
    (5_000.0, "5k"),
    (10_000.0, "10k"),
    (15_000.0, "15k"),
];

/// The window a suppressor draws its spectrum in.
///
/// The analyser's own range — see `simulate::spectrum`, which clamps to
/// these — so a bin lands where the number says rather than where a
/// second opinion about the scale puts it.
const SUPPRESS_FLOOR_DB: f64 = -40.0;
const SUPPRESS_CEIL_DB: f64 = 16.0;

/// Which edge of a band.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Low,
    High,
}

/// The delay: its repeats, on a line — drawn the way its machine makes
/// them.
///
/// The substrate is `delay-ui`'s own centrepiece idea at strip size:
/// time runs left to right over a window wide enough to hold several
/// taps, each one shorter than the last by the feedback, and the dry
/// hit stands at the origin so the first gap IS the delay time. What
/// the FAMILY changes is the shape of one repeat, exactly as the
/// plugin's faces do: a digital delay's are identical bars, a tape's
/// round off and darken, a chip's smear wider each pass, a pitch
/// delay's climb, a rhythmic one's land on a grid, a reversed one's
/// swell into the tap. You should know which delay you are looking at
/// before you read a word.
///
/// Live, the repeats are lit by the wet return: a phrase decaying
/// through the taps brightens each in turn, so the panel plays the
/// delay rather than describing it.
fn echo(
    scene: &mut Scene,
    palette: &Palette,
    echo: Echo,
    meters: &Meters,
    at: Panel,
    rack: Rack,
    lit: Option<Grip>,
) {
    let bottom = at.y + at.height;
    let floor = bottom - 1.0;
    let family = echo.family();
    if rack.detailed() {
        rule(scene, palette.grid, Line::new((at.x, floor), (at.x + at.width, floor)));
        // A rhythmic delay lands on a grid, so the grid is drawn: a
        // beat every delay time, a bar every four.
        if family == DelayFamily::Rhythmic {
            let taps = echo_taps(echo, at);
            for (k, (x, _)) in taps.iter().enumerate() {
                let bar = k % 4 == 0;
                rule(
                    scene,
                    if bar { palette.grid_beat } else { palette.grid },
                    Line::new((*x, at.y), (*x, floor)),
                );
            }
        }
    }
    let wet = DELAY_INK;
    let time_held = lit == Some(Grip::Time);
    let feedback_held = lit == Some(Grip::Feedback);
    let mix = f64::from(echo.mix).clamp(0.0, 1.0);
    let feedback = f64::from(echo.feedback).clamp(0.0, 0.99);
    let mut previous_x = at.x + 3.0;
    for (k, (x, level)) in echo_taps(echo, at).into_iter().enumerate() {
        // The dry hit, then the repeats at the level the feedback
        // leaves them. The mix decides how strongly they are INKED
        // rather than how tall they are: a quiet delay is still a delay
        // with those repeats at those times, and shrinking them would
        // confuse the two settings.
        let height = at.height * 0.9 * level;
        let dry = k == 0;
        // Lit by the signal: the dry hit by the input, each repeat by
        // the wet return that has had k−1 more passes of feedback.
        let glow = if meters.is_empty() {
            0.0
        } else if dry {
            f64::from(meters.sat_peak).clamp(0.0, 1.0)
        } else {
            (f64::from(meters.delay_wet) * feedback.powi(i32::try_from(k).unwrap_or(1).saturating_sub(1))
                * 3.0)
                .clamp(0.0, 1.0)
        };
        let base = if dry { 1.0 } else { 0.35 + mix * 0.65 };
        let alpha = crate::mcp::f64_to_f32((base * glow.mul_add(0.3, 0.7)).clamp(0.0, 1.0));
        let ink = if dry { palette.text.multiply_alpha(alpha) } else { wet.multiply_alpha(alpha) };
        let held = (dry && time_held) || (!dry && feedback_held) || (k == 1 && time_held);
        let width = if dry || held { 2.0 } else { 1.4 };
        match family {
            _ if dry => rule_wide(scene, ink, Line::new((x, floor), (x, floor - height)), width),
            DelayFamily::Digital | DelayFamily::Rhythmic => {
                rule_wide(scene, ink, Line::new((x, floor), (x, floor - height)), width);
            }
            // Tape: the top rounds off and each pass is darker — the
            // head loses treble every time round.
            DelayFamily::Tape => {
                let dull = crate::mcp::f64_to_f32(crate::num::coord(k).mul_add(-0.12, 1.0).clamp(0.3, 1.0));
                let mut path = BezPath::new();
                path.move_to((x, floor));
                path.line_to((x, floor - height + 2.0));
                path.quad_to((x, floor - height), (x + 2.0, floor - height));
                scene.stroke(
                    &Stroke::new(width).with_caps(vello::kurbo::Cap::Round),
                    Affine::IDENTITY,
                    ink.multiply_alpha(dull),
                    None,
                    &path,
                );
            }
            // A chip: each repeat is wider than the last — the bucket
            // line smears.
            DelayFamily::Analog => {
                let half = crate::num::coord(k).mul_add(0.55, 0.6);
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    ink,
                    None,
                    &Rect::new(x - half, floor - height, x + half, floor),
                );
            }
            // Pitch: each repeat's foot rises by the interval.
            DelayFamily::Pitch => {
                let rise = 3.2 * crate::num::coord(k);
                rule_wide(scene, ink, Line::new((x, floor - rise), (x, floor - rise - height)), width);
                rule(scene, palette.grid_beat, Line::new((x - 2.0, floor - rise), (x + 2.0, floor - rise)));
            }
            // Special: a reversed repeat swells INTO the tap; the rest
            // are a repeat that is no longer one — a soft blob whose
            // height is its level.
            DelayFamily::Special => {
                if echo.style == DelayStyle::Reverse {
                    let mut wedge = BezPath::new();
                    wedge.move_to((previous_x + 2.0, floor));
                    wedge.line_to((x, floor));
                    wedge.line_to((x, floor - height));
                    wedge.close_path();
                    scene.fill(Fill::NonZero, Affine::IDENTITY, ink.multiply_alpha(0.8), None, &wedge);
                } else {
                    let rx = crate::num::coord(k).mul_add(0.5, 1.6);
                    scene.fill(
                        Fill::NonZero,
                        Affine::IDENTITY,
                        ink.multiply_alpha(0.7),
                        None,
                        &vello::kurbo::Ellipse::new((x, floor - height / 2.0), (rx, height / 2.0), 0.0),
                    );
                }
            }
        }
        previous_x = x;
    }
}

/// Where the delay's taps land, and how tall each is (0..1).
///
/// The dry hit first, then the repeats at the level the feedback
/// leaves them. Shared by the drawing and the hit test: a tap you grab
/// has to be where the tap is drawn.
#[must_use]
pub fn echo_taps(echo: Echo, at: Panel) -> Vec<(f64, f64)> {
    let left = at.x + 3.0;
    // A window of four and a half taps at the current time, so the
    // picture keeps its shape as the time changes rather than the taps
    // marching off the end. What moves with the time is the SPACING,
    // which is the thing the number means.
    let time = f64::from(echo.time).max(1.0);
    let window = time * 4.5;
    let feedback = f64::from(echo.feedback).clamp(0.0, 0.99);
    let rhythmic = echo.family() == DelayFamily::Rhythmic;
    let mut out = Vec::with_capacity(16);
    let mut level = 1.0_f64;
    let mut when = 0.0_f64;
    for tap in 0..16_usize {
        let x = (when / window).mul_add(at.width - 6.0, left);
        if x > at.x + at.width || level < 0.03 {
            break;
        }
        out.push((x, level));
        let step = if rhythmic {
            PATTERN.get(tap.rem_euclid(PATTERN.len())).copied().unwrap_or(1.0)
        } else {
            1.0
        };
        when += time * step;
        level *= feedback;
    }
    out
}

/// Where a rhythmic delay's taps land: on subdivisions rather than
/// evenly, in delay times.
const PATTERN: [f64; 5] = [1.0, 0.5, 0.75, 0.5, 1.25];

/// The reverb: an impulse, the gap before its tail, and the tail — the
/// algorithm's own.
///
/// `reverb-ui`'s third centrepiece — "the recorded thing itself: an
/// impulse and its decay envelope" — which is the only picture of a
/// reverb that survives being a hundred and thirty pixels wide. But
/// the envelope is not an exponential drawn from the decay knob: it is
/// the algorithm's impulse response, rendered by the plugin's own
/// probe (see [`crate::live::tail`]). That is what makes a plate and a
/// spring with the same decay different pictures — a plate is dense
/// from the first millisecond, a spring drips, a bloom rises before it
/// falls — because they are different sounds.
///
/// While the render is on its way the exponential stands in, dashed:
/// a guess that looked like a measurement would be believed.
fn room(
    scene: &mut Scene,
    palette: &Palette,
    room: Room,
    meters: &Meters,
    at: Panel,
    rack: Rack,
    lit: Option<Grip>,
) {
    let bottom = at.y + at.height;
    let floor = bottom - 1.0;
    if rack.detailed() {
        rule(scene, palette.grid, Line::new((at.x, floor), (at.x + at.width, floor)));
    }
    let ink = REVERB_INK;
    let mix = f64::from(room.mix).clamp(0.0, 1.0);
    let geometry = RoomGeometry::of(room, at);

    // The dry impulse.
    rule_wide(
        scene,
        palette.text,
        Line::new((at.x, floor), (at.x, at.height.mul_add(-0.92, floor))),
        2.0,
    );

    // The tail: the rendered envelope, dB below its peak, across the
    // decay window. The mix scales its height — a quiet reverb is a
    // low tail — but never below a quarter, or a reverb at 5% would
    // be a floor line and nothing else.
    let tail = crate::live::tail(room.key());
    let scale = mix.mul_add(0.75, 0.25) * at.height * 0.92;
    let points: Vec<(f64, f64)> = tail
        .envelope
        .iter()
        .enumerate()
        .map(|(i, db)| {
            let t = crate::num::coord(i) / crate::num::coord(crate::live::TAIL_BINS.saturating_sub(1));
            let level = (1.0 + f64::from(*db) / 60.0).clamp(0.0, 1.0);
            (geometry.start + t * (geometry.end - geometry.start), floor - level * scale)
        })
        .collect();
    // Lit by the wet return: a reverb doing nothing is an outline, one
    // washing a chorus is a filled shape.
    if !meters.is_empty()
        && let (Some(first), Some(last)) = (points.first().copied(), points.last().copied())
    {
        let wet = f64::from(meters.reverb_wet).clamp(0.0, 1.0);
        let mut area = BezPath::new();
        area.move_to((first.0, floor));
        for point in points.iter().copied() {
            area.line_to(point);
        }
        area.line_to((last.0, floor));
        area.close_path();
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            ink.multiply_alpha(crate::mcp::f64_to_f32(wet.mul_add(0.45, 0.08))),
            None,
            &area,
        );
    }
    let held = matches!(lit, Some(Grip::Decay | Grip::Mix(Which::Reverb)));
    let width = if held { 2.0 } else { 1.4 };
    if tail.exact {
        curve(scene, ink, points.into_iter(), width);
    } else {
        dashed(scene, ink, points.into_iter(), width);
    }
    if rack.detailed() {
        let held = lit == Some(Grip::Predelay);
        rule_wide(
            scene,
            if held { palette.text } else { palette.grid_beat },
            Line::new((geometry.start, at.y), (geometry.start, floor)),
            if held { 1.8 } else { 1.0 },
        );
    }
}

/// Where the reverb's tail begins and ends in a panel.
///
/// The window is the decay, so a long reverb fills the panel and a
/// short one does not reach the end — which is the comparison you want
/// across a mixer. Shared by the drawing and the hit test.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoomGeometry {
    /// Where the tail starts: the predelay, along the decay window.
    pub start: f64,
    /// The panel's right edge — the decay reaches it by definition.
    pub end: f64,
}

impl RoomGeometry {
    #[must_use]
    pub fn of(room: Room, at: Panel) -> Self {
        let window = (f64::from(room.decay) * 1000.0).max(1.0);
        let start = (f64::from(room.predelay) / window).clamp(0.0, 0.5).mul_add(at.width, at.x);
        Self {
            start,
            end: at.x + at.width,
        }
    }
}

/// The saturator: its curve, where the signal is on it, and what it
/// is adding.
///
/// A transfer curve alone says drive and nothing else, because every
/// saturator is an S. The model underneath has the two things that
/// separate a valve from a transistor — asymmetry, and the harmonics
/// that produces — and the plugin measures the second with a probe. So
/// the panel answers the three questions a mixer asks of a saturator:
///
/// - **Is it happening?** The curve is lit from the origin out to the
///   signal's peak this frame. A track well under the knee lights the
///   straight part; a slammed one lights the bend.
/// - **What is it adding?** A ladder of seven bars, H2 through H8, from
///   the probe — evens in the phase's yellow, odds in grey. A biased
///   triode draws a yellow ladder, a symmetric rail a grey one, and you
///   can tell them apart from across the room. It breathes with the
///   signal: the probe runs at three levels and the live peak reads
///   between them.
/// - **Which circuit?** The glyph beside the name, from the plugin's
///   own classification.
fn sat(
    scene: &mut Scene,
    palette: &Palette,
    tone: &Tone,
    meters: &Meters,
    at: Panel,
    rack: Rack,
    lit: Option<Grip>,
) {
    let pre = &tone.sat;
    let whole = at;
    let (at, ladder_box) = sat_split(at, rack);
    let right = at.x + at.width;
    let bottom = at.y + at.height;
    let mid_y = at.y + at.height / 2.0;
    let mid_x = at.x + at.width / 2.0;

    // How hard it is saturating: what the stage is adding at the level
    // the signal is at, as the ladder measures it — the sum of the
    // rungs, on a square root so the first decibel of colour shows.
    // With nothing playing it is the full-scale ladder, so a recorded
    // rack still says how hot the setting is.
    let ladder = crate::live::ladder(pre);
    let rungs = if meters.is_empty() {
        ladder.full()
    } else {
        let db = 20.0 * f64::from(meters.sat_peak.max(1e-4)).log10();
        ladder.at(crate::mcp::f64_to_f32(db))
    };
    let heat = (f64::from(rungs.iter().sum::<f32>()) / 0.9).clamp(0.0, 1.0).powf(0.7);
    glow(scene, whole, (mid_x, mid_y), heat);

    if rack.detailed() {
        rule(scene, palette.grid, Line::new((at.x, mid_y), (right, mid_y)));
        rule(scene, palette.grid, Line::new((mid_x, at.y), (mid_x, bottom)));
        // Unity, so the curve's departure from it IS the saturation.
        // Without it a gentle drive and a hard one are both "an S", and
        // the thing you are looking for is how far from straight it
        // has gone.
        rule(
            scene,
            palette.grid_beat,
            Line::new((at.x, bottom), (right, at.y)),
        );
    }

    let mut samples = [(0.0_f32, 0.0_f32); SAMPLES];
    saturate_dsp::preamp::analysis::transfer_curve(pre, &mut samples);
    // A quantiser has no transfer curve of its own — there is no curve
    // that produces an alias — but it does one thing to this one: it
    // turns it into a staircase, and that is the honest picture of it.
    if !tone.sat_digital.is_transparent() {
        let levels = (f64::from(tone.sat_digital.bits.clamp(1.0, 16.0)) - 1.0).exp2();
        for (_, y) in &mut samples {
            *y = crate::mcp::f64_to_f32((f64::from(*y) * levels).round() / levels);
        }
    }
    let place = |(x, y): &(f32, f32)| {
        (
            f64::midpoint(f64::from(*x), 1.0).mul_add(at.width, at.x),
            (mid_y - f64::from(*y) * at.height / 2.0).clamp(at.y, bottom),
        )
    };
    let held = matches!(lit, Some(Grip::Drive | Grip::Bias));
    curve(scene, SAT_INK, samples.iter().map(place), if held { 2.2 } else { 1.5 });

    // The lit reach: from the origin out to the signal's peak, both
    // ways. One stroke, and the whole "is it doing anything" answer.
    if !meters.is_empty() {
        let peak = f64::from(meters.sat_peak).clamp(0.0, 1.0);
        let tint = SAT_HOT;
        let reach: Vec<(f64, f64)> = samples
            .iter()
            .filter(|(x, _)| f64::from(x.abs()) <= peak)
            .map(place)
            .collect();
        if reach.len() >= 2 {
            if let (Some(a), Some(b)) = (reach.first().copied(), reach.last().copied()) {
                dot(scene, tint, a, 1.8);
                dot(scene, tint, b, 1.8);
            }
            curve(scene, tint, reach.into_iter(), 2.4);
        }
    }

    // The ladder.
    if let Some(lb) = ladder_box {
        let floor = lb.y + lb.height - 1.0;
        rule(scene, palette.grid, Line::new((lb.x, floor), (lb.x + lb.width, floor)));
        let tint = SAT_INK;
        let n = crate::num::coord(crate::live::RUNGS);
        let gap = 1.2;
        let bar = ((lb.width - 2.0 - gap * (n - 1.0)) / n).max(1.0);
        let tall = lb.height - 4.0;
        for (k, rung) in rungs.iter().enumerate() {
            // Rung k is harmonic k+2, so the evens are the even k. Drawn
            // on a square root so the fourth and sixth are visible
            // beside a second that dwarfs them; the header prints the
            // share as a number.
            let even = k % 2 == 0;
            let h = f64::from(rung.clamp(0.0, 1.0)).sqrt() * tall;
            let x = crate::num::coord(k).mul_add(bar + gap, lb.x + 1.0);
            let ink = if even { tint } else { palette.text_faint.multiply_alpha(0.75) };
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                ink,
                None,
                &Rect::new(x, floor - h, x + bar, floor),
            );
        }
        // The tilt, as a wedge under the ladder: which end of the
        // spectrum meets the knee first.
        if rack.detailed() {
            let lean = (f64::from(pre.tilt_db()) / 12.0).clamp(-1.0, 1.0);
            let held = lit == Some(Grip::Tilt);
            rule_wide(
                scene,
                if held { palette.text } else { palette.text_faint },
                Line::new(
                    (lb.x + 2.0, lean.mul_add(1.5, floor + 3.0)),
                    (lb.x + lb.width - 2.0, lean.mul_add(-1.5, floor + 3.0)),
                ),
                if held { 1.6 } else { 0.9 },
            );
        }
    }
}

/// The saturator's ink: red. Every other unit takes the strip's own
/// colours; this one is the one that makes heat, and it says so.
const SAT_INK: Color = Color::from_rgba8(0xff, 0x4a, 0x3d, 0xff);

/// The lit reach's ink — hotter than the curve, toward white.
const SAT_HOT: Color = Color::from_rgba8(0xff, 0xb3, 0x47, 0xff);

/// The glow's ink — the red of a stage that is working.
const SAT_GLOW: Color = Color::from_rgba8(0xff, 0x2e, 0x0c, 0xff);

/// The saturator's ground, lit from behind by how hard it is working.
///
/// A wash over the panel and a radial glow from the curve's centre,
/// both scaled by `heat` (0..1): a stage adding nothing sits on the
/// rack's own ground, and one at the top of its drive glows like the
/// valve it is modelling. Heat is raised to 0.7 so unity stays cool
/// and the top of the range is where the fire is. One gradient fill — cheap, and recorded with
/// the rest of the panel since heat is a fact about the settings and
/// the level, both of which the rack already rebuilds on.
fn glow(scene: &mut Scene, at: Panel, centre: (f64, f64), heat: f64) {
    if heat <= 0.02 {
        return;
    }
    let alpha = |share: f64| crate::mcp::f64_to_f32((share * heat).clamp(0.0, 1.0));
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        SAT_GLOW.multiply_alpha(alpha(0.22)),
        None,
        &at.rect(),
    );
    let radius = crate::mcp::f64_to_f32(at.width.max(at.height) * 0.7);
    let paint = anyrender::Paint::Gradient(
        vello::peniko::Gradient::new_radial((centre.0, centre.1), radius).with_stops([
            (0.0, SAT_HOT.multiply_alpha(alpha(0.7))),
            (0.35, SAT_GLOW.multiply_alpha(alpha(0.55))),
            (1.0, SAT_GLOW.multiply_alpha(0.0)),
        ]),
    );
    scene.fill(Fill::NonZero, Affine::IDENTITY, &paint, None, &at.rect());
}

/// The saturator's body: the curve's square on the left, the ladder's
/// column on the right — or the whole body for the curve when there
/// is no room for a ladder anyone could read.
#[must_use]
pub const fn sat_split(body: Panel, rack: Rack) -> (Panel, Option<Panel>) {
    if !rack.detailed() || body.width < 80.0 {
        return (body, None);
    }
    let ladder_w = body.width * 0.34;
    let curve = Panel {
        width: body.width - ladder_w - 4.0,
        ..body
    };
    let ladder = Panel {
        x: body.x + body.width - ladder_w,
        width: ladder_w,
        height: body.height - 6.0,
        ..body
    };
    (curve, Some(ladder))
}

/// An open polyline, dashed — for a picture that is a guess rather
/// than a measurement.
fn dashed(
    scene: &mut Scene,
    color: Color,
    points: impl Iterator<Item = (f64, f64)>,
    width: f64,
) {
    let mut path = BezPath::new();
    for (i, point) in points.enumerate() {
        if i == 0 {
            path.move_to(point);
        } else {
            path.line_to(point);
        }
    }
    if path.is_empty() {
        return;
    }
    scene.stroke(
        &Stroke::new(width).with_dashes(0.0, [2.5, 2.0]),
        Affine::IDENTITY,
        color,
        None,
        &path,
    );
}

/// The machine a unit is, in eight pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Circuit(Circuit),
    Delay(DelayFamily),
    Room(RoomFamily),
}

/// How wide a header glyph is, with its gap to the name.
pub const GLYPH_W: f64 = 11.0;

/// Draw a glyph with its left edge at `x`, sitting on `baseline`.
///
/// Paths, never text: a glyph is drawn into every header of every
/// strip, and a glyph run is the most expensive thing the renderer
/// draws. Each is the plugin's own face at the smallest size it still
/// reads: a valve, reels, a chip, an arch, a coil.
fn glyph(scene: &mut Scene, ink: Color, glyph: Glyph, x: f64, baseline: f64) {
    match glyph {
        Glyph::Circuit(circuit) => circuit_glyph(scene, ink, circuit, x, baseline),
        Glyph::Delay(family) => delay_glyph(scene, ink, family, x, baseline),
        Glyph::Room(family) => room_glyph(scene, ink, family, x, baseline),
    }
}

/// The saturator's five circuits, in the plugin's own rail order —
/// which is `saturate_profiles::CATEGORIES`' order, chip for chip.
const CIRCUITS: [Circuit; 5] = [
    Circuit::Valve,
    Circuit::Tape,
    Circuit::Core,
    Circuit::Solid,
    Circuit::Steps,
];

/// The machine selector: one chip per family, the current family lit,
/// and the machine's name beside them.
///
/// The chips are the family glyphs — the same drawings as the header's
/// — so a row of them reads as the set of machines this unit can be,
/// and the lit one as which it is. A family with several machines
/// (three halls, two springs) is walked by clicking its chip again;
/// the name says which member you are on.
fn selector(
    scene: &mut Scene,
    palette: &Palette,
    font: &Font,
    tone: &Tone,
    which: Which,
    at: Panel,
    lit: Option<Grip>,
) {
    const SIZE: f32 = 6.5;
    let (chips, current, name): (Vec<Glyph>, usize, &str) = match which {
        Which::Sat => {
            let profile = saturate_profiles::PROFILES.get(tone.sat_profile);
            (
                CIRCUITS.iter().map(|c| Glyph::Circuit(*c)).collect(),
                profile
                    .and_then(|p| saturate_profiles::category_of(p.id))
                    .map_or(0, |(category, _)| category),
                profile.map_or("", |p| p.name),
            )
        }
        Which::Delay => (
            DelayFamily::ALL.iter().map(|f| Glyph::Delay(*f)).collect(),
            DelayFamily::ALL.iter().position(|f| *f == tone.delay.family()).unwrap_or(0),
            tone.delay.style.label(),
        ),
        _ => (
            RoomFamily::ALL.iter().map(|f| Glyph::Room(*f)).collect(),
            RoomFamily::ALL.iter().position(|f| *f == tone.reverb.family()).unwrap_or(0),
            tone.reverb.algorithm.name(),
        ),
    };
    let baseline = at.y + at.height - 4.0;
    let tint = unit_ink(which).unwrap_or_else(|| phase_tint(which.phase()));
    for (i, chip) in chips.iter().enumerate() {
        let x = crate::num::coord(i).mul_add(CHIP, at.x + 2.0);
        let is_current = i == current;
        let hovered = lit == Some(Grip::Choose(which, i));
        let ink = if is_current {
            palette.text
        } else if hovered {
            palette.text_dim
        } else {
            palette.text_faint.multiply_alpha(0.6)
        };
        glyph(scene, ink, *chip, x, baseline);
        // The current family is underlined in the phase's colour: a
        // lit glyph among dim ones says "this one", the bar says it in
        // the colour the rail uses for this pass.
        if is_current {
            rule_wide(
                scene,
                tint,
                Line::new((x - 1.0, at.y + at.height - 1.0), (x + 9.0, at.y + at.height - 1.0)),
                1.5,
            );
        }
    }
    let chips_w = crate::num::coord(chips.len()).mul_add(CHIP, 2.0);
    let name_w = font.width(name, SIZE);
    if chips_w + name_w + 4.0 <= at.width {
        crate::tcp::glyphs(
            scene,
            font,
            palette.text_dim,
            name,
            at.x + at.width - name_w,
            baseline,
            SIZE,
        );
    }
}

/// The de-esser's ink: yellow — the Tone phase's own, and the colour
/// of the top end it is there to take the edge off.
const DEESS_INK: Color = Color::from_rgba8(0xfa, 0xcc, 0x15, 0xff);

/// The delay's colour: blue. The machine, its repeats, its knobs and
/// its selector, and the delay tracks in the template — one colour
/// for the one thing.
pub const DELAY_INK: Color = Color::from_rgba8(0x3b, 0x82, 0xf6, 0xff);

/// The reverb's colour: purple, likewise.
pub const REVERB_INK: Color = Color::from_rgba8(0x8b, 0x5c, 0xf6, 0xff);

/// The decay-rate EQ's colour: a lighter violet — the reverb's family,
/// and never the EQ's own look, so a graph of time is not read as a
/// graph of gain.
const DECAY_INK: Color = Color::from_rgba8(0xa7, 0x8b, 0xfa, 0xff);

/// The colour a unit is drawn in where it has one of its own.
const fn unit_ink(which: Which) -> Option<Color> {
    match which {
        Which::Sat => Some(SAT_INK),
        Which::Delay => Some(DELAY_INK),
        Which::Reverb => Some(REVERB_INK),
        _ => None,
    }
}

/// The preset row: one chip per preset, brighter to darker, the loaded
/// one lit.
///
/// Each chip carries a swatch that runs from warm white to deep blue
/// down the row — a template's rows run bright and close to dark and
/// far, and the swatch says where a chip sits on that run before you
/// read its name.
fn presets(scene: &mut Scene, palette: &Palette, font: &Font, tone: &Tone, at: Panel, lit: Option<Grip>) {
    const SIZE: f32 = 6.5;
    if tone.presets.is_empty() {
        return;
    }
    let count = crate::num::coord(tone.presets.len().max(2).saturating_sub(1));
    let mut left = at.x + 2.0;
    for (i, preset) in tone.presets.iter().enumerate() {
        let w = preset_chip_width(&preset.name);
        if left + w > at.x + at.width {
            break;
        }
        let current = tone.preset == Some(i);
        let hovered = lit == Some(Grip::Preset(i));
        let t = crate::num::coord(i) / count;
        let swatch = swatch_at(t);
        let chip = Rect::new(left, at.y + 2.0, left + w, at.y + at.height - 2.0);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            if current {
                palette.tcp_meter_well.multiply_alpha(1.0)
            } else {
                palette.tcp_meter_well.multiply_alpha(0.5)
            },
            None,
            &chip.to_rounded_rect(2.0),
        );
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            swatch,
            None,
            &Rect::new(left + 2.0, at.y + 4.0, left + 8.0, at.y + at.height - 4.0).to_rounded_rect(1.0),
        );
        let ink = if current {
            palette.text
        } else if hovered {
            palette.text_dim
        } else {
            palette.text_faint
        };
        crate::tcp::glyphs(scene, font, ink, &preset.name, left + 11.0, at.y + at.height - 5.0, SIZE);
        if current {
            rule_wide(
                scene,
                swatch,
                Line::new((left + 1.0, at.y + at.height - 1.0), (left + w - 1.0, at.y + at.height - 1.0)),
                1.5,
            );
        }
        left += w + 2.0;
    }
}

/// Where a chip sits on the bright-to-dark run, as a colour.
fn swatch_at(t: f64) -> Color {
    let bright = (0xff_u8, 0xe6_u8, 0xa0_u8);
    let dark = (0x3b_u8, 0x4a_u8, 0x8a_u8);
    let mix = |a: u8, b: u8| {
        let v = f64::from(a).mul_add(1.0 - t, f64::from(b) * t);
        crate::num::index(v.round()).min(255)
    };
    Color::from_rgba8(
        u8::try_from(mix(bright.0, dark.0)).unwrap_or(0),
        u8::try_from(mix(bright.1, dark.1)).unwrap_or(0),
        u8::try_from(mix(bright.2, dark.2)).unwrap_or(0),
        0xff,
    )
}

/// A row of the effect's knobs.
///
/// Five, evenly across, each an arc from seven o'clock to five with
/// the travel filled in the phase's colour and a pointer on the value.
/// Legends under them at a detailed width. What the plugin's face has
/// under its centrepiece, at strip size.
fn knobs(scene: &mut Scene, palette: &Palette, font: &Font, tone: &Tone, at: Panel, rack: Rack, lit: Option<Grip>) {
    const SIZE: f32 = 5.5;
    let labels = knob_labels(tone.role);
    let each = at.width / crate::num::coord(KNOBS);
    let r = (each * 0.32).min(at.height * 0.32).max(4.0);
    let tint = match tone.role {
        Role::Reverb => REVERB_INK,
        Role::Delay => DELAY_INK,
        _ => phase_tint(Which::Knobs.phase()),
    };
    for (i, label) in labels.iter().enumerate() {
        let cx = crate::num::coord(i).mul_add(each, at.x + each / 2.0);
        let cy = at.y + r + 3.0;
        let value = knob_value(tone, Which::Knobs, i);
        let held = lit == Some(Grip::Knob(Which::Knobs, i));
        knob_arc(scene, palette, (cx, cy), r, value, tint, held);
        if rack.detailed() {
            let w = font.width(label, SIZE);
            crate::tcp::glyphs(scene, font, palette.text_faint, label, cx - w / 2.0, at.y + at.height - 2.0, SIZE);
        }
    }
}

/// One knob: its track, its travel, its pointer.
fn knob_arc(scene: &mut Scene, palette: &Palette, centre: (f64, f64), r: f64, value: f64, tint: Color, held: bool) {
    use std::f64::consts::PI;
    // Seven o'clock to five o'clock, clockwise: 270 degrees of travel.
    let start = PI * 0.75;
    let sweep = PI * 1.5;
    let arc = |from: f64, to: f64| vello::kurbo::Arc::new(centre, (r, r), from, to - from, 0.0);
    scene.stroke(&Stroke::new(if held { 2.4 } else { 1.6 }), Affine::IDENTITY, palette.grid_beat, None, &arc(start, start + sweep));
    let to = value.clamp(0.0, 1.0).mul_add(sweep, start);
    if value > 0.005 {
        scene.stroke(&Stroke::new(if held { 2.4 } else { 1.6 }), Affine::IDENTITY, tint, None, &arc(start, to));
    }
    let (px, py) = (to.cos().mul_add(r, centre.0), to.sin().mul_add(r, centre.1));
    rule_wide(scene, if held { palette.text } else { palette.text_dim }, Line::new(centre, (px, py)), 1.2);
}

/// The widener: the stereo field as a fan, as wide as the setting.
///
/// A semicircle is the whole field; the filled wedge is how much of it
/// the track occupies — a sliver for mono, the speakers at unity, and
/// past them when pushed. The one picture of width that survives a
/// strip.
fn wide(scene: &mut Scene, palette: &Palette, tone: &Tone, at: Panel, rack: Rack, lit: Option<Grip>) {
    use std::f64::consts::PI;
    let centre = (at.x + at.width / 2.0, at.y + at.height - 4.0);
    let r = (at.width / 2.0 - 4.0).min(at.height - 8.0).max(6.0);
    let tint = phase_tint(Which::Wide.phase());
    let field = vello::kurbo::Arc::new(centre, (r, r), PI, PI, 0.0);
    scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, palette.grid_beat, None, &field);
    if rack.detailed() {
        // The speakers, at unity.
        for side in [-1.0_f64, 1.0] {
            let angle = PI * side.mul_add(0.25, 0.5);
            let (x, y) = ((-angle.cos()).mul_add(r, centre.0), (-angle.sin()).mul_add(r, centre.1));
            rule(scene, palette.grid_beat, Line::new(centre, (x, y)));
        }
    }
    // The wedge: half the field per unit of width.
    let half = (f64::from(tone.wide).clamp(0.0, 2.0) * PI / 4.0).min(PI / 2.0);
    let held = lit == Some(Grip::Knob(Which::Wide, 0));
    let mut wedge = BezPath::new();
    wedge.move_to(centre);
    let steps = 24;
    for k in 0..=steps {
        let t = crate::num::coord(k) / crate::num::coord(steps);
        let angle = (t * 2.0).mul_add(half, -PI / 2.0 - half);
        wedge.line_to((angle.cos().mul_add(r, centre.0), angle.sin().mul_add(r, centre.1)));
    }
    wedge.close_path();
    scene.fill(Fill::NonZero, Affine::IDENTITY, tint.multiply_alpha(if held { 0.55 } else { 0.35 }), None, &wedge);
    scene.stroke(&Stroke::new(if held { 2.0 } else { 1.2 }), Affine::IDENTITY, tint, None, &wedge);
}

/// The pitch shifter: the note, and where it goes.
///
/// Two bars — the input at unity and the shifted copy raised or
/// lowered by the interval — and the interval as a number, because an
/// octave is an octave and a bar's height is not a thing anyone can
/// read to the semitone.
fn pitch(scene: &mut Scene, palette: &Palette, font: &Font, tone: &Tone, at: Panel, rack: Rack, lit: Option<Grip>) {
    let tint = phase_tint(Which::Pitch.phase());
    let floor = at.y + at.height - 2.0;
    let mid = at.y + at.height / 2.0;
    rule(scene, palette.grid_beat, Line::new((at.x, mid), (at.x + at.width, mid)));
    let held = matches!(lit, Some(Grip::Knob(Which::Pitch, _)));
    // The input.
    let x0 = at.width.mul_add(0.3, at.x);
    rule_wide(scene, palette.text, Line::new((x0, floor), (x0, mid)), 3.0);
    // The shift: up for a positive interval, down for a negative one,
    // a quarter of the panel per octave.
    let shift = f64::from(tone.pitch) / 12.0 * at.height * 0.25;
    let x1 = at.width.mul_add(0.6, at.x);
    let top = (mid - shift).clamp(at.y + 2.0, floor);
    let alpha = crate::mcp::f64_to_f32(f64::from(tone.pitch_mix).mul_add(0.7, 0.3));
    rule_wide(scene, tint.multiply_alpha(alpha), Line::new((x1, floor), (x1, top)), if held { 4.0 } else { 3.0 });
    rule(scene, tint.multiply_alpha(0.6), Line::new((x0, mid), (x1, top)));
    if rack.detailed() {
        let text = format!("{:+}", tone.pitch);
        crate::tcp::glyphs(scene, font, palette.text, &text, x1 + 6.0, top.max(at.y + 8.0), 7.0);
    }
}

/// A stroked glyph path, round-capped.
fn stroke_glyph(scene: &mut Scene, ink: Color, path: &BezPath, width: f64) {
    scene.stroke(
        &Stroke::new(width).with_caps(vello::kurbo::Cap::Round),
        Affine::IDENTITY,
        ink,
        None,
        path,
    );
}

/// The saturator's circuit, in eight pixels.
fn circuit_glyph(scene: &mut Scene, ink: Color, circuit: Circuit, x: f64, baseline: f64) {
    let top = baseline - 7.0;
    let stroke = |scene: &mut Scene, path: &BezPath, w: f64| stroke_glyph(scene, ink, path, w);
    let mut path = BezPath::new();
    match circuit {
        // A valve: a rounded envelope on a base.
        Circuit::Valve => {
            path.move_to((x + 1.0, baseline));
            path.line_to((x + 1.0, top + 3.0));
            path.quad_to((x + 1.0, top), (x + 4.0, top));
            path.quad_to((x + 7.0, top), (x + 7.0, top + 3.0));
            path.line_to((x + 7.0, baseline));
            stroke(scene, &path, 0.9);
        }
        // Tape: two reels.
        Circuit::Tape => reels(scene, ink, x, top),
        // A core: windings.
        Circuit::Core => {
            for i in 0..4 {
                let cx = crate::num::coord(i).mul_add(2.0, x + 1.0);
                path.move_to((cx, baseline));
                path.quad_to((cx, top), (cx + 2.0, top));
            }
            stroke(scene, &path, 0.8);
        }
        // Solid state: the corner.
        Circuit::Solid => {
            path.move_to((x, baseline));
            path.line_to((x + 4.0, top + 1.0));
            path.line_to((x + 8.0, top + 1.0));
            stroke(scene, &path, 1.0);
        }
        // Steps.
        Circuit::Steps => {
            path.move_to((x, baseline));
            for i in 0..3 {
                let sx = crate::num::coord(i).mul_add(2.6, x);
                let sy = crate::num::coord(i.saturating_add(1)).mul_add(-2.2, baseline);
                path.line_to((sx, sy));
                path.line_to((sx + 2.6, sy));
            }
            stroke(scene, &path, 0.9);
        }
    }
}

/// Two reels — tape, whether it is a saturator's or a delay's.
fn reels(scene: &mut Scene, ink: Color, x: f64, top: f64) {
    let mut path = BezPath::new();
    for cx in [x + 2.0, x + 6.5] {
        path.extend(vello::kurbo::Circle::new((cx, top + 3.5), 1.8).to_path(0.1).elements().iter().copied());
    }
    path.move_to((x + 2.0, top + 1.7));
    path.line_to((x + 6.5, top + 1.7));
    stroke_glyph(scene, ink, &path, 0.8);
}

/// The delay's family, in eight pixels.
fn delay_glyph(scene: &mut Scene, ink: Color, family: DelayFamily, x: f64, baseline: f64) {
    let top = baseline - 7.0;
    let stroke = |scene: &mut Scene, path: &BezPath, w: f64| stroke_glyph(scene, ink, path, w);
    let mut path = BezPath::new();
    match family {
        // Digital: three exact ticks.
        DelayFamily::Digital => {
            for i in 0..3 {
                let tx = crate::num::coord(i).mul_add(3.0, x + 1.0);
                path.move_to((tx, baseline));
                path.line_to((tx, top + 1.0));
            }
            stroke(scene, &path, 1.0);
        }
        DelayFamily::Tape => reels(scene, ink, x, top),
        // Analog: a chip with legs.
        DelayFamily::Analog => {
            path.extend(Rect::new(x + 1.0, top + 1.5, x + 7.0, baseline - 1.5).to_path(0.1).elements().iter().copied());
            for i in 0..3 {
                let lx = crate::num::coord(i).mul_add(2.0, x + 2.0);
                path.move_to((lx, top + 1.5));
                path.line_to((lx, top));
                path.move_to((lx, baseline - 1.5));
                path.line_to((lx, baseline));
            }
            stroke(scene, &path, 0.8);
        }
        // Pitch: a staircase of repeats.
        DelayFamily::Pitch => {
            for i in 0..3 {
                let tx = crate::num::coord(i).mul_add(3.0, x + 1.0);
                let rise = crate::num::coord(i) * 2.0;
                path.move_to((tx, baseline - rise));
                path.line_to((tx, rise.mul_add(-0.5, top + 2.0)));
            }
            stroke(scene, &path, 1.0);
        }
        // Rhythmic: the tap grid.
        DelayFamily::Rhythmic => {
            for (i, j) in [(0, 0), (1, 0), (0, 1), (1, 1), (2, 0)] {
                let cx = crate::num::coord(i).mul_add(3.0, x + 1.5);
                let cy = crate::num::coord(j).mul_add(3.5, top + 2.0);
                path.extend(vello::kurbo::Circle::new((cx, cy), 0.9).to_path(0.1).elements().iter().copied());
            }
            scene.fill(Fill::NonZero, Affine::IDENTITY, ink, None, &path);
        }
        // Special: a smear.
        DelayFamily::Special => {
            path.extend(vello::kurbo::Ellipse::new((x + 4.0, top + 3.5), (3.5, 2.0), 0.4).to_path(0.1).elements().iter().copied());
            stroke(scene, &path, 0.8);
        }
    }
}

/// The reverb's family, in eight pixels.
fn room_glyph(scene: &mut Scene, ink: Color, family: RoomFamily, x: f64, baseline: f64) {
    let top = baseline - 7.0;
    let stroke = |scene: &mut Scene, path: &BezPath, w: f64| stroke_glyph(scene, ink, path, w);
    let mut path = BezPath::new();
    match family {
        // A hall: the arch.
        RoomFamily::Hall => {
            path.move_to((x, baseline));
            path.line_to((x, top + 3.0));
            path.quad_to((x, top), (x + 4.0, top));
            path.quad_to((x + 8.0, top), (x + 8.0, top + 3.0));
            path.line_to((x + 8.0, baseline));
            stroke(scene, &path, 0.9);
        }
        // A plate: the sheet, hung.
        RoomFamily::Plate => {
            path.extend(Rect::new(x + 0.5, top + 2.0, x + 7.5, baseline).to_path(0.1).elements().iter().copied());
            path.move_to((x + 2.0, top + 2.0));
            path.line_to((x + 2.0, top));
            path.move_to((x + 6.0, top + 2.0));
            path.line_to((x + 6.0, top));
            stroke(scene, &path, 0.8);
        }
        // A room: a box in perspective.
        RoomFamily::Room => {
            path.move_to((x, baseline));
            path.line_to((x + 2.0, top + 2.0));
            path.line_to((x + 6.0, top + 2.0));
            path.line_to((x + 8.0, baseline));
            path.close_path();
            stroke(scene, &path, 0.9);
        }
        // A spring: the coil.
        RoomFamily::Spring => {
            path.move_to((x, baseline - 1.0));
            for i in 0..4 {
                let sx = crate::num::coord(i).mul_add(2.0, x);
                path.line_to((sx + 1.0, top + 1.0));
                path.line_to((sx + 2.0, baseline - 1.0));
            }
            stroke(scene, &path, 0.8);
        }
        // Ambient: the halo.
        RoomFamily::Ambient => {
            path.extend(vello::kurbo::Circle::new((x + 4.0, top + 3.5), 3.0).to_path(0.1).elements().iter().copied());
            path.extend(vello::kurbo::Circle::new((x + 4.0, top + 3.5), 1.0).to_path(0.1).elements().iter().copied());
            stroke(scene, &path, 0.8);
        }
        // Random: scattered.
        RoomFamily::Random => {
            for (dx, dy) in [(1.0, 1.0), (5.5, 2.5), (3.0, 5.0), (7.0, 6.0), (2.0, 3.5)] {
                path.extend(vello::kurbo::Circle::new((x + dx, top + dy), 0.8).to_path(0.1).elements().iter().copied());
            }
            scene.fill(Fill::NonZero, Affine::IDENTITY, ink, None, &path);
        }
        // Special: the ladder of lamps.
        RoomFamily::Special => {
            path.move_to((x + 1.5, baseline));
            path.line_to((x + 1.5, top));
            path.move_to((x + 6.5, baseline));
            path.line_to((x + 6.5, top));
            for i in 0..3 {
                let ry = crate::num::coord(i).mul_add(2.5, top + 1.0);
                path.move_to((x + 1.5, ry));
                path.line_to((x + 6.5, ry));
            }
            stroke(scene, &path, 0.8);
        }
        // Convolution: the recorded thing.
        RoomFamily::Convolution => {
            path.move_to((x, baseline - 3.0));
            for (i, h) in [7.0, 2.0, 5.0, 3.5, 4.5, 4.0, 3.0].into_iter().enumerate() {
                let sx = x + 1.0 + crate::num::coord(i);
                path.line_to((sx, baseline - h));
                path.line_to((sx + 0.5, (h - 3.0).mul_add(0.5, baseline - 3.0)));
            }
            stroke(scene, &path, 0.7);
        }
    }
}

/// A panel's header: what it is on the left — with the glyph of which
/// machine, where the unit is a family of them — and what it is set
/// to on the right.
///
/// The value is right-aligned so the panels' numbers line up down the
/// rack — which is what lets you compare two strips by running your
/// eye down them rather than reading six numbers.
///
/// The value is dropped rather than elided when the panel is too narrow
/// for both: a truncated "−14d…" is a number you have to open the
/// plugin to check, which is worse than one you know is not shown.
#[expect(clippy::too_many_arguments, reason = "a drawing and everything it needs")]
fn header(
    scene: &mut Scene,
    palette: &Palette,
    font: &Font,
    name: &str,
    mark: Option<Glyph>,
    value: &str,
    bypassed: bool,
    at: Panel,
) {
    const SIZE: f32 = 7.0;
    let baseline = at.y + f64::from(SIZE);
    // The header stays ABOVE the scrim, because it is the one thing on
    // a bypassed panel you still need: which processor this is, and
    // that it is off. Its own ink dims instead.
    let (name_ink, value_ink) = if bypassed {
        (palette.text_faint.multiply_alpha(0.55), palette.text_faint)
    } else {
        (palette.text_faint, palette.text_dim)
    };
    let mut left = at.x;
    if let Some(mark) = mark {
        glyph(scene, name_ink, mark, left, baseline - 0.5);
        left += GLYPH_W;
    }
    crate::tcp::glyphs(scene, font, name_ink, name, left, baseline, SIZE);

    // "BYPASS" replaces the value, because the value is what the
    // processor WOULD do and it is not doing it. Leaving the numbers up
    // would be a panel reporting a setting that is having no effect.
    let value = if bypassed { "BYPASS" } else { value };
    let name_w = font.width(name, SIZE) + (left - at.x);
    let value_w = font.width(value, SIZE);
    if name_w + value_w + 6.0 > at.width {
        return;
    }
    crate::tcp::glyphs(
        scene,
        font,
        value_ink,
        value,
        at.x + at.width - value_w,
        baseline,
        SIZE,
    );
}

/// A colour the plugin states, as a colour this window can paint.
///
/// The plugin's UIs are DOMs, so they name colours as `#rrggbb` —
/// `eq_graph_model::freq_to_color` returns one and
/// `comp_graph_svg::colors` is a table of them. Parsing is how a vello
/// host reads the same values rather than keeping a second table that
/// drifts.
fn hex(value: &str) -> Color {
    let digits = value.trim_start_matches('#');
    let channel = |from: usize| {
        digits
            .get(from..from.saturating_add(2))
            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
    };
    match (channel(0), channel(2), channel(4)) {
        (Some(r), Some(g), Some(b)) => Color::from_rgba8(r, g, b, 0xff),
        // A table that stopped producing hex is a bug in the plugin,
        // not a reason for the rack to draw nothing: grey is visible
        // and obviously not a decision.
        _ => Color::from_rgba8(0x88, 0x88, 0x88, 0xff),
    }
}

/// A band's colour, from the plugin's own frequency map.
///
/// `eq_graph_model::freq_to_color` sweeps the hue red to violet across
/// the audible range and hands back a hex string, because its first
/// consumer was a DOM. Parsing it here is cheaper than a second colour
/// map that would drift from the editor's — and drifting is exactly
/// what must not happen, since the point of the colour is that a band
/// is the same colour in the strip as it is in the plugin window.
fn band_color(hz: f64) -> Color {
    hex(&eq_ui::eq_graph_model::freq_to_color(hz))
}

/// A filled circle — a handle, or a marker on a curve.
fn dot(scene: &mut Scene, color: Color, at: (f64, f64), r: f64) {
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        color,
        None,
        &vello::kurbo::Circle::new(at, r),
    );
}

/// A line of a given width — the threshold, which has to be findable.
fn rule_wide(scene: &mut Scene, color: Color, line: Line, width: f64) {
    scene.stroke(&Stroke::new(width), Affine::IDENTITY, color, None, &line);
}

/// One hairline.
fn rule(scene: &mut Scene, color: Color, line: Line) {
    scene.stroke(
        &Stroke::new(1.0),
        Affine::IDENTITY,
        color,
        None,
        &line,
    );
}

/// An open polyline through `points`.
fn curve(
    scene: &mut Scene,
    color: Color,
    points: impl Iterator<Item = (f64, f64)>,
    width: f64,
) {
    let mut path = BezPath::new();
    for (i, point) in points.enumerate() {
        if i == 0 {
            path.move_to(point);
        } else {
            path.line_to(point);
        }
    }
    if path.is_empty() {
        return;
    }
    scene.stroke(
        &Stroke::new(width).with_caps(vello::kurbo::Cap::Round),
        Affine::IDENTITY,
        color,
        None,
        &path,
    );
}

const fn f64_to_f32(value: f64) -> f32 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::as_conversions,
        reason = "a dB value in a display range of sixty; f32 holds it exactly"
    )]
    let narrowed = value as f32;
    narrowed
}

/// A plausible Tone chain, until tracks carry their own.
///
/// The rack has nowhere to read real settings from yet. Not because the
/// chain is unreachable — `FxChain::parameters` is right there — but
/// because what comes back is unusable: daw-standalone adds an FX entry
/// by name without loading a binary, so its parameters fall through to
/// the stored path and arrive as `Param 1..N` at 0.5. `bin/chain-probe`
/// demonstrates it against a real session.
///
/// A rack fed `Param 3 = 0.5` would draw a curve that looks like
/// information and is not, which is worse than one that is honestly
/// made up. So this stands in, varied by track index so that a mixer
/// full of racks looks like a mixer full of different decisions rather
/// than one curve repeated twenty times.
#[must_use]
pub fn placeholder(index: usize) -> Tone {
    let voice = Character::of(index);
    // A drift within the voice, so two kicks are not one kick. Small
    // enough that they still read as the same decision made twice.
    let drift = crate::num::coord(index % 5) - 2.0;
    let mut tone = Tone {
        // Rescue is the same surgery on every track, roughly: a
        // high-pass and one cut. It is the pass you make before you
        // know what the track is going to be.
        rescue_eq: vec![
            band(0, 30.0 + drift * 4.0, -18.0, 0.7, EqBandShape::LowCut),
            band(1, 240.0 * 1.1_f64.powf(drift), -3.5, 2.4, EqBandShape::Bell),
        ],
        gate: Gate {
            threshold: f64_to_f32(-42.0 + drift * 3.0),
            ..Gate::default()
        },
        rescue_comp: Comp {
            threshold: f64_to_f32(-8.0 + drift),
            ratio: 2.2,
            ..voice.comp(drift)
        },
        eq: voice.bands(drift),
        comp: voice.comp(drift),
        de_ess: Suppress::sibilance(),
        resonance: Suppress::broadband(),
        // The relational pass carves rather than shapes: one wide dip
        // where something else lives.
        space: vec![band(
            0,
            900.0 * 1.35_f64.powf(drift),
            -2.5,
            0.9,
            EqBandShape::Bell,
        )],
        delay: Echo {
            time: f64_to_f32(180.0 + 60.0 * (drift + 2.0)),
            style: voice.delay_style(),
            ..Echo::default()
        },
        reverb: Room {
            decay: f64_to_f32(1.1 + 0.35 * (drift + 2.0)),
            algorithm: voice.reverb(),
            ..Room::default()
        },
        // Not all one zoom: a vocal worked at ±3 and a room mic at ±18
        // is the normal case, and a desk where every graph is drawn to
        // the same range hides the fact that the range is a choice.
        eq_range: match index % 3 {
            0 => DEFAULT_EQ_RANGE,
            1 => DEFAULT_EQ_RANGE.saturating_sub(1),
            _ => DEFAULT_EQ_RANGE.saturating_add(1),
        },
        sat: {
            let mut pre = ClassAPreamp::new(f64_to_f32(DISPLAY_RATE));
            // A single-ended stage: a triode grid above and iron below,
            // biased off centre. `Clean` is wire, and wire into the
            // output clamp draws a hard clipper's flat-ramp-flat — a
            // picture of a limiter, not of saturation. Replaced by the
            // voice's profile below, through the plugin's own `apply`.
            pre.positive = SideShaper::Tube;
            pre.negative = SideShaper::Transformer;
            pre.drive = f64_to_f32(voice.drive() + drift * 0.3);
            pre.q_point = 0.25;
            pre
        },
        sat_profile: 0,
        sat_digital: saturate_dsp::digital::DigitalStage::new(),
        pre_eq: Vec::new(),
        post_eq: Vec::new(),
        decay_eq: Vec::new(),
        wide: 1.0,
        pitch: 0,
        pitch_mix: 0.5,
        role: Role::Channel,
        presets: Vec::new(),
        preset: None,
        bypass: Bypass::default(),
    };
    // Each voice on its own circuit, so a mixer of racks shows the five
    // faces; the drive the voice wants survives the profile.
    let drive = tone.sat.drive;
    tone.set_sat_profile(saturate_profiles::profile_index(voice.sat_profile()).unwrap_or(0));
    tone.sat.drive = drive;
    tone
}

/// A placeholder for a track of a given role.
///
/// An FX return carries its presets — brighter to darker — and opens on
/// the one its name asks for, so a "Long" verb is a long verb before
/// anything is clicked.
#[must_use]
pub fn placeholder_for(role: Role, index: usize, name: &str) -> Tone {
    let mut tone = placeholder(index);
    tone.role = role;
    let presets = match role {
        Role::Reverb => reverb_presets(),
        Role::Delay => delay_presets(),
        Role::Wide => wide_presets(),
        Role::Pitch => pitch_presets(),
        Role::Channel | Role::Bus => Vec::new(),
    };
    if presets.is_empty() {
        return tone;
    }
    // Open on the preset the track is named after, else the first.
    let lower = name.to_lowercase();
    let wanted = presets
        .iter()
        .position(|p| lower.contains(&p.name.to_lowercase()))
        .unwrap_or(0);
    tone.presets = presets;
    tone.load_preset(wanted);
    tone
}

fn preset(name: &str, mut tone: Tone) -> Preset {
    tone.presets = Vec::new();
    tone.preset = None;
    Preset {
        name: name.to_owned(),
        tone: Box::new(tone),
    }
}

/// A high shelf at 6 kHz, which is most of what "darker" means.
fn shelf(gain: f64) -> EqBand {
    band(0, 6_000.0, gain, 0.7, EqBandShape::HighShelf)
}

/// The reverb presets: brighter and shorter to darker and longer, the
/// way a template's rows run.
fn reverb_presets() -> Vec<Preset> {
    let base = || {
        let mut t = placeholder(0);
        t.role = Role::Reverb;
        t.pre_eq = vec![band(0, 180.0, -18.0, 0.7, EqBandShape::LowCut)];
        t
    };
    let mut room = base();
    room.reverb = Room {
        algorithm: AlgorithmType::Room,
        decay: 0.6,
        predelay: 8.0,
        damping: 0.15,
        size: 0.35,
        diffusion: 0.6,
        mix: 0.2,
    };
    room.post_eq = vec![shelf(1.5)];
    let mut short = base();
    short.reverb = Room {
        algorithm: AlgorithmType::Plate,
        decay: 1.4,
        predelay: 20.0,
        damping: 0.3,
        size: 0.5,
        diffusion: 0.8,
        mix: 0.2,
    };
    short.post_eq = vec![shelf(-1.5)];
    let mut long = base();
    long.reverb = Room {
        algorithm: AlgorithmType::Hall,
        decay: 2.6,
        predelay: 40.0,
        damping: 0.5,
        size: 0.7,
        diffusion: 0.8,
        mix: 0.18,
    };
    long.post_eq = vec![shelf(-4.0)];
    long.decay_eq = vec![band(0, 300.0, -6.0, 0.7, EqBandShape::LowShelf), band(1, 5_000.0, -6.0, 0.7, EqBandShape::HighShelf)];
    let mut moment = base();
    moment.reverb = Room {
        algorithm: AlgorithmType::Cloud,
        decay: 5.5,
        predelay: 60.0,
        damping: 0.65,
        size: 0.9,
        diffusion: 0.9,
        mix: 0.25,
    };
    moment.post_eq = vec![shelf(-7.0)];
    moment.decay_eq = vec![band(0, 250.0, -9.0, 0.7, EqBandShape::LowShelf), band(1, 4_000.0, -9.0, 0.7, EqBandShape::HighShelf)];
    let mut throw = base();
    throw.reverb = Room {
        algorithm: AlgorithmType::Shimmer,
        decay: 8.0,
        predelay: 90.0,
        damping: 0.7,
        size: 1.0,
        diffusion: 1.0,
        mix: 0.35,
    };
    throw.post_eq = vec![shelf(-9.0), band(1, 2_500.0, 3.0, 1.2, EqBandShape::Bell)];
    throw.decay_eq = vec![band(0, 2_000.0, 6.0, 1.0, EqBandShape::Bell)];
    vec![
        preset("Room", room),
        preset("Short", short),
        preset("Long", long),
        preset("Moment", moment),
        preset("Throw", throw),
    ]
}

/// The delay presets, brighter and tighter to darker and wider.
fn delay_presets() -> Vec<Preset> {
    let base = || {
        let mut t = placeholder(1);
        t.role = Role::Delay;
        t
    };
    let mut slap = base();
    slap.delay = Echo { time: 95.0, feedback: 0.08, mix: 0.3, tone: 0.3, width: 0.2, style: DelayStyle::Tape };
    slap.post_eq = vec![shelf(-1.0)];
    let mut short = base();
    short.delay = Echo { time: 187.0, feedback: 0.25, mix: 0.25, tone: 0.15, width: 0.5, style: DelayStyle::Clean };
    short.post_eq = vec![band(0, 200.0, -18.0, 0.7, EqBandShape::LowCut)];
    let mut long = base();
    long.delay = Echo { time: 375.0, feedback: 0.42, mix: 0.22, tone: 0.55, width: 0.85, style: DelayStyle::Tape };
    long.post_eq = vec![band(0, 250.0, -18.0, 0.7, EqBandShape::LowCut), shelf(-5.0)];
    let mut throw = base();
    throw.delay = Echo { time: 750.0, feedback: 0.55, mix: 0.4, tone: 0.7, width: 1.0, style: DelayStyle::Bbd };
    throw.post_eq = vec![band(0, 400.0, -18.0, 0.7, EqBandShape::LowCut), band(1, 1_000.0, 6.0, 2.0, EqBandShape::Bell), shelf(-9.0)];
    vec![
        preset("Slap", slap),
        preset("Short", short),
        preset("Long", long),
        preset("Throw", throw),
    ]
}

fn wide_presets() -> Vec<Preset> {
    let mut subtle = placeholder(2);
    subtle.role = Role::Wide;
    subtle.wide = 1.25;
    let mut wide = placeholder(2);
    wide.role = Role::Wide;
    wide.wide = 1.7;
    vec![preset("Subtle", subtle), preset("Wide", wide)]
}

fn pitch_presets() -> Vec<Preset> {
    let mut up = placeholder(3);
    up.role = Role::Pitch;
    up.pitch = 12;
    up.pitch_mix = 0.35;
    let mut down = placeholder(3);
    down.role = Role::Pitch;
    down.pitch = -12;
    down.pitch_mix = 0.4;
    vec![preset("Oct+", up), preset("Oct-", down)]
}

/// What a track sounds like, as far as a placeholder can know.
///
/// Taken from the track's INDEX rather than its name, for the same
/// reason [`crate::simulate`] does: it is stable across a rename and
/// works on a session whose tracks are called things this file has
/// never heard of. The point is that a rack of racks reads as a set of
/// decisions — a kick cut at 400 and lifted at 60, a cymbal rolled off
/// at the bottom and opened at the top — rather than one curve
/// repeated with a wobble in it.
///
/// It is still a placeholder. When a chain can be read, this is the
/// shape the real settings arrive in; until then it is what makes the
/// panel worth looking at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Character {
    /// Kicks, floor toms, bass — weight at the bottom and a box to cut.
    Low,
    /// Snares and racks — body, and a crack to find above it.
    Mid,
    /// Cymbals, hats, air — nothing useful below, everything above.
    High,
    /// Rooms and buses — gentle, wide, barely there.
    Broad,
}

impl Character {
    /// Which voice a track gets.
    ///
    /// Hashed rather than `track % 4`, because a period of four lines
    /// up with any regular stride — and the mixer has one: at a width
    /// where the rack is drawn, every eighth strip is on screen, every
    /// one of them ≡ 0 mod 4, and a whole desk of placeholder settings
    /// came out identical.
    const fn of(track: usize) -> Self {
        let mixed = (track ^ (track >> 2)).wrapping_mul(2_654_435_761);
        match mixed % 4 {
            0 => Self::Low,
            1 => Self::Mid,
            2 => Self::High,
            _ => Self::Broad,
        }
    }

    /// The curve this voice usually wants.
    fn bands(self, drift: f64) -> Vec<EqBand> {
        let nudge = |hz: f64| hz * 1.06_f64.powf(drift);
        match self {
            Self::Low => vec![
                band(0, nudge(55.0), 4.0, 0.8, EqBandShape::LowShelf),
                band(1, nudge(380.0), -5.5, 1.6, EqBandShape::Bell),
                band(2, nudge(3_200.0), 3.0, 1.0, EqBandShape::Bell),
                band(3, nudge(9_000.0), -2.0, 0.7, EqBandShape::HighShelf),
            ],
            Self::Mid => vec![
                band(0, nudge(90.0), -4.0, 0.7, EqBandShape::LowShelf),
                band(1, nudge(220.0), 2.5, 1.2, EqBandShape::Bell),
                band(2, nudge(1_100.0), -3.0, 2.0, EqBandShape::Bell),
                band(3, nudge(6_500.0), 4.5, 0.8, EqBandShape::HighShelf),
            ],
            Self::High => vec![
                band(0, nudge(300.0), -7.0, 0.6, EqBandShape::LowShelf),
                band(1, nudge(900.0), -2.5, 1.4, EqBandShape::Bell),
                band(2, nudge(5_000.0), 1.5, 1.1, EqBandShape::Bell),
                band(3, nudge(12_000.0), 4.0, 0.7, EqBandShape::HighShelf),
            ],
            Self::Broad => vec![
                band(0, nudge(70.0), -2.0, 0.7, EqBandShape::LowShelf),
                band(1, nudge(500.0), -1.5, 0.9, EqBandShape::Bell),
                band(2, nudge(2_500.0), 1.0, 0.8, EqBandShape::Bell),
                band(3, nudge(11_000.0), 2.0, 0.7, EqBandShape::HighShelf),
            ],
        }
    }

    /// And the compression. The times are the decision: a kick wants
    /// the transient through and a room wants none of it.
    fn comp(self, drift: f64) -> Comp {
        let (threshold, ratio, attack, release) = match self {
            Self::Low => (-12.0, 4.0, 12.0, 140.0),
            Self::Mid => (-16.0, 5.0, 4.0, 90.0),
            Self::High => (-20.0, 2.5, 1.0, 200.0),
            Self::Broad => (-24.0, 2.0, 25.0, 400.0),
        };
        Comp {
            threshold: f64_to_f32(threshold + drift),
            ratio: f64_to_f32((ratio + drift * 0.3).clamp(1.0, 20.0)),
            knee: 6.0,
            attack: f64_to_f32((attack * 1.15_f64.powf(drift)).clamp(0.1, 200.0)),
            release: f64_to_f32((release * 1.12_f64.powf(drift)).clamp(5.0, 3_000.0)),
        }
    }

    /// Which saturator this voice runs through, by profile id.
    const fn sat_profile(self) -> &'static str {
        match self {
            Self::Low => "transformer",
            Self::Mid => "triode",
            Self::High => "tape",
            Self::Broad => "transistor",
        }
    }

    /// Which delay machine this voice would be sent to. Varied so a
    /// mixer of racks shows the family faces rather than one repeated.
    const fn delay_style(self) -> DelayStyle {
        match self {
            Self::Low => DelayStyle::Clean,
            Self::Mid => DelayStyle::Tape,
            Self::High => DelayStyle::Bbd,
            Self::Broad => DelayStyle::Shimmer,
        }
    }

    /// And which space.
    const fn reverb(self) -> AlgorithmType {
        match self {
            Self::Low => AlgorithmType::Room,
            Self::Mid => AlgorithmType::Plate,
            Self::High => AlgorithmType::Hall,
            Self::Broad => AlgorithmType::Cloud,
        }
    }

    const fn drive(self) -> f64 {
        match self {
            Self::Low => 3.4,
            Self::Mid => 2.6,
            Self::High => 1.6,
            Self::Broad => 2.0,
        }
    }
}

/// Something in the rack you can take hold of.
///
/// Not a control in the strip's sense: these live inside a curve's own
/// axes, so what a drag means depends on which curve it started in. A
/// band moves in frequency AND gain at once, which is the gesture an
/// EQ is actually used with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Grip {
    /// An EQ band: which EQ, and its index in that one's list.
    ///
    /// The chain has three EQs now — rescue, tone and relational — so a
    /// grip that named only an index would move whichever one the
    /// dispatch happened to find first.
    Band(Which, usize),
    /// A threshold — the red line across a display, dragged up and down
    /// the level axis it is drawn on. Both compressors have one, so
    /// does the gate, and so do the two suppressors.
    Threshold(Which),
    /// A compressor's ratio.
    Ratio(Which),
    /// Its attack.
    Attack(Which),
    /// And its release.
    Release(Which),
    /// The gate's hold — the flat top of its table.
    Hold(Which),
    /// The gate's range — the fainter line under its threshold, how far
    /// down what stays shut goes.
    Range,
    /// The saturator's drive.
    Drive,
    /// Its bias — the operating point, dragged sideways so the curve
    /// leans.
    Bias,
    /// Its tilt — which end of the spectrum meets the knee first, the
    /// wedge under the ladder.
    Tilt,
    /// A suppressor's depth — the ribbon, pulled down for more.
    Depth(Which),
    /// A suppressor's sharpness — how wide the reference it compares
    /// against is.
    Sharpness(Which),
    /// One edge of a suppressor's band.
    Edge(Which, Side),
    /// The delay's time — the first repeat, dragged sideways.
    Time,
    /// Its feedback — any later repeat, dragged up and down.
    Feedback,
    /// The reverb's decay — the tail's reach.
    Decay,
    /// Its predelay — the gap rule between the impulse and the tail.
    Predelay,
    /// A wet/dry mix: the delay's, the reverb's, the saturator's.
    Mix(Which),
    /// One chip of a unit's machine selector — clicked to choose that
    /// family. A switch, like the glyph.
    Choose(Which, usize),
    /// One of the track's presets — clicked to load it. A switch.
    Preset(usize),
    /// One knob of a unit's knob row (or the widener's and the pitch
    /// shifter's single figures), dragged up and down.
    Knob(Which, usize),
    /// A unit's machine glyph — clicked to cycle to the next style or
    /// algorithm. A switch, like a bypass: it acts on the click.
    Family(Which),
    /// A panel's header — clicked to switch that processor out.
    ///
    /// The header, because it is the one strip of a panel that is not
    /// a control: everything else in there sets a value, and a bypass
    /// is not a value. It is also the part that stays legible under the
    /// scrim, so the way out is where the way in was.
    Bypass(Which),
    /// A phase's container bar — clicked to fold it shut.
    ///
    /// Not a `Bypass`: folding changes what you can SEE and bypassing
    /// changes what the track sounds like, and a rack where the two
    /// gestures looked alike would be one click away from an unintended
    /// mix decision.
    Phase(session::mix_phases::MixPhase),
    /// An EQ graph's zoom — the chip at the top of the panel saying
    /// what range it is drawn to.
    ///
    /// A zoom is not a parameter: it changes nothing about the sound,
    /// only how much of it you can see. It is here because it is a
    /// thing in the panel you point at, and everything in a panel you
    /// point at has to be something the one hit test can name.
    Scale(Which),
}

impl Grip {
    /// Whether this grip is one of the compressor's knobs.
    ///
    /// They share a drag law — a knob is a knob — and differ only in
    /// the range they map onto, which is the one thing each has to say
    /// for itself.
    #[must_use]
    pub const fn is_knob(self) -> bool {
        matches!(self, Self::Ratio(_) | Self::Attack(_) | Self::Release(_))
    }

    /// Whether this grip is a switch rather than a value.
    ///
    /// A switch acts on the CLICK and has no drag; a value does the
    /// opposite. The caller needs to know which before it decides
    /// whether a press is the start of a gesture.
    #[must_use]
    pub const fn is_switch(self) -> bool {
        matches!(
            self,
            Self::Bypass(_)
                | Self::Scale(_)
                | Self::Phase(_)
                | Self::Family(_)
                | Self::Choose(..)
                | Self::Preset(_)
        )
    }

    /// Which panel this grip lives in.
    #[must_use]
    pub const fn panel(self) -> Which {
        match self {
            Self::Band(which, _)
            | Self::Threshold(which)
            | Self::Ratio(which)
            | Self::Attack(which)
            | Self::Release(which)
            | Self::Hold(which)
            | Self::Depth(which)
            | Self::Sharpness(which)
            | Self::Edge(which, _)
            | Self::Mix(which)
            | Self::Family(which)
            | Self::Choose(which, _)
            | Self::Knob(which, _)
            | Self::Scale(which)
            | Self::Bypass(which) => which,
            Self::Drive | Self::Bias | Self::Tilt => Which::Sat,
            Self::Range => Which::Gate,
            Self::Preset(_) => Which::Presets,
            Self::Time | Self::Feedback => Which::Delay,
            Self::Decay | Self::Predelay => Which::Reverb,
            // A header belongs to no unit: it caps a run of them.
            Self::Phase(_) => Which::Eq,
        }
    }
}

/// How close a pointer has to be to take hold of something.
///
/// Generous next to [`HANDLE`], because a handle is drawn at the size
/// it reads best and grabbed at the size a hand can hit. Six pixels is
/// about a millimetre at these densities.
const GRAB: f64 = 6.0;

/// A box in panel coordinates.
///
/// Small enough not to want `kurbo::Rect`'s arithmetic, and stated here
/// so the chip's drawing and its hit test are the same four numbers.
#[derive(Clone, Copy, Debug)]
pub struct Chip {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Chip {
    #[must_use]
    pub fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// Where the EQ's zoom readout sits: centred at the top of the graph.
///
/// The top MIDDLE because the corners are taken — the header's name is
/// at one end of the panel and its summary at the other — and because
/// the middle of the top edge is the one part of a response curve that
/// is reliably empty: bands live out along the frequency axis and the
/// curve itself runs through the middle height.
#[must_use]
pub fn scale_chip(body: Panel) -> Chip {
    const WIDTH: f64 = 30.0;
    const HEIGHT: f64 = 12.0;
    Chip {
        x: body.x + (body.width - WIDTH) / 2.0,
        y: body.y + 1.0,
        width: WIDTH.min(body.width),
        height: HEIGHT.min(body.height),
    }
}

/// The EQ panel's axes, as the plugin's own interaction model wants
/// them.
///
/// `GraphMapper` takes ONE padding for both axes because a plugin's
/// graph is a box inside a window; a rack panel is a box at an
/// arbitrary offset inside a strip. So the mapper is built at the
/// origin and the panel's corner is added back — which keeps every
/// conversion the plugin's, and leaves this the only place the two
/// coordinate systems meet.
fn mapper(body: Panel, db_range: f64) -> GraphMapper {
    GraphMapper::new(20.0, 20_000.0, db_range, body.width, body.height, 0.0)
}

/// What is under a point in the rack, if anything.
///
/// `panel` is the rack's whole box in the same coordinates as `x` and
/// `y` — the strip's, not the window's.
#[must_use]
#[expect(clippy::too_many_arguments, reason = "a hit test and everything it reads")]
pub fn grip_at(
    panels: &[Which],
    tone: &Tone,
    panel: Panel,
    folded: Folded,
    x: f64,
    y: f64,
) -> Option<Grip> {
    let rack = Rack::at(panel.width);
    if !rack.on() {
        return None;
    }
    for (row, at) in chain(panels, panel, folded) {
        // A container bar answers for its whole width: it is fifteen
        // pixels tall and the chevron in it is six, and a fold you have
        // to hit exactly is a fold nobody uses.
        let Row::Unit(which) = row else {
            let Row::Head(phase) = row else { continue };
            if y >= at.y && y < at.y + at.height {
                return Some(Grip::Phase(phase));
            }
            continue;
        };
        let body = body_of(at, rack);
        if body.width <= 0.0 || body.height <= 0.0 {
            continue;
        }
        // The preset row has no header and no body: it is chips.
        if which == Which::Presets {
            if y < at.y || y >= at.y + at.height {
                continue;
            }
            return preset_chip_at(tone, at, x).map(Grip::Preset);
        }
        // The header first: it sits above the body, and a click there
        // is a bypass rather than whatever the body would have done —
        // except on the machine glyph, which cycles the machine.
        if rack.detailed() && y >= at.y && y < body.y {
            let inner = at.inset(2.0);
            if which.glyph_switches() && x >= inner.x && x < inner.x + GLYPH_W {
                return Some(Grip::Family(which));
            }
            return Some(Grip::Bypass(which));
        }
        if y < body.y || y > body.y + body.height {
            continue;
        }
        // A bypassed panel has one control left, and it is the one that
        // brings it back. Grabbing a band you cannot see through the
        // scrim would move a setting with no effect.
        if tone.bypass.is(which) {
            return Some(Grip::Bypass(which));
        }
        // Routed by what the panel IS, not by which one it is: three
        // EQs share a hit test and two compressors share another, and
        // the grip carries the panel so the drag knows which.
        match which {
            // Bands are only grabbable where they are DRAWN, which is
            // only at `Full` — a handle you cannot see is a handle you
            // cannot aim at, and grabbing one by accident moves a
            // setting you did not know was there.
            _ if which.is_spectral() && !rack.detailed() => {}
            Which::RescueEq | Which::Eq | Which::Space | Which::PreEq | Which::PostEq | Which::DecayEq => {
                // The zoom chip first: it is small, it sits over the
                // graph, and a band that happened to be under it would
                // otherwise take every click aimed at it.
                if scale_chip(body).contains(x, y) {
                    return Some(Grip::Scale(which));
                }
                // The plugin's own hit test, not a second one written
                // here: `nearest_band` already decides which of four
                // overlapping bands you meant, and a rack that decided
                // differently from the editor would be two EQs.
                let bands = tone.bands_ref(which);
                if let Some((index, _)) = interaction::nearest_band(
                    bands,
                    mapper(body, tone.eq_db_range()),
                    x - body.x,
                    y - body.y,
                    GRAB,
                ) {
                    return Some(Grip::Band(which, index));
                }
                // And nothing else. The empty graph used to answer for
                // the zoom so a wheel anywhere over it would zoom —
                // but the rack scrolls now, and a panel that swallowed
                // the wheel would be a hole in the scroll the size of
                // an EQ. The chip is the zoom's target; the rest of the
                // graph belongs to the gesture that moves the chain.
            }
            // A suppressor: the band's edges where they are drawn, then
            // the display in three bands of its own — the reference up
            // top is the threshold, the ribbon through the middle is
            // the depth, the floor is the sharpness. Zones rather than
            // curves, because the curves move with the audio and a
            // grip that moved with the audio would be a grip you could
            // not aim at.
            Which::DeEss | Which::DeEssIn | Which::Resonance => {
                return Some(suppress_grip(tone, which, body, rack, x, y));
            }
            Which::Comp => return Some(comp_grip(tone.comp, which, body, rack, x, y)),
            Which::RescueComp => {
                return Some(comp_grip(tone.rescue_comp, which, body, rack, x, y));
            }
            Which::Gate => return Some(gate_grip(tone.gate, body, rack, x, y)),
            Which::Knobs => return Some(Grip::Knob(which, knob_at(body, x))),
            Which::Wide => return Some(Grip::Knob(which, 0)),
            // The interval on the top half, the mix on the bottom.
            Which::Pitch => {
                let index = usize::from(y >= body.y + body.height / 2.0);
                return Some(Grip::Knob(which, index));
            }
            Which::Presets => {}
            // The ladder is the tilt; the curve's centre is the bias;
            // the rest of the curve is the drive.
            Which::Sat => {
                if lane_of(body, which, rack).is_some_and(|strip| strip.contains(x, y)) {
                    return Some(selector_grip(which, body, rack, x));
                }
                let (curve_box, ladder_box) = sat_split(display_of(body, which, rack), rack);
                if ladder_box.is_some_and(|lb| lb.contains(x, y)) {
                    return Some(Grip::Tilt);
                }
                let mid_x = curve_box.x + curve_box.width / 2.0;
                if rack.detailed() && (x - mid_x).abs() <= GRAB * 1.5 {
                    return Some(Grip::Bias);
                }
                return Some(Grip::Drive);
            }
            // The first repeat is the time; any later one is the
            // feedback. Elsewhere on the panel, the time — it is the
            // parameter a delay IS.
            Which::Delay | Which::Reverb
                if lane_of(body, which, rack).is_some_and(|strip| strip.contains(x, y)) =>
            {
                return Some(selector_grip(which, body, rack, x));
            }
            Which::Delay => {
                let body = display_of(body, which, rack);
                let taps = echo_taps(tone.delay, body);
                let nearest = taps
                    .iter()
                    .enumerate()
                    .map(|(k, (tx, _))| (k, (x - tx).abs()))
                    .filter(|(_, away)| *away <= GRAB)
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
                return Some(match nearest {
                    Some((k, _)) if k >= 2 => Grip::Feedback,
                    _ => Grip::Time,
                });
            }
            // The gap rule is the predelay, the tail's end is the decay,
            // and the tail's height is the mix.
            Which::Reverb => {
                let body = display_of(body, which, rack);
                let geometry = RoomGeometry::of(tone.reverb, body);
                if rack.detailed() && (x - geometry.start).abs() <= GRAB {
                    return Some(Grip::Predelay);
                }
                if x >= body.width.mul_add(0.6, body.x) {
                    return Some(Grip::Decay);
                }
                return Some(Grip::Mix(which));
            }
        }
    }
    None
}

/// What is under a point in a machine selector strip: the chip, or the
/// bypass past the chips (the strip is part of the panel, and the
/// panel's empty space is the bypass everywhere else).
fn selector_grip(which: Which, body: Panel, rack: Rack, x: f64) -> Grip {
    let strip = lane_of(body, which, rack).unwrap_or(body);
    let chips = match which {
        Which::Sat => CIRCUITS.len(),
        Which::Delay => DelayFamily::ALL.len(),
        _ => RoomFamily::ALL.len(),
    };
    let index = crate::num::index(((x - strip.x - 2.0) / CHIP).floor());
    if index < chips && x >= strip.x + 2.0 {
        Grip::Choose(which, index)
    } else {
        Grip::Bypass(which)
    }
}

/// What is under a point in the gate's panel.
///
/// The glyph's edges first, then the range line, then the threshold —
/// which is everything else, the way a fader's groove is a fader's.
fn gate_grip(gate: Gate, body: Panel, rack: Rack, x: f64, y: f64) -> Grip {
    let display = display_of(body, Which::Gate, rack);
    if !rack.detailed() {
        return Grip::Threshold(Which::Gate);
    }
    let to_y = |db: f64| display.y + comp_ui::comp_graph_svg::db_to_y(db, display.height);
    let bottom = display.y + display.height;
    let line = to_y(f64::from(gate.threshold)).clamp(display.y, bottom);
    let floor = to_y(f64::from(gate.threshold + gate.range)).clamp(display.y, bottom);
    if let Some(shape) = GateGlyph::of(gate, display, line, floor)
        && let Some(grip) = shape.grip_at(x, y)
    {
        return grip;
    }
    // The range line, only where it is clear of the threshold's — two
    // lines a pixel apart are one line, and that one is the threshold.
    if (y - floor).abs() <= GRAB && floor - line > GRAB * 2.0 {
        return Grip::Range;
    }
    Grip::Threshold(Which::Gate)
}

/// What is under a point in a suppressor's panel.
fn suppress_grip(tone: &Tone, which: Which, body: Panel, rack: Rack, x: f64, y: f64) -> Grip {
    let set = match which {
        Which::DeEss | Which::DeEssIn => tone.de_ess,
        _ => tone.resonance,
    };
    let display = display_of(body, which, rack);
    if rack.detailed() {
        let zoom = SuppressZoom::of(set);
        for (hz, side) in [(f64::from(set.low), Side::Low), (f64::from(set.high), Side::High)] {
            if (x - zoom.x_of(hz, display)).abs() <= GRAB {
                return Grip::Edge(which, side);
            }
        }
        // The lane under the display: the resonance panel's comb is
        // the sharpness (how narrow a peak has to be to earn a tooth),
        // the de-esser's fire lane is the depth (how hard it fires).
        if let Some(lane) = lane_of(body, which, rack)
            && y >= lane.y
        {
            return if which == Which::Resonance {
                Grip::Sharpness(which)
            } else {
                Grip::Depth(which)
            };
        }
    }
    let third = display.height / 3.0;
    if y < display.y + third {
        Grip::Threshold(which)
    } else if y < display.y + third * 2.0 {
        Grip::Depth(which)
    } else {
        Grip::Sharpness(which)
    }
}

/// The compressor's display — the whole panel.
///
/// Kept as a function because everything that reads this panel has to
/// agree about it: the drawing, the hit test, the drag's
/// pixels-to-decibels, and the level trace that lands on its axis. A
/// threshold that moved faster than the line under it is a line that is
/// not under your finger.
///
/// It used to return three columns, with the attack and release as
/// ramps in narrow gutters either side. That cost a third of the
/// panel's width, drew time on an axis nobody reads an envelope on, and
/// put the two controls away from the thing they act on — while the
/// display beside them was already showing a reduction over time, which
/// is exactly what they shape. See `envelope`.
#[must_use]
pub const fn comp_split(body: Panel, _rack: Rack) -> Panel {
    body
}

/// How far down the display the ratio's arrow hangs, in pixels.
///
/// The reduction a full-scale signal would take, on the display's own
/// dB axis — which is what the ratio DOES, rather than a length chosen
/// to look proportional. It saturates as the ratio climbs, because the
/// effect does: past about eight to one a harder ratio takes very
/// little more off, and an arrow that kept growing would be claiming
/// otherwise. The number is in the header for the cases where that
/// distinction matters.
#[must_use]
pub fn ratio_drop(comp: Comp, height: f64) -> f64 {
    ratio_reduction(comp) / 60.0 * height
}

/// The same, in decibels — what the ratio takes off a full-scale
/// signal.
///
/// The quantity the glyph's floor sits at, and the one a drag on it
/// moves. Stated on its own because the drag has to invert it.
#[must_use]
pub fn ratio_reduction(comp: Comp) -> f64 {
    let threshold = f64::from(comp.threshold);
    let ratio = f64::from(comp.ratio).max(1.0);
    -threshold * (1.0 - 1.0 / ratio)
}

/// What is under a point in the compressor's panel.
///
/// The envelope's parts first, because they are lines inside the panel
/// the threshold otherwise owns. Everything else is the threshold: it
/// is a line across a display, and a line one pixel tall is not
/// something you aim at — the whole display is its target, the way a
/// fader's groove is a fader's.
fn comp_grip(comp: Comp, which: Which, body: Panel, rack: Rack, x: f64, y: f64) -> Grip {
    let display = comp_split(body, rack);
    // The envelope's parts first — they are lines inside the display,
    // and the display would otherwise swallow them.
    if rack.detailed()
        && let Some(shape) = Envelope::of(comp, display, threshold_y(comp, display))
        && let Some(grip) = shape.grip_at(x, y)
    {
        // The envelope names the compressor it was measured on, not the
        // one this panel is.
        return match grip {
            Grip::Attack(_) => Grip::Attack(which),
            Grip::Release(_) => Grip::Release(which),
            _ => Grip::Ratio(which),
        };
    }
    // And everything else is the threshold: it is a line across a
    // display, and a line one pixel tall is not something you aim at —
    // the whole display is its target, the way a fader's groove is a
    // fader's.
    Grip::Threshold(which)
}

/// How far a band's gain can go, in decibels.
///
/// The PARAMETER's limit, not the display's — those parted company when
/// the graph learned to zoom. The plugin's gain runs −30..30, and a
/// clamp tied to whatever the graph happened to be showing would mean
/// zooming in silently narrowed what the EQ could do.
pub const EQ_GAIN_LIMIT: f64 = 30.0;

/// Turn the wheel over a grip.
///
/// The EQ's is the plugin's own: unmodified scroll follows the band —
/// a cut filter's meaningful width is its slope, everything else's is
/// Q — and the modifier layers gain, dynamic range, or both on top.
/// None of that is decided here; [`interaction::wheel_target`] decides
/// it, and the rack calls it so that a wheel over a strip and a wheel
/// over the editor do the same thing.
pub fn wheel(tone: &mut Tone, grip: Grip, mods: Mods, delta_y: f64) {
    match grip {
        Grip::Band(which, index) => {
            let Some(band) = tone.bands(which).and_then(|set| set.get_mut(index)) else {
                return;
            };
            let uses_slope = band.shape.uses_slope();
            match interaction::wheel_target(mods, uses_slope) {
                interaction::WheelTarget::Q | interaction::WheelTarget::Slope => {
                    interaction::wheel_band(
                        band,
                        delta_y,
                        uses_slope,
                        interaction::fine_scale(mods),
                    );
                }
                // Dynamic range is not in this rack's model yet — the
                // band type carries it, the curve does not draw it —
                // so a chord that asks for it moves the gain it shares
                // an axis with rather than doing nothing.
                interaction::WheelTarget::Gain
                | interaction::WheelTarget::DynRange
                | interaction::WheelTarget::GainAndRange => {
                    let step = interaction::gain_step(delta_y, mods);
                    let next = f64::from(band.gain) + step;
                    band.gain = interaction::drag_gain_for_shape(
                        band.shape,
                        band.gain,
                        next.clamp(-EQ_GAIN_LIMIT, EQ_GAIN_LIMIT),
                    );
                }
            }
        }
        // A notch of threshold is a dB, and a notch of drive is a
        // tenth — the same relation the two drags have.
        Grip::Threshold(which) => {
            let step = interaction::gain_step(delta_y, mods);
            if let Some((now, _, _)) = tone.threshold(which) {
                tone.set_threshold(which, f64_to_f32(f64::from(now) + step));
            }
        }
        // A switch does not turn, and neither does a container.
        Grip::Bypass(_) | Grip::Phase(_) | Grip::Family(_) | Grip::Choose(..) => {}
        // A notch is a stop, not a fraction of one: the range is a list
        // the plugin publishes and the wheel walks it. Down is further
        // out, which is the direction a wheel zooms out everywhere
        // else.
        Grip::Scale(_) => {
            tone.zoom_eq(if delta_y < 0.0 { 1 } else { -1 });
        }
        // A notch of a knob is a fortieth of its travel, which is about
        // the resolution a hand expects from one — fine enough to place
        // a 3:1 exactly, coarse enough to cross the range.
        Grip::Ratio(which) | Grip::Attack(which) | Grip::Release(which) => {
            let step = interaction::gain_step(delta_y, mods) / 40.0;
            if let Some(comp) = tone.compressor(which) {
                let to = knob_norm(*comp, grip) + step;
                set_knob(comp, grip, to);
            }
        }
        Grip::Drive => {
            let step = interaction::gain_step(delta_y, mods) * 0.1;
            let moved = f64::from(tone.sat.drive) + step;
            tone.sat.drive = f64_to_f32(moved.clamp(0.0, 10.0));
        }
        _ => wheel_more(tone, grip, mods, delta_y),
    }
}

/// The wheel over the grips the newer faces added.
///
/// Split from [`wheel`] by size alone: the law is the same — a notch
/// is about a fiftieth of a control's travel, or a twentieth of a
/// time — and each arm says what its notch is.
fn wheel_more(tone: &mut Tone, grip: Grip, mods: Mods, delta_y: f64) {
    match grip {
        // A notch of bias is a fiftieth of its travel; of tilt, half a
        // decibel — the resolution a hand expects from each.
        Grip::Bias => {
            let step = interaction::gain_step(delta_y, mods) * 0.02;
            tone.sat.q_point = f64_to_f32((f64::from(tone.sat.q_point) + step).clamp(-1.0, 1.0));
        }
        Grip::Tilt => {
            let step = interaction::gain_step(delta_y, mods) * 0.5;
            let to = (f64::from(tone.sat.tilt_db()) + step).clamp(-12.0, 12.0);
            tone.sat.set_tilt_db(f64_to_f32(to));
        }
        // The gate's times share the compressor knobs' law: a notch is
        // a fortieth of the travel.
        Grip::Hold(_) => {
            let step = interaction::gain_step(delta_y, mods) / 40.0;
            let to = log_norm(f64::from(tone.gate.hold), 1.0, 2_000.0) + step;
            tone.gate.hold = f64_to_f32(log_denorm(to.clamp(0.0, 1.0), 1.0, 2_000.0));
        }
        Grip::Range => {
            let step = interaction::gain_step(delta_y, mods);
            tone.gate.range = f64_to_f32((f64::from(tone.gate.range) + step).clamp(-80.0, 0.0));
        }
        // A suppressor's depth in fiftieths, its sharpness likewise,
        // and its band edges by a twelfth of an octave a notch.
        Grip::Depth(which) => {
            let step = interaction::gain_step(delta_y, mods) * 0.02;
            if let Some(set) = tone.suppressor(which) {
                set.depth = f64_to_f32((f64::from(set.depth) + step).clamp(0.0, 1.0));
            }
        }
        Grip::Sharpness(which) => {
            let step = interaction::gain_step(delta_y, mods) * 0.02;
            if let Some(set) = tone.suppressor(which) {
                set.sharpness = f64_to_f32((f64::from(set.sharpness) + step).clamp(0.0, 1.0));
            }
        }
        Grip::Edge(which, side) => {
            let ratio = (interaction::gain_step(delta_y, mods) / 12.0).exp2();
            move_edge(tone, which, side, ratio);
        }
        // Time by a twentieth of itself a notch — a delay is set by
        // ear in proportion, not in milliseconds — and feedback in
        // fiftieths.
        Grip::Time => {
            let ratio = interaction::gain_step(delta_y, mods).mul_add(0.05, 1.0);
            tone.delay.time = f64_to_f32((f64::from(tone.delay.time) * ratio).clamp(1.0, 2_500.0));
        }
        Grip::Feedback => {
            let step = interaction::gain_step(delta_y, mods) * 0.02;
            tone.delay.feedback = f64_to_f32((f64::from(tone.delay.feedback) + step).clamp(0.0, 0.99));
        }
        Grip::Decay => {
            let ratio = interaction::gain_step(delta_y, mods).mul_add(0.05, 1.0);
            tone.reverb.decay = f64_to_f32((f64::from(tone.reverb.decay) * ratio).clamp(0.1, 12.0));
        }
        Grip::Predelay => {
            let step = interaction::gain_step(delta_y, mods) * 2.0;
            tone.reverb.predelay = f64_to_f32((f64::from(tone.reverb.predelay) + step).clamp(0.0, 250.0));
        }
        Grip::Mix(which) => {
            let step = interaction::gain_step(delta_y, mods) * 0.02;
            if let Some(mix) = tone.mix(which) {
                *mix = f64_to_f32((f64::from(*mix) + step).clamp(0.0, 1.0));
            }
        }
        // A notch of a knob is a fortieth of its travel — the
        // compressor's law; a semitone for the pitch shifter, because
        // an interval is an integer.
        Grip::Knob(Which::Pitch, 0) => {
            let step = if delta_y < 0.0 { 1 } else { -1 };
            tone.pitch = tone.pitch.saturating_add(step).clamp(-24, 24);
        }
        Grip::Knob(which, index) => {
            let step = interaction::gain_step(delta_y, mods) / 40.0;
            let to = knob_value(tone, which, index) + step;
            set_knob_value(tone, which, index, to);
        }
        _ => {}
    }
}

/// Move one edge of a suppressor's band by a ratio, keeping the band
/// at least a third of an octave wide and inside the audible range.
fn move_edge(tone: &mut Tone, which: Which, side: Side, ratio: f64) {
    let Some(set) = tone.suppressor(which) else {
        return;
    };
    let (low, high) = (f64::from(set.low), f64::from(set.high));
    match side {
        Side::Low => set.low = f64_to_f32((low * ratio).clamp(20.0, high / SUPPRESS_SHOULDER)),
        Side::High => set.high = f64_to_f32((high * ratio).clamp(low * SUPPRESS_SHOULDER, 20_000.0)),
    }
}

/// What a modified click on a band does, applied.
///
/// [`interaction::dot_action`] resolves the chord; this carries out the
/// two that need no selection model — bypass and shape. Selection is a
/// concept the editor has and a strip does not, so those arms fall
/// through to the plain click the caller was already making.
///
/// Returns whether anything changed, so the caller knows whether to
/// re-record rather than guessing.
pub fn dot_click(tone: &mut Tone, which: Which, index: usize, mods: Mods) -> bool {
    let Some(band) = tone.bands(which).and_then(|set| set.get_mut(index)) else {
        return false;
    };
    match interaction::dot_action(mods) {
        interaction::DotAction::ToggleBypass => {
            band.enabled = !band.enabled;
            true
        }
        interaction::DotAction::CycleShape => {
            band.shape = next_shape(band.shape);
            true
        }
        interaction::DotAction::CycleSlope => {
            if let Some(slope) = band.slope.as_mut() {
                *slope = if *slope >= 10.0 { 1.0 } else { *slope + 1.0 };
                return true;
            }
            false
        }
        // Select, AddToSelection and RangeSelect all need a selection
        // model. A strip's rack has one band under the pointer and no
        // way to show a set, so these are a plain grab.
        interaction::DotAction::Select
        | interaction::DotAction::AddToSelection
        | interaction::DotAction::RangeSelect => false,
    }
}

/// The next shape in the cycle.
///
/// The shapes a strip rack can draw, in the order the editor cycles
/// them. Not every shape the model has — `filter_type_for_position`
/// reaches for cuts and notches when a band is CREATED, and creating
/// one is a gesture this rack does not have yet.
const fn next_shape(shape: EqBandShape) -> EqBandShape {
    match shape {
        EqBandShape::Bell => EqBandShape::LowShelf,
        EqBandShape::LowShelf => EqBandShape::HighShelf,
        EqBandShape::HighShelf => EqBandShape::Notch,
        _ => EqBandShape::Bell,
    }
}

/// Put a grip back where it started.
///
/// Double-clicking a control to default it is the gesture every DAW
/// has, and it is the one that makes a control safe to explore: you can
/// drag something to see what it does knowing the way back is one
/// gesture rather than a memory of the number.
///
/// A band goes FLAT rather than to some authored frequency — its
/// frequency is where you put it and its gain is the decision, so
/// undoing the decision is undoing the gain.
pub fn reset(tone: &mut Tone, grip: Grip) {
    match grip {
        Grip::Band(which, index) => {
            if let Some(band) = tone.bands(which).and_then(|set| set.get_mut(index)) {
                band.gain = 0.0;
            }
        }
        Grip::Threshold(which) => {
            let to = match which {
                Which::Gate => Gate::default().threshold,
                Which::DeEss | Which::DeEssIn => Suppress::sibilance().threshold,
                Which::Resonance => Suppress::broadband().threshold,
                _ => Comp::default().threshold,
            };
            tone.set_threshold(which, to);
        }
        // Resetting a bypass is switching it back in, which is what
        // the double-click would have done anyway.
        Grip::Bypass(which) => tone.bypass = {
            let mut next = tone.bypass;
            next.toggle(which);
            next
        },
        Grip::Ratio(which) => {
            if let Some(comp) = tone.compressor(which) {
                comp.ratio = Comp::default().ratio;
            }
        }
        Grip::Attack(Which::Gate) => tone.gate.attack = Gate::default().attack,
        Grip::Release(Which::Gate) => tone.gate.release = Gate::default().release,
        Grip::Attack(which) => {
            if let Some(comp) = tone.compressor(which) {
                comp.attack = Comp::default().attack;
            }
        }
        Grip::Release(which) => {
            if let Some(comp) = tone.compressor(which) {
                comp.release = Comp::default().release;
            }
        }
        // Unity: a preamp at drive one is the wire it is modelled on.
        Grip::Drive => tone.sat.drive = 1.0,
        Grip::Bias => tone.sat.q_point = 0.0,
        Grip::Tilt => tone.sat.set_tilt_db(0.0),
        Grip::Hold(_) => tone.gate.hold = Gate::default().hold,
        Grip::Range => tone.gate.range = Gate::default().range,
        Grip::Depth(which) | Grip::Sharpness(which) | Grip::Edge(which, _) => {
            let fresh = match which {
                Which::DeEss | Which::DeEssIn => Suppress::sibilance(),
                _ => Suppress::broadband(),
            };
            if let Some(set) = tone.suppressor(which) {
                match grip {
                    Grip::Depth(_) => set.depth = fresh.depth,
                    Grip::Sharpness(_) => set.sharpness = fresh.sharpness,
                    _ => {
                        set.low = fresh.low;
                        set.high = fresh.high;
                    }
                }
            }
        }
        Grip::Time => tone.delay.time = Echo::default().time,
        Grip::Feedback => tone.delay.feedback = Echo::default().feedback,
        Grip::Decay => tone.reverb.decay = Room::default().decay,
        Grip::Predelay => tone.reverb.predelay = Room::default().predelay,
        Grip::Mix(which) => {
            let fresh = match which {
                Which::Delay => Echo::default().mix,
                Which::Reverb => Room::default().mix,
                _ => 1.0,
            };
            if let Some(mix) = tone.mix(which) {
                *mix = fresh;
            }
        }
        Grip::Scale(_) => tone.eq_range = DEFAULT_EQ_RANGE,
        // A machine is a choice, not a value with a default to go back
        // to; and folding is not a setting on the track, so there is
        // nothing to put back — see `Fold`.
        Grip::Family(_) | Grip::Choose(..) | Grip::Phase(_) | Grip::Preset(_) => {}
        // Back to what the loaded preset had, if one was loaded — a
        // knob's default is the preset's value, not a number.
        Grip::Knob(which, index) => {
            if let Some(preset) = tone.preset.and_then(|i| tone.presets.get(i)) {
                let was = knob_value(&preset.tone, which, index);
                set_knob_value(tone, which, index, was);
            }
        }
    }
}

/// Move a grip by a pixel delta.
///
/// Pixels rather than fractions because these axes are not linear in
/// the same way: a band's frequency is logarithmic and its gain is not,
/// so the conversion has to happen against the panel the drag is in.
#[expect(clippy::too_many_arguments, reason = "a drag and everything it reads")]
pub fn drag(
    tone: &mut Tone,
    grip: Grip,
    panels: &[Which],
    panel: Panel,
    folded: Folded,
    mods: Mods,
    dx: f64,
    dy: f64,
) {
    let rack = Rack::at(panel.width);
    // The grip names its own panel now — see `Grip::panel`. It used to
    // be inferred from the grip's KIND, which stopped working the
    // moment the chain had three EQs and two compressors.
    let Some((_, at)) = units(panels, panel, folded)
        .into_iter()
        .find(|(which, _)| *which == grip.panel())
    else {
        return;
    };
    let body = body_of(at, rack);
    if body.width <= 0.0 || body.height <= 0.0 {
        return;
    }
    match grip {
        Grip::Band(which, index) => {
            let map = mapper(body, tone.eq_db_range());
            let Some(band) = tone.bands(which).and_then(|set| set.get_mut(index)) else {
                return;
            };
            // Where the band is now, in the graph's own coordinates,
            // moved by the pointer's delta.
            let x = (map.freq_to_x(f64::from(band.frequency)) + dx).clamp(0.0, body.width);
            let y = (map.db_to_y(f64::from(band.gain)) + dy).clamp(0.0, body.height);
            // Which axes move is the plugin's decision: Alt pins the
            // gain so you can hunt for where a cut belongs without
            // losing how much of it you had, and Cmd turns the vertical
            // into resonance.
            match interaction::drag_mode(mods, dx, dy) {
                interaction::DragMode::Free => {
                    band.frequency = f64_to_f32(map.x_to_freq(x));
                    band.gain = interaction::drag_gain_for_shape(
                        band.shape,
                        band.gain,
                        map.y_to_db(y).clamp(-EQ_GAIN_LIMIT, EQ_GAIN_LIMIT),
                    );
                }
                interaction::DragMode::FreqOnly => {
                    band.frequency = f64_to_f32(map.x_to_freq(x));
                }
                interaction::DragMode::GainOnly => {
                    band.gain = interaction::drag_gain_for_shape(
                        band.shape,
                        band.gain,
                        map.y_to_db(y).clamp(-EQ_GAIN_LIMIT, EQ_GAIN_LIMIT),
                    );
                }
                interaction::DragMode::Resonance => {
                    // Vertical travel as a scroll: the plugin's own Q
                    // step, so a drag and a wheel move it by the same
                    // law rather than two that nearly agree.
                    interaction::wheel_band(band, dy, false, interaction::fine_scale(mods));
                }
            }
        }
        Grip::Threshold(which) => {
            // Against the DISPLAY's height, not the panel's: the line
            // is drawn on the display, and a threshold that moved
            // against a taller box would run ahead of the line the
            // pointer is holding.
            let display = comp_split(body, rack);
            let Some((now, low, high)) = tone.threshold(which) else {
                return;
            };
            let per_db = display.height / f64::from(high - low);
            let dy = dy * interaction::fine_scale(mods);
            let moved = f64::from(now) - dy / per_db.max(f64::EPSILON);
            tone.set_threshold(which, f64_to_f32(moved));
        }
        // The two times are WIDTHS on the envelope, so they are
        // dragged sideways — time is the horizontal axis on this
        // display and on every other envelope anyone has drawn. Right
        // is longer, for both, because both are durations and neither
        // is the opposite of the other.
        //
        // Against the span the glyph maps them onto, so the edge being
        // moved stays under the finger moving it.
        // The gate's three times are widths on its table, a quarter of
        // the display each — see `GateGlyph::of`.
        Grip::Attack(Which::Gate) | Grip::Hold(Which::Gate) | Grip::Release(Which::Gate) => {
            let display = display_of(body, Which::Gate, rack);
            let dx = dx * interaction::fine_scale(mods);
            let span = (display.width * 0.25).max(1.0);
            let (value, low, high): (&mut f32, f64, f64) = match grip {
                Grip::Attack(_) => (&mut tone.gate.attack, 0.1, 200.0),
                Grip::Hold(_) => (&mut tone.gate.hold, 1.0, 2_000.0),
                _ => (&mut tone.gate.release, 5.0, 3_000.0),
            };
            let moved = (log_norm(f64::from(*value), low, high) + dx / span).clamp(0.0, 1.0);
            *value = f64_to_f32(log_denorm(moved, low, high));
        }
        Grip::Attack(which) | Grip::Release(which) => {
            let display = comp_split(body, rack);
            let dx = dx * interaction::fine_scale(mods);
            let span = (display.width * 0.4).max(1.0);
            if let Some(comp) = tone.compressor(which) {
                let moved = knob_norm(*comp, grip) + dx / span;
                set_knob(comp, grip, moved);
            }
        }
        // The floor is pulled DOWN for more, which is the direction the
        // signal goes. Against the display's own dB height, so it stays
        // under the finger — and the ratio keeps moving past the point
        // where the depth stops growing, because the depth saturates
        // and the setting does not.
        // A switch has no drag. Dragging off one is how you change your
        // mind about pressing it, which is the mixer's own rule — and a
        // zoom is a switch between stops.
        Grip::Bypass(_) | Grip::Scale(_) | Grip::Phase(_) => {}
        Grip::Drive => {
            // A quarter of the panel's height is the whole range, so a
            // short drag is a real change — drive is the parameter you
            // nudge, not the one you sweep.
            let per_unit = body.height / 4.0;
            let dy = dy * interaction::fine_scale(mods);
            let moved = f64::from(tone.sat.drive) - dy / per_unit.max(f64::EPSILON);
            tone.sat.drive = f64_to_f32(moved.clamp(0.0, 10.0));
        }
        _ => drag_more(tone, grip, body, rack, mods, dx, dy),
    }
}

/// The drags the newer faces added — split from [`drag`] by size alone.
/// Each arm states the axis its grip moves on and what a panel width or
/// height is worth on it.
fn drag_more(tone: &mut Tone, grip: Grip, body: Panel, rack: Rack, mods: Mods, dx: f64, dy: f64) {
    match grip {
        Grip::Ratio(which) => {
            let display = comp_split(body, rack);
            let dy = dy * interaction::fine_scale(mods);
            let per_db = display.height / 60.0;
            let Some(comp) = tone.compressor(which).copied() else {
                return;
            };
            let threshold = f64::from(comp.threshold);
            let reduced = (ratio_reduction(comp) + dy / per_db.max(f64::EPSILON))
                .clamp(0.0, -threshold);
            // Back to a ratio: reduction = -T(1 - 1/R), so
            // R = 1 / (1 + reduction/T). Clamped at the top because the
            // last decibel of reduction costs an unbounded ratio.
            let ratio = if -threshold <= f64::EPSILON {
                1.0
            } else {
                1.0 / (1.0 + reduced / threshold)
            };
            if let Some(comp) = tone.compressor(which) {
                comp.ratio = f64_to_f32(ratio.clamp(1.0, 20.0));
            }
        }

        // The bias leans the curve, so it is dragged the way the curve
        // leans: sideways, half the curve's width for the whole range.
        Grip::Bias => {
            let (curve_box, _) = sat_split(display_of(body, Which::Sat, rack), rack);
            let dx = dx * interaction::fine_scale(mods);
            let per_unit = (curve_box.width / 2.0).max(1.0);
            let moved = f64::from(tone.sat.q_point) + dx / per_unit;
            tone.sat.q_point = f64_to_f32(moved.clamp(-1.0, 1.0));
        }
        // Up is the top: a drag up the ladder tips the emphasis
        // toward the highs. The ladder's height is ±12 dB.
        Grip::Tilt => {
            let (_, ladder_box) = sat_split(display_of(body, Which::Sat, rack), rack);
            let per_db = ladder_box.map_or(body.height, |lb| lb.height) / 24.0;
            let dy = dy * interaction::fine_scale(mods);
            let to = (f64::from(tone.sat.tilt_db()) - dy / per_db.max(f64::EPSILON)).clamp(-12.0, 12.0);
            tone.sat.set_tilt_db(f64_to_f32(to));
        }
        // The range line is pulled DOWN for more, against the display's
        // own dB axis, like the ratio's arrow.
        Grip::Range => {
            let display = display_of(body, Which::Gate, rack);
            let per_db = display.height / 60.0;
            let dy = dy * interaction::fine_scale(mods);
            let moved = f64::from(tone.gate.range) - dy / per_db.max(f64::EPSILON);
            tone.gate.range = f64_to_f32(moved.clamp(-80.0, 0.0));
        }
        // The ribbon is pulled down for more: the display's height is
        // the whole range. Sharpness the same way — down is narrower,
        // which is what a tooth is.
        Grip::Depth(which) | Grip::Sharpness(which) => {
            let display = display_of(body, which, rack);
            let dy = dy * interaction::fine_scale(mods);
            let per_unit = display.height.max(1.0);
            if let Some(set) = tone.suppressor(which) {
                let value = if matches!(grip, Grip::Depth(_)) { &mut set.depth } else { &mut set.sharpness };
                *value = f64_to_f32((f64::from(*value) + dy / per_unit).clamp(0.0, 1.0));
            }
        }
        // An edge moves along the zoomed axis it is drawn on: a panel
        // width is the window's whole span in decades.
        Grip::Edge(which, side) => {
            let display = display_of(body, which, rack);
            let set = match which {
                Which::DeEss | Which::DeEssIn => tone.de_ess,
                _ => tone.resonance,
            };
            let zoom = SuppressZoom::of(set);
            let dx = dx * interaction::fine_scale(mods);
            let decades = (zoom.high / zoom.low).log10();
            let ratio = 10.0_f64.powf(dx / display.width.max(1.0) * decades);
            move_edge(tone, which, side, ratio);
        }
        // The first repeat's x IS the time: the window is four and a
        // half times, so a pixel is that many milliseconds.
        Grip::Time => {
            let body = display_of(body, Which::Delay, rack);
            let dx = dx * interaction::fine_scale(mods);
            let per_ms = (body.width - 6.0).max(1.0) / (f64::from(tone.delay.time).max(1.0) * 4.5);
            let moved = f64::from(tone.delay.time) + dx / per_ms.max(f64::EPSILON);
            tone.delay.time = f64_to_f32(moved.clamp(1.0, 2_500.0));
        }
        // The repeats' heights are the feedback: the panel's height is
        // the range.
        Grip::Feedback => {
            let body = display_of(body, Which::Delay, rack);
            let dy = dy * interaction::fine_scale(mods);
            let moved = f64::from(tone.delay.feedback) - dy / body.height.max(1.0);
            tone.delay.feedback = f64_to_f32(moved.clamp(0.0, 0.99));
        }
        // The tail reaches the right edge at its decay, so a drag right
        // lengthens it in proportion: a panel width is the decay
        // itself.
        Grip::Decay => {
            let body = display_of(body, Which::Reverb, rack);
            let dx = dx * interaction::fine_scale(mods);
            let ratio = 1.0 + dx / body.width.max(1.0);
            tone.reverb.decay = f64_to_f32((f64::from(tone.reverb.decay) * ratio.max(0.2)).clamp(0.1, 12.0));
        }
        // The gap rule moves along the decay window.
        Grip::Predelay => {
            let body = display_of(body, Which::Reverb, rack);
            let dx = dx * interaction::fine_scale(mods);
            let window = f64::from(tone.reverb.decay).max(0.05) * 1000.0;
            let moved = f64::from(tone.reverb.predelay) + dx / body.width.max(1.0) * window;
            tone.reverb.predelay = f64_to_f32(moved.clamp(0.0, 250.0));
        }
        Grip::Mix(which) => {
            let body = display_of(body, which, rack);
            let dy = dy * interaction::fine_scale(mods);
            let per_unit = body.height.max(1.0);
            if let Some(mix) = tone.mix(which) {
                *mix = f64_to_f32((f64::from(*mix) - dy / per_unit).clamp(0.0, 1.0));
            }
        }
        Grip::Knob(which, index) => drag_knob(tone, which, index, mods, dy),
        _ => {}
    }
}

/// A knob's drag: its whole travel is a hundred pixels, up for more.
/// The pitch shifter's interval steps a semitone every six, because an
/// interval is an integer.
fn drag_knob(tone: &mut Tone, which: Which, index: usize, mods: Mods, dy: f64) {
    let dy = dy * interaction::fine_scale(mods);
    if which == Which::Pitch && index == 0 {
        let steps = crate::num::quantise(-dy / 6.0, 1.0);
        tone.pitch = tone.pitch.saturating_add(steps).clamp(-24, 24);
        return;
    }
    let to = knob_value(tone, which, index) - dy / 100.0;
    set_knob_value(tone, which, index, to);
}

/// A knob's value as a fraction of its travel — see [`knob_labels`]
/// for which knob is which.
#[must_use]
pub fn knob_value(tone: &Tone, which: Which, index: usize) -> f64 {
    match (which, tone.role, index) {
        (Which::Knobs, Role::Reverb, 0) => f64::from(tone.reverb.size),
        (Which::Knobs, Role::Reverb, 1) => f64::from(tone.reverb.damping),
        (Which::Knobs, Role::Reverb, 2) => f64::from(tone.reverb.diffusion),
        (Which::Knobs, Role::Reverb, 3) => f64::from(tone.reverb.predelay) / 250.0,
        (Which::Knobs, Role::Reverb, _) => f64::from(tone.reverb.mix),
        (Which::Knobs, _, 0) => log_norm(f64::from(tone.delay.time), 1.0, 2_500.0),
        (Which::Knobs, _, 1) => f64::from(tone.delay.feedback) / 0.99,
        (Which::Knobs, _, 2) => f64::from(tone.delay.tone),
        (Which::Knobs, _, 3) => f64::from(tone.delay.width),
        (Which::Knobs, _, _) => f64::from(tone.delay.mix),
        (Which::Wide, _, _) => f64::from(tone.wide) / 2.0,
        (Which::Pitch, _, 0) => (f64::from(tone.pitch) + 24.0) / 48.0,
        (Which::Pitch, _, _) => f64::from(tone.pitch_mix),
        _ => 0.0,
    }
}

/// And setting it, clamped to the travel.
pub fn set_knob_value(tone: &mut Tone, which: Which, index: usize, to: f64) {
    let to = to.clamp(0.0, 1.0);
    let f = f64_to_f32(to);
    match (which, tone.role, index) {
        (Which::Knobs, Role::Reverb, 0) => tone.reverb.size = f,
        (Which::Knobs, Role::Reverb, 1) => tone.reverb.damping = f,
        (Which::Knobs, Role::Reverb, 2) => tone.reverb.diffusion = f,
        (Which::Knobs, Role::Reverb, 3) => tone.reverb.predelay = f64_to_f32(to * 250.0),
        (Which::Knobs, Role::Reverb, _) => tone.reverb.mix = f,
        (Which::Knobs, _, 0) => tone.delay.time = f64_to_f32(log_denorm(to, 1.0, 2_500.0)),
        (Which::Knobs, _, 1) => tone.delay.feedback = f64_to_f32(to * 0.99),
        (Which::Knobs, _, 2) => tone.delay.tone = f,
        (Which::Knobs, _, 3) => tone.delay.width = f,
        (Which::Knobs, _, _) => tone.delay.mix = f,
        (Which::Wide, _, _) => tone.wide = f64_to_f32(to * 2.0),
        (Which::Pitch, _, 0) => tone.pitch = crate::num::quantise(to.mul_add(48.0, -24.0), 1.0).clamp(-24, 24),
        (Which::Pitch, _, _) => tone.pitch_mix = f,
        _ => {}
    }
}

/// What the knob row's five knobs are, for a role.
#[must_use]
pub const fn knob_labels(role: Role) -> [&'static str; KNOBS] {
    match role {
        Role::Reverb => ["SIZE", "DAMP", "DIFF", "PRE", "MIX"],
        _ => ["TIME", "FDBK", "TONE", "WIDE", "MIX"],
    }
}

/// How many knobs the row holds.
pub const KNOBS: usize = 5;

/// Which knob of the row a point is over.
#[must_use]
pub fn knob_at(body: Panel, x: f64) -> usize {
    let each = body.width / crate::num::coord(KNOBS);
    crate::num::index(((x - body.x) / each.max(1.0)).floor()).min(KNOBS.saturating_sub(1))
}

/// Which preset chip a point is over, if any.
#[must_use]
pub fn preset_chip_at(tone: &Tone, at: Panel, x: f64) -> Option<usize> {
    let mut left = at.x + 2.0;
    for (i, preset) in tone.presets.iter().enumerate() {
        let w = preset_chip_width(&preset.name);
        if x >= left && x < left + w {
            return Some(i);
        }
        left += w + 2.0;
    }
    None
}

/// A chip's width for a name: the swatch, the text, the air.
fn preset_chip_width(name: &str) -> f64 {
    crate::num::coord(name.len()).mul_add(3.6, 14.0)
}

/// How tall the preset row is.
pub const PRESETS_H: f64 = 18.0;

/// The Tone settings for every track the window has opened.
///
/// Keyed by GUID, not by row: a preset can hide a track and a fold can
/// move one, and a rack that followed a row index would show the
/// neighbour's EQ the moment either happened.
///
/// This is where a real chain's parameters will land. Until then it is
/// seeded from [`placeholder`] — which means a rack is editable NOW,
/// against state the window owns, in exactly the shape the plugin's
/// parameters will take. The binding is the piece that is missing; the
/// UI above it is not waiting on anything.
#[derive(Clone, Debug, Default)]
pub struct Store {
    by_guid: std::collections::HashMap<String, Tone>,
}

impl Store {
    /// Make sure every track has settings, without disturbing the ones
    /// that already do.
    ///
    /// Called before a record rather than lazily inside one, so that
    /// recording can take `&self` — a `&mut` threaded through the
    /// drawing would put a lock between the mixer and its strips.
    pub fn seed(&mut self, rows: &[(daw_proto::Track, u32)]) {
        // The folders above each track, nearest last: a row's depth
        // says how many of the folders before it still enclose it.
        let mut folders: Vec<(u32, String)> = Vec::new();
        for (index, (track, depth)) in rows.iter().enumerate() {
            folders.retain(|(at, _)| *at < *depth);
            let ancestors: Vec<String> = folders.iter().map(|(_, name)| name.clone()).collect();
            let role = Role::of(&track.name, track.is_folder, &ancestors);
            if track.is_folder {
                folders.push((*depth, track.name.clone()));
            }
            self.by_guid
                .entry(track.guid.clone())
                .or_insert_with(|| placeholder_for(role, index, &track.name));
        }
    }

    /// A track's settings, if it has any.
    #[must_use]
    pub fn get(&self, guid: &str) -> Option<&Tone> {
        self.by_guid.get(guid)
    }

    /// The same, to change.
    pub fn edit(&mut self, guid: &str) -> Option<&mut Tone> {
        self.by_guid.get_mut(guid)
    }

    /// Render every reverb tail in the store, now, on this thread.
    ///
    /// For a bench or a test that must be deterministic — a tail that
    /// arrived between two renders of the same frame would make them
    /// differ.
    pub fn prerender_tails(&self) {
        for tone in self.by_guid.values() {
            tone.prerender_tails();
        }
    }
}

fn band(index: usize, frequency: f64, gain: f64, q: f64, shape: EqBandShape) -> EqBand {
    EqBand {
        index,
        used: true,
        enabled: true,
        frequency: f64_to_f32(frequency),
        gain: f64_to_f32(gain),
        q: f64_to_f32(q),
        shape,
        // The cut filters' slope order; `None` leaves the band on its Q,
        // which is what every shape the rack places uses.
        slope: None,
        focus: false,
        stereo_mode: StereoMode::default(),
        name: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The panels are their own height, from the top, and do not
    /// stretch to whatever rack they are given — a frequency response
    /// is readable at a hundred and seventy pixels and no more readable
    /// at four hundred.
    #[test]
    fn the_panels_do_not_stretch_to_fill_the_rack() {
        let tall = Panel {
            x: 0.0,
            y: 0.0,
            width: 133.0,
            height: 900.0,
        };
        let laid = layout(&[Which::Eq, Which::Comp, Which::Sat], tall);
        assert_eq!(laid.len(), 3);
        for (which, at) in &laid {
            assert!(
                (at.height - which.natural()).abs() < 0.01,
                "{which:?} took {} rather than {}",
                at.height,
                which.natural()
            );
        }
        // From the top, under the phase's container bar, and the space
        // below is left alone.
        assert!((laid[0].1.y - tall.y - HEAD_H).abs() < f64::EPSILON);
        let used = super::tall(&[Which::Eq, Which::Comp, Which::Sat], super::Folded::default());
        assert!(used < tall.height, "the rack filled everything it was given");
    }

    /// A rack too short for its chain SCROLLS rather than shrinking.
    ///
    /// The panels used to scale down together to fit, which meant every
    /// processor added shrank every processor already there — three
    /// units at two hundred pixels is three unreadable units. Keeping
    /// their natural heights and running off the bottom is what makes
    /// the chain survive growing, and `scroll_span` is how far the box
    /// has to travel to see the rest.
    #[test]
    fn a_short_rack_scrolls_rather_than_shrinking() {
        let panels = [Which::Eq, Which::Comp, Which::Sat];
        let short = Panel {
            x: 0.0,
            y: 0.0,
            width: 133.0,
            height: 200.0,
        };
        let laid = layout(&panels, short);
        assert_eq!(laid.len(), 3, "a panel was dropped");
        for (which, at) in &laid {
            assert!(
                (at.height - which.natural()).abs() < f64::EPSILON,
                "{which:?} was shrunk to {}",
                at.height
            );
        }
        let bottom = laid.last().map_or(0.0, |(_, at)| at.y + at.height);
        assert!(bottom > short.height, "the chain fitted, so nothing was proved");
        assert!(
            (super::scroll_span(&panels, short.height, super::Folded::default()) - (bottom - short.height)).abs() < 0.01,
            "the span does not reach the last panel's floor"
        );

        // And a box tall enough to hold the chain does not scroll.
        assert!(super::scroll_span(&panels, 4000.0, super::Folded::default()).abs() < f64::EPSILON);
    }

    /// Scrolling is the panel's own `y`, so the drawing and the hit
    /// test cannot disagree about it — they are handed the same moved
    /// panel rather than each applying an offset.
    #[test]
    fn a_scroll_moves_every_panel_by_the_same_amount() {
        let panels = [Which::Eq, Which::Comp, Which::Sat];
        let box_at = Panel { x: 0.0, y: 40.0, width: 133.0, height: 200.0 };
        let moved = Panel { y: box_at.y - 75.0, ..box_at };
        for ((_, rest), (_, down)) in layout(&panels, box_at).iter().zip(layout(&panels, moved)) {
            assert!((rest.y - down.y - 75.0).abs() < f64::EPSILON);
            assert!((rest.height - down.height).abs() < f64::EPSILON);
        }
    }

    /// The tiers are ordered, and each threshold is where its tier
    /// starts — written against the constants, because pasted widths go
    /// stale the moment a threshold moves and then test nothing.
    /// The focus tier hands the panel to the plugin, and the tier
    /// below it keeps the rack's own abbreviation. Two looks from one
    /// painter, and the threshold between them is a width.
    #[test]
    fn the_focus_tier_is_the_editing_one() {
        assert!(LEGIBLE < FOCUSED, "the tiers must not overlap");
        assert_eq!(Rack::at(FOCUSED), Rack::Focus);
        assert_eq!(Rack::at(FOCUSED - 0.5), Rack::Full);
        assert!(Rack::at(FOCUSED).editing());
        assert!(!Rack::at(FOCUSED - 0.5).editing());
        // Both are detailed: headers, markers and grips belong to each.
        assert!(Rack::at(FOCUSED).detailed());
        assert!(Rack::at(LEGIBLE).detailed());
        assert!(!Rack::at(SHAPE).detailed());
    }

    /// A focused panel still gives up its header line — the numbers are
    /// what you check the curve against, and they do not stop mattering
    /// because there is more room.
    #[test]
    fn every_detailed_tier_keeps_its_header() {
        let panel = Panel {
            x: 0.0,
            y: 0.0,
            width: 300.0,
            height: 200.0,
        };
        for rack in [Rack::Focus, Rack::Full] {
            let body = body_of(panel, rack);
            assert!(
                body.y > panel.inset(2.0).y,
                "{rack:?} did not reserve a header"
            );
        }
        assert!(
            (body_of(panel, Rack::Curves).y - panel.inset(2.0).y).abs() < f64::EPSILON,
            "the narrow tier has no room for one"
        );
    }

    /// A focused strip may not be lent below the width that made it
    /// focused — the whole point of opening one is that it stays open.
    #[test]
    fn a_focused_rack_will_not_be_lent_away() {
        assert_eq!(Rack::Focus.floor(), Some(FOCUSED));
    }

    #[test]
    fn the_rack_sheds_in_order() {
        assert!(SHAPE < LEGIBLE, "the tiers must not overlap");
        assert_eq!(Rack::at(LEGIBLE), Rack::Full);
        assert_eq!(Rack::at(LEGIBLE - 0.5), Rack::Curves);
        assert_eq!(Rack::at(SHAPE), Rack::Curves);
        assert_eq!(Rack::at(SHAPE - 0.5), Rack::Off);
        assert!(Rack::Full < Rack::Curves && Rack::Curves < Rack::Off);
        assert!(Rack::at(LEGIBLE).on() && !Rack::at(SHAPE - 0.5).on());
    }

    /// A rack with no room records nothing at all, rather than three
    /// slivers of well with no curve in them.
    #[test]
    fn a_flat_rack_draws_nothing() {
        let mut scene = Scene::new();
        let palette = Palette::from_theme(&daw_ui::theming::Theme::dark());
        let font = Font::embedded().expect("the embedded font");
        record(
            &mut scene,
            &palette,
            &font,
            &placeholder(0),
            &[Which::Eq, Which::Comp, Which::Sat],
            Panel {
                x: 0.0,
                y: 0.0,
                width: 300.0,
                height: 8.0,
            },
            super::Folded::default(),
        );
        assert!(scene.commands.is_empty());
    }

    /// And a rack with room records all three panels.
    #[test]
    fn a_full_rack_draws_three_panels() {
        let mut scene = Scene::new();
        let palette = Palette::from_theme(&daw_ui::theming::Theme::dark());
        let font = Font::embedded().expect("the embedded font");
        record(
            &mut scene,
            &palette,
            &font,
            &placeholder(0),
            &[Which::Eq, Which::Comp, Which::Sat],
            Panel {
                x: 0.0,
                y: 0.0,
                width: 300.0,
                height: 180.0,
            },
            super::Folded::default(),
        );
        // Three grounds, three curves, the rules and the labels.
        assert!(
            scene.commands.len() > 9,
            "a full rack should record more than its three grounds: {}",
            scene.commands.len()
        );
    }

    /// The EQ curve is the plugin's, so a boost must come back as one:
    /// the response at a boosted band's centre is above unity.
    #[test]
    fn the_curve_is_the_plugins_own() {
        let tone = placeholder(0);
        let at_3k = calculate_combined_response(&tone.eq, 3_000.0, DISPLAY_RATE);
        let at_300 = calculate_combined_response(&tone.eq, 300.0, DISPLAY_RATE);
        assert!(at_3k > 0.0, "the 3k bell boosts: {at_3k}");
        assert!(at_300 < 0.0, "the 300 bell cuts: {at_300}");
    }
}

#[cfg(test)]
mod suppress_tests {
    use super::{SUPPRESS_CEIL_DB, SUPPRESS_FLOOR_DB, Suppress, Which};

    /// The analyser speaks DECIBELS, and the suppressor's window has to
    /// be the analyser's own.
    ///
    /// This is the bug that made both suppressors unreadable: the bins
    /// were read as a 0..1 height, so every one of them pinned to the
    /// ceiling or sat on the floor, and the difference between a peak
    /// and its neighbourhood — which is the entire detection — came out
    /// as either nothing or everything.
    #[test]
    fn the_window_is_the_analysers_own() {
        let simulated = crate::simulate::frame(2, 1.25).spectrum;
        for db in &simulated {
            assert!(
                f64::from(*db) >= SUPPRESS_FLOOR_DB && f64::from(*db) <= SUPPRESS_CEIL_DB,
                "the analyser produced {db} dB, outside the window the suppressor draws"
            );
        }
        assert!(SUPPRESS_FLOOR_DB < SUPPRESS_CEIL_DB);
    }

    /// A suppressor acts only inside its band, which is the whole
    /// difference between the de-esser and the broadband one.
    #[test]
    fn the_band_is_what_tells_the_two_apart() {
        let ess = Suppress::sibilance();
        let broad = Suppress::broadband();
        assert!(ess.low > broad.low, "the de-esser should start higher up");
        assert!(
            f64::from(ess.high - ess.low) < f64::from(broad.high - broad.low),
            "the de-esser should be the narrower of the two"
        );
        // Both draw the same picture, and both are spectral.
        assert!(Which::DeEss.is_spectral() && Which::Resonance.is_spectral());
    }
}

#[cfg(test)]
mod fold_tests {
    use super::{Fold, Folded};
    use session::mix_phases::MixPhase as P;

    /// A fold is per phase and survives being asked about again.
    #[test]
    fn a_phase_folds_and_unfolds() {
        let mut shut = Folded::default();
        assert!(!shut.is(P::Tone));
        shut.toggle(P::Tone);
        assert!(shut.is(P::Tone));
        assert!(!shut.is(P::Rescue), "folding one folded another");
        assert!(shut.any());
        shut.toggle(P::Tone);
        assert!(!shut.any());
    }

    /// Synced, one fold answers for every track — which is what keeps
    /// the chain in register across a mixer.
    #[test]
    fn synced_folds_reach_every_track() {
        let mut fold = Fold::shared();
        fold.toggle("kick", P::Rescue);
        assert!(fold.of("kick").is(P::Rescue));
        assert!(fold.of("snare").is(P::Rescue), "the fold stayed on one track");
    }

    /// And unsynced, it reaches only the one you folded.
    #[test]
    fn per_track_folds_stay_on_their_track() {
        let mut fold = Fold::default();
        fold.toggle("kick", P::Rescue);
        assert!(fold.of("kick").is(P::Rescue));
        assert!(!fold.of("snare").is(P::Rescue));
    }

    /// Going synced takes the track you were on as the answer for
    /// everyone, rather than throwing the fold away — the fold you just
    /// made is almost always the one you meant.
    #[test]
    fn syncing_adopts_the_track_you_were_on() {
        let mut fold = Fold::default();
        fold.toggle("kick", P::Polish);
        fold.sync(true, "kick");
        assert!(fold.of("snare").is(P::Polish));
        assert!(fold.of("anything at all").is(P::Polish));
    }
}

#[cfg(test)]
mod container_tests {
    use super::{ALL_PANELS, Folded, HEAD_H, Panel, Row, Which, chain, tall, units};
    use session::mix_phases::MixPhase as P;

    fn box_at() -> Panel {
        Panel {
            x: 0.0,
            y: 0.0,
            width: 133.0,
            height: 4000.0,
        }
    }

    /// Every phase gets exactly one container, and it sits above the
    /// units it caps.
    #[test]
    fn each_phase_gets_one_header_above_its_units() {
        let rows = chain(&ALL_PANELS, box_at(), Folded::default());
        let mut seen: Vec<P> = Vec::new();
        let mut head: Option<(P, f64)> = None;
        for (row, at) in &rows {
            match row {
                Row::Head(phase) => {
                    assert!(!seen.contains(phase), "{phase:?} got two containers");
                    seen.push(*phase);
                    head = Some((*phase, at.y));
                }
                Row::Unit(which) => {
                    let (phase, y) = head.expect("a unit before any container");
                    assert_eq!(which.phase(), phase, "{which:?} under {phase:?}");
                    assert!(at.y >= y, "{which:?} sat above its own container");
                }
            }
        }
        assert_eq!(seen.len(), 5, "expected one container per phase: {seen:?}");
    }

    /// Folding a phase takes its units out of the column and leaves its
    /// container — you have to be able to unfold it.
    #[test]
    fn folding_hides_the_units_and_keeps_the_bar() {
        let mut shut = Folded::default();
        shut.toggle(P::Tone);
        let rows = chain(&ALL_PANELS, box_at(), shut);
        assert!(
            rows.iter().any(|(row, _)| *row == Row::Head(P::Tone)),
            "the container went with its units"
        );
        for which in [Which::Eq, Which::Comp, Which::Sat] {
            assert!(
                !rows.iter().any(|(row, _)| *row == Row::Unit(which)),
                "{which:?} survived the fold"
            );
        }
        // And the rest of the chain is still there.
        assert!(rows.iter().any(|(row, _)| *row == Row::Unit(Which::Delay)));
    }

    /// A fold shortens the column by what it hid, which is what makes
    /// the gesture worth making — the chain gets nearer to fitting.
    #[test]
    fn folding_shortens_the_column() {
        let open = tall(&ALL_PANELS, Folded::default());
        let mut shut = Folded::default();
        shut.toggle(P::Tone);
        let folded = tall(&ALL_PANELS, shut);
        assert!(folded < open, "{folded} was not shorter than {open}");
        // By the units it hid, and not by the container itself.
        let hidden: f64 = [Which::Eq, Which::Comp, Which::Sat]
            .iter()
            .map(|w| w.natural() + super::GAP)
            .sum();
        assert!((open - folded - hidden).abs() < 0.01, "{open} - {folded}");
        assert!(HEAD_H > 0.0);
    }

    /// Folding everything leaves five bars and nothing else.
    #[test]
    fn folding_everything_leaves_the_bars() {
        let mut shut = Folded::default();
        for phase in P::ALL {
            shut.toggle(phase);
        }
        assert!(units(&ALL_PANELS, box_at(), shut).is_empty());
        assert_eq!(chain(&ALL_PANELS, box_at(), shut).len(), 5);
    }
}

#[cfg(test)]
mod phase_tests {
    use super::{Which, panels_for};
    use session::mix_phases::MixPhase as P;

    /// Every phase shows the whole chain.
    ///
    /// It used to show a subset per phase — the EQ alone in Rescue, no
    /// rack at all in Balance — so changing phase changed which
    /// processors EXISTED and a strip reorganised itself under you. A
    /// chain is a chain; a phase's job is to say which part of it to
    /// look at, which is focus rather than membership.
    #[test]
    fn every_phase_shows_the_whole_chain() {
        for phase in P::ALL {
            assert_eq!(panels_for(phase), super::ALL_PANELS, "{phase:?} showed a subset");
        }
    }

    /// The chain is ordered BY phase, and every unit knows which one it
    /// belongs to — which is what the rail's buttons will focus once
    /// there is a setting for it. Out of order, "focus the Tone phase"
    /// would mean scrolling to two places at once.
    #[test]
    fn the_chain_runs_in_phase_order() {
        let order = |phase: P| P::ALL.iter().position(|p| *p == phase);
        let mut last = None;
        for which in super::ALL_PANELS {
            let at = order(which.phase()).expect("a phase in the canonical order");
            if let Some(before) = last {
                assert!(at >= before, "{which:?} is out of phase order");
            }
            last = Some(at);
        }
    }

    /// Every unit that draws a frequency response says so, because they
    /// share one hit test and one graph — and the ones that do not must
    /// not be routed into it.
    #[test]
    fn the_spectral_units_are_the_ones_with_curves() {
        for which in super::ALL_PANELS {
            let spectral = matches!(
                which,
                Which::RescueEq | Which::Eq | Which::Space | Which::DeEss | Which::Resonance
            );
            assert_eq!(which.is_spectral(), spectral, "{which:?}");
        }
    }

    /// Every phase answers — a `match` that grew a hole would be a
    /// phase button that silently kept the previous rack.
    #[test]
    fn every_phase_answers() {
        for phase in P::ALL {
            let panels = panels_for(phase);
            assert!(!panels.is_empty(), "{phase:?} asked for {panels:?}");
        }
    }
}

#[cfg(test)]
mod grip_tests {
    use super::{Grip, Mods, Panel, Store, Which, drag, grip_at, placeholder};

    const ALL: [Which; 3] = [Which::Eq, Which::Comp, Which::Sat];

    fn rack() -> Panel {
        Panel {
            x: 0.0,
            y: 0.0,
            width: 133.0,
            height: 600.0,
        }
    }

    /// A band is grabbed where it is DRAWN. The layout is shared by the
    /// two, so this asserts they have not drifted apart — which is the
    /// only way a grip can move the wrong band.
    #[test]
    fn a_band_is_grabbed_where_it_is_drawn() {
        let tone = placeholder(0);
        let (which, at) = super::layout(&ALL, rack())
            .into_iter()
            .find(|(which, _)| *which == Which::Eq)
            .expect("an EQ panel");
        assert_eq!(which, Which::Eq);
        let body = super::body_of(at, super::Rack::at(rack().width));
        let freq = fts_audio_ui::axis::FreqAxis::audible();
        let db = fts_audio_ui::axis::DbAxis::symmetric(tone.eq_db_range());
        let band = &tone.eq[2];
        let x = freq.freq_to_x(f64::from(band.frequency), body.x, body.x + body.width);
        let y = db.db_to_y(f64::from(band.gain), body.y, body.y + body.height);
        assert_eq!(grip_at(&ALL, &tone, rack(), super::Folded::default(), x, y), Some(Grip::Band(Which::Eq, 2)));
    }

    /// And a point well away from every band grabs nothing, rather
    /// than the nearest band at any distance or the graph itself.
    ///
    /// It briefly answered `Scale`, so a wheel anywhere over the graph
    /// would zoom. The rack scrolls now, and a panel that swallowed the
    /// wheel would be a hole in the scroll the size of an EQ — so the
    /// empty graph belongs to the gesture that moves the chain, and the
    /// zoom keeps its chip.
    #[test]
    fn empty_space_in_the_eq_grabs_nothing() {
        let tone = placeholder(0);
        // Inside the EQ's own body, far from any band — below the
        // phase's container bar, which owns the first fifteen pixels.
        let y = super::HEAD_H + 14.0;
        assert_eq!(
            grip_at(&ALL, &tone, rack(), super::Folded::default(), 3.0, y),
            None
        );
    }

    /// The zoom chip is at the top middle and wins over whatever is
    /// under it — a band that happened to sit there would otherwise
    /// take every click aimed at the chip.
    #[test]
    fn the_zoom_chip_takes_its_own_clicks() {
        let mut tone = placeholder(0);
        let (_, at) = super::layout(&ALL, rack())
            .into_iter()
            .find(|(which, _)| *which == Which::Eq)
            .expect("an EQ panel");
        let body = super::body_of(at, super::Rack::at(rack().width));
        let chip = super::scale_chip(body);
        assert_eq!(
            grip_at(&ALL, &tone, rack(), super::Folded::default(), chip.x + chip.width / 2.0, chip.y + 2.0),
            Some(Grip::Scale(Which::Eq))
        );
        // It is a switch, so a click acts and a drag does not.
        assert!(Grip::Scale(Which::Eq).is_switch());

        // Clicking cycles the stops and comes back round.
        let start = tone.eq_db_range();
        let steps = eq_ui::eq_graph_model::DB_RANGE_STEPS.len();
        for _ in 0..steps {
            tone.cycle_eq_range();
        }
        assert!((tone.eq_db_range() - start).abs() < f64::EPSILON);
    }

    /// The default is the plugin's own (±6 dB), the wheel walks the
    /// plugin's own stops, and neither end runs away.
    #[test]
    fn the_zoom_walks_the_plugins_stops() {
        let mut tone = placeholder(0);
        assert!((tone.eq_db_range() - eq_ui::eq_graph_model::DEFAULT_DB_RANGE).abs() < f64::EPSILON);

        // Down is out, which is the way a wheel zooms out everywhere.
        super::wheel(&mut tone, Grip::Scale(Which::Eq), Mods::default(), -1.0);
        let out = tone.eq_db_range();
        assert!(out > eq_ui::eq_graph_model::DEFAULT_DB_RANGE, "{out}");
        super::wheel(&mut tone, Grip::Scale(Which::Eq), Mods::default(), 1.0);
        assert!(
            (tone.eq_db_range() - eq_ui::eq_graph_model::DEFAULT_DB_RANGE).abs() < f64::EPSILON
        );

        for _ in 0..20 {
            super::wheel(&mut tone, Grip::Scale(Which::Eq), Mods::default(), -1.0);
        }
        let widest = *eq_ui::eq_graph_model::DB_RANGE_STEPS
            .last()
            .expect("a widest stop");
        assert!((tone.eq_db_range() - widest).abs() < f64::EPSILON);
        for _ in 0..20 {
            super::wheel(&mut tone, Grip::Scale(Which::Eq), Mods::default(), 1.0);
        }
        let tightest = eq_ui::eq_graph_model::DB_RANGE_STEPS[0];
        assert!((tone.eq_db_range() - tightest).abs() < f64::EPSILON);

        // And a double-click puts it back where it started.
        super::reset(&mut tone, Grip::Scale(Which::Eq));
        assert!(
            (tone.eq_db_range() - eq_ui::eq_graph_model::DEFAULT_DB_RANGE).abs() < f64::EPSILON
        );
    }

    /// Zooming the graph must not narrow what the EQ can DO. The
    /// display range and the parameter's limit parted company when the
    /// graph learned to zoom, and a clamp tied to the view would mean
    /// zooming in silently capped the gain.
    #[test]
    fn zooming_in_does_not_clamp_the_gain() {
        let mut tone = placeholder(0);
        tone.eq_range = 0;
        for _ in 0..200 {
            super::wheel(&mut tone, Grip::Band(Which::Eq, 0), Mods::default(), 1.0);
        }
        assert!(
            f64::from(tone.eq[0].gain) > eq_ui::eq_graph_model::DB_RANGE_STEPS[0],
            "a ±3 view capped the gain at ±3: {}",
            tone.eq[0].gain
        );
        assert!(f64::from(tone.eq[0].gain) <= super::EQ_GAIN_LIMIT);
    }

    /// Dragging a band up raises its gain and dragging it right raises
    /// its frequency — the two axes it is drawn against.
    #[test]
    fn a_band_follows_the_pointer() {
        let mut tone = placeholder(0);
        let (before_f, before_g) = (tone.eq[1].frequency, tone.eq[1].gain);
        drag(&mut tone, Grip::Band(Which::Eq, 1), &ALL, rack(), super::Folded::default(), Mods::default(), 12.0, -20.0);
        assert!(tone.eq[1].frequency > before_f, "right is higher");
        assert!(tone.eq[1].gain > before_g, "up is more gain");
    }

    /// A band cannot be dragged out of its own axes.
    #[test]
    fn a_band_stays_inside_the_panel() {
        let mut tone = placeholder(0);
        for _ in 0..50 {
            drag(&mut tone, Grip::Band(Which::Eq, 0), &ALL, rack(), super::Folded::default(), Mods::default(), 400.0, -400.0);
        }
        assert!(tone.eq[0].gain <= super::f64_to_f32(super::EQ_GAIN_LIMIT));
        assert!(tone.eq[0].frequency <= 24_000.0);
        for _ in 0..50 {
            drag(&mut tone, Grip::Band(Which::Eq, 0), &ALL, rack(), super::Folded::default(), Mods::default(), -400.0, 400.0);
        }
        assert!(tone.eq[0].gain >= -super::f64_to_f32(super::EQ_GAIN_LIMIT));
        assert!(tone.eq[0].frequency > 0.0);
    }

    /// Up is a higher threshold — less compression — which is the way
    /// a fader moves for more level.
    #[test]
    fn dragging_the_threshold_up_compresses_less() {
        let mut tone = placeholder(0);
        let before = tone.comp.threshold;
        drag(&mut tone, Grip::Threshold(Which::Comp), &ALL, rack(), super::Folded::default(), Mods::default(), 0.0, -10.0);
        assert!(tone.comp.threshold > before);
        for _ in 0..200 {
            drag(&mut tone, Grip::Threshold(Which::Comp), &ALL, rack(), super::Folded::default(), Mods::default(), 0.0, 40.0);
        }
        assert!(tone.comp.threshold >= -60.0, "the threshold clamps");
    }

    /// Drive clamps at zero rather than going negative, which would
    /// invert the curve.
    #[test]
    fn drive_stays_positive() {
        let mut tone = placeholder(0);
        for _ in 0..200 {
            drag(&mut tone, Grip::Drive, &ALL, rack(), super::Folded::default(), Mods::default(), 0.0, 40.0);
        }
        assert!(tone.sat.drive >= 0.0);
    }

    /// The store hands a track its own settings and keeps them apart —
    /// keyed by GUID, so a preset hiding a track cannot shuffle them.
    #[test]
    fn the_store_keeps_tracks_apart() {
        let rows: Vec<(daw_proto::Track, u32)> = ["a", "b"]
            .into_iter()
            .map(|guid| {
                (
                    daw_proto::Track {
                        guid: guid.to_owned(),
                        ..daw_proto::Track::default()
                    },
                    0,
                )
            })
            .collect();
        let mut store = Store::default();
        store.seed(&rows);
        store.edit("a").expect("a's settings").comp.threshold = -3.0;
        assert!((store.get("a").expect("a").comp.threshold + 3.0).abs() < f32::EPSILON);
        assert!(store.get("b").expect("b").comp.threshold < -3.0);
        assert!(store.get("nonesuch").is_none());
    }

    /// Seeding twice does not reset what was edited in between — the
    /// rows are re-derived on every fold and every preset.
    #[test]
    fn seeding_again_keeps_edits() {
        let rows: Vec<(daw_proto::Track, u32)> = vec![(
            daw_proto::Track {
                guid: "a".to_owned(),
                ..daw_proto::Track::default()
            },
            0,
        )];
        let mut store = Store::default();
        store.seed(&rows);
        store.edit("a").expect("a").sat.drive = 9.0;
        store.seed(&rows);
        assert!((store.get("a").expect("a").sat.drive - 9.0).abs() < f32::EPSILON);
    }
}

#[cfg(test)]
mod tier_tests {
    use super::{Grip, Panel, Which, grip_at, placeholder};

    const ALL: [Which; 3] = [Which::Eq, Which::Comp, Which::Sat];

    fn rack_of(width: f64) -> Panel {
        Panel {
            x: 0.0,
            y: 0.0,
            width,
            height: 600.0,
        }
    }

    /// A handle you cannot see is a handle you cannot aim at. Below the
    /// tier that draws them, the EQ panel grabs nothing — grabbing a
    /// band by accident moves a setting you did not know was there.
    #[test]
    fn bands_are_only_grabbable_where_they_are_drawn() {
        let tone = placeholder(0);
        let wide = rack_of(133.0);
        let narrow = rack_of(super::SHAPE + 1.0);
        assert_eq!(super::Rack::at(narrow.width), super::Rack::Curves);

        // The same band, in both tiers.
        let (_, at) = super::layout(&ALL, wide)
            .into_iter()
            .find(|(which, _)| *which == Which::Eq)
            .expect("an EQ panel");
        let body = super::body_of(at, super::Rack::Full);
        let freq = fts_audio_ui::axis::FreqAxis::audible();
        let db = fts_audio_ui::axis::DbAxis::symmetric(tone.eq_db_range());
        let band = &tone.eq[2];
        let y = db.db_to_y(f64::from(band.gain), body.y, body.y + body.height);

        let x_wide = freq.freq_to_x(f64::from(band.frequency), body.x, body.x + body.width);
        assert_eq!(grip_at(&ALL, &tone, wide, super::Folded::default(), x_wide, y), Some(Grip::Band(Which::Eq, 2)));

        let narrow_body = super::body_of(
            super::layout(&ALL, narrow)
                .into_iter()
                .find(|(which, _)| *which == Which::Eq)
                .expect("an EQ panel")
                .1,
            super::Rack::Curves,
        );
        let x_narrow = freq.freq_to_x(
            f64::from(band.frequency),
            narrow_body.x,
            narrow_body.x + narrow_body.width,
        );
        assert_eq!(grip_at(&ALL, &tone, narrow, super::Folded::default(), x_narrow, y), None);
    }

    /// The compressor and the saturator stay grabbable when the rack
    /// narrows: their curves still show what the drag is doing.
    #[test]
    fn the_single_value_panels_survive_the_narrow_tier() {
        let tone = placeholder(0);
        let narrow = rack_of(super::SHAPE + 1.0);
        let comp = super::layout(&ALL, narrow)
            .into_iter()
            .find(|(which, _)| *which == Which::Comp)
            .expect("a comp panel")
            .1;
        let inside = comp.y + comp.height / 2.0;
        assert_eq!(
            grip_at(&ALL, &tone, narrow, super::Folded::default(), narrow.width / 2.0, inside),
            Some(Grip::Threshold(Which::Comp))
        );
    }

    /// A rack too narrow to draw at all grabs nothing.
    #[test]
    fn an_absent_rack_grabs_nothing() {
        let tone = placeholder(0);
        let off = rack_of(30.0);
        assert_eq!(super::Rack::at(off.width), super::Rack::Off);
        assert_eq!(grip_at(&ALL, &tone, off, super::Folded::default(), 15.0, 300.0), None);
    }
}

#[cfg(test)]
mod reset_tests {
    use super::{Comp, Grip, Mods, Which, drag, placeholder, reset};

    const ALL: [Which; 3] = [Which::Eq, Which::Comp, Which::Sat];

    fn rack() -> super::Panel {
        super::Panel {
            x: 0.0,
            y: 0.0,
            width: 133.0,
            height: 600.0,
        }
    }

    /// Reset undoes a drag, whatever the drag was — which is the whole
    /// promise: you can move something to find out what it does.
    #[test]
    fn reset_undoes_a_drag() {
        let mut tone = placeholder(0);
        let before = tone.eq[1].gain;
        drag(&mut tone, Grip::Band(Which::Eq, 1), &ALL, rack(), super::Folded::default(), Mods::default(), 0.0, -40.0);
        assert!(
            (tone.eq[1].gain - before).abs() > 0.5,
            "the drag moved nothing"
        );
        reset(&mut tone, Grip::Band(Which::Eq, 1));
        assert!(tone.eq[1].gain.abs() < f32::EPSILON, "the band went flat");

        drag(&mut tone, Grip::Threshold(Which::Comp), &ALL, rack(), super::Folded::default(), Mods::default(), 0.0, -30.0);
        reset(&mut tone, Grip::Threshold(Which::Comp));
        assert!(
            (tone.comp.threshold - Comp::default().threshold).abs() < f32::EPSILON,
            "the threshold went back to its default"
        );

        drag(&mut tone, Grip::Drive, &ALL, rack(), super::Folded::default(), Mods::default(), 0.0, -60.0);
        reset(&mut tone, Grip::Drive);
        assert!((tone.sat.drive - 1.0).abs() < f32::EPSILON, "drive is unity");
    }

    /// A band reset keeps its FREQUENCY: where you put it is not the
    /// decision, how much you did there is.
    #[test]
    fn resetting_a_band_keeps_where_it_sits() {
        let mut tone = placeholder(0);
        drag(&mut tone, Grip::Band(Which::Eq, 2), &ALL, rack(), super::Folded::default(), Mods::default(), 20.0, -20.0);
        let moved = tone.eq[2].frequency;
        reset(&mut tone, Grip::Band(Which::Eq, 2));
        assert!((tone.eq[2].frequency - moved).abs() < f32::EPSILON);
    }

    /// A band that is not there is not a panic.
    #[test]
    fn resetting_a_missing_band_is_harmless() {
        let mut tone = placeholder(0);
        reset(&mut tone, Grip::Band(Which::Eq, 99));
    }
}

#[cfg(test)]
mod plugin_interaction_tests {
    use super::{EQ_GAIN_LIMIT, Grip, Mods, Panel, Which, drag, dot_click, grip_at, placeholder, wheel};
    use eq_ui::eq_graph_model::EqBandShape;

    const ALL: [Which; 3] = [Which::Eq, Which::Comp, Which::Sat];

    fn rack() -> Panel {
        Panel {
            x: 7.0,
            y: 11.0,
            width: 133.0,
            height: 600.0,
        }
    }

    /// The rack asks the PLUGIN which band you meant, so a rack and an
    /// editor cannot disagree about it. This pins the delegation: the
    /// answer here is `nearest_band`'s answer, offset by the panel.
    #[test]
    fn the_hit_test_is_the_plugins_own() {
        let tone = placeholder(0);
        let (_, at) = super::layout(&ALL, rack())
            .into_iter()
            .find(|(which, _)| *which == Which::Eq)
            .expect("an EQ panel");
        let body = super::body_of(at, super::Rack::at(rack().width));
        let map = super::mapper(body, tone.eq_db_range());
        for index in [0_usize, 2, 3] {
            let band = &tone.eq[index];
            let x = body.x + map.freq_to_x(f64::from(band.frequency));
            let y = body.y + map.db_to_y(f64::from(band.gain));
            assert_eq!(
                grip_at(&ALL, &tone, rack(), super::Folded::default(), x, y),
                Some(Grip::Band(Which::Eq, index)),
                "band {index} was not found where the plugin puts it"
            );
        }
    }

    /// Alt pins the gain: you found the right amount of cut and now you
    /// are hunting for where it belongs. This is the plugin's rule, and
    /// it used to be "free in both axes, always".
    #[test]
    fn alt_moves_a_band_in_frequency_alone() {
        let mut tone = placeholder(0);
        let (before_f, before_g) = (tone.eq[1].frequency, tone.eq[1].gain);
        let alt = Mods::new(true, false, false);
        drag(&mut tone, Grip::Band(Which::Eq, 1), &ALL, rack(), super::Folded::default(), alt, 15.0, -30.0);
        assert!(tone.eq[1].frequency > before_f, "frequency followed");
        assert!(
            (tone.eq[1].gain - before_g).abs() < f32::EPSILON,
            "gain was pinned"
        );
    }

    /// Cmd turns a vertical drag into resonance, which is the one
    /// gesture that changes a parameter the curve shows only indirectly.
    #[test]
    fn cmd_drags_resonance() {
        let mut tone = placeholder(0);
        let before = tone.eq[1].q;
        let cmd = Mods::new(false, false, true);
        drag(&mut tone, Grip::Band(Which::Eq, 1), &ALL, rack(), super::Folded::default(), cmd, 0.0, -20.0);
        assert!((tone.eq[1].q - before).abs() > f32::EPSILON, "q moved");
        assert!(tone.eq[1].q > 0.0 && tone.eq[1].q <= 18.0, "and stayed sane");
    }

    /// Shift is the fine-tune modifier everywhere, including the two
    /// panels that are not the EQ.
    #[test]
    fn shift_is_fine_everywhere() {
        let coarse = {
            let mut tone = placeholder(0);
            drag(&mut tone, Grip::Threshold(Which::Comp), &ALL, rack(), super::Folded::default(), Mods::default(), 0.0, -20.0);
            tone.comp.threshold
        };
        let fine = {
            let mut tone = placeholder(0);
            let shift = Mods::new(false, true, false);
            drag(&mut tone, Grip::Threshold(Which::Comp), &ALL, rack(), super::Folded::default(), shift, 0.0, -20.0);
            tone.comp.threshold
        };
        let from = placeholder(0).comp.threshold;
        assert!(
            (fine - from).abs() < (coarse - from).abs(),
            "fine moved {} where coarse moved {}",
            fine - from,
            coarse - from
        );
    }

    /// An unmodified wheel over a bell moves its Q — the plugin's
    /// default, because a bell's meaningful width is its resonance.
    #[test]
    fn the_wheel_follows_the_band() {
        let mut tone = placeholder(0);
        tone.eq[1].shape = EqBandShape::Bell;
        let before = tone.eq[1].q;
        wheel(&mut tone, Grip::Band(Which::Eq, 1), Mods::default(), -1.0);
        assert!(tone.eq[1].q > before, "scrolling up tightened it");
    }

    /// And Cmd+wheel moves the gain instead.
    #[test]
    fn cmd_wheel_moves_the_gain() {
        let mut tone = placeholder(0);
        let before = tone.eq[1].gain;
        wheel(&mut tone, Grip::Band(Which::Eq, 1), Mods::new(false, false, true), -1.0);
        assert!(tone.eq[1].gain > before);
        assert!(tone.eq[1].gain <= super::f64_to_f32(EQ_GAIN_LIMIT));
    }

    /// Alt-clicking a band bypasses it, which is the chord the editor
    /// uses and the one that makes A/B-ing a decision possible.
    #[test]
    fn alt_click_bypasses_a_band() {
        let mut tone = placeholder(0);
        assert!(tone.eq[0].enabled);
        assert!(dot_click(&mut tone, Which::Eq, 0, Mods::new(true, false, false)));
        assert!(!tone.eq[0].enabled);
        assert!(dot_click(&mut tone, Which::Eq, 0, Mods::new(true, false, false)));
        assert!(tone.eq[0].enabled);
    }

    /// A plain click is not an action — it is the start of a drag, and
    /// has to fall through so the caller can take hold of the band.
    #[test]
    fn a_plain_click_is_not_an_action() {
        let mut tone = placeholder(0);
        assert!(!dot_click(&mut tone, Which::Eq, 0, Mods::default()));
        assert_eq!(tone.eq[0], placeholder(0).eq[0], "nothing changed");
    }

    /// Alt+Cmd cycles the shape, and the cycle comes back round.
    #[test]
    fn alt_cmd_cycles_the_shape() {
        let mut tone = placeholder(0);
        tone.eq[1].shape = EqBandShape::Bell;
        let chord = Mods::new(true, false, true);
        let mut seen = vec![tone.eq[1].shape];
        for _ in 0..4 {
            assert!(dot_click(&mut tone, Which::Eq, 1, chord));
            seen.push(tone.eq[1].shape);
        }
        assert_eq!(seen.first(), seen.last(), "the cycle returned");
        assert!(seen.len() > 2, "and it passed through others");
    }
}

#[cfg(test)]
mod colour_tests {
    use super::band_color;

    /// The marker's colour is the plugin's, not one invented here — a
    /// band has to be the same colour in the strip as it is in the
    /// plugin window, or the colour is telling you two things.
    #[test]
    fn a_band_takes_the_plugins_own_hue() {
        for hz in [40.0, 250.0, 1_000.0, 4_000.0, 16_000.0] {
            let hex = eq_ui::eq_graph_model::freq_to_color(hz);
            let expected = hex.trim_start_matches('#');
            let got = band_color(hz);
            let [r, g, b, _] = got.to_rgba8().to_u8_array();
            assert_eq!(
                format!("{r:02x}{g:02x}{b:02x}"),
                expected,
                "{hz} Hz did not match the plugin"
            );
        }
    }

    /// And it sweeps, so two bands an octave apart are told apart by it.
    #[test]
    fn the_hue_moves_across_the_spectrum() {
        let low = band_color(60.0).to_rgba8().to_u8_array();
        let high = band_color(12_000.0).to_rgba8().to_u8_array();
        assert_ne!(low, high, "the low and the air band look the same");
    }
}

#[cfg(test)]
mod comp_tests {
    use super::{Comp, Grip, Mods, Panel, Rack, Which, drag, grip_at, placeholder, reset};

    const ALL: [Which; 3] = [Which::Eq, Which::Comp, Which::Sat];

    fn rack() -> Panel {
        Panel {
            x: 0.0,
            y: 0.0,
            width: 133.0,
            height: 600.0,
        }
    }

    fn comp_panel() -> Panel {
        let at = super::layout(&ALL, rack())
            .into_iter()
            .find(|(which, _)| *which == Which::Comp)
            .expect("a comp panel")
            .1;
        super::body_of(at, Rack::at(rack().width))
    }

    /// The threshold is a line across the display, so the display is
    /// what you grab — a line one pixel tall is not something you aim
    /// at, the way a fader's groove is the fader.
    #[test]
    fn the_display_grabs_the_threshold() {
        let tone = placeholder(0);
        let body = comp_panel();
        // Low and left: the glyph lives along the threshold line and
        // the arrow hangs from the middle of it, so the rest of the
        // display is what is being claimed here.
        let inside = body.y + body.height * 0.9;
        assert_eq!(
            grip_at(&ALL, &tone, rack(), super::Folded::default(), body.x + body.width * 0.1, inside),
            Some(Grip::Threshold(Which::Comp))
        );
    }

    /// The glyph's parts are grabbed where they are drawn: the lead-in
    /// ramp is the attack, the lead-out ramp is the release, the arrow
    /// off the line is the ratio. Away from all three the display is
    /// still the threshold's.
    #[test]
    fn the_glyph_grabs_its_own_parts() {
        let mut tone = placeholder(0);
        tone.comp.threshold = -18.0;
        tone.comp.ratio = 6.0;
        tone.comp.attack = 20.0;
        tone.comp.release = 200.0;
        let display = super::comp_split(comp_panel(), Rack::at(rack().width));
        let level = super::threshold_y(tone.comp, display);
        let shape = super::Envelope::of(tone.comp, display, level).expect("a glyph to grab");
        for (grip, edge) in [(Grip::Attack(Which::Comp), shape.fall()), (Grip::Release(Which::Comp), shape.rise())] {
            let mid = (
                f64::midpoint(edge[0].0, edge[1].0),
                f64::midpoint(edge[0].1, edge[1].1),
            );
            assert_eq!(
                grip_at(&ALL, &tone, rack(), super::Folded::default(), mid.0, mid.1),
                Some(grip),
                "{grip:?} at {mid:?}"
            );
        }
        // The arrow, halfway down its own shaft.
        let (shaft, _) = shape.arrow().expect("an arrow to grab");
        assert_eq!(
            grip_at(
                &ALL,
                &tone,
                rack(),
                super::Folded::default(),
                shaft[0].0,
                f64::midpoint(shaft[0].1, shaft[1].1)
            ),
            Some(Grip::Ratio(Which::Comp))
        );
        // Well below the arrow's tip is nothing but the display, which
        // belongs to the threshold.
        assert_eq!(
            grip_at(&ALL, &tone, rack(), super::Folded::default(), shaft[0].0, display.y + display.height - 2.0),
            Some(Grip::Threshold(Which::Comp))
        );
    }

    /// Dragging the threshold DOWN lowers it, because it is drawn on an
    /// axis where down is quieter — the one gesture where following the
    /// pointer is the whole point.
    #[test]
    fn the_threshold_follows_the_pointer_down() {
        let mut tone = placeholder(0);
        let before = tone.comp.threshold;
        drag(&mut tone, Grip::Threshold(Which::Comp), &ALL, rack(), super::Folded::default(), Mods::default(), 0.0, 20.0);
        assert!(tone.comp.threshold < before, "down is a lower threshold");
        drag(&mut tone, Grip::Threshold(Which::Comp), &ALL, rack(), super::Folded::default(), Mods::default(), 0.0, -40.0);
        assert!(tone.comp.threshold > before, "and up is a higher one");
    }

    /// Each edge moves its own parameter and nothing else — including
    /// the threshold, which shares the display with all three.
    #[test]
    fn each_edge_moves_one_thing() {
        for grip in [Grip::Ratio(Which::Comp), Grip::Attack(Which::Comp), Grip::Release(Which::Comp)] {
            let mut tone = placeholder(0);
            let was = tone.comp;
            drag(&mut tone, grip, &ALL, rack(), super::Folded::default(), Mods::default(), 30.0, 30.0);
            let now = tone.comp;
            let moved = [
                (now.ratio - was.ratio).abs() > f32::EPSILON,
                (now.attack - was.attack).abs() > f32::EPSILON,
                (now.release - was.release).abs() > f32::EPSILON,
            ];
            assert_eq!(moved.iter().filter(|m| **m).count(), 1, "{grip:?}");
            assert!(
                (now.threshold - was.threshold).abs() < f32::EPSILON,
                "{grip:?} moved the threshold"
            );
        }
    }

    /// Attack and release are logarithmic — a millisecond matters at
    /// the fast end and twenty do not at the slow — so the same drag
    /// moves far less down there.
    #[test]
    fn the_times_are_logarithmic() {
        let step = |from: f32| {
            let mut tone = placeholder(0);
            tone.comp.attack = from;
            drag(&mut tone, Grip::Attack(Which::Comp), &ALL, rack(), super::Folded::default(), Mods::default(), 15.0, 0.0);
            tone.comp.attack - from
        };
        assert!(step(1.0) < step(100.0), "the fast end moves in smaller steps");
    }

    /// Every control clamps to its own range rather than running away.
    ///
    /// Each is driven in the direction its own edge runs: the times are
    /// widths, so right is longer; the floor hangs, so down is harder.
    #[test]
    fn the_controls_clamp() {
        let mut tone = placeholder(0);
        for _ in 0..80 {
            drag(&mut tone, Grip::Ratio(Which::Comp), &ALL, rack(), super::Folded::default(), Mods::default(), 0.0, 60.0);
            drag(&mut tone, Grip::Attack(Which::Comp), &ALL, rack(), super::Folded::default(), Mods::default(), 60.0, 0.0);
            drag(&mut tone, Grip::Release(Which::Comp), &ALL, rack(), super::Folded::default(), Mods::default(), 60.0, 0.0);
        }
        assert!((tone.comp.ratio - 20.0).abs() < 0.01, "{}", tone.comp.ratio);
        assert!((tone.comp.attack - 200.0).abs() < 0.5, "{}", tone.comp.attack);
        assert!((tone.comp.release - 3_000.0).abs() < 5.0, "{}", tone.comp.release);

        let mut back = placeholder(0);
        for _ in 0..80 {
            drag(&mut back, Grip::Ratio(Which::Comp), &ALL, rack(), super::Folded::default(), Mods::default(), 0.0, -60.0);
            drag(&mut back, Grip::Attack(Which::Comp), &ALL, rack(), super::Folded::default(), Mods::default(), -60.0, 0.0);
            drag(&mut back, Grip::Release(Which::Comp), &ALL, rack(), super::Folded::default(), Mods::default(), -60.0, 0.0);
        }
        assert!((back.comp.ratio - 1.0).abs() < 0.01, "{}", back.comp.ratio);
        assert!((back.comp.attack - 0.1).abs() < 0.01, "{}", back.comp.attack);
        assert!((back.comp.release - 5.0).abs() < 0.01, "{}", back.comp.release);
    }

    /// Each control moves the way its own edge runs: you pull the floor
    /// DOWN for a harder ratio, because down is more reduction and that
    /// is the direction the signal goes; and you drag either time to
    /// the RIGHT for a longer one, because both are durations on one
    /// horizontal axis and neither is the opposite of the other.
    #[test]
    fn each_control_follows_its_own_edge() {
        let was = placeholder(0).comp;

        let mut tone = placeholder(0);
        drag(&mut tone, Grip::Ratio(Which::Comp), &ALL, rack(), super::Folded::default(), Mods::default(), 0.0, 20.0);
        assert!(tone.comp.ratio > was.ratio, "down did not harden the ratio");

        let mut tone = placeholder(0);
        drag(&mut tone, Grip::Attack(Which::Comp), &ALL, rack(), super::Folded::default(), Mods::default(), 20.0, 0.0);
        assert!(tone.comp.attack > was.attack, "right did not lengthen the attack");

        let mut tone = placeholder(0);
        drag(&mut tone, Grip::Release(Which::Comp), &ALL, rack(), super::Folded::default(), Mods::default(), 20.0, 0.0);
        assert!(
            tone.comp.release > was.release,
            "right did not lengthen the release"
        );
    }

    /// The glyph's depth is how much the ratio takes off a full-scale
    /// signal, so it grows with the ratio and vanishes at unity — where
    /// the compressor is taking nothing off and a drawn reduction would
    /// be claiming otherwise.
    #[test]
    fn the_depth_measures_what_the_ratio_does() {
        let at = |ratio: f32| {
            let mut comp = Comp::default();
            comp.threshold = -20.0;
            comp.ratio = ratio;
            super::ratio_drop(comp, 150.0)
        };
        assert!(at(1.0) < 0.01, "unity drew a reduction");
        assert!(at(4.0) > at(2.0));
        assert!(at(12.0) > at(4.0));
        // And it saturates, which is what the effect does — the header
        // carries the number for the cases where that matters.
        assert!(at(20.0) - at(12.0) < at(4.0) - at(2.0));
    }

    /// The line lands where the pointer put it. One split decides
    /// where the display is, so the pixel the line is drawn at and the
    /// decibel the drag produces cannot disagree — a threshold running
    /// ahead of the line you are holding is the bug this rules out.
    #[test]
    fn the_threshold_tracks_the_pointer() {
        let mut tone = placeholder(0);
        let body = comp_panel();
        let rack_tier = Rack::at(rack().width);
        let display = super::comp_split(body, rack_tier);
        let at_db = |comp: Comp| {
            display.y + comp_ui::comp_graph_svg::db_to_y(f64::from(comp.threshold), display.height)
        };
        let before = at_db(tone.comp);
        let moved = 24.0;
        drag(&mut tone, Grip::Threshold(Which::Comp), &ALL, rack(), super::Folded::default(), Mods::default(), 0.0, moved);
        let after = at_db(tone.comp);
        assert!(
            (after - before - moved).abs() < 0.5,
            "the line moved {} for a drag of {moved}",
            after - before
        );
    }

    /// And each resets to its own default.
    #[test]
    fn each_knob_resets_to_its_own_default() {
        let mut tone = placeholder(0);
        for grip in [Grip::Ratio(Which::Comp), Grip::Attack(Which::Comp), Grip::Release(Which::Comp), Grip::Threshold(Which::Comp)] {
            drag(&mut tone, grip, &ALL, rack(), super::Folded::default(), Mods::default(), 0.0, -25.0);
            reset(&mut tone, grip);
        }
        let default = Comp::default();
        assert!((tone.comp.ratio - default.ratio).abs() < f32::EPSILON);
        assert!((tone.comp.attack - default.attack).abs() < f32::EPSILON);
        assert!((tone.comp.release - default.release).abs() < f32::EPSILON);
        assert!((tone.comp.threshold - default.threshold).abs() < f32::EPSILON);
    }
}

/// A track's analyser, and the rack drawn from it.
///
/// The spectrum is the one part of a rack that moves with the audio,
/// and the plugin paints the whole graph in one call — so a moving
/// spectrum makes the WHOLE rack live, which at sixty strips is four
/// milliseconds a frame to redraw four curves that did not change.
///
/// The engine publishes at about 30 Hz and the window draws at 240, so
/// seven frames in eight are redrawing the same picture. This keeps the
/// picture: the rack is rebuilt when the spectrum is REPLACED, and
/// replayed otherwise, which is the same trade the recorded scene makes
/// one level up.
#[derive(Clone, Debug, Default)]
pub struct Analyser {
    meters: Meters,
    /// The rack as last built, and the box it was built for.
    built: Option<(f64, f64, std::sync::Arc<Scene>)>,
}

impl Analyser {
    /// Replace the meters, which invalidates the picture.
    pub fn set(&mut self, meters: Meters) {
        self.meters = meters;
        self.built = None;
    }

    /// Throw the picture away without touching the bins.
    ///
    /// For the other reason a rack changes: a setting moved. The cache
    /// is keyed on the spectrum because that is what usually changes,
    /// but a knob turned or a processor switched out changes the same
    /// picture — and a rack that waited for the next meter frame to
    /// notice would lag a gesture by a thirtieth of a second.
    pub fn invalidate(&mut self) {
        self.built = None;
    }

    #[must_use]
    pub fn bins(&self) -> &[f32] {
        &self.meters.spectrum
    }

    /// Everything the rack has to show that moves.
    #[must_use]
    pub const fn meters(&self) -> &Meters {
        &self.meters
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.meters.is_empty()
    }

    /// The rack for this box, built once per spectrum.
    ///
    /// `build` is only called on a miss, so the caller may do the full
    /// plugin paint inside it without thinking about how often.
    pub fn rack(
        &mut self,
        width: f64,
        height: f64,
        build: impl FnOnce() -> Scene,
    ) -> std::sync::Arc<Scene> {
        if let Some((w, h, scene)) = &self.built
            && (w - width).abs() < 0.5
            && (h - height).abs() < 0.5
        {
            return std::sync::Arc::clone(scene);
        }
        let built = std::sync::Arc::new(build());
        self.built = Some((width, height, std::sync::Arc::clone(&built)));
        built
    }
}

/// What the compressor's display is showing, over time.
///
/// The threshold is a line across a level axis, which only means
/// anything if there are levels on it. This is the history the line is
/// read against: one input peak per meter frame, newest last.
///
/// A ring in the sense that it drops the oldest rather than growing —
/// a mixer left open for an hour would otherwise hold a hundred
/// thousand floats per track for a display two hundred pixels wide.
#[derive(Clone, Debug, Default)]
pub struct Levels {
    peaks: std::collections::VecDeque<f32>,
    /// The de-esser's deepest cut per frame, in dB — the fire lane.
    fired: std::collections::VecDeque<f32>,
    /// The input trace, built once per CHANGE rather than once per
    /// frame.
    ///
    /// The engine publishes levels at about 30 Hz and the window draws
    /// at whatever the display does — 240 here. Rebuilding a
    /// Catmull-Rom through seventy points, as a string, and parsing it
    /// back, on every frame for every strip, was a third of a
    /// millisecond across sixty racks to redraw a line that had not
    /// moved seven frames out of eight.
    ///
    /// Keyed by the box it was built for, because the path is in the
    /// panel's own pixels: a mixer that resizes has to rebuild, and a
    /// mixer that does not never does.
    path: Option<(f64, f64, std::sync::Arc<BezPath>)>,
    /// And the gain reduction, which depends on the COMPRESSOR as well
    /// as on the levels — so it carries the settings it was built for
    /// and rebuilds when a knob moves.
    reduction: Option<(f64, f64, Comp, std::sync::Arc<BezPath>)>,
}

/// How often the level history is published, in hertz.
///
/// The engine's meter rate, which the simulation matches. The
/// reduction's envelope is computed per sample of that history, so it
/// has to know how long a sample is — a 3ms attack and a 33ms sample
/// means the reduction arrives inside one step, and pretending
/// otherwise would draw every compressor as slow.
pub const PUBLISH_HZ: u32 = 30;

/// How many frames of level the display holds.
///
/// Long enough to see a phrase arrive and short enough that the
/// transient you just played is still on the screen.
///
/// Not more: the trace is the one thing in the rack that is rebuilt
/// every frame, and the plugin's `smooth_path` is a Catmull-Rom through
/// every sample — so this number is multiplied by the strip count on
/// every frame the mixer draws. Doubling it cost half a millisecond
/// across sixty racks and bought a smoother line nobody could see,
/// because a strip is a hundred and thirty pixels wide and this is
/// already more samples than it has columns at a glance width.
pub const HISTORY: usize = 72;

impl Levels {
    /// Record one frame's peak.
    ///
    /// A reading equal to the last one is dropped: a silent track
    /// publishes the same zero thirty times a second, and rebuilding
    /// its trace for that is the cost this cache exists to remove.
    pub fn push(&mut self, peak: f32) {
        let peak = peak.clamp(0.0, 1.0);
        if self.peaks.back().is_some_and(|last| {
            (last - peak).abs() < 1e-4 && self.peaks.len() >= HISTORY
        }) {
            return;
        }
        if self.peaks.len() >= HISTORY {
            self.peaks.pop_front();
        }
        self.peaks.push_back(peak);
        self.path = None;
        self.reduction = None;
    }

    /// Record how hard the de-esser fired this frame, in dB.
    ///
    /// Kept in step with the peaks — one entry per frame — so the fire
    /// lane and the level trace above it share a time axis.
    pub fn push_fire(&mut self, db: f32) {
        if self.fired.len() >= HISTORY {
            self.fired.pop_front();
        }
        self.fired.push_back(db.max(0.0));
    }

    /// The peaks, oldest first.
    pub fn peaks(&self) -> impl Iterator<Item = f32> + '_ {
        self.peaks.iter().copied()
    }

    /// The input trace, for a display of this size.
    ///
    /// Built through the plugin's own `smooth_path` — Catmull-Rom, the
    /// shape its editor draws the input wave with — and kept until the
    /// levels or the size change. Up from the bottom, because that is
    /// what a level is.
    fn path(&mut self, width: f64, height: f64) -> Option<std::sync::Arc<BezPath>> {
        if let Some((w, h, path)) = &self.path
            && (w - width).abs() < 0.5
            && (h - height).abs() < 0.5
        {
            return Some(std::sync::Arc::clone(path));
        }
        let d = comp_ui::comp_graph_svg::smooth_path(&self.scaled(), width, height, true, true);
        let built = std::sync::Arc::new(BezPath::from_svg(&d).ok()?);
        self.path = Some((width, height, std::sync::Arc::clone(&built)));
        Some(built)
    }

    /// How much the compressor took off each of those samples, in dB.
    ///
    /// The TARGET comes from the plugin's own `compress_transfer` — the
    /// same function the threshold marker and the ratio arrow are drawn
    /// from, so the amount the display says is being removed is the
    /// amount the maths says. What it does between targets is the
    /// attack and the release.
    ///
    /// Which is the whole reason those two have no control of their
    /// own any more. They were knobs, then they were curves in the
    /// margins, and both were a description of something the panel was
    /// already drawing in the right axes and the right place — a
    /// reduction, over time. A trace that snapped to its target was a
    /// compressor with no attack and no release, and the numbers beside
    /// it were claiming otherwise.
    #[must_use]
    pub fn reduction_db(&self, comp: Comp) -> Vec<f32> {
        // One-pole coefficients, per sample of history. The history is
        // published at the engine's rate, so a sample is about 33ms —
        // which is coarse next to a 3ms attack, and exactly why the
        // coefficient is computed from the interval rather than
        // assumed: a time constant shorter than the frame rate has to
        // arrive within one sample, not over three.
        let step = 1000.0 / f64::from(crate::tone::PUBLISH_HZ);
        let coefficient = |ms: f32| {
            let ms = f64::from(ms).max(0.01);
            crate::mcp::f64_to_f32((-step / ms).exp())
        };
        let (attack, release) = (coefficient(comp.attack), coefficient(comp.release));

        let mut held = 0.0_f32;
        self.peaks
            .iter()
            .map(|&peak| {
                let target = if peak <= 0.0 {
                    0.0
                } else {
                    let db = 20.0 * peak.log10();
                    let out = comp_ui::comp_graph_svg::compress_transfer(
                        db,
                        comp.threshold,
                        comp.ratio,
                        comp.knee,
                    );
                    (db - out).max(0.0)
                };
                // Toward more reduction at the attack rate, back toward
                // none at the release rate. A compressor's asymmetry is
                // the whole of what those two words mean.
                let rate = if target > held { attack } else { release };
                held = target + (held - target) * rate;
                held
            })
            .collect()
    }

    /// The gain-reduction trace, hanging DOWN from the top.
    ///
    /// Down because that is the direction the compressor moves the
    /// signal, and because the input is already coming up from the
    /// floor — two traces growing the same way would be two readings
    /// of one thing rather than a cause and its effect.
    fn reduction_path(
        &mut self,
        comp: Comp,
        width: f64,
        height: f64,
    ) -> Option<std::sync::Arc<BezPath>> {
        if let Some((w, h, was, path)) = &self.reduction
            && (w - width).abs() < 0.5
            && (h - height).abs() < 0.5
            && *was == comp
        {
            return Some(std::sync::Arc::clone(path));
        }
        let gr = comp_ui::comp_graph_svg::scale_gr_wave(&self.reduction_db(comp));
        if gr.iter().all(|v| *v <= f32::EPSILON) {
            return None;
        }
        let d = comp_ui::comp_graph_svg::smooth_path(&gr, width, height, false, true);
        let built = std::sync::Arc::new(BezPath::from_svg(&d).ok()?);
        self.reduction = Some((width, height, comp, std::sync::Arc::clone(&built)));
        Some(built)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.peaks.is_empty()
    }

    /// The history as the plugin's own display scale wants it.
    ///
    /// `scale_input_peak` maps a linear peak through dB, so the trace
    /// is log-scaled the way the compressor's editor draws it — and the
    /// way the threshold line above it is positioned.
    #[must_use]
    pub fn scaled(&self) -> Vec<f32> {
        let raw: Vec<f32> = self.peaks.iter().copied().collect();
        comp_ui::comp_graph_svg::scale_input_wave(&raw)
    }
}

/// Draw the level history into a compressor panel.
///
/// Live, not recorded: this is the one part of the rack that changes
/// thirty times a second, and re-recording a mixer for it would be the
/// whole panel's cost to move one trace. The well, the ladder, the
/// threshold line and the knobs stay in the recording underneath.
///
/// The path is the plugin's — `smooth_path` is what its own editor
/// draws the input wave with, Catmull-Rom through the samples — so the
/// trace has the same shape in a strip as in the plugin window.
pub fn levels(
    scene: &mut Scene,
    panels: &[Which],
    levels: &mut Levels,
    tone: &Tone,
    panel: Panel,
    folded: Folded,
) {
    if levels.is_empty() {
        return;
    }
    let rack = Rack::at(panel.width);
    if !rack.on() {
        return;
    }
    // Both compressors and the gate: all three read a level against a
    // threshold, and all three are unreadable without the level. It
    // used to draw one panel because there used to be one.
    for which in [Which::Gate, Which::RescueComp, Which::Comp] {
        // A bypassed processor is not processing, so it has no display:
        // the scrim over it says so, and a waveform moving under it
        // would say the opposite.
        if tone.bypass.is(which) {
            continue;
        }
        let comp = match which {
            Which::RescueComp => tone.rescue_comp,
            Which::Comp => tone.comp,
            // The gate's trace is the level alone — what it does to the
            // signal is open or shut, not an amount, so there is no
            // reduction curve to draw under it.
            _ => Comp {
                ratio: 1.0,
                ..tone.comp
            },
        };
        one_level(scene, panels, levels, comp, which, panel, folded, rack);
    }
    // The two lanes: when the gate was open, and when the de-esser
    // fired. Both are strips of time under a display, built from the
    // same history the trace above them is, so a segment in the lane
    // is under the hit that opened it.
    lanes(scene, panels, levels, tone, panel, folded, rack);
}

/// The door lane and the fire lane.
fn lanes(
    scene: &mut Scene,
    panels: &[Which],
    levels: &Levels,
    tone: &Tone,
    panel: Panel,
    folded: Folded,
    rack: Rack,
) {
    for (which, at) in units(panels, panel, folded) {
        if !matches!(which, Which::Gate | Which::DeEss | Which::DeEssIn) || tone.bypass.is(which) {
            continue;
        }
        let body = body_of(at, rack);
        let Some(lane) = lane_of(body, which, rack) else {
            continue;
        };
        if lane.width < 2.0 || lane.height < 2.0 {
            continue;
        }
        let per = lane.width / crate::num::coord(HISTORY);
        if which == Which::Gate {
            {
                // Open where the level is over the threshold, and for
                // the hold after it drops back, and ramping shut over
                // the release — the setting drawn where it acts.
                let tint = palette_open();
                let step_ms = 1000.0 / f64::from(PUBLISH_HZ);
                let hold = f64::from(tone.gate.hold) / step_ms;
                let release = f64::from(tone.gate.release) / step_ms;
                let mut open_until = -1.0_f64;
                let mut path = BezPath::new();
                let mut run: Option<f64> = None;
                let count = levels.peaks.len();
                let offset = HISTORY.saturating_sub(count);
                for (i, peak) in levels.peaks().enumerate() {
                    let i_f = crate::num::coord(i);
                    let db = if peak <= 0.0 { -120.0 } else { 20.0 * f64::from(peak).log10() };
                    if db > f64::from(tone.gate.threshold) {
                        open_until = i_f + hold;
                    }
                    let open = i_f <= open_until;
                    let x = crate::num::coord(i.saturating_add(offset)).mul_add(per, lane.x);
                    match (open, run) {
                        (true, None) => run = Some(x),
                        (false, Some(from)) => {
                            let to = release.min(6.0).mul_add(per, x);
                            path.move_to((from, lane.y + lane.height - 1.0));
                            path.line_to((from + 1.0, lane.y + 1.0));
                            path.line_to((x, lane.y + 1.0));
                            path.line_to((to.min(lane.x + lane.width), lane.y + lane.height - 1.0));
                            path.close_path();
                            run = None;
                        }
                        _ => {}
                    }
                }
                if let Some(from) = run {
                    path.extend(Rect::new(from, lane.y + 1.0, lane.x + lane.width, lane.y + lane.height - 1.0)
                        .to_path(0.1).elements().iter().copied());
                }
                if !path.is_empty() {
                    scene.fill(Fill::NonZero, Affine::IDENTITY, tint.multiply_alpha(0.85), None, &path);
                }
            }
        } else {
            {
                let red = hex(comp_ui::comp_graph_svg::colors::REDUCTION_EDGE);
                let count = levels.fired.len();
                let offset = HISTORY.saturating_sub(count);
                let mut path = BezPath::new();
                for (i, db) in levels.fired.iter().enumerate() {
                    if *db <= 0.05 {
                        continue;
                    }
                    let h = (f64::from(*db) / 9.0).clamp(0.0, 1.0) * (lane.height - 2.0);
                    let x = crate::num::coord(i.saturating_add(offset)).mul_add(per, lane.x);
                    path.extend(Rect::new(x, lane.y + lane.height - 1.0 - h, x + per - 0.3, lane.y + lane.height - 1.0)
                        .to_path(0.1).elements().iter().copied());
                }
                if !path.is_empty() {
                    scene.fill(Fill::NonZero, Affine::IDENTITY, red.multiply_alpha(0.85), None, &path);
                }
            }
        }
    }
}

/// The colour a gate's open segments are lit in.
///
/// The Balance phase's green: "on", in the same hue the rail says it.
fn palette_open() -> Color {
    phase_tint(session::mix_phases::MixPhase::Balance)
}

/// One processor's level trace.
fn one_level(
    scene: &mut Scene,
    panels: &[Which],
    levels: &mut Levels,
    comp: Comp,
    which: Which,
    panel: Panel,
    folded: Folded,
    rack: Rack,
) {
    let Some((_, at)) = units(panels, panel, folded)
        .into_iter()
        .find(|(at_which, _)| *at_which == which)
    else {
        return;
    };
    let body = body_of(at, rack);
    // The display is the part above the knobs — the same split `comp`
    // makes, so the trace lands on the axis the threshold is on.
    // The display, between the two ramps — the trace has to land on the
    // axis the threshold line is on.
    let body = comp_split(body, rack);
    if body.width < 2.0 || body.height < 2.0 {
        return;
    }
    let at = Affine::translate((body.x, body.y));
    // The signal, in the compressor's own readout grey, up from the
    // floor. Grey because it is the thing being acted ON — it carries
    // no state of its own, and every coloured thing on this panel means
    // something. The plugin's grey rather than one chosen here, so a
    // waveform in a strip and a waveform in the editor are the same
    // shade of not-a-decision.
    if let Some(path) = levels.path(body.width, body.height) {
        // Filled, because what you are reading is how much of the
        // display the signal takes up against the line across it — an
        // outline makes that a comparison of two lines instead. Dim,
        // because the threshold is drawn over it and a solid fill makes
        // the line the thing you cannot see.
        let grey = hex(comp_ui::comp_graph_svg::colors::GREY);
        scene.fill(Fill::NonZero, at, grey.multiply_alpha(0.30), None, path.as_ref());
        scene.stroke(&Stroke::new(1.0), at, grey.multiply_alpha(0.9), None, path.as_ref());
    }
    // And what the compressor took off, red, hanging down from the top.
    // Down is the direction it moves the signal; red because it is the
    // one thing on this panel that is a REDUCTION, and it shares its
    // colour with the threshold that caused it.
    if let Some(path) = levels.reduction_path(comp, body.width, body.height) {
        let fill = hex(comp_ui::comp_graph_svg::colors::REDUCTION_FILL);
        let edge = hex(comp_ui::comp_graph_svg::colors::REDUCTION_EDGE);
        scene.fill(Fill::NonZero, at, fill.multiply_alpha(0.45), None, path.as_ref());
        scene.stroke(&Stroke::new(1.0), at, edge, None, path.as_ref());
    }
}

#[cfg(test)]
mod level_tests {
    use super::{HISTORY, Levels};

    /// The history is bounded: a mixer left open for an hour holds a
    /// display's worth per track, not an hour's.
    #[test]
    fn the_history_is_bounded() {
        let mut levels = Levels::default();
        for i in 0..HISTORY * 4 {
            levels.push((i % 100) as f32 / 100.0);
        }
        assert_eq!(levels.scaled().len(), HISTORY);
    }

    /// Empty is empty — a track that has never played draws nothing
    /// rather than a flat line at the floor, which would read as
    /// silence rather than as no data.
    #[test]
    fn nothing_recorded_is_nothing_drawn() {
        assert!(Levels::default().is_empty());
        assert!(Levels::default().scaled().is_empty());
    }

    /// And the scale is the plugin's, so the trace and the threshold
    /// line above it are on one axis.
    #[test]
    fn the_trace_uses_the_plugins_scale() {
        let mut levels = Levels::default();
        levels.push(1.0);
        levels.push(0.5);
        let got = levels.scaled();
        let want = comp_ui::comp_graph_svg::scale_input_wave(&[1.0, 0.5]);
        assert_eq!(got, want);
    }
}

#[cfg(test)]
mod reduction_tests {
    use super::{Comp, Levels};

    fn hits() -> Levels {
        let mut levels = Levels::default();
        // A loud hit decaying into a quiet floor, twice.
        for _ in 0..2 {
            for i in 0..20_u8 {
                levels.push(0.7 * (-f32::from(i) / 6.0).exp() + 0.004);
            }
        }
        levels
    }

    /// The reduction is what the compressor is taking off, computed
    /// through its own transfer — so a loud sample above the threshold
    /// reduces and a quiet stretch below it recovers.
    ///
    /// Recovers rather than stops: the trace is an envelope follower
    /// now, so it lets go at the release rate instead of snapping to
    /// nothing the instant the signal drops. That is the whole reason
    /// the attack and release are visible at all.
    #[test]
    fn what_crosses_the_threshold_reduces_and_the_rest_recovers() {
        let comp = Comp {
            threshold: -12.0,
            ratio: 4.0,
            knee: 0.0,
            release: 60.0,
            ..Comp::default()
        };
        let gr = hits().reduction_db(comp);
        let peak = gr.iter().copied().fold(0.0_f32, f32::max);
        let quiet = gr.iter().copied().fold(f32::MAX, f32::min);
        assert!(peak > 1.0, "nothing was reduced: {gr:?}");
        assert!(quiet < peak / 4.0, "nothing recovered: {gr:?}");
        assert!(gr.iter().all(|g| *g >= 0.0), "a reduction went negative");
    }

    /// The release is what the recovery takes, so a slower one is still
    /// holding reduction where a faster one has let go. This is the
    /// claim the envelope glyph makes about its lead-out edge.
    #[test]
    fn a_slower_release_holds_the_reduction_longer() {
        let at = |release: f32| {
            let comp = Comp {
                threshold: -12.0,
                ratio: 4.0,
                knee: 0.0,
                release,
                ..Comp::default()
            };
            hits()
                .reduction_db(comp)
                .iter()
                .copied()
                .fold(f32::MAX, f32::min)
        };
        assert!(at(2_000.0) > at(30.0), "{} vs {}", at(2_000.0), at(30.0));
    }

    /// A harder ratio takes more off the same signal. This is the claim
    /// the display makes every time a knob moves, and the one that
    /// would go unnoticed if the trace were computed from anything but
    /// the plugin's own transfer.
    #[test]
    fn a_harder_ratio_reduces_more() {
        let at = |ratio: f32| {
            let comp = Comp {
                threshold: -18.0,
                ratio,
                knee: 0.0,
                ..Comp::default()
            };
            hits().reduction_db(comp).iter().copied().fold(0.0, f32::max)
        };
        assert!(at(8.0) > at(2.0), "{} was not more than {}", at(8.0), at(2.0));
    }

    /// A ratio of one is no compressor at all, so nothing hangs from
    /// the top — and the path is `None` rather than a flat nothing,
    /// which is what keeps a silent panel from paying for a trace.
    #[test]
    fn unity_reduces_nothing() {
        let comp = Comp {
            threshold: -30.0,
            ratio: 1.0,
            knee: 0.0,
            ..Comp::default()
        };
        let gr = hits().reduction_db(comp);
        assert!(gr.iter().all(|g| *g < 0.01), "1:1 reduced something");
        assert!(hits().reduction_path(comp, 100.0, 50.0).is_none());
    }

    /// The two traces grow in opposite directions: the signal up from
    /// the floor, the reduction down from the top. Two growing the same
    /// way would be two readings of one thing rather than a cause and
    /// its effect.
    #[test]
    fn the_traces_grow_apart() {
        let comp = Comp {
            threshold: -20.0,
            ratio: 6.0,
            knee: 0.0,
            ..Comp::default()
        };
        let mut levels = hits();
        let (w, h) = (120.0, 60.0);
        let input = levels.path(w, h).expect("an input trace");
        let gr = levels.reduction_path(comp, w, h).expect("a reduction trace");
        let mean_y = |path: &vello::kurbo::BezPath| {
            let points: Vec<f64> = path
                .elements()
                .iter()
                .filter_map(|e| match e {
                    vello::kurbo::PathEl::MoveTo(p) | vello::kurbo::PathEl::LineTo(p) => Some(p.y),
                    vello::kurbo::PathEl::CurveTo(_, _, p) => Some(p.y),
                    _ => None,
                })
                .collect();
            points.iter().sum::<f64>() / points.len().max(1) as f64
        };
        assert!(
            mean_y(&input) > mean_y(&gr),
            "the input sat above the reduction"
        );
    }
}

#[cfg(test)]
mod hex_tests {
    use super::hex;

    /// The plugin states its colours as hex because its UIs are DOMs.
    /// This is the one place they become paint, so it has to read them
    /// exactly — a waveform a shade off is a second table that has
    /// already started drifting.
    #[test]
    fn a_plugin_colour_survives_the_trip() {
        for value in [
            comp_ui::comp_graph_svg::colors::GREY,
            comp_ui::comp_graph_svg::colors::REDUCTION_EDGE,
            comp_ui::comp_graph_svg::colors::THRESHOLD,
        ] {
            let [r, g, b, a] = hex(value).to_rgba8().to_u8_array();
            assert_eq!(format!("#{r:02x}{g:02x}{b:02x}"), value);
            assert_eq!(a, 0xff, "a stated colour is opaque; alpha is the caller's");
        }
    }

    /// Anything that is not a colour is visibly not one, rather than
    /// nothing — a rack that drew nothing would look like a rack with
    /// no signal.
    #[test]
    fn a_broken_value_is_still_visible() {
        for bad in ["", "#", "nonesuch", "#12"] {
            let [r, g, b, a] = hex(bad).to_rgba8().to_u8_array();
            assert_eq!((r, g, b, a), (0x88, 0x88, 0x88, 0xff));
        }
    }
}

#[cfg(test)]
mod bypass_tests {
    use super::{Bypass, Grip, Mods, Panel, Rack, Which, drag, grip_at, placeholder, reset};

    const ALL: [Which; 3] = [Which::Eq, Which::Comp, Which::Sat];

    fn rack() -> Panel {
        Panel {
            x: 0.0,
            y: 0.0,
            width: 133.0,
            height: 600.0,
        }
    }

    /// Per PROCESSOR, because the question is almost always "what does
    /// this sound like without the compressor", not "without any of it".
    #[test]
    fn each_processor_switches_out_alone() {
        let mut bypass = Bypass::default();
        assert!(!bypass.any());
        bypass.toggle(Which::Comp);
        assert!(bypass.is(Which::Comp));
        assert!(!bypass.is(Which::Eq) && !bypass.is(Which::Sat));
        assert!(bypass.any());
        bypass.toggle(Which::Comp);
        assert!(!bypass.any());
    }

    /// The header is the switch, and it is above the body — so a click
    /// there is a bypass rather than whatever the body would have done.
    #[test]
    fn the_header_is_the_switch() {
        let tone = placeholder(0);
        for which in ALL {
            let at = super::layout(&ALL, rack())
                .into_iter()
                .find(|(w, _)| *w == which)
                .expect("a panel")
                .1;
            // A pixel inside the panel but above its body.
            let y = at.y + 3.0;
            assert_eq!(
                grip_at(&ALL, &tone, rack(), super::Folded::default(), at.x + at.width / 2.0, y),
                Some(Grip::Bypass(which)),
                "{which:?}"
            );
        }
    }

    /// A bypassed panel has ONE control left, and it is the one that
    /// brings it back: grabbing a band through the scrim would move a
    /// setting that is having no effect.
    #[test]
    fn a_bypassed_panel_only_offers_its_way_back() {
        let mut tone = placeholder(0);
        let body = super::body_of(
            super::layout(&ALL, rack())
                .into_iter()
                .find(|(w, _)| *w == Which::Eq)
                .expect("an EQ panel")
                .1,
            Rack::at(rack().width),
        );
        let map = super::mapper(body, tone.eq_db_range());
        let band = &tone.eq[2];
        let x = body.x + map.freq_to_x(f64::from(band.frequency));
        let y = body.y + map.db_to_y(f64::from(band.gain));
        assert_eq!(grip_at(&ALL, &tone, rack(), super::Folded::default(), x, y), Some(Grip::Band(Which::Eq, 2)));

        tone.bypass.toggle(Which::Eq);
        assert_eq!(
            grip_at(&ALL, &tone, rack(), super::Folded::default(), x, y),
            Some(Grip::Bypass(Which::Eq)),
            "a band was still grabbable through the scrim"
        );
    }

    /// A switch has no drag and no turn — dragging off one is how you
    /// change your mind about pressing it.
    #[test]
    fn a_switch_does_not_drag() {
        let mut tone = placeholder(0);
        let was = tone.bypass;
        let grip = Grip::Bypass(Which::Sat);
        assert!(grip.is_switch());
        drag(&mut tone, grip, &ALL, rack(), super::Folded::default(), Mods::default(), 30.0, -30.0);
        assert_eq!(tone.bypass, was, "a drag flipped a switch");
        super::wheel(&mut tone, grip, Mods::default(), -1.0);
        assert_eq!(tone.bypass, was, "a wheel flipped a switch");
    }

    /// And resetting one switches it back — which is what a
    /// double-click would have done anyway.
    #[test]
    fn resetting_a_switch_flips_it() {
        let mut tone = placeholder(0);
        tone.bypass.toggle(Which::Eq);
        reset(&mut tone, Grip::Bypass(Which::Eq));
        assert!(!tone.bypass.is(Which::Eq));
    }
}

#[cfg(test)]
mod character_tests {
    use super::placeholder;

    /// Each track gets its OWN chain, not one curve with a wobble in
    /// it: a rack of racks has to read as a set of decisions.
    #[test]
    fn neighbouring_tracks_get_different_chains() {
        let chains: Vec<_> = (0..4).map(placeholder).collect();
        for (i, a) in chains.iter().enumerate() {
            for b in chains.iter().skip(i + 1) {
                assert!(
                    a.eq.iter().zip(&b.eq).any(|(x, y)| {
                        (x.frequency - y.frequency).abs() > 1.0
                            || (x.gain - y.gain).abs() > 0.5
                    }),
                    "two chains had the same EQ"
                );
                assert!(
                    (a.comp.attack - b.comp.attack).abs() > 0.1
                        || (a.comp.ratio - b.comp.ratio).abs() > 0.1,
                    "two chains had the same compressor"
                );
            }
        }
    }

    /// And the difference is a decision, not noise: the voice that
    /// lives at the bottom lifts there, and the one that lives at the
    /// top cuts there.
    #[test]
    fn a_low_voice_and_a_high_one_disagree_about_the_bottom() {
        let low = placeholder(0);
        let high = placeholder(2);
        assert!(low.eq[0].gain > 0.0, "the low voice cut its own register");
        assert!(high.eq[0].gain < 0.0, "the high voice kept the mud");
        assert!(
            high.eq[3].gain > low.eq[3].gain,
            "the high voice was darker than the low one"
        );
    }

    /// Nothing arrives bypassed — a rack you have to switch on before
    /// it does anything is a rack that looks broken.
    #[test]
    fn a_new_chain_is_running() {
        assert!(!placeholder(3).bypass.any());
    }
}

#[cfg(test)]
mod face_tests {
    //! The faces the six units grew: each grip is where its picture is.
    use super::{
        ALL_PANELS, Folded, Grip, Mods, Panel, Rack, RoomGeometry, Side, Which, body_of, display_of,
        drag, echo_taps, grip_at, lane_of, placeholder, reset, sat_split, units, wheel,
    };
    use crate::live::Meters;

    fn rack() -> Panel {
        Panel {
            x: 0.0,
            y: 0.0,
            width: 133.0,
            height: 2_400.0,
        }
    }

    fn body(which: Which) -> Panel {
        let (_, at) = units(&ALL_PANELS, rack(), Folded::default())
            .into_iter()
            .find(|(w, _)| *w == which)
            .expect("the unit is in the chain");
        body_of(at, Rack::at(rack().width))
    }

    fn grip(x: f64, y: f64, tone: &super::Tone) -> Option<Grip> {
        grip_at(&ALL_PANELS, tone, rack(), Folded::default(), x, y)
    }

    /// The gate's table: its three edges are its three times, and the
    /// range line is its own grip where it is clear of the threshold.
    #[test]
    fn the_gates_table_is_grabbed_by_its_edges() {
        let tone = placeholder(0);
        let at = display_of(body(Which::Gate), Which::Gate, Rack::Full);
        let to_y = |db: f64| at.y + comp_ui::comp_graph_svg::db_to_y(db, at.height);
        let line = to_y(f64::from(tone.gate.threshold));
        let floor = to_y(f64::from(tone.gate.threshold + tone.gate.range));
        let shape = super::GateGlyph::of(tone.gate, at, line, floor).expect("room for a glyph");
        assert_eq!(shape.grip_at(shape.open.0, shape.open.1 + 1.0), Some(Grip::Attack(Which::Gate)));
        let mid_run = (f64::midpoint(shape.open.0, shape.close.0), shape.open.1);
        assert_eq!(shape.grip_at(mid_run.0, mid_run.1), Some(Grip::Hold(Which::Gate)));
        assert_eq!(shape.grip_at(shape.end.0, shape.end.1 - 1.0), Some(Grip::Release(Which::Gate)));
        // The range line, well away from the glyph.
        assert_eq!(grip(at.x + at.width - 4.0, floor, &tone), Some(Grip::Range));
        // And the threshold everywhere else.
        assert_eq!(grip(at.x + at.width - 4.0, at.y + 2.0, &tone), Some(Grip::Threshold(Which::Gate)));
    }

    /// Dragging the hold right lengthens it; resetting puts it back.
    #[test]
    fn the_hold_is_dragged_sideways() {
        let mut tone = placeholder(0);
        let was = tone.gate.hold;
        drag(&mut tone, Grip::Hold(Which::Gate), &ALL_PANELS, rack(), Folded::default(), Mods::default(), 20.0, 0.0);
        assert!(tone.gate.hold > was, "{} should exceed {was}", tone.gate.hold);
        reset(&mut tone, Grip::Hold(Which::Gate));
        assert!((tone.gate.hold - super::Gate::default().hold).abs() < f32::EPSILON);
    }

    /// The first repeat is the time; a later one is the feedback; the
    /// glyph cycles the machine.
    #[test]
    fn the_delays_repeats_are_its_grips() {
        let mut tone = placeholder(1);
        let at = display_of(body(Which::Delay), Which::Delay, Rack::Full);
        let taps = echo_taps(tone.delay, at);
        assert!(taps.len() >= 3, "enough repeats to grab: {}", taps.len());
        let y = at.y + at.height / 2.0;
        assert_eq!(grip(taps[1].0, y, &tone), Some(Grip::Time));
        assert_eq!(grip(taps[2].0, y, &tone), Some(Grip::Feedback));
        let was = tone.delay.time;
        drag(&mut tone, Grip::Time, &ALL_PANELS, rack(), Folded::default(), Mods::default(), 10.0, 0.0);
        assert!(tone.delay.time > was);
        let fb = tone.delay.feedback;
        drag(&mut tone, Grip::Feedback, &ALL_PANELS, rack(), Folded::default(), Mods::default(), 0.0, -20.0);
        assert!(tone.delay.feedback > fb, "up is more feedback");
        // The spacing follows the time.
        let after = echo_taps(tone.delay, at);
        assert!((after[1].0 - after[0].0 - (taps[1].0 - taps[0].0)).abs() < 1e-6, "the window scales with the time, so the first gap holds its share");
        // The header glyph is the family switch.
        let (_, unit) = units(&ALL_PANELS, rack(), Folded::default())
            .into_iter()
            .find(|(w, _)| *w == Which::Delay)
            .expect("a delay");
        assert_eq!(grip(unit.x + 4.0, unit.y + 4.0, &tone), Some(Grip::Family(Which::Delay)));
        let style = tone.delay.style;
        tone.delay.cycle_style();
        assert_ne!(tone.delay.style, style);
        for _ in 0..20 {
            tone.delay.cycle_style();
        }
    }

    /// The reverb: the gap rule is the predelay, the far end the decay,
    /// the rest the mix — and the decay drag lengthens the tail.
    #[test]
    fn the_reverbs_tail_is_its_grips() {
        let mut tone = placeholder(2);
        let at = display_of(body(Which::Reverb), Which::Reverb, Rack::Full);
        let geometry = RoomGeometry::of(tone.reverb, at);
        let y = at.y + at.height / 2.0;
        assert_eq!(grip(geometry.start, y, &tone), Some(Grip::Predelay));
        assert_eq!(grip(at.x + at.width - 5.0, y, &tone), Some(Grip::Decay));
        assert_eq!(grip(at.width.mul_add(0.4, at.x), y, &tone), Some(Grip::Mix(Which::Reverb)));
        let was = tone.reverb.decay;
        drag(&mut tone, Grip::Decay, &ALL_PANELS, rack(), Folded::default(), Mods::default(), 30.0, 0.0);
        assert!(tone.reverb.decay > was);
        wheel(&mut tone, Grip::Predelay, Mods::default(), -1.0);
        assert!(tone.reverb.predelay > super::Room::default().predelay - 1.0);
        let algorithm = tone.reverb.algorithm;
        tone.reverb.cycle_algorithm();
        assert_ne!(tone.reverb.algorithm, algorithm);
        for _ in 0..20 {
            tone.reverb.cycle_algorithm();
        }
    }

    /// The selector strip under a delay or a reverb: one chip per
    /// family, and a click on one picks a machine from it — the next
    /// member if you are already there.
    #[test]
    fn the_selector_picks_a_family() {
        let mut tone = placeholder(0);
        let strip = lane_of(body(Which::Delay), Which::Delay, Rack::Full).expect("a selector");
        // Chip 1 is tape.
        let x = super::CHIP.mul_add(1.5, strip.x + 2.0);
        assert_eq!(grip(x, strip.y + 4.0, &tone), Some(Grip::Choose(Which::Delay, 1)));
        assert!(Grip::Choose(Which::Delay, 1).is_switch());
        tone.delay.choose_family(1);
        assert_eq!(tone.delay.family(), delay_dsp::engine::Family::Tape);
        // Rhythmic has three machines: clicking again walks them.
        tone.delay.choose_family(4);
        let first = tone.delay.style;
        tone.delay.choose_family(4);
        assert_ne!(tone.delay.style, first);
        assert_eq!(tone.delay.family(), delay_dsp::engine::Family::Rhythmic);
        // Past the chips is the bypass, not a chip.
        assert_eq!(grip(strip.x + strip.width - 2.0, strip.y + 4.0, &tone), Some(Grip::Bypass(Which::Delay)));
        // And the reverb's picks an algorithm of the family.
        let strip = lane_of(body(Which::Reverb), Which::Reverb, Rack::Full).expect("a selector");
        assert_eq!(grip(super::CHIP.mul_add(2.5, strip.x + 2.0), strip.y + 4.0, &tone), Some(Grip::Choose(Which::Reverb, 2)));
        tone.reverb.choose_family(2);
        assert_eq!(tone.reverb.family(), reverb_dsp::algorithm::Family::Plate);
        tone.reverb.choose_family(99);
        assert_eq!(tone.reverb.family(), reverb_dsp::algorithm::Family::Plate);
    }

    /// The saturator's selector picks a circuit through the plugin's
    /// own rail rule, and the digital circuit draws steps.
    #[test]
    fn the_saturators_selector_picks_a_circuit() {
        let mut tone = placeholder(0);
        let strip = lane_of(body(Which::Sat), Which::Sat, Rack::Full).expect("a selector");
        assert_eq!(grip(super::CHIP.mul_add(4.5, strip.x + 2.0), strip.y + 4.0, &tone), Some(Grip::Choose(Which::Sat, 4)));
        let drive = tone.sat.drive;
        tone.choose_sat_family(4);
        assert_eq!(tone.sat_circuit(), saturate_dsp::preamp::Circuit::Steps);
        assert_eq!(saturate_profiles::PROFILES[tone.sat_profile].name, "Clip");
        tone.choose_sat_family(4);
        assert_eq!(saturate_profiles::PROFILES[tone.sat_profile].name, "Bitcrush");
        assert!(!tone.sat_digital.is_transparent(), "a bitcrusher quantises");
        tone.choose_sat_family(0);
        assert_eq!(tone.sat_circuit(), saturate_dsp::preamp::Circuit::Valve);
        assert!(tone.sat_digital.is_transparent());
        // The drive knob rides across: a transformer at its drive is a
        // triode at the same knob, not the same gain.
        assert!(tone.sat.drive > 1.0 && (tone.sat.drive - drive).abs() < 8.0);
        tone.cycle_sat();
        assert_eq!(saturate_profiles::PROFILES[tone.sat_profile].name, "Pentode");
    }

    /// A suppressor: edges where they are drawn, then threshold, depth,
    /// sharpness down the display.
    #[test]
    fn a_suppressor_is_three_bands_and_two_edges() {
        let mut tone = placeholder(3);
        let at = display_of(body(Which::DeEss), Which::DeEss, Rack::Full);
        let zoom = super::SuppressZoom::of(tone.de_ess);
        let low_x = zoom.x_of(f64::from(tone.de_ess.low), at);
        assert_eq!(grip(low_x, at.y + 10.0, &tone), Some(Grip::Edge(Which::DeEss, Side::Low)));
        let mid_x = at.x + at.width / 2.0;
        assert_eq!(grip(mid_x, at.y + 2.0, &tone), Some(Grip::Threshold(Which::DeEss)));
        assert_eq!(grip(mid_x, at.y + at.height / 2.0, &tone), Some(Grip::Depth(Which::DeEss)));
        assert_eq!(grip(mid_x, at.y + at.height - 2.0, &tone), Some(Grip::Sharpness(Which::DeEss)));
        // The lane under it is the depth for a de-esser and the
        // sharpness for the resonance panel.
        let lane = lane_of(body(Which::DeEss), Which::DeEss, Rack::Full).expect("a fire lane");
        assert_eq!(grip(mid_x, lane.y + 2.0, &tone), Some(Grip::Depth(Which::DeEss)));
        let was = tone.de_ess.low;
        wheel(&mut tone, Grip::Edge(Which::DeEss, Side::Low), Mods::default(), -1.0);
        assert!(tone.de_ess.low > was, "the wheel moves the edge up");
        assert!(tone.de_ess.low < tone.de_ess.high);
        let depth = tone.de_ess.depth;
        drag(&mut tone, Grip::Depth(Which::DeEss), &ALL_PANELS, rack(), Folded::default(), Mods::default(), 0.0, 20.0);
        assert!(tone.de_ess.depth > depth, "down is deeper");
    }

    /// The saturator: the ladder is the tilt, the centre the bias, the
    /// curve the drive — and a signal lights the curve.
    #[test]
    fn the_saturators_ladder_and_centre_are_grips() {
        let mut tone = placeholder(0);
        let at = display_of(body(Which::Sat), Which::Sat, Rack::Full);
        let (curve, ladder) = sat_split(at, Rack::Full);
        let ladder = ladder.expect("a ladder at a working width");
        assert_eq!(grip(ladder.x + 3.0, ladder.y + 3.0, &tone), Some(Grip::Tilt));
        assert_eq!(grip(curve.x + curve.width / 2.0, curve.y + 5.0, &tone), Some(Grip::Bias));
        assert_eq!(grip(curve.x + 3.0, curve.y + 5.0, &tone), Some(Grip::Drive));
        drag(&mut tone, Grip::Bias, &ALL_PANELS, rack(), Folded::default(), Mods::default(), 15.0, 0.0);
        assert!(tone.sat.q_point > 0.25);
        let tilt = tone.sat.tilt_db();
        wheel(&mut tone, Grip::Tilt, Mods::default(), -1.0);
        assert!(tone.sat.tilt_db() > tilt, "up is toward the highs");
        reset(&mut tone, Grip::Tilt);
        assert!(tone.sat.tilt_db().abs() < f32::EPSILON);

        // Drawn with a signal, the rack has more in it than without:
        // the lit reach, the ribbon, the wet fill.
        let palette = crate::arrangement::Palette::from_theme(&daw_ui::theming::Theme::dark());
        let font = crate::text::Font::embedded().expect("the embedded font");
        let mut still = anyrender::Scene::new();
        super::draw(&mut still, &palette, &font, &tone, &Meters::default(), &ALL_PANELS, rack(), Folded::default(), None);
        let mut moving = anyrender::Scene::new();
        let meters = crate::simulate::meters(0, 1.25, &tone);
        super::draw(&mut moving, &palette, &font, &tone, &meters, &ALL_PANELS, rack(), Folded::default(), None);
        assert!(moving.commands.len() > still.commands.len());
    }

    /// The machines say which they are: three units carry a glyph, and
    /// two of them switch on it.
    #[test]
    fn three_units_carry_a_machine_glyph() {
        let tone = placeholder(0);
        let with: Vec<Which> = ALL_PANELS.iter().copied().filter(|w| w.glyph(&tone).is_some()).collect();
        assert_eq!(with, vec![Which::Sat, Which::Delay, Which::Reverb]);
        assert!(Which::Sat.glyph_switches() && Which::Delay.glyph_switches() && Which::Reverb.glyph_switches());
        assert!(!Which::Gate.glyph_switches());
    }

    /// The lanes draw from the history: a gate fed hits above its
    /// threshold shows open segments, a de-esser fed fires shows them.
    #[test]
    fn the_lanes_draw_the_history() {
        let tone = placeholder(0);
        let mut levels = super::Levels::default();
        let mut silent = anyrender::Scene::new();
        super::lanes(&mut silent, &ALL_PANELS, &levels, &tone, rack(), Folded::default(), Rack::Full);
        for i in 0..40 {
            levels.push(if i % 8 < 3 { 0.5 } else { 0.001 });
            levels.push_fire(if i % 8 == 1 { 6.0 } else { 0.0 });
        }
        let mut busy = anyrender::Scene::new();
        super::lanes(&mut busy, &ALL_PANELS, &levels, &tone, rack(), Folded::default(), Rack::Full);
        assert!(busy.commands.len() >= silent.commands.len() + 2, "a door lane and a fire lane");
    }

    /// Every grip names a panel that is in the chain.
    #[test]
    fn every_grip_belongs_to_a_unit() {
        for grip in [
            Grip::Hold(Which::Gate),
            Grip::Range,
            Grip::Bias,
            Grip::Tilt,
            Grip::Depth(Which::Resonance),
            Grip::Sharpness(Which::DeEss),
            Grip::Edge(Which::DeEss, Side::High),
            Grip::Time,
            Grip::Feedback,
            Grip::Decay,
            Grip::Predelay,
            Grip::Mix(Which::Sat),
            Grip::Family(Which::Reverb),
        ] {
            assert!(ALL_PANELS.contains(&grip.panel()), "{grip:?}");
        }
        assert!(Grip::Family(Which::Delay).is_switch());
        assert!(!Grip::Time.is_switch());
    }
}
