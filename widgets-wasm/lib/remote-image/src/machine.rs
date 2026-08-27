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

//! Pure state machine for a fetched picture — no SDK calls, unit-testable.
//! The widget turns host callbacks into [`Event`]s, runs [`step`],
//! and executes the returned [`Action`]s.
//! Folding the in-flight decode into the states that own it
//! makes a stale completion impossible to install.

use bmc_wasm_sdk::{BitmapId, ImageJobId};

/// An in-flight host decode; its completion is matched by `job`.
#[derive(Clone, Copy)]
pub struct Decode {
    pub job: ImageJobId,
    pub aspect: f32,
}

/// Overlay badge on a shown picture.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Badge {
    Fresh,
    Updating,
    /// Can't reach the source; the shown picture is old but still valid data.
    Stale,
    /// The source answered with an unusable payload (broken / oversized image).
    Error(ErrorKind),
}

/// Why no picture is shown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ErrorKind {
    LoadFailed,
    TooLarge,
    BadImage,
}

/// What the widget is showing.
pub enum View {
    Loading {
        decode: Option<Decode>,
    },
    Failed(ErrorKind),
    Shown {
        bitmap: BitmapId,
        aspect: f32,
        badge: Badge,
        decode: Option<Decode>,
    },
}

/// Something the host reported.
#[derive(Clone, Copy)]
pub enum Event {
    Restored {
        bitmap: BitmapId,
        aspect: f32,
        remaining_ms: u32,
        saved_at_secs: i64,
    },
    RestoredStale {
        bitmap: BitmapId,
        aspect: f32,
        saved_at_secs: i64,
    },
    Woke {
        remaining_ms: u32,
        saved_at_secs: i64,
    },
    WokeStale {
        saved_at_secs: i64,
    },
    RestoreMiss,
    DecodeStarted {
        job: ImageJobId,
        aspect: f32,
    },
    Decoded {
        job: ImageJobId,
        bitmap: BitmapId,
    },
    DecodeFailed {
        job: ImageJobId,
    },
    FetchError {
        kind: ErrorKind,
        transient: bool,
    },
    /// The widget should now be showing a different picture:
    /// an edited URL or sizing, or a fresh publication date.
    TargetChanged,
    Reload,
    Sleep,
}

/// A host side effect for the widget to run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    EnablePollAfter(u32),
    ResumePoll,
    DisablePoll,
    Retry,
    /// Reject a broken 2xx body: mark failing, keep the anchor, retry sooner.
    DeferPoll,
    /// Flag stale after a reply was banked ok (a late decode failure).
    MarkStale,
    /// Seed the staleness anchor from a restored cache timestamp.
    SeedAnchor(i64),
    RequestFrame,
}

