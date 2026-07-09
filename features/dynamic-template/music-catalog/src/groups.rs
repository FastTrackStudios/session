//! Canonical instrument-group slot registry — the REAPER 128-slot partition.
//!
//! REAPER exposes 128 track-group slots. Rather than let groups accrete
//! ad-hoc, FastTrackStudio partitions the 128 slots into fixed **instrument-
//! category bands** so every project's groups are organized the same way:
//! drums own the first block, bass the next, and so on. This partition is
//! canonical and shared — the REAPER extension writes these labels into the
//! project's `TRACK_GROUP_NAME` slots, and any other repo can reason about
//! slot ownership without re-deriving it.
//!
//! Group identity is a hierarchical **path** (top-level first), whose
//! elements match the classification-engine group names (`dynamic-template`)
//! and the color-lookup keys ([`crate::lookup`]). e.g. `["Guitars", "Electric"]`.

use crate::Color;

/// Total REAPER track-group slots. REAPER addresses these as four 32-bit
/// windows (slots 1–32, 33–64, 65–96, 97–128).
pub const TOTAL_GROUP_SLOTS: u16 = 128;

/// One instrument-category band: a contiguous run of group slots owned by a
/// single canonical group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupSlotBand {
    /// Canonical hierarchical group path, top-level first, e.g.
    /// `["Guitars", "Electric"]`. Elements match the classification-engine
    /// group names and the color-lookup keys.
    pub path: &'static [&'static str],
    /// Short label written to REAPER's `TRACK_GROUP_NAME` slots,
    /// e.g. `"Electric Gtr"`.
    pub label: &'static str,
    /// 1-based first slot (inclusive).
    pub start: u16,
    /// Number of slots in the band.
    pub count: u16,
}

impl GroupSlotBand {
    /// 1-based last slot (inclusive).
    pub const fn end(&self) -> u16 {
        self.start + self.count - 1
    }

    /// Whether the 1-based `slot` falls inside this band.
    pub const fn contains(&self, slot: u16) -> bool {
        slot >= self.start && slot <= self.end()
    }

    /// Canonical color for this band, resolved through the hierarchical path
    /// lookup (e.g. `Guitars/Electric` → electric-guitar color).
    pub fn color(&self) -> Option<Color> {
        crate::lookup::color_for_path(self.path)
    }
}

/// The canonical FTS instrument-category partition over the 128 group slots.
/// Bands are contiguous and non-overlapping; slots past the last band
/// (121–128) are spare. Total assigned = 120.
pub const SLOT_BANDS: &[GroupSlotBand] = &[
    GroupSlotBand {
        path: &["Drums"],
        label: "Drums",
        start: 1,
        count: 10,
    },
    GroupSlotBand {
        path: &["Bass"],
        label: "Bass",
        start: 11,
        count: 10,
    },
    GroupSlotBand {
        path: &["Guitars", "Electric"],
        label: "Electric Gtr",
        start: 21,
        count: 20,
    },
    GroupSlotBand {
        path: &["Guitars", "Acoustic"],
        label: "Acoustic Gtr",
        start: 41,
        count: 20,
    },
    GroupSlotBand {
        path: &["Keys"],
        label: "Keys",
        start: 61,
        count: 10,
    },
    GroupSlotBand {
        path: &["Synths"],
        label: "Synths",
        start: 71,
        count: 10,
    },
    GroupSlotBand {
        path: &["Vocals", "Lead"],
        label: "Lead Vocal",
        start: 81,
        count: 20,
    },
    GroupSlotBand {
        // Path element matches the classification config's sub-group name
        // ("BGVs"); the label is the friendly REAPER group-slot name.
        path: &["Vocals", "BGVs"],
        label: "Background Vox",
        start: 101,
        count: 20,
    },
];

/// First spare slot (one past the last band), or `TOTAL_GROUP_SLOTS + 1` if
/// the bands fill every slot.
pub fn spare_start() -> u16 {
    SLOT_BANDS.last().map(|b| b.end() + 1).unwrap_or(1)
}

/// The band owning `slot` (1-based), or `None` for spare slots.
pub fn band_for_slot(slot: u16) -> Option<&'static GroupSlotBand> {
    SLOT_BANDS.iter().find(|b| b.contains(slot))
}

/// Look up a band by category, matching the short REAPER label, any element
/// of the canonical path, or the full `a/b` path string — all case-
/// insensitive. So `"electric"`, `"Electric Gtr"`, and `"guitars/electric"`
/// all resolve to the same band.
pub fn band_for_category(name: &str) -> Option<&'static GroupSlotBand> {
    let needle = name.trim();
    SLOT_BANDS.iter().find(|b| {
        b.label.eq_ignore_ascii_case(needle)
            || b.path.iter().any(|p| p.eq_ignore_ascii_case(needle))
            || b.path.join("/").eq_ignore_ascii_case(needle)
    })
}

/// The human label for a slot, e.g. `"Drums 03"` or `"Spare 02"`.
pub fn slot_label(slot: u16) -> String {
    match band_for_slot(slot) {
        Some(b) => format!("{} {:02}", b.label, slot - b.start + 1),
        None => format!("Spare {:02}", slot.saturating_sub(spare_start()) + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_are_contiguous_non_overlapping_and_fit() {
        let mut expected_next = 1;
        for b in SLOT_BANDS {
            assert_eq!(b.start, expected_next, "band {} not contiguous", b.label);
            expected_next = b.end() + 1;
        }
        assert!(
            expected_next - 1 <= TOTAL_GROUP_SLOTS,
            "bands overflow 128 slots"
        );
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
        assert_eq!(band_for_slot(25).unwrap().label, "Electric Gtr");
        assert!(band_for_slot(125).is_none());
        assert_eq!(band_for_category("keys").unwrap().start, 61);
        // Path-element and full-path forms resolve to the same band.
        assert_eq!(band_for_category("electric").unwrap().start, 21);
        assert_eq!(band_for_category("guitars/electric").unwrap().start, 21);
        assert_eq!(band_for_category("Electric Gtr").unwrap().start, 21);
    }
}
