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
///
/// The bitmap id names a slot, not a snapshot, so these pixels reach the shown
/// picture whether the view wants them or not, and its aspect has to arrive
/// with them. Only the host ends a decode — [`Event::Decoded`],
/// [`Event::DecodeFailed`] or [`Event::DecodeAbandoned`] — so every other
/// transition carries it through, [`Fate`] recording what became of it.
#[derive(Clone, Copy)]
pub struct Decode {
    pub job: ImageJobId,
    pub aspect: f32,
    pub fate: Fate,
}

/// What the widget will do with an in-flight decode's pixels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fate {
    /// Show them: they are the picture that was asked for.
    Show,
    /// Show them under the pill: still the picture that was asked for, only no
    /// longer the newest. The newer one has been asked for — in flight, or
    /// waiting on a retry if the slot this decode holds refused its body.
    ShowSuperseded,
    /// Drop them: the target changed while they were decoding, so they answer
    /// a question nobody is asking. The host's one decode slot is theirs until
    /// they land, and the new target's fetch is waiting on it.
    Discard,
}

impl Decode {
    fn supersede(self) -> Self {
        match self.fate {
            // Fetching a different picture cannot make the widget want
            // one it has already stopped asking for.
            Fate::Discard => self,
            Fate::Show | Fate::ShowSuperseded => Self {
                fate: Fate::ShowSuperseded,
                ..self
            },
        }
    }

