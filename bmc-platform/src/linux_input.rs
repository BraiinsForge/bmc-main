// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

//! Capability-based discovery of evdev touchscreen nodes on Linux
//! appliance images.
//!
//! The appliance runtime (OpenWrt with `libudev-zero`) does not carry a
//! pre-populated udev database, so libinput's udev-backed enumeration is
//! unavailable. This module walks `/sys/class/input/event*`, reads the
//! kernel-published capability bitmaps, and returns the canonical
//! `/dev/input/eventN` node for **the** touchscreen — that is, any node
//! that declares `ABS_X`, `ABS_Y`, and `BTN_TOUCH`.
//!
//! Multiple evdev handlers can legitimately attach to a single physical
//! input device (QEMU's virtio tablet does this, and some HW touch
//! controllers expose secondary multitouch handlers). Returning all of
//! them would double-deliver touch events on the compositor side, so the
//! public function is deliberately `Option<PathBuf>`: candidates are
//! grouped by their parent input device and the lowest-index handler per
//! physical device is chosen. Across multiple *distinct* physical
//! touchscreens, the lowest-index candidate wins and a warning is
//! emitted — the appliance contract is "one panel", and anything else
//! should be pinned explicitly via the caller's configuration surface.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const SYSFS_INPUT_ROOT: &str = "/sys/class/input";
const DEVFS_INPUT_ROOT: &str = "/dev/input";

// evdev bit positions; see include/uapi/linux/input-event-codes.h.
const ABS_X: usize = 0x00;
const ABS_Y: usize = 0x01;
const ABS_MT_POSITION_X: usize = 0x35;
const ABS_MT_POSITION_Y: usize = 0x36;
const BTN_TOUCH: usize = 0x14a;

const WORD_BITS: usize = usize::BITS as usize;

/// Return the canonical `/dev/input/eventN` path of the touchscreen on
/// this appliance, or `None` when none is present.
#[must_use]
pub fn discover_touch_node() -> Option<PathBuf> {
    discover_touch_node_in(Path::new(SYSFS_INPUT_ROOT), Path::new(DEVFS_INPUT_ROOT))
}

fn discover_touch_node_in(sysfs_input: &Path, devfs_input: &Path) -> Option<PathBuf> {
    let candidates = enumerate_touchscreens(sysfs_input, devfs_input);
    if candidates.is_empty() {
        return None;
    }

    // Group by parent input device: multiple event nodes can share a
    // single physical panel, and libinput will double-deliver if we hand
    // it both.
    let mut by_parent: BTreeMap<PathBuf, Candidate> = BTreeMap::new();
    for candidate in candidates {
        by_parent
            .entry(candidate.parent.clone())
            .and_modify(|existing| {
                if candidate.index < existing.index {
                    *existing = candidate.clone();
                }
            })
            .or_insert(candidate);
    }

    let distinct_devices = by_parent.len();
    if distinct_devices > 1 {
        tracing::warn!(
            "input discovery: {distinct_devices} distinct touchscreens found; \
             picking the lowest-index node — pin one explicitly if this is wrong"
        );
    }

    by_parent
        .into_values()
        .min_by_key(|c| c.index)
        .map(|c| c.path)
}

#[derive(Clone, Debug)]
struct Candidate {
    index: usize,
    path: PathBuf,
    parent: PathBuf,
}

fn enumerate_touchscreens(sysfs_input: &Path, devfs_input: &Path) -> Vec<Candidate> {
    let Ok(entries) = fs::read_dir(sysfs_input) else {
        tracing::warn!("input discovery: cannot read {}", sysfs_input.display());
        return Vec::new();
    };

    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            let index = name_str.strip_prefix("event")?.parse::<usize>().ok()?;
            let event_dir = entry.path();
            if !is_touchscreen(&event_dir) {
                return None;
            }
            let parent = resolve_parent(&event_dir)?;
            Some(Candidate {
                index,
                path: devfs_input.join(name_str.as_ref()),
                parent,
            })
        })
        .collect()
}

fn is_touchscreen(event_dir: &Path) -> bool {
    let Some(abs) = read_bitmap(&event_dir.join("device/capabilities/abs")) else {
        return false;
    };
    let Some(key) = read_bitmap(&event_dir.join("device/capabilities/key")) else {
        return false;
    };
    // Accept both single-touch (legacy ABS_X/ABS_Y) and protocol-B
    // multi-touch-only devices (ABS_MT_POSITION_X/Y, no legacy axes —
    // QEMU's virtio-multitouch-pci and some real touch controllers fall
    // in this camp). BTN_TOUCH is required in either case.
    let has_legacy_axes = bit_set(&abs, ABS_X) && bit_set(&abs, ABS_Y);
    let has_mt_axes = bit_set(&abs, ABS_MT_POSITION_X) && bit_set(&abs, ABS_MT_POSITION_Y);
    (has_legacy_axes || has_mt_axes) && bit_set(&key, BTN_TOUCH)
}

/// Resolve the parent input device symlink to a canonical path so two
/// event handlers attached to the same physical device collapse into a
/// single map key.
fn resolve_parent(event_dir: &Path) -> Option<PathBuf> {
    fs::canonicalize(event_dir.join("device")).ok()
}

fn read_bitmap(path: &Path) -> Option<Vec<u64>> {
    fs::read_to_string(path).ok().map(|s| parse_bitmap(&s))
}

