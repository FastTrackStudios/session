//! Track-group manager.
//!
//! REAPER exposes 128 track-group slots. Rather than let groups accrete
//! ad-hoc, FTS partitions the 128 slots into fixed **instrument-category
//! bands** so a project's groups are always organized the same way: drums
//! own the first block, bass the next, and so on. This module owns that
//! partition plus the REAPER-side operations that realize it.
//!
//! REAPER FFI here is raw `low`-level (group naming + membership aren't
//! wrapped in `reaper-medium`), called on REAPER's **main thread** — same
//! pattern as [`crate::record_actions`] and [`crate::take_ranking`].

use reaper_high::Reaper;
use std::ffi::CString;
use tracing::{info, warn};

/// Total REAPER track-group slots (REAPER addresses these as four 32-bit
/// windows via `GetSetTrackGroupMembershipEx`'s `offset` arg).
pub const TOTAL_SLOTS: u16 = 128;

/// One instrument-category band: a contiguous run of group slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupBand {
    pub name: &'static str,
    /// 1-based first slot (inclusive).
    pub start: u16,
    /// Number of slots in the band.
    pub count: u16,
}

impl GroupBand {
    /// 1-based last slot (inclusive).
    pub const fn end(&self) -> u16 {
        self.start + self.count - 1
    }
    pub const fn contains(&self, slot: u16) -> bool {
        slot >= self.start && slot <= self.end()
    }
}

/// The canonical FTS instrument-category partition over the 128 group slots.
/// Bands are contiguous and non-overlapping; slots past the last band
/// (121–128) are spare. Total assigned = 120.
pub const BANDS: &[GroupBand] = &[
    GroupBand {
        name: "Drums",
        start: 1,
        count: 10,
    },
    GroupBand {
        name: "Bass",
        start: 11,
        count: 10,
    },
    GroupBand {
        name: "Electric Gtr",
        start: 21,
        count: 20,
    },
    GroupBand {
        name: "Acoustic Gtr",
        start: 41,
        count: 20,
    },
    GroupBand {
        name: "Keys",
        start: 61,
        count: 10,
    },
    GroupBand {
        name: "Synths",
        start: 71,
        count: 10,
    },
    GroupBand {
        name: "Lead Vocal",
        start: 81,
        count: 20,
    },
    GroupBand {
        name: "Background Vox",
        start: 101,
        count: 20,
    },
];

/// First spare slot (one past the last band), or `TOTAL_SLOTS + 1` if the
/// bands fill every slot.
fn spare_start() -> u16 {
    BANDS.last().map(|b| b.end() + 1).unwrap_or(1)
}

/// The band owning `slot` (1-based), or `None` for spare slots.
pub fn band_for_slot(slot: u16) -> Option<&'static GroupBand> {
    BANDS.iter().find(|b| b.contains(slot))
}

/// Look up a band by category name (case-insensitive).
pub fn band_for_category(name: &str) -> Option<&'static GroupBand> {
    BANDS.iter().find(|b| b.name.eq_ignore_ascii_case(name))
}

/// The human label for a slot, e.g. `"Drums 03"` or `"Spare 02"`.
pub fn slot_label(slot: u16) -> String {
    match band_for_slot(slot) {
        Some(b) => format!("{} {:02}", b.name, slot - b.start + 1),
        None => format!("Spare {:02}", slot.saturating_sub(spare_start()) + 1),
    }
}

/// Write the instrument-category partition into the current project's 128
/// track-group names (`TRACK_GROUP_NAME:X`). Idempotent — re-running just
/// rewrites the same labels. Must run on REAPER's main thread.
///
/// Returns the number of slots successfully named.
pub fn apply_group_naming() -> u32 {
    let reaper = Reaper::get();
    let low = reaper.medium_reaper().low();
    let mut named = 0u32;
    for slot in 1..=TOTAL_SLOTS {
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
        bands = BANDS.len(),
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
            warn!(band = band.name, "[group] no free slot in band");
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
            category = band.name,
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

    #[test]
    fn slot_windows() {
        assert_eq!(slot_window(1), (0, 1));
        assert_eq!(slot_window(32), (0, 1 << 31));
        assert_eq!(slot_window(33), (32, 1));
        assert_eq!(slot_window(65), (64, 1));
        assert_eq!(slot_window(128), (96, 1 << 31));
    }

    #[test]
    fn bands_are_contiguous_non_overlapping_and_fit() {
        let mut expected_next = 1;
        for b in BANDS {
            assert_eq!(b.start, expected_next, "band {} not contiguous", b.name);
            expected_next = b.end() + 1;
        }
        assert!(expected_next - 1 <= TOTAL_SLOTS, "bands overflow 128 slots");
    }

    #[test]
    fn slot_labels() {
        assert_eq!(slot_label(1), "Drums 01");
        assert_eq!(slot_label(10), "Drums 10");
        assert_eq!(slot_label(11), "Bass 01");
        assert_eq!(slot_label(81), "Lead Vocal 01");
        assert_eq!(slot_label(120), "Background Vox 20");
        assert_eq!(slot_label(121), "Spare 01");
        assert_eq!(slot_label(128), "Spare 08");
    }

    #[test]
    fn band_lookups() {
        assert_eq!(band_for_slot(25).unwrap().name, "Electric Gtr");
        assert!(band_for_slot(125).is_none());
        assert_eq!(band_for_category("keys").unwrap().start, 61);
    }
}
