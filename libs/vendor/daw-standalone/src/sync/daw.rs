//! `Standalone` root handle and shared state types for the sync backend.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use daw_proto::automation::{
    AutomationItem, Envelope, EnvelopePoint, EnvelopeRef, EnvelopeType, SendEnvelopeKind,
    TakeEnvelopeKind,
};
use daw_proto::midi::MidiNote;
use daw_proto::primitives::AutomationMode;
use daw_proto::{
    Daw, DawError, DawResult, Fx, FxChainContext, Item, LastTouchedFx, Marker, ProjectInfo,
    RecordInput, Region, Take, TempoPoint, Track, TrackRoute, Transport as TransportState,
};

/// Hashable key derived from `FxChainContext` for use as a HashMap key.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum FxChainKey {
    Track(String),
    Input(String),
    Monitoring,
}

impl From<&FxChainContext> for FxChainKey {
    fn from(ctx: &FxChainContext) -> Self {
        match ctx {
            FxChainContext::Track(g) => FxChainKey::Track(g.clone()),
            FxChainContext::Input(g) => FxChainKey::Input(g.clone()),
            FxChainContext::Monitoring => FxChainKey::Monitoring,
        }
    }
}

/// Track-side entry for an item. Stores the item plus its current track guid
/// (so cross-track lookups by item guid can find the owning track).
#[derive(Clone, Debug)]
pub struct ItemEntry {
    pub item: Item,
}

/// All takes for an item, plus the active take's index.
#[derive(Clone, Debug, Default)]
pub struct TakeList {
    pub active_idx: u32,
    pub takes: Vec<Take>,
}

/// Per-FX state stored in the chain. Keeps the public `Fx` plus an internal
/// state-chunk blob and parameter map (param idx -> normalized value).
#[derive(Clone, Debug)]
pub struct FxEntry {
    pub fx: Fx,
    pub state_chunk: String,
    pub params: HashMap<u32, f64>,
}

/// Per-track properties that aren't carried on the proto `Track` struct.
/// REAPER models these as `I_NCHAN`, `I_RECINPUT`, etc.; we store them
/// alongside the proto `Track` so routing/recording code can read them.
#[derive(Clone, Debug)]
pub struct TrackExt {
    /// Channel count. REAPER caps at 128. Standalone allows any
    /// `1..=128` (not constrained to stereo pairs). Default = 2.
    pub num_channels: u32,
    /// Record input source. `RecordInput::None` if none configured.
    pub record_input: RecordInput,
    /// Whether this track sends its output to the master/parent track
    /// (REAPER's `B_MAINSEND`). Default = true.
    pub parent_send_enabled: bool,
    /// TCP height override in pixels. `0` means host/default height.
    pub tcp_height_pixels: u32,
}

/// Stable hashable identity for an envelope on a track. Lifts
/// `EnvelopeRef`'s string-y / nested shape into something usable as a
/// `HashMap` key.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum EnvelopeKey {
    /// One of the predefined track envelopes (volume/pan/etc).
    Track(EnvelopeType),
    /// An FX parameter automation envelope.
    FxParam { fx_guid: String, param_index: u32 },
    /// Per-send automation. `send_index` is the index in the source
    /// track's send list at envelope-creation time; stable until the
    /// list is reordered.
    Send {
        send_index: u32,
        kind: SendEnvelopeKind,
    },
    /// Per-take automation. Keyed by `(item, take, kind)`. The
    /// owning project + track is implicit (resolved via item lookup).
    Take {
        item_guid: String,
        take_guid: String,
        kind: TakeEnvelopeKind,
    },
    /// Free-form by name (rare; ReaScript-style).
    Named(String),
}

impl EnvelopeKey {
    pub fn from_ref(env_ref: &EnvelopeRef) -> Self {
        match env_ref {
            EnvelopeRef::Type(t) => Self::Track(*t),
            EnvelopeRef::FxParam {
                fx_guid,
                param_index,
            } => Self::FxParam {
                fx_guid: fx_guid.clone(),
                param_index: *param_index,
            },
            EnvelopeRef::Send { send_index, kind } => Self::Send {
                send_index: *send_index,
                kind: *kind,
            },
            EnvelopeRef::Take {
                item_guid,
                take_guid,
                kind,
            } => Self::Take {
                item_guid: item_guid.clone(),
                take_guid: take_guid.clone(),
                kind: *kind,
            },
            EnvelopeRef::ByName(n) => Self::Named(n.clone()),
        }
    }
}

/// Per-envelope state. Points are kept sorted by time.
#[derive(Clone, Debug)]
pub struct EnvelopeData {
    pub visible: bool,
    pub armed: bool,
    pub automation_mode: AutomationMode,
    /// A lane of its own under the track, rather than overlaid.
    pub in_own_lane: bool,
    /// That lane's height in pixels.
    pub lane_height: u32,
    pub points: Vec<EnvelopePoint>,
    /// Automation items on this envelope, with their own curves.
    ///
    /// The curve lives beside the item because standalone has no pool
    /// store: two instances of one pool carry equal copies, and the
    /// setter writes through every instance sharing the id — which is
    /// pooling's observable behaviour without a second index.
    pub automation_items: Vec<(AutomationItem, Vec<EnvelopePoint>)>,
}

impl EnvelopeData {
    pub fn new() -> Self {
        Self {
            visible: false,
            armed: false,
            automation_mode: AutomationMode::TrimRead,
            in_own_lane: false,
            lane_height: 0,
            points: Vec::new(),
            automation_items: Vec::new(),
        }
    }