/// Parse a sysfs capability bitmap into an LSB-first word vector.
///
/// The kernel prints each `unsigned long` word as hex from the highest
/// word down, so we reverse to index bit `N` as
/// `words[N / BITS_PER_LONG] >> (N % BITS_PER_LONG)`.
fn parse_bitmap(s: &str) -> Vec<u64> {
    let mut words: Vec<u64> = s
        .split_whitespace()
        .map(|w| u64::from_str_radix(w, 16).unwrap_or(0))
        .collect();
    words.reverse();
    words
}

#[expect(
    clippy::integer_division,
    reason = "bit / WORD_BITS is exact by construction (bit is a usize bit index)"
)]
fn bit_set(bits: &[u64], bit: usize) -> bool {
    let word = bit / WORD_BITS;
    let offset = bit % WORD_BITS;
    bits.get(word).is_some_and(|w| (w >> offset) & 1 == 1)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix;
    use std::path::Path;

    use super::{bit_set, discover_touch_node_in, parse_bitmap};

    #[test]
    fn parse_bitmap_reverses_word_order() {
        assert_eq!(parse_bitmap("2 1"), vec![1, 2]);
    }

    #[test]
    fn bit_set_indexes_low_word_first() {
        let words = parse_bitmap("3");
        assert!(bit_set(&words, 0));
        assert!(bit_set(&words, 1));
        assert!(!bit_set(&words, 2));
    }

    #[test]
    #[expect(
        clippy::integer_division,
        reason = "exact bit-index arithmetic in a test fixture"
    )]
    fn bit_set_spans_word_boundary() {
        // BTN_TOUCH = 0x14a = 330; encode only that bit.
        let word = 330 / super::WORD_BITS;
        let offset = 330 % super::WORD_BITS;
        let mut values = vec![0_u64; word + 1];
        values[word] = 1_u64 << offset;
        let printed = values
            .iter()
            .rev()
            .map(|w| format!("{w:x}"))
            .collect::<Vec<_>>()
            .join(" ");
        let parsed = parse_bitmap(&printed);
        assert!(bit_set(&parsed, 330));
        assert!(!bit_set(&parsed, 329));
        assert!(!bit_set(&parsed, 331));
    }

    /// Build a fake `/sys/class/input/event<idx>` entry rooted in `sysfs`
    /// whose `device` symlink resolves under `inputs_root/<input_id>`,
    /// emulating how the kernel exposes evdev handlers under their
    /// parent input device.
    fn install_event_node(
        sysfs: &Path,
        inputs_root: &Path,
        index: usize,
        input_id: &str,
        abs: &str,
        key: &str,
    ) {
        let input_dir = inputs_root.join(input_id);
        fs::create_dir_all(input_dir.join("capabilities")).expect("BUG: create fake input device");
        fs::write(input_dir.join("capabilities/abs"), abs).expect("BUG: write fake abs");
        fs::write(input_dir.join("capabilities/key"), key).expect("BUG: write fake key");

        let event_dir = sysfs.join(format!("event{index}"));
        fs::create_dir_all(&event_dir).expect("BUG: create fake event node");
        unix::fs::symlink(&input_dir, event_dir.join("device"))
            .expect("BUG: link event node to input device");
    }

    fn touchscreen_key_bitmap() -> String {
        #[expect(
            clippy::integer_division,
            reason = "exact bit-index arithmetic in a test fixture"
        )]
        let word = 0x14a / super::WORD_BITS;
        let offset = 0x14a % super::WORD_BITS;
        let mut words = vec![0_u64; word + 1];
        words[word] = 1_u64 << offset;
        words
            .iter()
            .rev()
            .map(|w| format!("{w:x}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn picks_nothing_when_only_non_touch_devices_present() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let sysfs = tmp.path().join("sys");
        let inputs = tmp.path().join("inputs");
        fs::create_dir_all(&sysfs).expect("BUG: create sysfs root");

        // Power button: KEY-only, no ABS.
        install_event_node(&sysfs, &inputs, 0, "input0", "0", "100000");

        assert_eq!(
            discover_touch_node_in(&sysfs, Path::new("/dev/input")),
            None
        );
    }

    #[test]
    fn dedupes_duplicate_handlers_on_same_physical_device() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let sysfs = tmp.path().join("sys");
        let inputs = tmp.path().join("inputs");
        fs::create_dir_all(&sysfs).expect("BUG: create sysfs root");

        install_event_node(&sysfs, &inputs, 0, "input0", "0", "100000"); // power
        // Two evdev handlers share the same parent input device —
        // matches the VM's event1 + event3 / virtio tablet oddity.
        let touch_key = touchscreen_key_bitmap();
        install_event_node(&sysfs, &inputs, 1, "input1", "3", &touch_key);
        install_event_node(&sysfs, &inputs, 3, "input1", "3", &touch_key);

        let devfs = Path::new("/dev/input");
        assert_eq!(
            discover_touch_node_in(&sysfs, devfs),
            Some(devfs.join("event1"))
        );
    }

    #[test]
    fn picks_lowest_index_among_distinct_touchscreens() {
        let tmp = tempfile::tempdir().expect("BUG: tempdir");
        let sysfs = tmp.path().join("sys");
        let inputs = tmp.path().join("inputs");
        fs::create_dir_all(&sysfs).expect("BUG: create sysfs root");

        let touch_key = touchscreen_key_bitmap();
        install_event_node(&sysfs, &inputs, 2, "input2", "3", &touch_key);
        install_event_node(&sysfs, &inputs, 5, "input5", "3", &touch_key);

        let devfs = Path::new("/dev/input");
        assert_eq!(
            discover_touch_node_in(&sysfs, devfs),
            Some(devfs.join("event2"))
        );
    }
}