    fn discard(self) -> Self {
        Self {
            fate: Fate::Discard,
            ..self
        }
    }
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

impl View {
    /// The decode the host has still to report, if any.
    #[must_use]
    pub const fn decode(&self) -> Option<Decode> {
        match *self {
            Self::Loading { decode } | Self::Shown { decode, .. } => decode,
            // Only entered with nothing outstanding.
            Self::Failed(_) => None,
        }
    }
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
    /// A decode the host finished while the widget was dormant, and reclaimed
    /// without reporting. Its result is on flash and nowhere else, so there is
    /// nothing left to wait for and nothing to install here.
    DecodeAbandoned {
        job: ImageJobId,
    },
    FetchError {
        kind: ErrorKind,
        transient: bool,
    },
    /// The widget was asked for a different picture — an edited URL or sizing.
    /// What is on screen is no longer what was asked for, so it goes.
    TargetChanged,
    /// The source published something newer.
    /// What is on screen is still the picture that was asked for, only older,
    /// so it stays up while the new one is fetched.
    TargetSuperseded,
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
    // Survives every transition below that does not resolve it.
    let owed = view.decode();
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
                // A decode still outstanding is a newer picture on its way.
                badge: if owed.is_some() {
                    Badge::Updating
                } else {
                    Badge::Fresh
                },
                decode: owed,
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
                decode: owed,
            },
            vec![A::SeedAnchor(saved_at_secs), A::ResumePoll, A::RequestFrame],
        ),
        E::RestoreMiss => (
            View::Loading { decode: owed },
            vec![A::ResumePoll, A::RequestFrame],
        ),
        // A new target drops the shown picture (Loading, not stale-over-wrong);
        // `TargetSuperseded` is the case that keeps it.
        // A decode stays tracked, marked `Discard`: it holds the host's one
        // decode slot until it lands, so fetching now risks handing the new
        // target's body to a `set_fit_ref` that has to refuse it. Its
        // completion frees the slot and starts the fetch.
        // Evict stays out of here — it needs render scope (host import traps otherwise).
        E::TargetChanged => (
            View::Loading {
                decode: owed.map(Decode::discard),
            },
            if owed.is_some() {
                vec![A::RequestFrame]
            } else {
                vec![A::ResumePoll, A::RequestFrame]
            },
        ),
        // A refresh of a shown picture keeps it and its badge; only the decode is tracked.
        E::DecodeStarted { job, aspect } => {
            // Replacing an owed decode would forget a job whose pixels still
            // reach the slot, so the picture would be drawn from them at the
            // new job's aspect. `classify_body` refuses a body while one is
            // owed precisely so this cannot happen.
            debug_assert!(
                owed.is_none(),
                "BUG: a decode started while the host still owed one"
            );
            let decode = Some(Decode {
                job,
                aspect,
                fate: Fate::Show,
            });
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
            // These pixels went into the slot whatever the widget wanted, so a
            // picture restored over that slot would be drawn from them. Nothing
            // is shown until the fetch the freed slot now allows lands.
            View::Loading { decode: Some(d) }
            | View::Shown {
                decode: Some(d), ..
            } if d.job == job && d.fate == Fate::Discard => (
                View::Loading { decode: None },
                vec![A::ResumePoll, A::RequestFrame],
            ),
            View::Loading { decode: Some(d) }
            | View::Shown {
                decode: Some(d), ..
            } if d.job == job => (
                View::Shown {
                    bitmap,
                    aspect: d.aspect,
                    // The refresh that superseded this decode is still coming.
                    badge: if d.fate == Fate::ShowSuperseded {
                        Badge::Updating
                    } else {
                        Badge::Fresh
                    },
                    decode: None,
                },
                vec![A::RequestFrame],
            ),
            // A completion the view never tracked — ignore.
            other => (other, vec![]),
        },
        E::DecodeFailed { job } => match view {
            // Nothing reached the slot, so whatever is shown stays; the point
            // of a discarded decode is the slot its failure frees.
            View::Loading { decode: Some(d) } if d.job == job && d.fate == Fate::Discard => {
                (View::Loading { decode: None }, vec![A::ResumePoll])
            }
            View::Shown {
                bitmap,
                aspect,
                badge,
                decode: Some(d),
            } if d.job == job && d.fate == Fate::Discard => (
                View::Shown {
                    bitmap,
                    aspect,
                    badge,
                    decode: None,
                },
                vec![A::ResumePoll],
            ),
            // A failed decode registers nothing, so the picture is untouched
            // and the refresh that superseded it decides the badge.
            View::Shown {
                bitmap,
                aspect,
                badge,
                decode: Some(d),
            } if d.job == job && d.fate == Fate::ShowSuperseded => (
                View::Shown {
                    bitmap,
                    aspect,
                    badge,
                    decode: None,
                },
                vec![],
            ),
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
            // A decode is still owed, so a picture is on its way: keep waiting
            // rather than posting an error the completion is about to disprove.
            // The failure still paces the poll — and it has to name a delay
            // itself: this is the arm a body refused for the busy decode slot
            // lands in, the reply was HTTP-ok, and a poll with no interval
            // schedules nothing off an ok reply. Without it the refused fetch
            // is never re-asked and the completion leaves the pill up.
            _ if owed.is_some() => (
                View::Loading { decode: owed },
                if transient {
                    vec![A::RequestFrame, A::Retry]
                } else {
                    vec![A::RequestFrame, A::DeferPoll]
                },
            ),
            // No picture to keep. A non-transient failure (bad body, oversized)
            // still must slow the poll: without DeferPoll the widget falls back
            // to the fast retry_ms cadence and hammers an unfixable URL forever.
            _ if transient => (View::Failed(kind), vec![A::RequestFrame]),
            _ => (View::Failed(kind), vec![A::RequestFrame, A::DeferPoll]),
        },
        // Both fetch again over a picture that is still worth showing:
        // a newer publication supersedes it, a reload re-asks for it.
        E::Reload | E::TargetSuperseded => match view {
            View::Shown {
                bitmap,
                aspect,
                decode,
                ..
            } => (
                View::Shown {
                    bitmap,
                    aspect,
                    badge: Badge::Updating,
                    decode: decode.map(Decode::supersede),
                },
                vec![A::ResumePoll, A::RequestFrame],
            ),
            _ => (
                View::Loading {
                    decode: owed.map(Decode::supersede),
                },
                vec![A::ResumePoll, A::RequestFrame],
            ),
        },
        // Only the poll stops. A decode carries on in the host and lands on
        // flash whether the widget is watching, so it stays tracked and the
        // wake asks the SDK whether it is still coming.
        E::Sleep => match view {
            // Nothing to hold on to, and the wake restores before it draws.
            View::Failed(_) => (View::Loading { decode: None }, vec![A::DisablePoll]),
            kept => (kept, vec![A::DisablePoll]),
        },
        E::DecodeAbandoned { job } => match view {
            // The slot is free and the fetch it was blocking is still owed.
            View::Loading { decode: Some(d) }
            | View::Shown {
                decode: Some(d), ..
            } if d.job == job && d.fate == Fate::Discard => {
                (View::Loading { decode: None }, vec![A::ResumePoll])
            }
            View::Loading { decode: Some(d) } if d.job == job => {
                (View::Loading { decode: None }, vec![])
            }
            // The cached picture moved under the widget; the restore that
            // follows brings its pixels and aspect.
            View::Shown {
                bitmap,
                aspect,
                badge,
                decode: Some(d),
            } if d.job == job => (
                View::Shown {
                    bitmap,
                    aspect,
                    badge,
                    decode: None,
                },
                vec![],
            ),
            other => (other, vec![]),
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
    fn decode(n: u32, aspect: f32) -> Decode {
        Decode {
            job: job(n),
            aspect,
            fate: Fate::Show,
        }
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
            decode: Some(decode(1, 1.5)),
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
            decode: Some(decode(2, 1.0)),
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
    fn a_target_change_waits_for_the_slot_its_decode_holds() {
        let v = shown(Badge::Fresh, Some(decode(5, 1.0)));
        let (next, actions) = step(v, Event::TargetChanged);
        assert!(
            matches!(next, View::Loading { decode: Some(d) }
                if d.job == job(5) && d.fate == Fate::Discard),
            "the host owes this decode whatever the widget now wants"
        );
        assert_eq!(
            actions,
            vec![Action::RequestFrame],
            "fetching now would hand the new target's body to a full decode slot"
        );

        let (after, actions) = step(
            next,
            Event::Decoded {
                job: job(5),
                bitmap: bmp(9),
            },
        );
        assert!(
            matches!(after, View::Loading { decode: None }),
            "the pixels landed in the slot but answer a target nobody asked for"
        );
        assert!(
            actions.contains(&Action::ResumePoll),
            "the freed slot is what the new target's fetch was waiting for"
        );
    }

    #[test]
    fn a_discarded_decode_failing_frees_the_slot_too() {
        let (next, _) = step(
            shown(Badge::Fresh, Some(decode(5, 1.0))),
            Event::TargetChanged,
        );
        let (after, actions) = step(next, Event::DecodeFailed { job: job(5) });
        assert!(matches!(after, View::Loading { decode: None }));
        assert!(actions.contains(&Action::ResumePoll));
    }

    #[test]
    fn a_discarded_decode_reclaimed_while_dormant_frees_the_slot_too() {
        let (next, _) = step(
            shown(Badge::Fresh, Some(decode(5, 1.0))),
            Event::TargetChanged,
        );
        let (dormant, _) = step(next, Event::Sleep);
        let (after, actions) = step(dormant, Event::DecodeAbandoned { job: job(5) });
        assert!(matches!(after, View::Loading { decode: None }));
        assert!(
            actions.contains(&Action::ResumePoll),
            "the wake still owes a fetch for the target that was changed to"
        );
    }

    #[test]
    fn a_restored_picture_goes_when_the_decode_it_waits_on_is_discarded() {
        let (changed, _) = step(
            shown(Badge::Fresh, Some(decode(5, 1.0))),
            Event::TargetChanged,
        );
        let (restored, _) = step(
            changed,
            Event::Restored {
                bitmap: bmp(1),
                aspect: 1.0,
                remaining_ms: 4_000,
                saved_at_secs: 900,
            },
        );
        let (after, _) = step(
            restored,
            Event::Decoded {
                job: job(5),
                bitmap: bmp(9),
            },
        );
        assert!(
            matches!(after, View::Loading { decode: None }),
            "the host put the discarded pixels in the slot the restored picture \
             was drawn from, so that picture cannot stay on screen"
        );
    }

    #[test]
    fn a_reload_cannot_re_want_a_discarded_decode() {
        let (changed, _) = step(
            shown(Badge::Fresh, Some(decode(5, 1.0))),
            Event::TargetChanged,
        );
        let (reloaded, _) = step(changed, Event::Reload);
        assert!(
            matches!(reloaded, View::Loading { decode: Some(d) } if d.fate == Fate::Discard),
            "a reload asks for the current target, which these pixels are not"
        );
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
        let v = shown(Badge::Fresh, Some(decode(1, 1.0)));
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
    fn a_superseded_target_keeps_the_picture_a_changed_one_drops_it() {
        let (superseded, actions) = step(shown(Badge::Fresh, None), Event::TargetSuperseded);
        assert!(
            matches!(
                superseded,
                View::Shown {
                    badge: Badge::Updating,
                    ..
                }
            ),
            "a newer publication leaves the old picture up until the new one decodes"
        );
        assert!(actions.contains(&Action::ResumePoll));

        let (changed, _) = step(shown(Badge::Fresh, None), Event::TargetChanged);
        assert!(
            matches!(changed, View::Loading { .. }),
            "an edited target means the shown picture is not what was asked for"
        );
    }

    #[test]
    fn a_superseded_target_with_nothing_shown_loads() {
        let (next, actions) = step(View::Loading { decode: None }, Event::TargetSuperseded);
        assert!(matches!(next, View::Loading { decode: None }));
        assert!(actions.contains(&Action::ResumePoll));
    }

    #[test]
    fn a_superseded_target_keeps_tracking_its_in_flight_decode() {
        let v = shown(Badge::Fresh, Some(decode(5, 2.0)));
        let (next, _) = step(v, Event::TargetSuperseded);
        let (after, actions) = step(
            next,
            Event::Decoded {
                job: job(5),
                bitmap: bmp(9),
            },
        );
        assert!(
            matches!(after, View::Shown { bitmap, aspect, badge: Badge::Updating, .. }
                if bitmap == bmp(9) && aspect == 2.0),
            "the host gave the slot this decode's pixels, so its aspect comes too, \
             and the publication that superseded it is still being fetched"
        );
        assert!(actions.contains(&Action::RequestFrame));
    }

    #[test]
    fn a_reload_during_a_decode_takes_the_completion_and_keeps_updating() {
        let v = shown(Badge::Fresh, Some(decode(5, 2.0)));
        let (next, _) = step(v, Event::Reload);
        assert!(
            matches!(next, View::Shown { decode: Some(d), .. }
                if d.job == job(5) && d.fate == Fate::ShowSuperseded),
            "the decode the reload replaced is still running and still owns the slot"
        );
        let (after, _) = step(
            next,
            Event::Decoded {
                job: job(5),
                bitmap: bmp(9),
            },
        );
        assert!(
            matches!(after, View::Shown { bitmap, aspect, badge: Badge::Updating, decode: None }
                if bitmap == bmp(9) && aspect == 2.0),
            "aspect and pixels move together, and the pill stays up until the reload lands"
        );
    }

    #[test]
    fn a_superseded_decode_failing_leaves_the_picture_alone() {
        let v = shown(Badge::Fresh, Some(decode(5, 2.0)));
        let (next, _) = step(v, Event::Reload);
        let (after, actions) = step(next, Event::DecodeFailed { job: job(5) });
        assert!(
            matches!(
                after,
                View::Shown {
                    badge: Badge::Updating,
                    decode: None,
                    ..
                }
            ),
            "nothing reached the slot, so the picture keeps its badge \
             and the reload decides what happens next"
        );
        assert!(
            actions.is_empty(),
            "no repaint and no stale mark for a picture that never changed"
        );
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
    fn a_wake_restore_replaces_what_the_dormant_view_held() {
        let (dormant, _) = step(shown(Badge::Fresh, None), Event::Sleep);
        assert!(matches!(dormant, View::Shown { .. }));
        let (woken, _) = step(
            dormant,
            Event::Restored {
                bitmap: bmp(2),
                aspect: 2.0,
                remaining_ms: 4_000,
                saved_at_secs: 900,
            },
        );
        assert!(
            matches!(woken, View::Shown { bitmap, aspect, .. }
                if bitmap == bmp(2) && aspect == 2.0),
            "a decode that finished while dormant swapped the cached picture, \
             so the restored dimensions have to beat the ones the view held"
        );
    }

    #[test]
    fn sleep_keeps_a_decode_the_host_still_owes() {
        let (dormant, actions) = step(shown(Badge::Fresh, Some(decode(3, 2.0))), Event::Sleep);
        assert!(
            matches!(dormant, View::Shown { decode: Some(d), .. } if d.job == job(3)),
            "the decode runs on in the host, so a completion arriving after \
             the wake needs a view that still accepts it"
        );
        assert_eq!(actions, vec![Action::DisablePoll]);
    }

    #[test]
    fn a_wake_restore_keeps_a_decode_the_host_still_owes() {
        let (dormant, _) = step(shown(Badge::Fresh, Some(decode(3, 2.0))), Event::Sleep);
        let (woken, _) = step(
            dormant,
            Event::Restored {
                bitmap: bmp(2),
                aspect: 1.0,
                remaining_ms: 4_000,
                saved_at_secs: 900,
            },
        );
        assert!(
            matches!(
                woken,
                View::Shown {
                    badge: Badge::Updating,
                    decode: Some(_),
                    ..
                }
            ),
            "flash held the older picture and the decode names a newer one"
        );
        let (after, _) = step(
            woken,
            Event::Decoded {
                job: job(3),
                bitmap: bmp(2),
            },
        );
        assert!(
            matches!(after, View::Shown { aspect, badge: Badge::Fresh, decode: None, .. }
                if aspect == 2.0),
            "the decode's own aspect describes the pixels it put in the slot"
        );
    }

    #[test]
    fn a_restore_miss_keeps_a_decode_the_host_still_owes() {
        let (next, actions) = step(
            View::Loading {
                decode: Some(decode(3, 2.0)),
            },
            Event::RestoreMiss,
        );
        assert!(matches!(next, View::Loading { decode: Some(_) }));
        assert!(actions.contains(&Action::ResumePoll));
    }

    #[test]
    fn a_fetch_failure_waits_for_a_decode_it_is_owed() {
        let (next, actions) = step(
            View::Loading {
                decode: Some(decode(3, 2.0)),
            },
            Event::FetchError {
                kind: ErrorKind::LoadFailed,
                transient: true,
            },
        );
        assert!(
            matches!(next, View::Loading { decode: Some(_) }),
            "a fetch the decode slot had no room for says nothing about \
             the decode occupying it"
        );
        assert!(actions.contains(&Action::RequestFrame));
        assert!(
            actions.contains(&Action::Retry),
            "the reply was HTTP-ok, so only this re-asks the refused fetch"
        );
    }

    #[test]
    fn a_broken_body_arriving_over_an_owed_decode_defers_rather_than_retries() {
        let (next, actions) = step(
            View::Loading {
                decode: Some(decode(3, 2.0)),
            },
            Event::FetchError {
                kind: ErrorKind::BadImage,
                transient: false,
            },
        );
        assert!(matches!(next, View::Loading { decode: Some(_) }));
        assert!(actions.contains(&Action::DeferPoll));
        assert!(!actions.contains(&Action::Retry));
    }

    #[test]
    fn an_abandoned_decode_stops_being_waited_for() {
        let (next, actions) = step(
            shown(Badge::Fresh, Some(decode(3, 2.0))),
            Event::DecodeAbandoned { job: job(3) },
        );
        assert!(matches!(next, View::Shown { decode: None, .. }));
        assert!(
            actions.is_empty(),
            "the restore that follows the wake is what redraws"
        );

        let (foreign, _) = step(
            shown(Badge::Fresh, Some(decode(3, 2.0))),
            Event::DecodeAbandoned { job: job(4) },
        );
        assert!(matches!(
            foreign,
            View::Shown {
                decode: Some(_),
                ..
            }
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