    /// Build a proto `Envelope` snapshot for this data + identity.
    pub fn to_proto(&self, track_guid: &str, key: &EnvelopeKey) -> Envelope {
        let (envelope_type, fx_guid, param_index, name) = match key {
            EnvelopeKey::Track(t) => (*t, None, None, envelope_default_name(*t).to_string()),
            EnvelopeKey::FxParam {
                fx_guid,
                param_index,
            } => (
                EnvelopeType::FxParam,
                Some(fx_guid.clone()),
                Some(*param_index),
                format!("FX {}: param {}", fx_guid, param_index),
            ),
            EnvelopeKey::Send { send_index, kind } => (
                // Map send kinds onto the closest track-envelope type
                // for the snapshot's `envelope_type` discriminator.
                match kind {
                    SendEnvelopeKind::Volume => EnvelopeType::Volume,
                    SendEnvelopeKind::Pan => EnvelopeType::Pan,
                    SendEnvelopeKind::Mute => EnvelopeType::Mute,
                },
                None,
                None,
                format!("Send {} {:?}", send_index, kind),
            ),
            EnvelopeKey::Take {
                item_guid: _,
                take_guid: _,
                kind,
            } => (
                // Take envelope kinds don't map onto track envelope
                // types cleanly (no Pitch type on the track side).
                // Surface as Volume + a name suffix so the discriminator
                // is at least non-FxParam.
                match kind {
                    TakeEnvelopeKind::Pan => EnvelopeType::Pan,
                    TakeEnvelopeKind::Mute => EnvelopeType::Mute,
                    _ => EnvelopeType::Volume,
                },
                None,
                None,
                format!("Take {:?}", kind),
            ),
            EnvelopeKey::Named(n) => (EnvelopeType::Volume, None, None, n.clone()),
        };
        Envelope {
            track_guid: track_guid.to_string(),
            envelope_type,
            name,
            fx_guid,
            param_index,
            visible: self.visible,
            armed: self.armed,
            automation_mode: self.automation_mode,
            in_own_lane: self.in_own_lane,
            lane_height: self.lane_height,
            automation_item_count: self.automation_items.len() as u32,
            point_count: self.points.len() as u32,
        }
    }
}

impl Default for EnvelopeData {
    fn default() -> Self {
        Self::new()
    }
}

fn envelope_default_name(t: EnvelopeType) -> &'static str {
    match t {
        EnvelopeType::Volume => "Volume",
        EnvelopeType::VolumePrefx => "Volume (Pre-FX)",
        EnvelopeType::Pan => "Pan",
        EnvelopeType::PanPrefx => "Pan (Pre-FX)",
        EnvelopeType::Width => "Width",
        EnvelopeType::WidthPrefx => "Width (Pre-FX)",
        EnvelopeType::Mute => "Mute",
        EnvelopeType::FxParam => "FX Param",
        EnvelopeType::PlayRate => "Playrate",
        EnvelopeType::Pitch => "Pitch",
        // A kind the enum does not name yet — the `.daw` format carries the
        // originating name alongside, so this default is only ever seen by
        // an envelope created without one.
        EnvelopeType::Custom => "Envelope",
    }
}

impl Default for TrackExt {
    fn default() -> Self {
        Self {
            num_channels: 2,
            record_input: RecordInput::None,
            parent_send_enabled: true,
            tcp_height_pixels: 0,
        }
    }
}

/// Per-project in-memory state.
pub struct ProjectState {
    pub info: ProjectInfo,
    pub transport: TransportState,
    pub regions: BTreeMap<u32, Region>,
    pub markers: BTreeMap<u32, Marker>,
    pub tempo_points: Vec<TempoPoint>,
    pub tracks: Vec<Track>,
    /// Per-track extended properties (channel count, record input) keyed
    /// by track GUID. Lazily populated on first mutation; missing keys
    /// resolve to `TrackExt::default()`.
    pub track_ext: HashMap<String, TrackExt>,
    /// MIDI note data keyed by take GUID. Only populated for MIDI takes.
    /// Notes are stored in insertion order; the `index` field on
    /// each `MidiNote` mirrors its position in the vec (rewritten on
    /// add/delete to keep references stable within a single read).
    pub midi_notes: HashMap<String, Vec<MidiNote>>,
    /// Per-take MIDI Control Change events, keyed by take guid.
    /// CC streams feed plugins per-block alongside note events.
    pub midi_ccs: HashMap<String, Vec<daw_proto::midi::MidiCC>>,
    /// Per-take MIDI Pitch Bend events.
    pub midi_pitch_bends: HashMap<String, Vec<daw_proto::midi::MidiPitchBend>>,
    /// Per-take MIDI Program Change events.
    pub midi_program_changes: HashMap<String, Vec<daw_proto::midi::MidiProgramChange>>,
    /// Per-take MIDI System Exclusive events (full F0..F7 frames).
    pub midi_sysex: HashMap<String, Vec<daw_proto::midi::MidiSysEx>>,
    /// Stretch markers per take guid, kept sorted by position.
    ///
    /// Keyed by take rather than item because a take is what carries
    /// timing: two takes of one item are two performances and warp
    /// independently.
    pub stretch_markers: HashMap<String, Vec<daw_proto::StretchMarker>>,
    /// Per-take stretch algorithm.
    pub stretch_modes: HashMap<String, daw_proto::StretchMode>,
    /// Take markers per take guid, kept sorted by source position.
    ///
    /// Annotations on the audio itself — a breath here, a sibilant
    /// there, a rating on a comp take. Anchored in *source* time so
    /// they stay on the sound when the item is trimmed or moved.
    pub take_markers: HashMap<String, Vec<daw_proto::TakeMarker>>,
    /// Per-take channel-pressure (mono aftertouch) events.
    pub midi_channel_pressures: HashMap<String, Vec<daw_proto::midi::MidiChannelPressure>>,
    /// Per-take poly-pressure (per-note aftertouch) events.
    pub midi_poly_pressures: HashMap<String, Vec<daw_proto::midi::MidiPolyPressure>>,
    /// Per-take MPE-style note-expression events (volume/pan/tuning/
    /// vibrato/expression/brightness/pressure per voice).
    pub midi_note_expressions: HashMap<String, Vec<daw_proto::midi::MidiNoteExpression>>,
    /// Automation envelopes keyed by `(track_guid, EnvelopeKey)`. Points
    /// are kept sorted by time within each envelope.
    pub envelopes: HashMap<(String, EnvelopeKey), EnvelopeData>,
    /// Global automation override (`None` = no override active).
    pub global_automation_override: Option<AutomationMode>,
    /// Project Media/FX Bay persistent state (retained paths +
    /// bay folder layout). `None` until the bay is first used.
    pub bay_state: Option<crate::media_bay::BayState>,
    /// Decoded audio sources keyed by take GUID. Populated by
    /// `crate::audio_engine::materialize::materialize_audio` (or
    /// directly via `attach_audio_source`). Read by the mixer during
    /// playback. Wrapped in `Arc` so the mixer can hold cheap refs.
    #[cfg(any(feature = "audio", feature = "decode"))]
    pub audio_sources: HashMap<String, std::sync::Arc<crate::audio_engine::AudioSource>>,
    pub next_region_id: u32,
    pub next_marker_id: u32,
    /// Per-track ext state keyed by `(track_guid, section, key)`.
    pub track_ext_state: HashMap<(String, String, String), String>,
    /// Project-scoped ext state keyed by `(section, key)`. Mirrors
    /// REAPER's GetProjExtState/SetProjExtState semantics — these
    /// values live with the project, not the host.
    pub project_ext_state: HashMap<(String, String), String>,

