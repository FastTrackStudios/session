//! Waveforms as fine as the zoom asks for, down to the samples.
//!
//! An item's waveform is read once, at [`crate::midi::WAVE_BLOCK`]
//! samples a point (about 21 ms at 48 kHz): plenty for a song on a
//! screen, and a blur zoomed in on one hit — which is where drums are
//! edited. So when the view is closer than that, the stretch on screen is
//! read again from the source, at the source's rate, and reduced to a
//! point every half pixel; zoomed in further than a pixel a sample, the
//! samples themselves are kept and drawn as a line through them.
//!
//! The reads are off the paint: a frame asks ([`get`]), draws the coarse
//! waveform while the detail is read, and the window is asked for a
//! frame when it arrives ([`generation`]). They run in parallel on the
//! waveform pool (`crate::wave_pool`), the newest ask first: a
//! slip-edited kit is thousands of items, each read on its own, and what
//! is on screen now matters more than what was a scroll ago. What was read is kept for the
//! item as it was read — a moved or slipped item is read again.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// A real item a lane draws: which, and where it was when asked.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub guid: String,
    /// Project seconds.
    pub at: f64,
    pub length: f64,
    /// Seconds into its source.
    pub offset: f64,
}

impl Source {
    const fn key(&self) -> (u64, u64, u64) {
        (
            self.at.to_bits(),
            self.length.to_bits(),
            self.offset.to_bits(),
        )
    }
}

/// A stretch of an item's audio, fine.
#[derive(Clone, Debug, PartialEq)]
pub struct Detail {
    /// Project seconds of the first point.
    pub from: f64,
    /// Seconds each point stands for.
    pub step: f64,
    /// Every point a sample (`(value, value)`), not a run's loudest.
    pub samples: bool,
    /// `(max, min)` per point; a sample's value twice.
    pub points: Vec<(f32, f32)>,
}

impl Detail {
    /// Project seconds after the last point.
    #[must_use]
    pub fn to(&self) -> f64 {
        crate::num::coord(self.points.len()).mul_add(self.step, self.from)
    }

    /// The points from `a` to `b` seconds, with each one's time.
    pub fn between(&self, a: f64, b: f64) -> impl Iterator<Item = (f64, (f32, f32))> + '_ {
        let step = self.step.max(f64::EPSILON);
        let first = crate::num::index(((a - self.from) / step).floor() - 1.0);
        let last = crate::num::index(((b - self.from) / step).ceil() + 1.0);
        self.points
            .iter()
            .enumerate()
            .skip(first)
            .take(last.saturating_sub(first).saturating_add(1))
            .map(move |(i, p)| (crate::num::coord(i).mul_add(step, self.from), *p))
    }
}

/// `samples` (interleaved, `channels` of them, at `rate`, the first at
/// `from` seconds) as points `per` samples each — or, `per` 1 or less,
/// the samples themselves (the channel furthest from zero at each).
#[must_use]
pub fn reduce(samples: &[f64], channels: usize, rate: f64, from: f64, per: usize) -> Detail {
    let channels = channels.max(1);
    let frames: Vec<(f32, f32)> = samples
        .chunks(channels)
        .map(|frame| {
            let (mut max, mut min) = (f64::NEG_INFINITY, f64::INFINITY);
            for &v in frame {
                max = max.max(v);
                min = min.min(v);
            }
            let f = |v: f64| num_traits::ToPrimitive::to_f32(&v).unwrap_or(0.0);
            (f(max), f(min))
        })
        .collect();
    if per <= 1 {
        let points = frames
            .iter()
            .map(|&(max, min)| {
                let v = if max.abs() >= min.abs() { max } else { min };
                (v, v)
            })
            .collect();
        return Detail {
            from,
            step: 1.0 / rate,
            samples: true,
            points,
        };
    }
    let points = frames
        .chunks(per)
        .map(|run| {
            run.iter()
                .fold((f32::NEG_INFINITY, f32::INFINITY), |(mx, mn), &(a, b)| {
                    (mx.max(a), mn.min(b))
                })
        })
        .collect();
    Detail {
        from,
        step: crate::num::coord(per) / rate,
        samples: false,
        points,
    }
}

