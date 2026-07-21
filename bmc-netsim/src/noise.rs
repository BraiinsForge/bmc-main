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

//! Deterministic noise keyed on a per-instance seed: same `(seed, t)` → same
//! value, so re-reads are stable and recorded series reproducible. The seed is
//! a hash of the device's identity, so siblings decorrelate.

/// splitmix64 finalizer — a fast integer avalanche hash.
#[must_use]
pub fn hash64(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// Fold a string key into a seed, so a field decorrelates from its siblings.
#[must_use]
pub fn mix(seed: u64, key: &str) -> u64 {
    let mut h = seed;
    for byte in key.bytes() {
        h = hash64(h ^ u64::from(byte));
    }
    h
}

/// Fold an array index into a seed, so sibling array elements decorrelate.
#[must_use]
pub fn mix_index(seed: u64, index: usize) -> u64 {
    hash64(seed ^ hash64(index as u64))
}

/// A deterministic value in `[0, 1)` at integer lattice point `n`.
fn lattice(seed: u64, n: i64) -> f64 {
    #[expect(clippy::cast_sign_loss, reason = "bit pattern reused as hash input")]
    let key = n as u64;
    let bits = hash64(seed ^ hash64(key)) >> 11;
    #[expect(clippy::cast_precision_loss, reason = "bits < 2^53 is exact in f64")]
    let numerator = bits as f64;
    // 2^53 — the largest power of two exactly representable in f64.
    numerator / 9_007_199_254_740_992.0
}

/// Smooth value noise in `[0, 1)`: smoothstep between lattice points, so nearby
/// `t` correlate. `t` is in lattice units — scale at the call site for wavelength.
#[must_use]
pub fn noise01(seed: u64, t: f64) -> f64 {
    let base = t.floor();
    let frac = t - base;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "elapsed seconds fit i64; a wrap only reshuffles the noise"
    )]
    let n = base as i64;
    let low = lattice(seed, n);
    let high = lattice(seed, n.wrapping_add(1));
    let smooth = frac * frac * (3.0 - 2.0 * frac);
    low + (high - low) * smooth
}

/// A stable value in `[0, 1)` from `seed` alone — the time-free [`noise01`].
#[must_use]
pub fn stable01(seed: u64) -> f64 {
    lattice(seed, 0)
}

#[cfg(test)]
mod tests {
    use super::{mix, noise01, stable01};

    #[test]
    fn noise_is_stable_for_same_seed_and_time() {
        let seed = mix(0, "device-01");
        assert!((noise01(seed, 12.5) - noise01(seed, 12.5)).abs() < f64::EPSILON);
    }

    #[test]
    fn noise_stays_in_unit_interval() {
        let seed = mix(0, "device-01");
        for step in 0..10_000 {
            let value = noise01(seed, f64::from(step) * 0.017);
            assert!((0.0..1.0).contains(&value), "{value} out of [0, 1)");
        }
    }

    #[test]
    fn distinct_keys_decorrelate() {
        let hashrate = mix(42, "hashrate");
        let power = mix(42, "power");
        let gap = (noise01(hashrate, 3.0) - noise01(power, 3.0)).abs();
        assert!(gap > f64::EPSILON, "distinct keys should decorrelate");
    }

    #[test]
    fn stable01_is_constant_per_seed_and_decorrelates() {
        let one = stable01(mix(0, "bos-01"));
        assert!((0.0..1.0).contains(&one), "{one} out of [0, 1)");
        assert!(
            (stable01(mix(0, "bos-01")) - one).abs() < f64::EPSILON,
            "same seed is stable",
        );
        assert!(
            (stable01(mix(0, "bos-02")) - one).abs() > f64::EPSILON,
            "distinct seeds decorrelate",
        );
    }
}
