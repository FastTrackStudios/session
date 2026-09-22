//! The web demo's audio: the session mixed on the page and played through
//! an AudioWorklet.
//!
//! The engine is the one the panels already drive (daw-standalone,
//! in-process, see [`crate::web_engine`]), so a fader or a mute is heard
//! the moment the engine takes it. What a browser adds is three things:
//!
//! - **Stems** stream from their Ogg Vorbis proxies ([`add_stem`]): each
//!   take's source is a `Streamed` one, decoded a few seconds at a time
//!   around the playhead by a feeder this loop runs.
//! - **Output** is an AudioWorklet that plays blocks the page posts it —
//!   a small queue, so the page renders ahead of the device by a quarter of
//!   a second and the playhead the panels see is held back by what is
//!   queued ([`latency_seconds`]).
//! - **Unlocking**: a browser starts audio only inside a gesture, so the
//!   context is made on the page's first press or key ([`unlock`]); until
//!   then the transport runs on the engine's own clock and plays silent.
//!
//! One loop on the page's event loop ([`install`]) does the work: feed the
//! stems within a time budget, then render until the queue is full. It
//! renders only while the transport advances, and a jump of the playhead
//! (a seek, a stop, a loop) empties the queue so the device follows at
//! once.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use daw_standalone::audio_engine::render::ProjectRenderer;
use daw_standalone::audio_engine::streamed::StreamFeeder;
use daw_standalone::sync::Standalone;
use daw_standalone::transport_engine::{TransportBundle, TransportShared};
use fts_sample::ogg_stream::OggStream;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::closure::Closure;

/// The output rate. The context is asked for it (the browser resamples to
/// the device), so it matches the engine's transport clock, which counts
/// at 48 kHz from before the context exists.
const RATE: u32 = 48_000;
/// Frames per rendered block.
const BLOCK: usize = 1024;
/// How far ahead of the device to render: enough to ride out a busy
/// frame of painting, little enough that a fader answers at once.
const AHEAD: u64 = RATE as u64 / 4;
/// How long a tick may spend decoding stems before it renders.
const DECODE_BUDGET_MS: u128 = 6;

/// The worklet: a queue of interleaved stereo blocks, played in order,
/// silence when it runs dry. It reports how much it has played, tagged
/// with the epoch of the last flush, so the page knows what is queued.
const PROCESSOR: &str = r"
class FtsOut extends AudioWorkletProcessor {
  constructor() {
    super();
    this.q = []; this.off = 0; this.epoch = 0; this.played = 0; this.tick = 0;
    this.port.onmessage = (e) => {
      const m = e.data;
      if (typeof m === 'number') { this.q = []; this.off = 0; this.epoch = m; this.played = 0; return; }
      this.q.push(m);
    };
  }
  process(_inputs, outputs) {
    const out = outputs[0];
    const l = out[0], r = out[1] || out[0];
    const n = l.length;
    let i = 0;
    while (i < n && this.q.length) {
      const c = this.q[0];
      const frames = c.length >> 1;
      const take = Math.min(frames - this.off, n - i);
      for (let k = 0; k < take; k++) {
        const j = (this.off + k) << 1;
        l[i + k] = c[j];
        r[i + k] = c[j + 1];
      }
      i += take; this.off += take; this.played += take;
      if (this.off >= frames) { this.q.shift(); this.off = 0; }
    }
    for (; i < n; i++) { l[i] = 0; r[i] = 0; }
    if ((++this.tick & 3) === 0) this.port.postMessage([this.epoch, this.played]);
    return true;
  }
}
registerProcessor('fts-out', FtsOut);
";

/// The device side, once unlocked.
struct Out {
    context: web_sys::AudioContext,
    port: web_sys::MessagePort,
    /// Kept alive for as long as the port reports.
    _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
}

struct Player {
    daw: Standalone,
    bundle: Arc<TransportBundle>,
    shared: Arc<TransportShared>,
    /// Whether the device drives the transport — only while its context
    /// runs. A context that never starts (no output device, a browser
    /// holding it suspended) leaves the engine's own clock keeping time,
    /// so the song still moves, silently.
    clocked: Cell<bool>,
    renderer: ProjectRenderer,
    feeders: RefCell<Vec<StreamFeeder<OggStream>>>,
    /// Which feeder a tick starts with — round robin, so one stem's
    /// catch-up does not starve the rest.
    first: Cell<usize>,
    out: RefCell<Option<Out>>,
    /// Set from the first unlock until the worklet is up, so a second
    /// press meanwhile makes no second context.
    starting: Cell<bool>,
    /// Frames posted, and played, since the last flush.
    sent: Cell<u64>,
    played: Rc<Cell<u64>>,
    epoch: Rc<Cell<u32>>,
    /// Where the next block renders from, while playing; `None` stopped.
    next: Cell<Option<u64>>,
}

