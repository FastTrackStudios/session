//! Track-group manager — REAPER-side operations over the canonical partition.
//!
//! REAPER exposes 128 track-group slots. The **partition** that assigns those
//! slots to fixed instrument-category bands is canonical and lives in
//! [`music_catalog::groups`] (the single source of truth shared across FTS
//! repos). This module owns only the REAPER-side operations that *realize*
//! that partition in a live project.
//!
//! REAPER FFI here is raw `low`-level (group naming + membership aren't
//! wrapped in `reaper-medium`), called on REAPER's **main thread** — same
//! pattern as [`crate::record_actions`] and [`crate::take_ranking`].

use music_catalog::groups::{SLOT_BANDS, TOTAL_GROUP_SLOTS, band_for_category, slot_label};
use reaper_high::Reaper;
use std::ffi::CString;
use tracing::{info, warn};

/// Write the instrument-category partition into the current project's 128
/// track-group names (`TRACK_GROUP_NAME:X`). Idempotent — re-running just
/// rewrites the same labels. Must run on REAPER's main thread.
///
/// Returns the number of slots successfully named.
pub fn apply_group_naming() -> u32 {
    let reaper = Reaper::get();
    let low = reaper.medium_reaper().low();
    let mut named = 0u32;
    for slot in 1..=TOTAL_GROUP_SLOTS {
        let Ok(desc) = CString::new(format!("TRACK_GROUP_NAME:{slot}")) else {
            continue;
        };
        // `is_set` reads `valuestr`; a NUL-terminated copy of the label is
        // all REAPER needs for the set path.
        let Ok(value) = CString::new(slot_label(slot)) else {
            continue;
        };
        let mut buf = value.into_bytes_with_nul();
        let ok = unsafe {
            low.GetSetProjectInfo_String(
                std::ptr::null_mut(), // current project
                desc.as_ptr(),
                buf.as_mut_ptr() as *mut std::os::raw::c_char,
                true, // is_set
            )
        };
        if ok {
            named += 1;
        } else {
            warn!(slot, "[group] failed to set TRACK_GROUP_NAME");
        }
    }
    info!(
        named,
        bands = SLOT_BANDS.len(),
        "[group] Applied instrument-category naming to track groups"
    );
    named
}

/// Flag families set for a "mutual" group — both `_LEAD` and `_FOLLOW` on
/// every member, so mute/solo/volume/etc. on any member moves the whole
/// group equally (no single lead).
const FLAG_FAMILIES: &[&str] = &[
    "MEDIA_EDIT",
    "VOLUME",
    "VOLUME_VCA",
    "PAN",
    "WIDTH",
    "MUTE",
    "SOLO",
    "RECARM",
    "POLARITY",
    "AUTOMODE",
];

/// The `GetSetTrackGroupMembershipEx` window + bit for a 1-based slot.
/// REAPER addresses the 128 slots as four 32-bit windows.
fn slot_window(slot: u16) -> (i32, u32) {
    let idx = slot - 1;
    let offset = (idx / 32) * 32;
    let mask = 1u32 << (idx % 32);
    (offset as i32, mask)
}

/// Assign the currently-selected tracks to the next free slot in `category`'s
/// band, as a mutual group (all flag families, lead+follow). Returns the
/// number of tracks assigned. Must run on REAPER's main thread.
pub fn assign_selected_to_category(category: &str) -> usize {
    let Some(band) = band_for_category(category) else {
        warn!(category, "[group] unknown category");
        return 0;
    };
    let reaper = Reaper::get();
    let low = reaper.medium_reaper().low();

    let probe = match CString::new("VOLUME_VCA_LEAD") {
        Ok(c) => c,
        Err(_) => return 0,
    };

    unsafe {
        // Collect the selection up front.
        let n_sel = low.CountSelectedTracks(std::ptr::null_mut());
        if n_sel == 0 {
            warn!("[group] no tracks selected to assign");
            return 0;
        }
        let mut selected = Vec::with_capacity(n_sel as usize);
        for i in 0..n_sel {
            let t = low.GetSelectedTrack(std::ptr::null_mut(), i);
            if !t.is_null() {
                selected.push(t);
            }
        }

        // Find the next free slot in the band — one whose VCA-lead bit is
        // unset on every track in the project.
        let n_tracks = low.CountTracks(std::ptr::null_mut());
        let mut free_slot = None;
        'slots: for slot in band.start..=band.end() {
            let (offset, mask) = slot_window(slot);
            for ti in 0..n_tracks {
                let t = low.GetTrack(std::ptr::null_mut(), ti);
                if t.is_null() {
                    continue;
                }
                let state = low.GetSetTrackGroupMembershipEx(t, probe.as_ptr(), offset, 0, 0);
                if state & mask != 0 {
                    continue 'slots; // slot in use
                }
            }
            free_slot = Some(slot);
            break;
        }
        let Some(slot) = free_slot else {
            warn!(band = band.label, "[group] no free slot in band");
            return 0;
        };

        let (offset, mask) = slot_window(slot);
        let undo = CString::new(format!(
            "Assign {} track(s) to group {}",
            selected.len(),
            slot_label(slot)
        ))
        .unwrap();
        low.Undo_BeginBlock();
        low.PreventUIRefresh(1);
        for &t in &selected {
            for fam in FLAG_FAMILIES {
                for suffix in ["_LEAD", "_FOLLOW"] {
                    if let Ok(name) = CString::new(format!("{fam}{suffix}")) {
                        low.GetSetTrackGroupMembershipEx(t, name.as_ptr(), offset, mask, mask);
                    }
                }
            }
        }
        low.PreventUIRefresh(-1);
        low.Undo_EndBlock(undo.as_ptr(), 1); // 1 = track config changes
        info!(
            category = band.label,
            slot,
            label = %slot_label(slot),
            tracks = selected.len(),
            "[group] Assigned selection to group slot"
        );
        selected.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The partition itself (band contiguity, slot labels, category lookups)
    // is tested in `music_catalog::groups`. Here we only cover the REAPER
    // membership bit-math that stays in this crate.
    #[test]
    fn slot_windows() {
        assert_eq!(slot_window(1), (0, 1));
        assert_eq!(slot_window(32), (0, 1 << 31));
        assert_eq!(slot_window(33), (32, 1));
        assert_eq!(slot_window(65), (64, 1));
        assert_eq!(slot_window(128), (96, 1 << 31));
    }
}