/// What was read, by item.
struct Held {
    key: (u64, u64, u64),
    detail: Arc<Detail>,
}

/// What is wanted, by item: the item, the span, the seconds a point.
type Want = (Source, f64, f64, f64);

#[derive(Default)]
struct Shared {
    held: HashMap<String, Held>,
    /// What is wanted, with when it was asked (higher is newer).
    wanted: HashMap<String, (Want, u64)>,
    /// Asked for and not yet read, so a frame does not ask again.
    reading: HashMap<String, Want>,
    /// Asks so far: the next one's order.
    asked: u64,
}

static SHARED: std::sync::LazyLock<Mutex<Shared>> =
    std::sync::LazyLock::new(|| Mutex::new(Shared::default()));
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Goes up each time detail arrives: a window compares it with the one
/// it drew to know to draw again.
#[must_use]
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// The most samples one read takes: past that the stretch is read at a
/// lower rate (the coarse waveform is near enough there anyway).
const MOST_SAMPLES: f64 = 4_000_000.0;

/// `source`'s audio from `a` to `b` project seconds at `step` seconds a
/// point or finer, if it has been read.
///
/// If not, it is asked for (with some to spare either side, so a small
/// pan does not read again) and this is `None`.
#[must_use]
pub fn get(source: &Source, a: f64, b: f64, step: f64) -> Option<Arc<Detail>> {
    let mut shared = SHARED.lock().ok()?;
    if let Some(held) = shared.held.get(&source.guid)
        && held.key == source.key()
        && held.detail.from <= a + 1e-9
        && held.detail.to() >= b - 1e-9
        && (held.detail.samples || held.detail.step <= step * 1.5)
    {
        return Some(Arc::clone(&held.detail));
    }
    let spare = (b - a) * 0.5;
    let want: Want = (
        source.clone(),
        (a - spare).max(source.at),
        (b + spare).min(source.at + source.length),
        step,
    );
    if shared.reading.get(&source.guid) != Some(&want)
        && shared.wanted.get(&source.guid).map(|(w, _)| w) != Some(&want)
    {
        shared.asked = shared.asked.wrapping_add(1);
        let order = shared.asked;
        shared.wanted.insert(source.guid.clone(), (want, order));
        drop(shared);
        // One read for each ask, of whatever is newest when it runs.
        read_newest();
    }
    None
}

/// Hold `detail` as `source`'s, as a read would.
#[cfg(test)]
pub fn hold(source: &Source, detail: Detail) {
    if let Ok(mut shared) = SHARED.lock() {
        shared.held.insert(
            source.guid.clone(),
            Held {
                key: source.key(),
                detail: Arc::new(detail),
            },
        );
    }
}

/// Read the newest ask on the waveform pool. Each ask queues one of
/// these; whichever runs takes what is newest then, so a stale ask is
/// read last, if at all.
#[cfg(feature = "native")]
fn read_newest() {
    crate::wave_pool::spawn(|| {
        let (guid, want) = {
            let Ok(mut shared) = SHARED.lock() else {
                return;
            };
            let Some(guid) = newest(&shared.wanted) else {
                return;
            };
            let Some((want, _)) = shared.wanted.remove(&guid) else {
                return;
            };
            shared.reading.insert(guid.clone(), want.clone());
            (guid, want)
        };
        let read = read(&want);
        if let Ok(mut shared) = SHARED.lock() {
            if shared.reading.get(&guid) == Some(&want) {
                shared.reading.remove(&guid);
            }
            if let Some(detail) = read {
                shared.held.insert(
                    guid,
                    Held {
                        key: want.0.key(),
                        detail: Arc::new(detail),
                    },
                );
                GENERATION.fetch_add(1, Ordering::Relaxed);
            }
        }
    });
}

/// Nowhere to read from in a page: the coarse waveform stays.
#[cfg(not(feature = "native"))]
const fn read_newest() {}

/// The item asked for last.
fn newest(wanted: &HashMap<String, (Want, u64)>) -> Option<String> {
    wanted
        .iter()
        .max_by_key(|(_, (_, order))| *order)
        .map(|(guid, _)| guid.clone())
}