    // Phase 2 storage ─────────────────────────────────────────────────────
    /// FX chains keyed by their context.
    pub fx_chains: HashMap<FxChainKey, Vec<FxEntry>>,
    /// Active preset index per FX guid. Tracks what the last
    /// `Effects::set_preset` call selected — no introspection of
    /// what presets the underlying plugin offers.
    pub fx_preset_index: HashMap<String, u32>,
    /// Named-config key/value store per FX, keyed by `(fx_guid, key)`.
    /// Mirrors REAPER's `TrackFX_GetNamedConfigParm` so clients that
    /// stash plugin-specific metadata can round-trip through standalone.
    pub fx_named_config: HashMap<(String, String), String>,
    /// Items keyed by item guid.
    pub items: HashMap<String, ItemEntry>,
    /// Per-track item ordering. `track_guid -> [item_guid, ...]`.
    pub items_by_track: HashMap<String, Vec<String>>,
    /// Takes keyed by item guid.
    pub takes: HashMap<String, TakeList>,
    /// Sends keyed by source track guid.
    pub sends: HashMap<String, Vec<TrackRoute>>,
    /// Receives keyed by destination track guid.
    pub receives: HashMap<String, Vec<TrackRoute>>,
    /// Hardware outputs keyed by source track guid.
    pub hw_outputs: HashMap<String, Vec<TrackRoute>>,
    /// Counter for synthesizing item GUIDs.
    pub next_item_counter: u64,
    /// Counter for synthesizing FX GUIDs.
    pub next_fx_counter: u64,
    /// Counter for synthesizing take GUIDs.
    pub next_take_counter: u64,
    /// Mutation generation — bumped by every `with_project_mut` /
    /// `write_project` pass. Readers (the audio renderer's snapshot
    /// cache) compare revisions instead of re-walking the state.
    pub revision: u64,
    /// Master output gain (linear, 1.0 = unity) — RPP `MASTER_VOLUME`.
    pub master_volume: f64,
    /// Master pan (−1…1).
    pub master_pan: f64,
    /// Master mute — RPP `MASTERMUTESOLO` bit 0.
    pub master_muted: bool,
}

impl ProjectState {
    pub fn new(info: ProjectInfo) -> Self {
        Self {
            info,
            transport: TransportState::new(),
            regions: BTreeMap::new(),
            markers: BTreeMap::new(),
            tempo_points: Vec::new(),
            tracks: Vec::new(),
            track_ext: HashMap::new(),
            midi_notes: HashMap::new(),
            midi_ccs: HashMap::new(),
            midi_pitch_bends: HashMap::new(),
            midi_program_changes: HashMap::new(),
            midi_sysex: HashMap::new(),
            stretch_markers: HashMap::new(),
            stretch_modes: HashMap::new(),
            take_markers: HashMap::new(),
            midi_channel_pressures: HashMap::new(),
            midi_poly_pressures: HashMap::new(),
            midi_note_expressions: HashMap::new(),
            envelopes: HashMap::new(),
            global_automation_override: None,
            bay_state: None,
            #[cfg(any(feature = "audio", feature = "decode"))]
            audio_sources: HashMap::new(),
            next_region_id: 0,
            next_marker_id: 0,
            track_ext_state: HashMap::new(),
            project_ext_state: HashMap::new(),
            fx_chains: HashMap::new(),
            fx_preset_index: HashMap::new(),
            fx_named_config: HashMap::new(),
            items: HashMap::new(),
            items_by_track: HashMap::new(),
            takes: HashMap::new(),
            sends: HashMap::new(),
            receives: HashMap::new(),
            hw_outputs: HashMap::new(),
            next_item_counter: 0,
            next_fx_counter: 0,
            next_take_counter: 0,
            revision: 0,
            master_volume: 1.0,
            master_pan: 0.0,
            master_muted: false,
        }
    }
}

/// The part of a project an *edit* touches, cloned whole for undo.
///
/// Not the whole [`ProjectState`]: transport, FX chains, routing and the
/// bay are not edit targets and some of them are not `Clone`. Everything
/// here is maps of plain data plus `Arc`s, so a snapshot is cheap — the
/// audio itself is refcounted, never copied.
#[derive(Clone)]
pub struct EditSnapshot {
    tracks: Vec<Track>,
    items: HashMap<String, ItemEntry>,
    items_by_track: HashMap<String, Vec<String>>,
    takes: HashMap<String, TakeList>,
    stretch_markers: HashMap<String, Vec<daw_proto::StretchMarker>>,
    stretch_modes: HashMap<String, daw_proto::StretchMode>,
    take_markers: HashMap<String, Vec<daw_proto::TakeMarker>>,
    midi_notes: HashMap<String, Vec<MidiNote>>,
    midi_ccs: HashMap<String, Vec<daw_proto::midi::MidiCC>>,
    envelopes: HashMap<(String, EnvelopeKey), EnvelopeData>,
    #[cfg(any(feature = "audio", feature = "decode"))]
    audio_sources: HashMap<String, std::sync::Arc<crate::audio_engine::AudioSource>>,
    next_item_counter: u64,
    next_take_counter: u64,
}

impl EditSnapshot {
    pub fn capture(p: &ProjectState) -> Self {
        Self {
            tracks: p.tracks.clone(),
            items: p.items.clone(),
            items_by_track: p.items_by_track.clone(),
            takes: p.takes.clone(),
            stretch_markers: p.stretch_markers.clone(),
            stretch_modes: p.stretch_modes.clone(),
            take_markers: p.take_markers.clone(),
            midi_notes: p.midi_notes.clone(),
            midi_ccs: p.midi_ccs.clone(),
            envelopes: p.envelopes.clone(),
            #[cfg(any(feature = "audio", feature = "decode"))]
            audio_sources: p.audio_sources.clone(),
            next_item_counter: p.next_item_counter,
            next_take_counter: p.next_take_counter,
        }
    }

    pub fn restore(self, p: &mut ProjectState) {
        p.tracks = self.tracks;
        p.items = self.items;
        p.items_by_track = self.items_by_track;
        p.takes = self.takes;
        p.stretch_markers = self.stretch_markers;
        p.stretch_modes = self.stretch_modes;
        p.take_markers = self.take_markers;
        p.midi_notes = self.midi_notes;
        p.midi_ccs = self.midi_ccs;
        p.envelopes = self.envelopes;
        #[cfg(any(feature = "audio", feature = "decode"))]
        {
            p.audio_sources = self.audio_sources;
        }
        p.next_item_counter = self.next_item_counter;
        p.next_take_counter = self.next_take_counter;
        // The renderer caches against the revision; a restore is a
        // mutation like any other.
        p.revision = p.revision.wrapping_add(1);
    }
}