thread_local! {
    static PLAYER: RefCell<Option<Rc<Player>>> = const { RefCell::new(None) };
}

fn player() -> Option<Rc<Player>> {
    PLAYER.with(|p| p.borrow().clone())
}

/// Start the audio loop for `project` on `daw`. Silent until [`unlock`].
pub fn install(daw: Standalone, project: &str) {
    let bundle = daw.transport_engine_for(project);
    let shared = Arc::clone(&bundle.shared);
    shared.set_sample_rate(RATE);
    let count = daw_proto::Tracks::count(
        &daw,
        daw_proto::ProjectContext::Project(project.to_owned()),
    ) as usize;
    daw.set_meters(daw_standalone::metering::Meters::new(count));
    let player = Rc::new(Player {
        renderer: ProjectRenderer::new(&daw, project, RATE),
        daw,
        bundle,
        shared,
        clocked: Cell::new(false),
        feeders: RefCell::default(),
        first: Cell::new(0),
        out: RefCell::new(None),
        starting: Cell::new(false),
        sent: Cell::new(0),
        played: Rc::new(Cell::new(0)),
        epoch: Rc::new(Cell::new(0)),
        next: Cell::new(None),
    });
    PLAYER.with(|p| *p.borrow_mut() = Some(Rc::clone(&player)));
    wasm_bindgen_futures::spawn_local(async move {
        loop {
            player.tick();
            gloo_timers::future::TimeoutFuture::new(10).await;
        }
    });
}

/// A stem streamed from its proxy's bytes, for the take `take` of
/// `project`.
///
/// # Errors
///
/// The bytes are not an Ogg Vorbis stream.
pub fn add_stem(project: &str, take: &str, bytes: Arc<[u8]>) -> Result<(), String> {
    use daw_standalone::audio_engine::source::AudioSource;
    use daw_standalone::audio_engine::streamed::Streamed;
    let Some(player) = player() else {
        return Err("no audio player installed".into());
    };
    let stream = OggStream::open(bytes).map_err(|e| e.to_string())?;
    let streamed = Streamed::new(stream.channels(), stream.sample_rate(), stream.frames());
    daw_standalone::audio_engine::materialize::attach_source(
        &player.daw,
        project,
        take,
        AudioSource::Streamed(streamed.clone()),
    );
    player
        .feeders
        .borrow_mut()
        .push(StreamFeeder::new(streamed, stream));
    Ok(())
}

/// How far the device is behind the engine's playhead, in seconds — what
/// the transport reading subtracts, so the playhead drawn is the one heard.
#[must_use]
pub fn latency_seconds() -> f64 {
    player().map_or(0.0, |p| {
        if p.out.borrow().is_none() || p.next.get().is_none() {
            return 0.0;
        }
        p.queued() as f64 / f64::from(RATE)
    })
}

/// Make and start the audio context. Call from inside a user gesture (a
/// press, a key) — the only place a browser lets audio start. Idempotent.
pub fn unlock() {
    let Some(player) = player() else { return };
    if let Some(out) = player.out.borrow().as_ref() {
        // Made already; a context the browser suspended resumes here.
        let _ = out.context.resume();
        return;
    }
    if player.starting.replace(true) {
        return;
    }
    let options = web_sys::AudioContextOptions::new();
    options.set_sample_rate(RATE as f32);
    let context = match web_sys::AudioContext::new_with_context_options(&options) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = ?e, "audio: no AudioContext");
            return;
        }
    };
    let _ = context.resume();
    // The module: a blob URL over the processor's source, so the page
    // serves no second file.
    let parts = js_sys::Array::of1(&wasm_bindgen::JsValue::from_str(PROCESSOR));
    let bag = web_sys::BlobPropertyBag::new();
    bag.set_type("text/javascript");
    let url = web_sys::Blob::new_with_str_sequence_and_options(&parts, &bag)
        .and_then(|blob| web_sys::Url::create_object_url_with_blob(&blob));
    let (Ok(worklet), Ok(url)) = (context.audio_worklet(), url) else {
        tracing::warn!("audio: no AudioWorklet");
        return;
    };
    let Ok(loading) = worklet.add_module(&url) else {
        tracing::warn!("audio: the worklet module did not load");
        return;
    };
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = wasm_bindgen_futures::JsFuture::from(loading).await {
            tracing::warn!(error = ?e, "audio: the worklet module did not load");
            return;
        }
        let options = web_sys::AudioWorkletNodeOptions::new();
        options.set_number_of_outputs(1);
        options.set_output_channel_count(&js_sys::Array::of1(&2.into()));
        let node = match web_sys::AudioWorkletNode::new_with_options(&context, "fts-out", &options)
        {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!(error = ?e, "audio: the worklet node did not start");
                return;
            }
        };
        let _ = node.connect_with_audio_node(&context.destination());
        let Ok(port) = node.port() else { return };
        let (played, epoch) = (Rc::clone(&player.played), Rc::clone(&player.epoch));
        let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(
            move |event: web_sys::MessageEvent| {
                let report = js_sys::Array::from(&event.data());
                let (Some(tag), Some(count)) =
                    (report.get(0).as_f64(), report.get(1).as_f64())
                else {
                    return;
                };
                if tag as u32 == epoch.get() {
                    played.set(count as u64);
                }
            },
        );
        port.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        tracing::info!(rate = context.sample_rate(), "audio: started");
        *player.out.borrow_mut() = Some(Out {
            context,
            port,
            _on_message: on_message,
        });
        player.flush();
    });
}