/// Read one stretch from the engine.
#[cfg(feature = "native")]
fn read((source, a, b, step): &Want) -> Option<Detail> {
    use daw::service::{ItemRef, ProjectContext, TakeRef};
    use daw_proto::{AudioAccessors as _, GetSamplesRequest};
    let daw = crate::open::local_engine()?;
    let project = ProjectContext::Project(crate::open::current_song()?);
    let accessor =
        daw.create_take_accessor(project, ItemRef::Guid(source.guid.clone()), TakeRef::Active)?;

    // The source's own rate and channels: a read of none says them.
    let format = daw.get_samples(GetSamplesRequest {
        accessor_id: accessor.clone(),
        sample_rate: 0.0,
        num_channels: 0,
        start_time: 0.0,
        num_samples: 0,
    });
    let span = (b - a).max(0.0);
    let native = format.sample_rate.max(1.0);
    let rate = native.min(MOST_SAMPLES / span.max(f64::EPSILON));
    let frames = num_traits::ToPrimitive::to_u32(&(span * rate).ceil()).unwrap_or(0);
    let channels = format.num_channels.max(1);
    let data = daw.get_samples(GetSamplesRequest {
        accessor_id: accessor.clone(),
        sample_rate: rate,
        num_channels: channels,
        start_time: a - source.at,
        num_samples: frames,
    });
    daw.destroy_accessor(&accessor);
    let per = crate::num::index((step * rate / 2.0).floor()).max(1);
    let channels = usize::try_from(channels).unwrap_or(1);
    Some(reduce(&data.samples, channels, rate, *a, per))
}

#[cfg(not(feature = "native"))]
const fn read(_: &Want) -> Option<Detail> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader takes the item asked for last: what is on screen now,
    /// before what was a scroll ago.
    #[test]
    fn the_newest_ask_is_read_first() {
        let want = |guid: &str| {
            (
                Source {
                    guid: guid.to_owned(),
                    at: 0.0,
                    length: 1.0,
                    offset: 0.0,
                },
                0.0,
                1.0,
                0.01,
            )
        };
        let mut wanted = HashMap::new();
        for (order, guid) in ["a", "b", "c"].iter().enumerate() {
            wanted.insert(
                (*guid).to_owned(),
                (want(guid), u64::try_from(order).unwrap_or(0)),
            );
        }
        wanted.insert("a".to_owned(), (want("a"), 9));
        assert_eq!(newest(&wanted).as_deref(), Some("a"), "asked again, newest");
        wanted.remove("a");
        assert_eq!(newest(&wanted).as_deref(), Some("c"));
    }

    /// Runs of samples become their loudest each way, both channels'.
    #[test]
    fn runs_keep_their_peaks() {
        let samples = [0.1, -0.2, 0.5, 0.0, -0.7, 0.3, 0.2, 0.2];
        let detail = reduce(&samples, 2, 4.0, 10.0, 2);
        assert!(!detail.samples);
        assert_eq!(detail.points, [(0.5, -0.2), (0.3, -0.7)]);
        assert!((detail.step - 0.5).abs() < 1e-12);
        assert!((detail.to() - 11.0).abs() < 1e-12);
    }

    /// Zoomed in past a sample a pixel, the samples are the points — each
    /// the channel further from zero, sign and all.
    #[test]
    fn at_the_sample_level_the_samples_are_kept() {
        let samples = [0.1, -0.2, 0.5, 0.0];
        let detail = reduce(&samples, 2, 48_000.0, 0.0, 1);
        assert!(detail.samples);
        assert_eq!(detail.points, [(-0.2, -0.2), (0.5, 0.5)]);
    }

    #[test]
    fn between_gives_the_points_in_a_span_with_their_times() {
        let detail = reduce(&[0.0; 10], 1, 10.0, 5.0, 1);
        let times: Vec<f64> = detail.between(5.3, 5.5).map(|(t, _)| t).collect();
        // The span, and a point past it either side so a line drawn
        // through them reaches its edges.
        assert!(times.first().is_some_and(|t| *t < 5.3), "{times:?}");
        assert!(times.last().is_some_and(|t| *t > 5.5), "{times:?}");
        assert!(times.len() <= 6, "and not the rest: {times:?}");
    }
}