/// One undo step: what the project looked like before the block ran.
pub struct UndoStep {
    pub label: String,
    pub snapshot: EditSnapshot,
}

/// Undo depth per project. An edit session is drags and applies, not a
/// text editor; 32 steps of maps-and-Arcs is small.
pub const UNDO_LIMIT: usize = 32;

/// Aggregate state for the standalone sync backend.
pub struct StandaloneState {
    pub projects: HashMap<String, ProjectState>,
    pub current_project_guid: Option<String>,
    /// Global ext state keyed by `(section, key)`.
    pub global_ext_state: HashMap<(String, String), String>,
    /// Last-touched FX, if any.
    pub last_touched_fx: Option<LastTouchedFx>,
    /// Buffered console output (for tests).
    pub console_log: Vec<String>,
    /// Undo stacks per project guid, most recent last.
    pub undo: HashMap<String, Vec<UndoStep>>,
    pub redo: HashMap<String, Vec<UndoStep>>,
    /// A `begin_undo_block` whose `end` has not arrived yet.
    pub pending_undo: HashMap<String, UndoStep>,
}

impl StandaloneState {
    fn new() -> Self {
        Self {
            projects: HashMap::new(),
            current_project_guid: None,
            global_ext_state: HashMap::new(),
            last_touched_fx: None,
            console_log: Vec::new(),
            undo: HashMap::new(),
            redo: HashMap::new(),
            pending_undo: HashMap::new(),
        }
    }
}

/// Root sync handle for the in-memory standalone backend.
///
/// Implements [`daw_proto::Daw`]. Construct with [`Standalone::new`] and
/// optionally seed projects via [`Standalone::seed_project`].
#[derive(Clone)]
pub struct Standalone {
    pub(crate) state: Arc<Mutex<StandaloneState>>,
    /// Per-project transport engine bundles (sample clock + subscriber pumps).
    /// Lazily created on first access. WASM-compatible; lock-free on the
    /// inner [`TransportShared`] atomics.
    pub(crate) transport_engines: Arc<
        Mutex<std::collections::HashMap<String, Arc<crate::transport_engine::TransportBundle>>>,
    >,
    /// Shared file-intake resolver used by `MediaBay`. Single source
    /// of truth so resolver installs persist across `media_bay()`
    /// handles and across threads.
    pub(crate) bay_resolver: Arc<Mutex<Option<Box<dyn crate::media_bay::BayFileResolver>>>>,
    /// Touch / latch state for realtime automation write modes.
    pub(crate) automation_touch: Arc<Mutex<crate::automation_touch::AutomationTouchState>>,
    /// Loaded plugin instances keyed by `fx_guid`. Separate from
    /// `ProjectState.fx_chains` (which carries proto-serializable
    /// `FxEntry` metadata only) so the renderer can `lock()` plugins
    /// independently of project state mutations.
    ///
    /// Locks on this mutex are poison-TOLERANT everywhere
    /// (`unwrap_or_else(PoisonError::into_inner)`): the audio callback takes
    /// it every block, and a control-thread panic while holding it must not
    /// cascade into a panic inside the extern "C" audio callback (which would
    /// abort the process). The guarded map stays structurally valid across a
    /// panic — worst case one entry holds a half-updated plugin state.
    pub(crate) plugin_instances:
        Arc<Mutex<std::collections::HashMap<String, Box<dyn crate::plugin::PluginInstance>>>>,
    /// Plugin binaries that have been explicitly loaded/validated via
    /// the PluginLoading service. Actual per-track DSP instances live
    /// in `plugin_instances` after an FX is inserted.
    pub(crate) loaded_plugins:
        Arc<Mutex<std::collections::HashMap<String, daw_proto::LoadedPluginInfo>>>,
    /// Injected built-in FX factory (see [`crate::plugin::FxFactory`]).
    /// `Effects::add` consults it for names the plugin-file backends
    /// don't claim; `list_installed` appends its catalog. `None` until
    /// the app installs one (`set_fx_factory`) — built-in names then
    /// fall back to synthetic no-DSP entries, as before.
    pub(crate) fx_factory: Arc<Mutex<Option<Arc<dyn crate::plugin::FxFactory>>>>,
    /// Track-domain stream hub. Mutating `Tracks` methods publish
    /// here; the architect stream layer attaches every subscriber
    /// sink (`TracksStreamSource`).
    pub(crate) track_events: architect::PubSub<daw_proto::track::TrackStreamEvent>,
    pub(crate) marker_events: architect::PubSub<daw_proto::marker::MarkerStreamEvent>,
    pub(crate) region_events: architect::PubSub<daw_proto::region::RegionStreamEvent>,
    pub(crate) tempo_map_events: architect::PubSub<daw_proto::tempo_map::TempoMapStreamEvent>,
    /// Transport-domain stream hub (`TransportStreamSource`). The
    /// per-project transport pump publishes state changes + ~30 Hz
    /// position ticks here; the architect `#[subscribe] fn events`
    /// stream layer fans them out. Subscribers filter by `project_guid`.
    pub(crate) transport_events: architect::PubSub<daw_proto::transport::TransportStreamEvent>,
    /// Cross-domain event-bus hub (`EventBusStreamSource`). Every
    /// per-domain publish site also wraps its event in [`DawEvent`]
    /// and publishes here; per-project transport pumps bridge
    /// transport state + position ticks in as well. Subscribers
    /// filter client-side.
    pub(crate) bus_events: architect::PubSub<daw_proto::event_bus::DawEvent>,
    /// Per-track peak-meter bank, written by the audio engine and read by the
    /// `Peaks` service. Replaceable (the track count is fixed per bank) so an
    /// audio engine can install a freshly-sized bank when it attaches; empty
    /// until then, so `track_peak` reports silence.
    pub(crate) meters: Arc<Mutex<Arc<crate::metering::Meters>>>,
    /// Meter-frame stream hub (`PeaksStreamSource`). A process-wide
    /// pump (spawned lazily by the first `set_meters`, i.e. when an
    /// audio engine attaches) reads the meter bank at ~30 Hz and
    /// publishes one [`MeterFrame`](daw_proto::MeterFrame) per tick;
    /// the architect `#[subscribe] fn meters` stream layer fans them
    /// out. Subscribers filter by `project_guid`.
    pub(crate) meter_events: architect::PubSub<daw_proto::MeterFrame>,
    /// Whether the meter pump has been spawned (once per process).
    pub(crate) meter_pump_started: Arc<std::sync::atomic::AtomicBool>,
    /// Producer end of the live / programmatic MIDI ring. Installed by
    /// `AudioEngine::with_project_prefs` (the consumer goes to the
    /// renderer via `set_live_midi`). `push_note_on`/`_off`/`_cc` push
    /// `LiveMidiEvent`s here; the renderer drains them once per block.
    /// `None` until an audio engine attaches; pushes are then dropped.
    #[cfg(feature = "audio")]
    pub(crate) live_midi_tx:
        Arc<Mutex<Option<rtrb::Producer<crate::audio_engine::render::LiveMidiEvent>>>>,
    /// wasm32 live / programmatic MIDI queue — the browser analog of
    /// `live_midi_tx`. The AudioWorkletGlobalScope is single-threaded, so a
    /// mutex-guarded `VecDeque` is uncontended by construction; it lives on
    /// `Standalone` (not the renderer) because the web render path builds a
    /// fresh `ProjectRenderer` per block. Pushed by `push_live_midi`;
    /// swapped out whole by the renderer once per block
    /// (`take_live_midi_wasm`).
    #[cfg(all(target_arch = "wasm32", any(feature = "decode", feature = "audio")))]
    pub(crate) live_midi_wasm:
        Arc<Mutex<std::collections::VecDeque<crate::audio_engine::render::LiveMidiEvent>>>,
}