/// Fold an event into the view, returning the next view and its side effects.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "exhaustive event × state transition table"
)]
pub fn step(view: View, event: Event) -> (View, Vec<Action>) {
    use Action as A;
    use Event as E;
    match event {
        E::Restored {
            bitmap,
            aspect,
            remaining_ms,
            saved_at_secs,
        } => (
            View::Shown {
                bitmap,
                aspect,
                badge: Badge::Fresh,
                decode: None,
            },
            vec![
                A::SeedAnchor(saved_at_secs),
                A::EnablePollAfter(remaining_ms),
                A::RequestFrame,
            ],
        ),
        E::RestoredStale {
            bitmap,
            aspect,
            saved_at_secs,
        } => (
            View::Shown {
                bitmap,
                aspect,
                badge: Badge::Updating,
                decode: None,
            },
            vec![A::SeedAnchor(saved_at_secs), A::ResumePoll, A::RequestFrame],
        ),
        E::Woke {
            remaining_ms,
            saved_at_secs,
        } => match view {
            View::Shown { bitmap, aspect, .. } => (
                View::Shown {
                    bitmap,
                    aspect,
                    badge: Badge::Fresh,
                    decode: None,
                },
                vec![
                    A::SeedAnchor(saved_at_secs),
                    A::EnablePollAfter(remaining_ms),
                    A::RequestFrame,
                ],
            ),
            other => (other, vec![]),
        },
        E::WokeStale { saved_at_secs } => match view {
            View::Shown { bitmap, aspect, .. } => (
                View::Shown {
                    bitmap,
                    aspect,
                    badge: Badge::Updating,
                    decode: None,
                },
                vec![A::SeedAnchor(saved_at_secs), A::ResumePoll, A::RequestFrame],
            ),
            other => (other, vec![]),
        },
        // A new target drops the shown picture (Loading, not stale-over-wrong);
        // evict stays out of here — it needs render scope (host import traps otherwise).
        E::RestoreMiss | E::TargetChanged => (
            View::Loading { decode: None },
            vec![A::ResumePoll, A::RequestFrame],
        ),
        // A refresh of a shown picture keeps it and its badge; only the decode is tracked.
        E::DecodeStarted { job, aspect } => {
            let decode = Some(Decode { job, aspect });
            match view {
                View::Shown {
                    bitmap,
                    aspect: shown,
                    badge,
                    ..
                } => (
                    View::Shown {
                        bitmap,
                        aspect: shown,
                        badge,
                        decode,
                    },
                    vec![A::RequestFrame],
                ),
                _ => (View::Loading { decode }, vec![A::RequestFrame]),
            }
        }
        E::Decoded { job, bitmap } => match view {
            View::Loading { decode: Some(d) }
            | View::Shown {
                decode: Some(d), ..
            } if d.job == job => (
                View::Shown {
                    bitmap,
                    aspect: d.aspect,
                    badge: Badge::Fresh,
                    decode: None,
                },
                vec![A::RequestFrame],
            ),
            // Superseded completion (e.g. after a target change) — ignore.
            other => (other, vec![]),
        },
        E::DecodeFailed { job } => match view {
            View::Loading { decode: Some(d) } if d.job == job => {
                (View::Failed(ErrorKind::BadImage), vec![A::RequestFrame])
            }
            // Fetch banked ok; flag the picture + mark stale so is_stale matches.
            View::Shown {
                bitmap,
                aspect,
                decode: Some(d),
                ..
            } if d.job == job => (
                View::Shown {
                    bitmap,
                    aspect,
                    badge: Badge::Error(ErrorKind::BadImage),
                    decode: None,
                },
                vec![A::RequestFrame, A::MarkStale],
            ),
            other => (other, vec![]),
        },
        E::FetchError { kind, transient } => match view {
            // Keep the last picture: unreachable → Stale (fast retry), bad body → Error.
            View::Shown {
                bitmap,
                aspect,
                decode,
                ..
            } => {
                let (badge, actions) = if transient {
                    (Badge::Stale, vec![A::RequestFrame, A::Retry])
                } else {
                    (Badge::Error(kind), vec![A::RequestFrame, A::DeferPoll])
                };
                (
                    View::Shown {
                        bitmap,
                        aspect,
                        badge,
                        decode,
                    },
                    actions,
                )
            }
            // No picture to keep. A non-transient failure (bad body, oversized)
            // still must slow the poll: without DeferPoll the widget falls back
            // to the fast retry_ms cadence and hammers an unfixable URL forever.
            _ if transient => (View::Failed(kind), vec![A::RequestFrame]),
            _ => (View::Failed(kind), vec![A::RequestFrame, A::DeferPoll]),
        },
        E::Reload => match view {
            View::Shown { bitmap, aspect, .. } => (
                View::Shown {
                    bitmap,
                    aspect,
                    badge: Badge::Updating,
                    decode: None,
                },
                vec![A::ResumePoll, A::RequestFrame],
            ),
            _ => (
                View::Loading { decode: None },
                vec![A::ResumePoll, A::RequestFrame],
            ),
        },
        E::Sleep => match view {
            View::Shown {
                bitmap,
                aspect,
                badge,
                ..
            } => (
                View::Shown {
                    bitmap,
                    aspect,
                    badge,
                    decode: None,
                },
                vec![A::DisablePoll],
            ),
            _ => (View::Loading { decode: None }, vec![A::DisablePoll]),
        },
    }
}

/// Substitute `{{width}}`/`{{height}}`; `None` unless the URL is `http(s)`.
#[must_use]
pub fn expand_url(url: &str, width: u32, height: u32) -> Option<String> {
    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return None;
    }
    Some(
        url.replace("{{width}}", &width.to_string())
            .replace("{{height}}", &height.to_string()),
    )
}