impl Player {
    fn queued(&self) -> u64 {
        self.sent.get().saturating_sub(self.played.get())
    }

    /// Empty the device's queue: a jump, a stop.
    fn flush(&self) {
        let epoch = self.epoch.get().wrapping_add(1);
        self.epoch.set(epoch);
        self.sent.set(0);
        self.played.set(0);
        if let Some(out) = self.out.borrow().as_ref() {
            let _ = out.port.post_message(&f64::from(epoch).into());
        }
    }

    fn tick(&self) {
        self.feed();
        self.render();
    }

    /// Decode stems toward the playhead, for up to the budget.
    fn feed(&self) {
        let mut feeders = self.feeders.borrow_mut();
        let count = feeders.len();
        if count == 0 {
            return;
        }
        let started = web_time::Instant::now();
        let first = self.first.get() % count;
        self.first.set(first + 1);
        let mut busy = true;
        while busy && started.elapsed().as_millis() < DECODE_BUDGET_MS {
            busy = false;
            for i in 0..count {
                busy |= feeders[(first + i) % count].pump(2048);
                if started.elapsed().as_millis() >= DECODE_BUDGET_MS {
                    break;
                }
            }
        }
    }

    /// Render ahead of the device while the transport advances.
    fn render(&self) {
        let Some((out, running)) = self.out.borrow().as_ref().map(|o| {
            (
                o.port.clone(),
                o.context.state() == web_sys::AudioContextState::Running,
            )
        }) else {
            return;
        };
        // The device drives the transport only while it runs; otherwise
        // the engine's own clock does, and nothing is queued.
        if !running {
            if self.clocked.replace(false) {
                self.bundle.enable_soft_clock();
                self.next.set(None);
                self.flush();
            }
            return;
        }
        if !self.clocked.replace(true) {
            self.bundle.disable_soft_clock();
            tracing::info!("audio: the device drives the transport");
        }
        if !self.shared.play_state().is_advancing() {
            if self.next.take().is_some() {
                self.flush();
                // Nothing renders while stopped, so nothing would write the
                // meters down: silence them, as a stopped engine's are.
                let meters = self.daw.meters();
                for i in 0..meters.len() {
                    if let Some(cell) = meters.cell(i) {
                        cell.write(0.0, 0.0, 0.0);
                    }
                }
            }
            return;
        }
        let playhead = self.shared.playhead_samples().0.max(0) as u64;
        // Anything but the block after the last is a jump: the queue holds
        // audio from somewhere else.
        if self.next.get() != Some(playhead) {
            self.flush();
        }
        let mut at = playhead;
        while self.queued() < AHEAD {
            let block = self.renderer.render_block(at, BLOCK);
            let samples = js_sys::Float32Array::from(&block.samples[..]);
            let transfer = js_sys::Array::of1(&samples.buffer());
            if out.post_message_with_transferable(&samples, &transfer).is_err() {
                break;
            }
            self.sent.set(self.sent.get() + BLOCK as u64);
            // `advance` answers where the block started; the playhead is
            // where the next one does (a loop may have wrapped it).
            self.shared.advance(BLOCK as u32);
            at = self.shared.playhead_samples().0.max(0) as u64;
        }
        self.next.set(Some(at));
    }
}

/// Unlock audio on the page's first press or key — a gesture is the only
/// place a browser lets a context start.
pub fn unlock_on_first_gesture() {
    let Some(window) = web_sys::window() else { return };
    let handler = Closure::<dyn FnMut(web_sys::Event)>::new(|_event: web_sys::Event| unlock());
    for kind in ["pointerdown", "keydown"] {
        let _ = window
            .add_event_listener_with_callback_and_bool(kind, handler.as_ref().unchecked_ref(), true);
    }
    // The page's lifetime: `unlock` is idempotent, so the listener stays.
    handler.forget();
}