impl Default for Standalone {
    fn default() -> Self {
        Self::new()
    }
}

impl Standalone {
    /// Create a new empty standalone backend.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(StandaloneState::new())),
            transport_engines: Arc::new(Mutex::new(std::collections::HashMap::new())),
            bay_resolver: Arc::new(Mutex::new(None)),
            automation_touch: Arc::new(Mutex::new(
                crate::automation_touch::AutomationTouchState::default(),
            )),
            plugin_instances: Arc::new(Mutex::new(std::collections::HashMap::new())),
            loaded_plugins: Arc::new(Mutex::new(std::collections::HashMap::new())),
            fx_factory: Arc::new(Mutex::new(None)),
            track_events: architect::PubSub::sliding(1024),
            marker_events: architect::PubSub::sliding(1024),
            region_events: architect::PubSub::sliding(1024),
            tempo_map_events: architect::PubSub::sliding(1024),
            transport_events: architect::PubSub::sliding(1024),
            bus_events: architect::PubSub::sliding(1024),
            meters: Arc::new(Mutex::new(crate::metering::Meters::empty())),
            meter_events: architect::PubSub::sliding(64),
            meter_pump_started: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            #[cfg(feature = "audio")]
            live_midi_tx: Arc::new(Mutex::new(None)),
            #[cfg(all(target_arch = "wasm32", any(feature = "decode", feature = "audio")))]
            live_midi_wasm: Arc::new(Mutex::new(std::collections::VecDeque::new())),
        }
    }

    /// The current per-track peak-meter bank (written by the audio engine,
    /// read by the `Peaks` service).
    pub fn meters(&self) -> Arc<crate::metering::Meters> {
        self.meters.lock().expect("meters poisoned").clone()
    }

    /// Install a peak-meter bank, replacing any previous one. An audio engine
    /// calls this when it attaches so its realtime callback and the `Peaks`
    /// service share the same cells.
    pub fn set_meters(&self, meters: Arc<crate::metering::Meters>) {
        *self.meters.lock().expect("meters poisoned") = meters;
    }

    /// Spawn the ~30 Hz meter pump (once per process): reads the
    /// current meter bank's atomics — RT-safe, the audio callback
    /// never blocks on it — builds one [`daw_proto::MeterFrame`] for
    /// the current project and publishes it on the `meter_events` hub
    /// (`PeaksStreamSource`). Called lazily from `meters_hub()` (the
    /// first stream subscription), which runs in async context — so a
    /// backend used purely synchronously (offline render, tests) never
    /// needs a runtime and never pays for the pump. An empty bank
    /// (engine detached) publishes nothing, so remote meters simply
    /// stop updating.
    pub(crate) fn spawn_meter_pump(&self) {
        use std::sync::atomic::Ordering;
        if self.meter_pump_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let this = self.clone();
        architect::platform::spawn(async move {
            loop {
                architect::platform::sleep(std::time::Duration::from_millis(33)).await;
                let bank = this.meters();
                if bank.is_empty() {
                    continue;
                }
                let Some(project_guid) = ({
                    let s = this.state.lock().expect("standalone state poisoned");
                    s.current_project_guid.clone()
                }) else {
                    continue;
                };
                let tracks = (0..bank.len())
                    .map(|i| {
                        let cell = bank.cell(i).expect("index < len");
                        daw_proto::TrackLevels {
                            peak_left: cell.peak(0),
                            peak_right: cell.peak(1),
                            hold_left: cell.hold(0),
                            hold_right: cell.hold(1),
                        }
                    })
                    .collect();
                this.meter_events.publish(daw_proto::MeterFrame {
                    project_guid,
                    tracks,
                });
            }
        });
    }

    /// Get or lazily-create the transport engine bundle for `guid`.
    /// Seeds initial BPM from the project's `Transport.tempo`.
    pub fn transport_engine_for(
        &self,
        guid: &str,
    ) -> Arc<crate::transport_engine::TransportBundle> {
        // Fast path: already created.
        {
            let engines = self.transport_engines.lock().expect("engines poisoned");
            if let Some(b) = engines.get(guid) {
                return b.clone();
            }
        }
        // Slow path: seed from project tempo + insert.
        let initial_bpm = self
            .with_project(guid, |p| p.transport.tempo.bpm())
            .unwrap_or(120.0);
        let bundle = Arc::new(crate::transport_engine::TransportBundle::spawn(
            48_000,
            initial_bpm,
        ));
        let (bundle, inserted) = {
            let mut engines = self.transport_engines.lock().expect("engines poisoned");
            match engines.entry(guid.to_string()) {
                std::collections::hash_map::Entry::Occupied(e) => (e.get().clone(), false),
                std::collections::hash_map::Entry::Vacant(v) => (v.insert(bundle).clone(), true),
            }
        };
        if inserted {
            // One process-wide pump per project bundle bridging
            // transport state + position ticks onto the event-bus hub
            // (replaces the per-bus-subscriber pumps of the old
            // `EventBus::subscribe`).
            self.spawn_bus_transport_bridge(guid, &bundle);
        }
        bundle
    }

    /// Bridge one project's transport stream onto the cross-domain
    /// event-bus hub. Runs for the life of the process; the bus hub
    /// fans events out to however many subscribers exist (zero-cost
    /// publish when there are none).
    fn spawn_bus_transport_bridge(
        &self,
        guid: &str,
        bundle: &Arc<crate::transport_engine::TransportBundle>,
    ) {
        use daw_proto::event_bus::DawEvent;
        use daw_proto::primitives::{Position, PositionInSeconds};
        use daw_proto::transport::{PositionTick, TransportEvent, TransportStreamEvent};

        let guid = guid.to_string();
        let bundle = bundle.clone();
        let initial_proto = self
            .with_project(&guid, |p| p.transport.clone())
            .unwrap_or_default();
        let bus = self.bus_events.clone();
        let transport = self.transport_events.clone();

        architect::platform::spawn(async move {
            // Publish to BOTH the dedicated transport stream
            // (`TransportStreamSource`) and the cross-domain event bus.
            let emit = |ev: TransportStreamEvent| {
                transport.publish(ev.clone());
                match ev {
                    TransportStreamEvent::State(s) => {
                        bus.publish(DawEvent::TransportState(s));
                    }
                    TransportStreamEvent::Position(p) => {
                        bus.publish(DawEvent::TransportPosition(p));
                    }
                }
            };

            // Initial snapshot so late subscribers sync without re-querying.
            emit(TransportStreamEvent::State(TransportEvent::Snapshot {
                project_guid: guid.clone(),
                state: initial_proto,
            }));

            // Poll the bundle's atomics at ~30 Hz and publish. Reading atomics
            // is RT-safe; the publish (mutex) happens only on this task, never
            // in the audio callback. `is_playing` rides on each position tick,
            // so the UI sees play/stop without a separate discrete event
            // (consumers that need the full state query `Transport::get_state`).
            loop {
                architect::platform::sleep(std::time::Duration::from_millis(33)).await;
                let snap = bundle.snapshot();
                let pos = Position::from_time(PositionInSeconds::from_seconds(snap.seconds.0));
                emit(TransportStreamEvent::Position(PositionTick {
                    project_guid: guid.clone(),
                    playhead: pos.clone(),
                    edit_cursor: pos,
                    is_playing: snap.play_state.is_advancing(),
                }));
            }
        });
    }

    /// Seed an empty project into the state.
    ///
    /// If this is the first seeded project, it becomes the current project.
    /// Returns the GUID of the seeded project for convenience.
    pub fn seed_project(&self, info: ProjectInfo) -> String {
        let guid = info.guid.clone();
        let mut s = self.state.lock().expect("standalone state poisoned");
        if s.current_project_guid.is_none() {
            s.current_project_guid = Some(guid.clone());
        }
        s.projects.insert(guid.clone(), ProjectState::new(info));
        guid
    }

    /// Mark `guid` as the current project. No-op if the project is not present.
    pub fn set_current_project(&self, guid: &str) {
        let mut s = self.state.lock().expect("standalone state poisoned");
        if s.projects.contains_key(guid) {
            s.current_project_guid = Some(guid.to_string());
        }
    }

    /// Run a closure with mutable access to a project's state.
    pub(crate) fn with_project_mut<R>(
        &self,
        guid: &str,
        f: impl FnOnce(&mut ProjectState) -> R,
    ) -> DawResult<R> {
        let mut s = self.state.lock().expect("standalone state poisoned");
        let project = s
            .projects
            .get_mut(guid)
            .ok_or_else(|| DawError::not_found("Project", guid))?;
        let r = f(project);
        // Every mutable pass counts as a mutation — false positives
        // just refresh a cache, staleness would corrupt playback.
        project.revision = project.revision.wrapping_add(1);
        Ok(r)
    }

    /// Run a closure with shared access to a project's state.
    pub(crate) fn with_project<R>(
        &self,
        guid: &str,
        f: impl FnOnce(&ProjectState) -> R,
    ) -> DawResult<R> {
        let s = self.state.lock().expect("standalone state poisoned");
        let project = s
            .projects
            .get(guid)
            .ok_or_else(|| DawError::not_found("Project", guid))?;
        Ok(f(project))
    }

    /// Attach a cpal-backed [`AudioEngine`](crate::audio_engine::AudioEngine)
    /// to project `guid`. Disables the project's soft clock so the
    /// audio callback becomes the sole driver of the playhead.
    /// Sample-accurate. Returns the engine; drop it to release the
    /// stream (and re-enable the soft clock manually if desired).
    #[cfg(feature = "audio")]
    pub fn attach_audio_engine(
        &self,
        guid: &str,
    ) -> Result<crate::audio_engine::AudioEngine, String> {
        // Project mode: cpal callback renders the full track / item /
        // routing graph via `ProjectRenderer`. Soft clock disabled
        // because the audio callback now drives `advance()`.
        crate::audio_engine::AudioEngine::attached_to(self, guid)
    }

    /// Whether a loaded plugin instance is backing the given FX
    /// guid. `false` for synthetic / failed-to-load entries.
    pub fn has_plugin_instance(&self, fx_guid: &str) -> bool {
        self.plugin_instances
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(fx_guid)
    }

    /// Register a **native** (built-in Rust DSP) FX instance under `fx_guid`, so
    /// the renderer's per-track FX chain processes it exactly like a hosted
    /// CLAP/VST3 plugin — there is no separate native-FX path, both live in
    /// `plugin_instances` and go through `PluginInstance::process_block`. The
    /// instance must already implement [`PluginInstance`](crate::plugin::PluginInstance)
    /// (e.g. Signal's NAM amp / cab-IR convolver). Returns any previous instance
    /// under the same guid (hand it off-thread to drop). Pair with a matching
    /// `fx_guid` entry in the track's FX chain so the renderer picks it up.
    /// Install the built-in FX factory (see [`crate::plugin::FxFactory`]).
    /// Replaces any previous factory.
    pub fn set_fx_factory(&self, factory: Arc<dyn crate::plugin::FxFactory>) {
        *self
            .fx_factory
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(factory);
    }

    /// The installed built-in FX factory, if any.
    pub fn fx_factory(&self) -> Option<Arc<dyn crate::plugin::FxFactory>> {
        self.fx_factory
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn insert_plugin_instance(
        &self,
        fx_guid: impl Into<String>,
        instance: Box<dyn crate::plugin::PluginInstance>,
    ) -> Option<Box<dyn crate::plugin::PluginInstance>> {
        self.plugin_instances
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(fx_guid.into(), instance)
    }

    /// Install the producer end of the live-MIDI ring. Called by
    /// `AudioEngine::with_project_prefs` after wiring the consumer to the
    /// renderer. Replaces any previous producer (e.g. on engine restart).
    #[cfg(feature = "audio")]
    pub(crate) fn set_live_midi_producer(
        &self,
        prod: rtrb::Producer<crate::audio_engine::render::LiveMidiEvent>,
    ) {
        if let Ok(mut slot) = self.live_midi_tx.lock() {
            *slot = Some(prod);
        }
    }

    /// Push a programmatic MIDI message to `track_guid`, delivered to that
    /// track's instrument/FX chain at the start of the next render block.
    /// Lock-free on the ring; dropped (and counted by the ring's slot
    /// accounting) if no engine is attached or the ring is full. `true`
    /// when the event was enqueued.
    ///
    /// Full-fidelity seam for hardware MIDI sources (e.g. `midicore`): unlike
    /// [`push_note_on`](Self::push_note_on) / [`push_cc`](Self::push_cc) this
    /// carries the original channel, release velocity, pitch-bend and program
    /// change rather than collapsing to monotimbral channel 0.
    #[cfg(all(not(target_arch = "wasm32"), feature = "audio"))]
    pub fn push_live_midi(&self, track_guid: &str, message: daw_proto::MidiEvent) -> bool {
        let mut guard = match self.live_midi_tx.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        let Some(prod) = guard.as_mut() else {
            return false;
        };
        prod.push(crate::audio_engine::render::LiveMidiEvent {
            track: track_guid.to_string(),
            offset: 0,
            message,
        })
        .is_ok()
    }

    /// wasm32 variant of [`push_live_midi`](Self::push_live_midi): queue a
    /// programmatic MIDI message for `track_guid` in the `VecDeque`-backed
    /// live-MIDI queue; the web render path drains it once per block.
    /// Bounded (drops + returns `false` past the cap) so a producer that
    /// outruns a stalled renderer can't grow the queue without limit.
    #[cfg(all(target_arch = "wasm32", any(feature = "decode", feature = "audio")))]
    pub fn push_live_midi(&self, track_guid: &str, message: daw_proto::MidiEvent) -> bool {
        /// Same order of magnitude as the native ring's capacity.
        const MAX_QUEUED: usize = 1024;
        let mut queue = self
            .live_midi_wasm
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if queue.len() >= MAX_QUEUED {
            return false;
        }
        queue.push_back(crate::audio_engine::render::LiveMidiEvent {
            track: track_guid.to_string(),
            offset: 0,
            message,
        });
        true
    }

    /// Discard every queued live-MIDI event (wasm), returning how many
    /// went.
    ///
    /// For a LIVE instrument a stale note is worse than no note. When
    /// rendering stops for a while — the context suspends, the tab goes to
    /// the background, a long handler stalls the thread — pressed notes
    /// keep queueing, and draining them when rendering resumes fires the
    /// lot at once: the player hears notes they pressed seconds ago, all
    /// together, which is worse than silence. The host flushes instead.
    #[cfg(all(target_arch = "wasm32", any(feature = "decode", feature = "audio")))]
    pub fn flush_live_midi(&self) -> usize {
        self.live_midi_wasm
            .lock()
            .map(|mut q| {
                let n = q.len();
                q.clear();
                n
            })
            .unwrap_or(0)
    }

    /// Live-MIDI events waiting to be drained (wasm).
    ///
    /// The renderer swaps this queue out once per block, so in a healthy
    /// rig it is 0 or a handful. A depth that STAYS above zero means events
    /// are being pushed faster than blocks are rendering — i.e. the notes a
    /// player pressed are sitting in a queue instead of sounding, and will
    /// arrive late in a burst when rendering catches up. That is exactly
    /// the "it plays my notes seconds later" failure, so it needs to be
    /// visible rather than inferred.
    #[cfg(all(target_arch = "wasm32", any(feature = "decode", feature = "audio")))]
    pub fn live_midi_depth(&self) -> usize {
        self.live_midi_wasm.lock().map(|q| q.len()).unwrap_or(0)
    }

    /// Swap out every queued wasm live-MIDI event (renderer-side, once per
    /// block). Returns the whole queue, leaving an empty one in place.
    #[cfg(all(target_arch = "wasm32", any(feature = "decode", feature = "audio")))]
    pub(crate) fn take_live_midi_wasm(
        &self,
    ) -> std::collections::VecDeque<crate::audio_engine::render::LiveMidiEvent> {
        let mut queue = self
            .live_midi_wasm
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::take(&mut *queue)
    }

    /// Programmatic Note-On (channel 0) to `track_guid`. A `velocity` of 0
    /// is a Note-Off by MIDI convention — most instruments treat it so.
    #[cfg(any(
        all(not(target_arch = "wasm32"), feature = "audio"),
        all(target_arch = "wasm32", any(feature = "decode", feature = "audio"))
    ))]
    pub fn push_note_on(&self, track_guid: &str, note: u8, velocity: u8) -> bool {
        use daw_proto::{Channel, KeyNumber, MidiEvent, Velocity};
        self.push_live_midi(
            track_guid,
            MidiEvent::NoteOn {
                channel: Channel::new(0),
                key: KeyNumber::new(note),
                velocity: Velocity::new(velocity),
            },
        )
    }

    /// Programmatic Note-Off (channel 0, release velocity 0) to `track_guid`.
    #[cfg(any(
        all(not(target_arch = "wasm32"), feature = "audio"),
        all(target_arch = "wasm32", any(feature = "decode", feature = "audio"))
    ))]
    pub fn push_note_off(&self, track_guid: &str, note: u8) -> bool {
        use daw_proto::{Channel, KeyNumber, MidiEvent, Velocity};
        self.push_live_midi(
            track_guid,
            MidiEvent::NoteOff {
                channel: Channel::new(0),
                key: KeyNumber::new(note),
                velocity: Velocity::new(0),
            },
        )
    }

    /// Programmatic Control-Change (channel 0) to `track_guid`.
    #[cfg(any(
        all(not(target_arch = "wasm32"), feature = "audio"),
        all(target_arch = "wasm32", any(feature = "decode", feature = "audio"))
    ))]
    pub fn push_cc(&self, track_guid: &str, controller: u8, value: u8) -> bool {
        use daw_proto::{Channel, ControllerNumber, ControllerValue, MidiEvent};
        self.push_live_midi(
            track_guid,
            MidiEvent::ControlChange {
                channel: Channel::new(0),
                controller: ControllerNumber::new(controller),
                value: ControllerValue::new(value),
            },
        )
    }

    /// Run `f` against the live FX instance backing `fx_guid`, under the
    /// `plugin_instances` lock (the same lock the renderer takes per block), and
    /// return its result. `None` if no instance is loaded under that guid.
    ///
    /// This is the in-place control seam for native FX instances whose
    /// parameters are NOT exposed as host params (e.g. Signal's NAM amp trims):
    /// the caller downcasts via [`PluginInstance`](crate::plugin::PluginInstance)
    /// and mutates the instance directly. Hosted plugins with real param
    /// surfaces should still go through the `FxParams` set path.
    pub fn with_plugin_instance<R>(
        &self,
        fx_guid: &str,
        f: impl FnOnce(&mut dyn crate::plugin::PluginInstance) -> R,
    ) -> Option<R> {
        let mut plugins = self
            .plugin_instances
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        plugins.get_mut(fx_guid).map(|p| f(p.as_mut()))
    }

    /// Remove the FX instance backing `fx_guid`, returning it (e.g. to drop off
    /// the audio thread). No-op if absent.
    pub fn remove_plugin_instance(
        &self,
        fx_guid: &str,
    ) -> Option<Box<dyn crate::plugin::PluginInstance>> {
        self.plugin_instances
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(fx_guid)
    }

    /// Whether the plugin backing `fx_guid` is currently activated
    /// (post-`prepare`, pre-`deactivate`). `false` for unloaded /
    /// missing entries.
    pub fn plugin_is_prepared(&self, fx_guid: &str) -> bool {
        self.plugin_instances
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(fx_guid)
            .map(|p| p.is_prepared())
            .unwrap_or(false)
    }

    /// Apply a previously-saved plugin state blob to the FX backing
    /// `fx_guid`. The blob format is the one returned by
    /// `PluginInstance::save_state` (DAW3-framed for VST3; raw bytes
    /// for CLAP). To load from a REAPER `.rpp` chunk, decode it
    /// through [`crate::rpp_state`] first.
    ///
    /// Returns `Err` if the FX guid isn't backed by a loaded plugin
    /// instance, or the plugin's `load_state` returned an error.
    pub fn apply_plugin_state(
        &self,
        fx_guid: &str,
        state: &[u8],
    ) -> Result<(), crate::plugin::PluginError> {
        let mut plugins = self
            .plugin_instances
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match plugins.get_mut(fx_guid) {
            Some(p) => p.load_state(state),
            None => Err(crate::plugin::PluginError::LoadFailed(format!(
                "no plugin instance for fx_guid {fx_guid}"
            ))),
        }
    }

    /// Save the current state of the FX backing `fx_guid`. Returns
    /// `Err` if there's no loaded plugin or its `save_state` failed.
    pub fn save_plugin_state(&self, fx_guid: &str) -> Result<Vec<u8>, crate::plugin::PluginError> {
        let mut plugins = self
            .plugin_instances
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match plugins.get_mut(fx_guid) {
            Some(p) => p.save_state(),
            None => Err(crate::plugin::PluginError::LoadFailed(format!(
                "no plugin instance for fx_guid {fx_guid}"
            ))),
        }
    }

    /// Project Media/FX Bay handle. Cheap to construct — clones an
    /// `Arc` over self. See [`crate::media_bay`] for the API.
    pub fn media_bay(&self) -> crate::media_bay::MediaBay {
        crate::media_bay::MediaBay::new(self.clone())
    }

    /// Read-only access to a project's audio source map. Returns
    /// `None` if the project doesn't exist. The decoded PCM is wrapped
    /// in `Arc`, so cloning the inner refs is cheap.
    #[cfg(any(feature = "audio", feature = "decode"))]
    pub fn audio_source(
        &self,
        project_guid: &str,
        take_guid: &str,
    ) -> Option<std::sync::Arc<crate::audio_engine::AudioSource>> {
        self.with_project(project_guid, |p| p.audio_sources.get(take_guid).cloned())
            .ok()
            .flatten()
    }

    /// Count of decoded audio sources for a project (diagnostic).
    #[cfg(any(feature = "audio", feature = "decode"))]
    pub fn audio_source_count(&self, project_guid: &str) -> usize {
        self.with_project(project_guid, |p| p.audio_sources.len())
            .unwrap_or(0)
    }

    /// Visit a project's state via a read-only closure. Test/debug
    /// hook; production code should go through the proto trait surface.
    pub fn read_project<R>(&self, guid: &str, f: impl FnOnce(&ProjectState) -> R) -> Option<R> {
        let s = self.state.lock().ok()?;
        s.projects.get(guid).map(f)
    }

    /// Visit a project's state via a mutable closure. Test/debug hook
    /// — production mutations should go through the proto trait
    /// surface so invariants stay consistent.
    pub fn write_project<R>(
        &self,
        guid: &str,
        f: impl FnOnce(&mut ProjectState) -> R,
    ) -> Option<R> {
        let mut s = self.state.lock().ok()?;
        let project = s.projects.get_mut(guid)?;
        let r = f(project);
        project.revision = project.revision.wrapping_add(1);
        Some(r)
    }

    /// Captured console messages, useful in tests.
    pub fn console_log(&self) -> Vec<String> {
        let s = self.state.lock().expect("standalone state poisoned");
        s.console_log.clone()
    }

    /// Set the synthetic last-touched FX (for tests).
    pub fn set_last_touched_fx(&self, fx: Option<LastTouchedFx>) {
        let mut s = self.state.lock().expect("standalone state poisoned");
        s.last_touched_fx = fx;
    }
}

impl Daw for Standalone {
    type Project<'a> = super::project::StandaloneProject<'a>;

    fn current_project(&self) -> DawResult<Self::Project<'_>> {
        let guid = {
            let s = self.state.lock().expect("standalone state poisoned");
            s.current_project_guid
                .clone()
                .ok_or_else(|| DawError::not_found("Project", "current"))?
        };
        self.with_project(&guid, |_| ())?;
        Ok(super::project::StandaloneProject::new(self, guid))
    }

    fn project(&self, guid: &str) -> DawResult<Self::Project<'_>> {
        self.with_project(guid, |_| ())?;
        Ok(super::project::StandaloneProject::new(
            self,
            guid.to_string(),
        ))
    }

    fn projects(&self) -> Vec<ProjectInfo> {
        let s = self.state.lock().expect("standalone state poisoned");
        s.projects.values().map(|p| p.info.clone()).collect()
    }

    fn show_console_msg(&self, msg: &str) {
        let mut s = self.state.lock().expect("standalone state poisoned");
        s.console_log.push(msg.to_string());
    }

    fn last_touched_fx(&self) -> Option<LastTouchedFx> {
        let s = self.state.lock().expect("standalone state poisoned");
        s.last_touched_fx.clone()
    }
}
