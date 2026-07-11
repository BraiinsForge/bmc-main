// Copyright (C) 2026  Braiins Systems s.r.o.

//! Per-instance state: named ring buffers of timestamped samples. A device's
//! opt-in sampler pushes into it; an accumulating endpoint reads it back.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

/// A value at a scenario time (seconds since start; negative from backfill).
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub t_s: f64,
    pub value: f64,
}

/// A time plus one value per series, in the order requested from [`Cache::rows`].
#[derive(Debug, Clone)]
pub struct Row {
    pub t_s: f64,
    pub values: Vec<f64>,
}

/// A capacity-bounded series: appending past the cap drops the oldest sample.
#[derive(Debug)]
struct Ring {
    capacity: usize,
    samples: VecDeque<Sample>,
}

impl Ring {
    fn push(&mut self, sample: Sample) {
        if self.capacity == 0 {
            return;
        }
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }
}

/// The device's cache: named ring buffers shared by the sampler and handlers.
#[derive(Debug)]
pub struct Cache {
    series: Mutex<HashMap<String, Ring>>,
}

impl Cache {
    /// Create a cache with a ring buffer per `(name, capacity)` series.
    #[must_use]
    pub fn new<I>(series: I) -> Self
    where
        I: IntoIterator<Item = (String, usize)>,
    {
        let map = series
            .into_iter()
            .map(|(name, capacity)| {
                (
                    name,
                    Ring {
                        capacity,
                        samples: VecDeque::with_capacity(capacity),
                    },
                )
            })
            .collect();
        Self {
            series: Mutex::new(map),
        }
    }

    /// Append a sample to each named series under one lock, so a concurrent
    /// `rows` read never sees a half-written tick; unregistered series drop.
    pub fn push_row(&self, samples: &[(&str, Sample)]) {
        let mut guard = self.series.lock().expect("BUG: cache mutex poisoned");
        for (series, sample) in samples {
            if let Some(ring) = guard.get_mut(*series) {
                ring.push(*sample);
            }
        }
    }

    /// Read the named series aligned into rows (oldest first) under one lock;
    /// length from the shortest. Empty if any series is unknown, so a name typo
    /// fails visibly instead of serving a wrong column.
    ///
    /// Rows align on the newest sample, because a ring evicts from the front:
    /// a shorter one has already dropped older ticks, so pairing by raw index
    /// reads its samples against a longer ring's earlier timestamps.
    #[must_use]
    pub fn rows(&self, series: &[&str]) -> Vec<Row> {
        let guard = self.series.lock().expect("BUG: cache mutex poisoned");
        let Some(rings) = series
            .iter()
            .map(|name| guard.get(*name))
            .collect::<Option<Vec<&Ring>>>()
        else {
            return Vec::new();
        };
        let len = rings
            .iter()
            .map(|ring| ring.samples.len())
            .min()
            .unwrap_or(0);
        let tail_start = |ring: &Ring| ring.samples.len() - len;
        (0..len)
            .map(|index| Row {
                t_s: rings[0].samples[tail_start(rings[0]) + index].t_s,
                values: rings
                    .iter()
                    .map(|ring| ring.samples[tail_start(ring) + index].value)
                    .collect(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{Cache, Sample};

    fn cache() -> Cache {
        Cache::new([("hashrate".to_owned(), 3)])
    }

    fn values(cache: &Cache, series: &str) -> Vec<f64> {
        cache
            .rows(&[series])
            .iter()
            .map(|row| row.values[0])
            .collect()
    }

    #[test]
    fn rows_of_unknown_series_is_empty() {
        assert!(cache().rows(&["missing"]).is_empty());
    }

    #[test]
    fn push_row_appends_in_order() {
        let cache = cache();
        for value in [1.0, 2.0] {
            cache.push_row(&[("hashrate", Sample { t_s: value, value })]);
        }
        assert_eq!(values(&cache, "hashrate"), vec![1.0, 2.0]);
    }

    #[test]
    fn push_past_capacity_drops_oldest() {
        let cache = cache();
        for value in [1.0, 2.0, 3.0, 4.0] {
            cache.push_row(&[("hashrate", Sample { t_s: value, value })]);
        }
        assert_eq!(
            values(&cache, "hashrate"),
            vec![2.0, 3.0, 4.0],
            "capacity 3 keeps the newest three"
        );
    }

    #[test]
    fn push_row_drops_unregistered_and_keeps_registered() {
        let cache = cache();
        cache.push_row(&[
            (
                "hashrate",
                Sample {
                    t_s: 1.0,
                    value: 5.0,
                },
            ),
            (
                "power",
                Sample {
                    t_s: 1.0,
                    value: 9.0,
                },
            ),
        ]);
        assert_eq!(values(&cache, "hashrate"), vec![5.0], "registered recorded");
    }

    #[test]
    fn rows_align_series_by_index() {
        let cache = Cache::new([("a".to_owned(), 3), ("b".to_owned(), 3)]);
        cache.push_row(&[
            (
                "a",
                Sample {
                    t_s: 1.0,
                    value: 10.0,
                },
            ),
            (
                "b",
                Sample {
                    t_s: 1.0,
                    value: 20.0,
                },
            ),
        ]);
        cache.push_row(&[
            (
                "a",
                Sample {
                    t_s: 2.0,
                    value: 11.0,
                },
            ),
            (
                "b",
                Sample {
                    t_s: 2.0,
                    value: 21.0,
                },
            ),
        ]);
        let rows = cache.rows(&["a", "b"]);
        let times: Vec<f64> = rows.iter().map(|row| row.t_s).collect();
        assert_eq!(times, vec![1.0, 2.0]);
        assert_eq!(rows[0].values, vec![10.0, 20.0]);
        assert_eq!(rows[1].values, vec![11.0, 21.0]);
    }

    #[test]
    fn rows_pair_unequal_rings_by_time_not_by_index() {
        // The short ring evicted the first two ticks: its index 0 holds t=3,
        // the long ring's holds t=1. Pairing by index would mismatch them.
        let cache = Cache::new([("long".to_owned(), 4), ("short".to_owned(), 2)]);
        for tick in 1..=4 {
            let t_s = f64::from(tick);
            cache.push_row(&[
                ("long", Sample { t_s, value: t_s }),
                (
                    "short",
                    Sample {
                        t_s,
                        value: t_s * 10.0,
                    },
                ),
            ]);
        }

        let rows = cache.rows(&["long", "short"]);
        let times: Vec<f64> = rows.iter().map(|row| row.t_s).collect();
        assert_eq!(
            times,
            vec![3.0, 4.0],
            "only the ticks both rings still hold"
        );
        assert_eq!(rows[0].values, vec![3.0, 30.0]);
        assert_eq!(rows[1].values, vec![4.0, 40.0]);
    }
}
