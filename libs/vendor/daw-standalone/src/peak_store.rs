//! Persistent source-level waveform peaks — REAPER `.reapeaks` sidecars.
//!
//! Take-level peaks depend on placement, play rate and stretch markers,
//! which is why [`crate::peak`]'s in-memory cache is revision-keyed. The
//! *expensive* part, though, is scanning the source PCM — and that is a
//! property of the media file alone. So the disk artifact lives at the
//! SOURCE level: a REAPER-compatible `.reapeaks` mipmap next to each
//! on-disk media file (`<mediafile>.reapeaks`, REAPER's own naming), so
//! REAPER and FTS share sidecars where projects overlap.
//!
//! Flow: the first peaks request for a source loads a valid sidecar, or
//! scans the PCM once, writes the sidecar, and keeps the parsed mipmap in
//! a process-global map (keyed by media path, revalidated by mtime).
//! Cold starts after that fold coarse-zoom peaks from the sidecar instead
//! of rescanning gigabytes of PCM. Fine zooms (below the finest mipmap
//! ratio, 160 samples/peak) still read PCM — the mipmap can't resolve
//! them.
//!
//! Validation matches REAPER's model: the sidecar stores the source
//! file's mtime (seconds), and we additionally require the channel
//! count, sample rate and length (within one fine peak window) to match
//! the opened source. Anything stale is recomputed and rewritten.
//!
//! In-memory sources ([`AudioSource::Memory`], compressed decodes) get no
//! sidecar — their `min_max_block` is a RAM walk, cheap enough to redo.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use dawfile_reaper::reapeaks::ReaPeaks;

use crate::audio_engine::AudioSource;

/// The sidecar path for a media file: REAPER's naming — the full file
/// name (extension included) plus `.reapeaks`, next to the media.
pub(crate) fn sidecar_path(media: &Path) -> PathBuf {
    let mut os = media.as_os_str().to_owned();
    os.push(".reapeaks");
    PathBuf::from(os)
}

/// Media-file mtime in whole seconds since the epoch — the stamp the
/// sidecar records for invalidation.
fn media_mtime_secs(media: &Path) -> Option<u64> {
    std::fs::metadata(media)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// A sidecar is valid for `source` iff its recorded mtime matches the
/// media file and its shape matches the opened audio: same channels,
/// same rate, and the finest level covers the source length (peak count
/// is `ceil(frames / spp)`, so the covered length may exceed the frame
/// count by at most one window).
fn is_valid(pk: &ReaPeaks, source: &AudioSource, media_mtime: u64) -> bool {
    let Some(fine) = pk.levels.first() else {
        return false;
    };
    let frames = source.frame_count() as u64;
    let spp = fine.samples_per_peak.max(1) as u64;
    pk.source_mtime == media_mtime
        && pk.channels == source.channels().max(1) as usize
        && pk.samplerate == source.sample_rate()
        && fine.count as u64 == frames.div_ceil(spp)
}

type Store = HashMap<PathBuf, (u64, Arc<ReaPeaks>)>;

/// Process-global parsed-sidecar map — `Standalone` is a cloneable
/// handle (same reason the peaks cache in [`crate::peak`] is global),
/// and one media file may back takes in several projects.
fn store() -> &'static Mutex<Store> {
    static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The parsed peaks for an on-disk media file: in-process map, else a
/// valid sidecar, else one PCM scan + sidecar write. `None` when the
/// media file doesn't exist (nothing to stamp the cache against).
pub(crate) fn get_or_build(media: &Path, source: &AudioSource) -> Option<Arc<ReaPeaks>> {
    let mtime = media_mtime_secs(media)?;
    if let Ok(map) = store().lock()
        && let Some((stamp, pk)) = map.get(media)
        && *stamp == mtime
    {
        return Some(pk.clone());
    }

    let side = sidecar_path(media);
    let pk = match ReaPeaks::read(&side) {
        Ok(pk) if is_valid(&pk, source, mtime) => pk,
        _ => {
            // Absent or stale: one scan of the source PCM, stamped with
            // the media mtime. The write is best-effort — a read-only
            // media directory just means the next cold start rescans.
            let mut pk = ReaPeaks::compute(
                source.channels().max(1) as usize,
                source.sample_rate(),
                source.frame_count(),
                |frame, ch| source.channel_interp(frame, frame, 0.0, ch),
            );
            pk.source_mtime = mtime;
            if let Err(err) = pk.write(&side) {
                tracing::warn!(
                    peaks.sidecar = %side.display(),
                    peaks.write_error = %err,
                    "reapeaks sidecar write failed; peaks stay in-memory only"
                );
            }
            pk
        }
    };
    let pk = Arc::new(pk);
    if let Ok(mut map) = store().lock() {
        map.insert(media.to_path_buf(), (mtime, pk.clone()));
    }
    Some(pk)
}

/// Min/max over source frames `[lo, hi)` for one channel, folded from
/// the mipmap (REAPER's pick: the coarsest level resolving the span).
/// Peaks cover fixed absolute windows, so the result may be up to one
/// window wider than the exact range on each side — conservative
/// (never narrower than the true min/max). Returns `(min, max)`.
pub(crate) fn min_max_block(pk: &ReaPeaks, lo: usize, hi: usize, channel: usize) -> (f32, f32) {
    let level = pk.level_for((hi.saturating_sub(lo)) as f64);
    let per = (level.samples_per_peak as usize).max(1);
    let a = lo / per;
    let b = hi.div_ceil(per).min(level.count).max(a);
    let (mut max, mut min) = (f32::MIN, f32::MAX);
    for p in a..b {
        let (pmax, pmin) = level.pair(pk.channels, channel, p);
        max = max.max(pmax);
        min = min.min(pmin);
    }
    if max < min { (0.0, 0.0) } else { (min, max) }
}