/// Milliseconds left before `saved_at + interval`; 0 once past.
#[must_use]
pub fn ttl_remaining(now_ms: u64, saved_at_ms: u64, interval_ms: u32) -> u32 {
    let elapsed = now_ms.saturating_sub(saved_at_ms);
    let remaining = u64::from(interval_ms).saturating_sub(elapsed);
    u32::try_from(remaining).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(n: u32) -> ImageJobId {
        ImageJobId::from_wire(n).expect("nonzero job id")
    }
    fn bmp(n: u16) -> BitmapId {
        BitmapId::from_wire(n).expect("nonzero bitmap id")
    }
    fn shown(badge: Badge, decode: Option<Decode>) -> View {
        View::Shown {
            bitmap: bmp(1),
            aspect: 1.0,
            badge,
            decode,
        }
    }

    #[test]
    fn decoded_matching_job_shows_image() {
        let v = View::Loading {
            decode: Some(Decode {
                job: job(1),
                aspect: 1.5,
            }),
        };
        let (next, actions) = step(
            v,
            Event::Decoded {
                job: job(1),
                bitmap: bmp(7),
            },
        );
        assert!(
            matches!(next, View::Shown { badge: Badge::Fresh, decode: None, aspect, .. } if aspect == 1.5)
        );
        assert!(actions.contains(&Action::RequestFrame));
    }

    #[test]
    fn decoded_stale_job_is_ignored() {
        let v = View::Loading {
            decode: Some(Decode {
                job: job(2),
                aspect: 1.0,
            }),
        };
        let (next, actions) = step(
            v,
            Event::Decoded {
                job: job(1),
                bitmap: bmp(7),
            },
        );
        assert!(matches!(next, View::Loading { decode: Some(_) }));
        assert!(actions.is_empty());
    }

    #[test]
    fn target_change_drops_in_flight_decode() {
        let v = shown(
            Badge::Fresh,
            Some(Decode {
                job: job(5),
                aspect: 1.0,
            }),
        );
        let (next, _) = step(v, Event::TargetChanged);
        assert!(matches!(next, View::Loading { decode: None }));
        // A late completion of the dropped job can no longer install.
        let (after, actions) = step(
            next,
            Event::Decoded {
                job: job(5),
                bitmap: bmp(9),
            },
        );
        assert!(matches!(after, View::Loading { decode: None }));
        assert!(actions.is_empty());
    }

    #[test]
    fn fresh_restore_schedules_at_ttl() {
        let (next, actions) = step(
            View::Loading { decode: None },
            Event::Restored {
                bitmap: bmp(1),
                aspect: 2.0,
                remaining_ms: 4_000,
                saved_at_secs: 900,
            },
        );
        assert!(matches!(
            next,
            View::Shown {
                badge: Badge::Fresh,
                ..
            }
        ));
        assert!(actions.contains(&Action::EnablePollAfter(4_000)));
        assert!(actions.contains(&Action::SeedAnchor(900)));
    }

    #[test]
    fn stale_restore_refreshes_now() {
        let (next, actions) = step(
            View::Loading { decode: None },
            Event::RestoredStale {
                bitmap: bmp(1),
                aspect: 1.0,
                saved_at_secs: 900,
            },
        );
        assert!(matches!(
            next,
            View::Shown {
                badge: Badge::Updating,
                ..
            }
        ));
        assert!(actions.contains(&Action::ResumePoll));
        assert!(actions.contains(&Action::SeedAnchor(900)));
    }

    #[test]
    fn miss_loads_now() {
        let (next, actions) = step(shown(Badge::Fresh, None), Event::RestoreMiss);
        assert!(matches!(next, View::Loading { decode: None }));
        assert!(actions.contains(&Action::ResumePoll));
    }

    #[test]
    fn normal_refresh_shows_no_overlay() {
        let (mid, _) = step(
            shown(Badge::Fresh, None),
            Event::DecodeStarted {
                job: job(1),
                aspect: 1.0,
            },
        );
        assert!(matches!(
            mid,
            View::Shown {
                badge: Badge::Fresh,
                decode: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn failed_refresh_flags_last_good_as_error() {
        let v = shown(
            Badge::Fresh,
            Some(Decode {
                job: job(1),
                aspect: 1.0,
            }),
        );
        let (next, actions) = step(v, Event::DecodeFailed { job: job(1) });
        assert!(matches!(
            next,
            View::Shown {
                badge: Badge::Error(ErrorKind::BadImage),
                decode: None,
                ..
            }
        ));
        assert!(
            actions.contains(&Action::MarkStale),
            "a failed decode flags the poll stale so is_stale reflects it"
        );
    }

    #[test]
    fn broken_body_flags_error_and_defers_not_fast_retries() {
        let (next, actions) = step(
            shown(Badge::Fresh, None),
            Event::FetchError {
                kind: ErrorKind::TooLarge,
                transient: false,
            },
        );
        assert!(matches!(
            next,
            View::Shown {
                badge: Badge::Error(ErrorKind::TooLarge),
                ..
            }
        ));
        assert!(actions.contains(&Action::DeferPoll));
        assert!(
            !actions.contains(&Action::Retry),
            "a broken body waits the interval, not a fast retry"
        );
    }

    #[test]
    fn target_change_reloads_without_evicting() {
        let (next, actions) = step(shown(Badge::Fresh, None), Event::TargetChanged);
        assert!(matches!(next, View::Loading { decode: None }));
        assert_eq!(actions, vec![Action::ResumePoll, Action::RequestFrame]);
    }

    #[test]
    fn non_transient_fetch_error_without_image_defers_not_fast_retries() {
        let (next, actions) = step(
            View::Loading { decode: None },
            Event::FetchError {
                kind: ErrorKind::TooLarge,
                transient: false,
            },
        );
        assert!(matches!(next, View::Failed(ErrorKind::TooLarge)));
        assert!(actions.contains(&Action::DeferPoll));
        assert!(
            !actions.contains(&Action::Retry),
            "an unfixable first-load failure waits the interval, not the 10s retry"
        );
    }

    #[test]
    fn transient_fetch_error_without_image_keeps_fast_retry() {
        let (next, actions) = step(
            View::Loading { decode: None },
            Event::FetchError {
                kind: ErrorKind::LoadFailed,
                transient: true,
            },
        );
        assert!(matches!(next, View::Failed(ErrorKind::LoadFailed)));
        assert!(
            !actions.contains(&Action::DeferPoll),
            "a network blip on first load keeps the fast retry_ms cadence"
        );
    }

    #[test]
    fn transient_fetch_error_with_image_retries() {
        let (next, actions) = step(
            shown(Badge::Fresh, None),
            Event::FetchError {
                kind: ErrorKind::LoadFailed,
                transient: true,
            },
        );
        assert!(matches!(
            next,
            View::Shown {
                badge: Badge::Stale,
                ..
            }
        ));
        assert!(actions.contains(&Action::Retry));
    }

    #[test]
    fn reload_with_image_updates_and_refetches() {
        let (next, actions) = step(shown(Badge::Stale, None), Event::Reload);
        assert!(matches!(
            next,
            View::Shown {
                badge: Badge::Updating,
                decode: None,
                ..
            }
        ));
        assert!(actions.contains(&Action::ResumePoll));
    }

    #[test]
    fn dormant_retains_bitmap_id_and_disables_poll() {
        let (next, actions) = step(shown(Badge::Fresh, None), Event::Sleep);
        assert!(matches!(
            next,
            View::Shown {
                bitmap,
                decode: None,
                ..
            } if bitmap == bmp(1)
        ));
        assert_eq!(actions, vec![Action::DisablePoll]);
    }

    #[test]
    fn dormant_then_wake_reuses_bitmap_id() {
        let (dormant, _) = step(shown(Badge::Fresh, None), Event::Sleep);
        assert!(matches!(dormant, View::Shown { .. }));
        let (woken, _) = step(
            dormant,
            Event::Woke {
                remaining_ms: 4_000,
                saved_at_secs: 900,
            },
        );
        assert!(matches!(
            woken,
            View::Shown {
                bitmap,
                aspect: 1.0,
                ..
            } if bitmap == bmp(1)
        ));
    }

    #[test]
    fn expand_url_substitutes_dims() {
        assert_eq!(
            expand_url("http://x/{{width}}x{{height}}", 64, 48).as_deref(),
            Some("http://x/64x48")
        );
        assert_eq!(expand_url("   ", 1, 1), None);
        assert_eq!(expand_url("ftp://x/img", 1, 1), None);
        assert_eq!(expand_url("not-a-url", 1, 1), None);
        assert_eq!(expand_url("http://x", 64, 48).as_deref(), Some("http://x"));
        assert_eq!(
            expand_url("https://x", 64, 48).as_deref(),
            Some("https://x")
        );
    }

    #[test]
    fn ttl_remaining_clamps_at_zero() {
        assert_eq!(ttl_remaining(1_000, 0, 5_000), 4_000);
        assert_eq!(ttl_remaining(5_000, 0, 5_000), 0);
        assert_eq!(ttl_remaining(9_000, 0, 5_000), 0);
    }
}
