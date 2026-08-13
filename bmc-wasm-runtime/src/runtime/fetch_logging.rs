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

//! Per-widget admission policy for fetch outcome logs.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use bmc_wasm_protocol::FetchOutcome;

use crate::host_api::{FetchCompletionContext, FetchRequestDigest, FetchRequestKey};

const FAILURE_REMINDER_INTERVAL_MS: u64 = 30 * 60 * 1_000;
/// A round number, not a derived one: widget URLs are quasi-static,
/// so a runtime is not expected to fail against this many distinct keys at once.
/// Past the cap, eviction turns suppression back into `First`,
/// so a runtime that does exceed it degrades toward logging every failure.
const MAX_FETCH_FAILURE_EPISODES: usize = 256;
/// Distinct causes one episode remembers, so that a flapping origin is logged
/// once per cause per window rather than once per attempt.
///
/// It doubles as the episode's budget of lines per reminder window. Both need
/// a bound and neither derives one, so they share a number: an origin rotating
/// more causes than this evicts the one it is about to repeat, and without the
/// budget every attempt would read as a change and log forever.
const MAX_FETCH_FAILURE_CAUSES: usize = 4;
/// How long an episode stays worth keeping once nothing touches it.
///
/// It has to exceed the reminder interval, or an origin polled more slowly
/// than that would lose its episode between attempts and read as new every
/// time. An outage still under way refreshes its episode on every attempt,
/// so going this long unseen means the widget has stopped asking.
const FETCH_EPISODE_IDLE_TIMEOUT_MS: u64 = 2 * FAILURE_REMINDER_INTERVAL_MS;

/// A 304 cache hit and an un-followed redirect are answers, not failures.
/// An informational 1xx arriving as a final status is not one of them.
/// `FetchOutcome::is_ok` stays narrower on purpose:
/// it is the guest-facing 2xx contract and cannot widen to carry this.
const fn is_non_failure(outcome: FetchOutcome) -> bool {
    matches!(outcome, FetchOutcome::Http(200..=399))
}

/// What tells two failures of the same request apart.
///
/// The refusal is reduced to a digest of the text an operator reads, so two
/// refusals differ here exactly when they read differently. Four of these
/// outlive every failing request for the length of an outage, and the refusal
/// they stand in for carries guest-named slots and fields.
///
/// Two refusals colliding here read as one cause,
/// costing the second a suppressed line — the same trade
/// `FetchRequestDigest` makes, accepted for the same reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FailureFingerprint {
    status: u32,
    refusal: Option<u64>,
}

impl FailureFingerprint {
    fn new(status: u32, context: &FetchCompletionContext) -> Self {
        let refusal = match context {
            FetchCompletionContext::CredentialRefusal(refusal) => {
                let mut hasher = DefaultHasher::new();
                refusal.to_string().hash(&mut hasher);
                Some(hasher.finish())
            }
            FetchCompletionContext::Normal | FetchCompletionContext::HermeticRefusal => None,
        };
        Self { status, refusal }
    }

    const fn status(self) -> u32 {
        self.status
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureAdmission {
    First,
    Changed,
    Reminder,
}

impl FailureAdmission {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::First => "first",
            Self::Changed => "changed",
            Self::Reminder => "reminder",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FetchLogDecision {
    LogSuccess,
    LogFailure {
        admission: FailureAdmission,
        previous_status: Option<u32>,
    },
    NoLog,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FailureEpisode {
    /// Most recently logged last. A cause already here is a return, not a
    /// change, so it waits for the window to roll instead of logging on every
    /// attempt.
    causes: Vec<FailureFingerprint>,
    /// Lines spent since the window last rolled.
    /// Every admitted line re-anchors the window,
    /// so a trickle of changes slides it along instead of rolling it,
    /// and this budget is what stops them.
    logged_in_window: usize,
    last_logged_at_ms: u64,
    last_seen_ms: u64,
}

impl FailureEpisode {
    /// Only a cause that reached the log is remembered. One the budget
    /// suppressed was never reported, so it has to still read as news
    /// whenever it comes back.
    ///
    /// The oldest goes before the newest arrives,
    /// so the buffer never reaches the length that would double its capacity
    /// and hold the excess for as long as the episode lives.
    fn remember(&mut self, cause: FailureFingerprint) {
        if self.causes.len() == MAX_FETCH_FAILURE_CAUSES {
            self.causes.remove(0);
        }
        self.causes.push(cause);
    }
}

#[derive(Debug, Default)]
pub(crate) struct FetchLogLimiter {
    episodes: HashMap<FetchRequestDigest, FailureEpisode>,
}

impl FetchLogLimiter {
    pub(crate) fn record(
        &mut self,
        key: &FetchRequestKey,
        status: u32,
        context: &FetchCompletionContext,
        now_ms: u64,
    ) -> FetchLogDecision {
        self.drop_idle_episodes(now_ms);

        if FetchOutcome::from_wire(status) == Some(FetchOutcome::Aborted)
            || matches!(context, FetchCompletionContext::HermeticRefusal)
        {
            return FetchLogDecision::NoLog;
        }

        let digest = key.digest();
        if FetchOutcome::from_wire(status).is_some_and(is_non_failure) {
            self.episodes.remove(&digest);
            return FetchLogDecision::LogSuccess;
        }

        let fingerprint = FailureFingerprint::new(status, context);
        if let Some(episode) = self.episodes.get_mut(&digest) {
            episode.last_seen_ms = episode.last_seen_ms.max(now_ms);

            let window_rolled = now_ms
                .checked_sub(episode.last_logged_at_ms)
                .is_some_and(|elapsed| elapsed >= FAILURE_REMINDER_INTERVAL_MS);

            let returning = episode.causes.iter().position(|seen| *seen == fingerprint);
            if let Some(position) = returning {
                if !window_rolled {
                    return FetchLogDecision::NoLog;
                }
                // Ordering is what `previous_status` reads back,
                // and it has to name the failure an operator was last shown,
                // so a cause moves to the end only on the attempt that logs.
                let cause = episode.causes.remove(position);
                episode.causes.push(cause);
                episode.logged_in_window = 1;
                episode.last_logged_at_ms = now_ms;
                return FetchLogDecision::LogFailure {
                    admission: FailureAdmission::Reminder,
                    previous_status: None,
                };
            }

            if episode.logged_in_window >= MAX_FETCH_FAILURE_CAUSES && !window_rolled {
                return FetchLogDecision::NoLog;
            }

            let previous_status = episode
                .causes
                .last()
                .copied()
                .map(FailureFingerprint::status);
            episode.remember(fingerprint);
            if window_rolled {
                episode.logged_in_window = 1;
                episode.last_logged_at_ms = now_ms;
            } else {
                episode.logged_in_window += 1;
                episode.last_logged_at_ms = episode.last_logged_at_ms.max(now_ms);
            }
            return FetchLogDecision::LogFailure {
                admission: FailureAdmission::Changed,
                previous_status,
            };
        }

        self.evict_if_full();
        self.episodes.insert(
            digest,
            FailureEpisode {
                causes: {
                    let mut causes = Vec::with_capacity(MAX_FETCH_FAILURE_CAUSES);
                    causes.push(fingerprint);
                    causes
                },
                logged_in_window: 1,
                last_logged_at_ms: now_ms,
                last_seen_ms: now_ms,
            },
        );
        FetchLogDecision::LogFailure {
            admission: FailureAdmission::First,
            previous_status: None,
        }
    }

    /// Swept on the next outcome rather than on a clock,
    /// which is the only moment a stale episode can do harm:
    /// by suppressing the `First` that failure deserves.
    /// A widget that stops fetching keeps its episodes,
    /// bounded as ever by `MAX_FETCH_FAILURE_EPISODES`.
    fn drop_idle_episodes(&mut self, now_ms: u64) {
        self.episodes.retain(|_, episode| {
            now_ms
                .checked_sub(episode.last_seen_ms)
                .is_none_or(|idle| idle < FETCH_EPISODE_IDLE_TIMEOUT_MS)
        });
    }

    fn evict_if_full(&mut self) {
        if self.episodes.len() < MAX_FETCH_FAILURE_EPISODES {
            return;
        }
        let oldest = self
            .episodes
            .iter()
            .min_by_key(|(_, episode)| episode.last_seen_ms)
            .map(|(digest, _)| *digest);
        if let Some(oldest) = oldest {
            self.episodes.remove(&oldest);
        }
    }
}

#[cfg(test)]
mod tests {
    use bmc_wasm_protocol::FetchOutcome;

    use super::{
        FAILURE_REMINDER_INTERVAL_MS, FETCH_EPISODE_IDLE_TIMEOUT_MS, FailureAdmission,
        FetchLogDecision, FetchLogLimiter, MAX_FETCH_FAILURE_CAUSES, MAX_FETCH_FAILURE_EPISODES,
    };
    use crate::host_api::{FetchCompletionContext, FetchRequestKey};
    use crate::runtime::CredentialRefusal;
    use crate::runtime::imports::credentials::SubstitutionError;

    fn key(method: &str, url: &str) -> FetchRequestKey {
        FetchRequestKey::new(method, url)
    }

    fn first() -> FetchLogDecision {
        FetchLogDecision::LogFailure {
            admission: FailureAdmission::First,
            previous_status: None,
        }
    }

    fn reminder() -> FetchLogDecision {
        FetchLogDecision::LogFailure {
            admission: FailureAdmission::Reminder,
            previous_status: None,
        }
    }

    fn changed(previous_status: u32) -> FetchLogDecision {
        FetchLogDecision::LogFailure {
            admission: FailureAdmission::Changed,
            previous_status: Some(previous_status),
        }
    }

    #[test]
    fn initial_success_is_debuggable_and_initial_failure_is_admitted() {
        let request = key("GET", "https://example.test/data");
        let mut limiter = FetchLogLimiter::default();

        assert_eq!(
            limiter.record(&request, 200, &FetchCompletionContext::Normal, 1,),
            FetchLogDecision::LogSuccess,
        );
        assert_eq!(
            limiter.record(&request, 500, &FetchCompletionContext::Normal, 2,),
            first(),
        );
    }

    #[test]
    fn identical_failures_are_suppressed_until_the_reminder_boundary() {
        let request = key("GET", "https://example.test/data");
        let mut limiter = FetchLogLimiter::default();

        assert_eq!(
            limiter.record(&request, 500, &FetchCompletionContext::Normal, 0),
            first(),
        );
        assert_eq!(
            limiter.record(&request, 500, &FetchCompletionContext::Normal, 1),
            FetchLogDecision::NoLog,
        );
        assert_eq!(
            limiter.record(
                &request,
                500,
                &FetchCompletionContext::Normal,
                FAILURE_REMINDER_INTERVAL_MS - 1,
            ),
            FetchLogDecision::NoLog,
        );
        assert_eq!(
            limiter.record(
                &request,
                500,
                &FetchCompletionContext::Normal,
                FAILURE_REMINDER_INTERVAL_MS,
            ),
            reminder(),
        );
        assert_eq!(
            limiter.record(
                &request,
                500,
                &FetchCompletionContext::Normal,
                FAILURE_REMINDER_INTERVAL_MS + 1,
            ),
            FetchLogDecision::NoLog,
        );
        assert_eq!(
            limiter.record(
                &request,
                500,
                &FetchCompletionContext::Normal,
                2 * FAILURE_REMINDER_INTERVAL_MS,
            ),
            reminder(),
        );
    }

    #[test]
    fn success_ends_an_episode() {
        let request = key("GET", "https://example.test/data");
        let mut limiter = FetchLogLimiter::default();

        assert_eq!(
            limiter.record(&request, 500, &FetchCompletionContext::Normal, 10),
            first(),
        );
        assert_eq!(
            limiter.record(&request, 204, &FetchCompletionContext::Normal, 11),
            FetchLogDecision::LogSuccess,
        );
        assert_eq!(
            limiter.record(&request, 500, &FetchCompletionContext::Normal, 12),
            first(),
        );
    }

    #[test]
    fn a_redirect_or_not_modified_ends_an_episode_rather_than_extending_it() {
        let request = key("GET", "https://example.test/data");
        let mut limiter = FetchLogLimiter::default();

        assert_eq!(
            limiter.record(&request, 500, &FetchCompletionContext::Normal, 1),
            first(),
        );
        assert_eq!(
            limiter.record(&request, 302, &FetchCompletionContext::Normal, 2),
            FetchLogDecision::LogSuccess,
            "an un-followed redirect is an answer, not a failure",
        );
        assert_eq!(
            limiter.record(&request, 500, &FetchCompletionContext::Normal, 3),
            first(),
            "the redirect ended the episode, so this failure starts a new one",
        );
        assert_eq!(
            limiter.record(&request, 304, &FetchCompletionContext::Normal, 4),
            FetchLogDecision::LogSuccess,
            "a conditional GET answered from cache is not a failure",
        );
        assert_eq!(
            limiter.record(&request, 500, &FetchCompletionContext::Normal, 5),
            first(),
            "and the cache hit ended that episode in its turn",
        );
    }

    #[test]
    fn each_new_raw_status_is_admitted_once() {
        let request = key("GET", "https://example.test/data");
        let mut limiter = FetchLogLimiter::default();

        assert_eq!(
            limiter.record(&request, 500, &FetchCompletionContext::Normal, 1),
            first(),
        );
        assert_eq!(
            limiter.record(&request, 503, &FetchCompletionContext::Normal, 2),
            changed(500),
        );
        assert_eq!(
            limiter.record(
                &request,
                FetchOutcome::Network.to_wire(),
                &FetchCompletionContext::Normal,
                3,
            ),
            changed(503),
        );
        assert_eq!(
            limiter.record(&request, 4_242, &FetchCompletionContext::Normal, 4),
            changed(FetchOutcome::Network.to_wire()),
        );
    }

    /// The flood BDK-700 exists to stop, reached by alternating causes rather
    /// than by repeating one: an origin answering 500, 503, 500, 503 every
    /// poll must cost two lines for the outage, not one per attempt.
    #[test]
    fn a_cause_returning_within_an_episode_is_suppressed_like_a_repeat() {
        let request = key("GET", "https://example.test/data");
        let mut limiter = FetchLogLimiter::default();

        assert_eq!(
            limiter.record(&request, 500, &FetchCompletionContext::Normal, 1),
            first(),
        );
        assert_eq!(
            limiter.record(&request, 503, &FetchCompletionContext::Normal, 2),
            changed(500),
        );
        assert_eq!(
            limiter.record(&request, 500, &FetchCompletionContext::Normal, 3),
            FetchLogDecision::NoLog,
            "a cause the episode has already named is not a change",
        );
        assert_eq!(
            limiter.record(&request, 503, &FetchCompletionContext::Normal, 4),
            FetchLogDecision::NoLog,
        );
    }

    /// `previous_status` is read as "what this replaces",
    /// so it has to name the failure the log last carried.
    /// A return the window suppressed reached no operator,
    /// and must not pass itself off as that failure.
    #[test]
    fn a_suppressed_return_does_not_become_the_previous_status() {
        let request = key("GET", "https://example.test/data");
        let mut limiter = FetchLogLimiter::default();

        assert_eq!(
            limiter.record(&request, 500, &FetchCompletionContext::Normal, 1),
            first(),
        );
        assert_eq!(
            limiter.record(&request, 503, &FetchCompletionContext::Normal, 2),
            changed(500),
        );
        assert_eq!(
            limiter.record(&request, 500, &FetchCompletionContext::Normal, 3),
            FetchLogDecision::NoLog,
        );

        assert_eq!(
            limiter.record(&request, 502, &FetchCompletionContext::Normal, 4),
            changed(503),
            "503 was the last failure logged; the 500 in between was suppressed",
        );
    }

    /// A rotation wider than the remembered set evicts each cause just before
    /// it returns, so every attempt reads as a change. Without a budget on the
    /// window that is the original flood, reached the long way round.
    #[test]
    fn an_endlessly_rotating_cause_cannot_outrun_the_window_budget() {
        const ROUNDS: usize = 100;

        let request = key("GET", "https://example.test/data");
        let mut limiter = FetchLogLimiter::default();

        let rotation = MAX_FETCH_FAILURE_CAUSES + 1;
        let status_of =
            |round: usize| u32::try_from(500 + round % rotation).expect("BUG: a status fits");
        let admitted = (0..ROUNDS)
            .filter(|round| {
                limiter.record(
                    &request,
                    status_of(*round),
                    &FetchCompletionContext::Normal,
                    0,
                ) != FetchLogDecision::NoLog
            })
            .count();

        assert_eq!(
            admitted, MAX_FETCH_FAILURE_CAUSES,
            "an episode spends its window budget once, however long the rotation runs",
        );
        let last_admitted = status_of(MAX_FETCH_FAILURE_CAUSES - 1);
        assert_eq!(
            limiter.record(
                &request,
                last_admitted,
                &FetchCompletionContext::Normal,
                FAILURE_REMINDER_INTERVAL_MS,
            ),
            reminder(),
            "the rolled window restates the outage once",
        );
        assert_eq!(
            limiter.record(
                &request,
                599,
                &FetchCompletionContext::Normal,
                FAILURE_REMINDER_INTERVAL_MS,
            ),
            changed(last_admitted),
            "and the budget it restored admits the next change, rather than the reminder standing alone",
        );
    }

    /// Past its budget an episode stays silent until the window rolls, so the
    /// line that finally breaks the silence is the operator's only sight of
    /// the outage. It must say which of the two it is.
    #[test]
    fn a_cause_first_seen_past_the_budget_is_admitted_as_a_change_not_a_reminder() {
        let request = key("GET", "https://example.test/data");
        let mut limiter = FetchLogLimiter::default();

        for status in 500..500 + u32::try_from(MAX_FETCH_FAILURE_CAUSES).expect("BUG: a cap fits") {
            limiter.record(&request, status, &FetchCompletionContext::Normal, 0);
        }
        assert_eq!(
            limiter.record(&request, 599, &FetchCompletionContext::Normal, 1),
            FetchLogDecision::NoLog,
            "the budget is spent, so a new cause waits for the window to roll",
        );

        assert_eq!(
            limiter.record(
                &request,
                599,
                &FetchCompletionContext::Normal,
                FAILURE_REMINDER_INTERVAL_MS,
            ),
            changed(503),
            "a cause suppressed unheard is still news, not an outage the log has named",
        );
    }

    /// An outage under way refreshes its episode on every attempt, so one that
    /// goes quiet is a widget that moved on. The state it pins is chosen by the
    /// widget, and a runtime lives for months.
    #[test]
    fn an_episode_left_untouched_is_dropped_rather_than_pinned_for_the_runtime() {
        let abandoned = key("GET", "https://example.test/abandoned");
        let polled = key("GET", "https://example.test/polled");
        let mut limiter = FetchLogLimiter::default();

        assert_eq!(
            limiter.record(&abandoned, 500, &FetchCompletionContext::Normal, 0),
            first(),
        );
        limiter.record(
            &polled,
            500,
            &FetchCompletionContext::Normal,
            FETCH_EPISODE_IDLE_TIMEOUT_MS,
        );

        assert!(
            !limiter.episodes.contains_key(&abandoned.digest()),
            "a request the widget stopped making must not hold its episode forever",
        );
    }

    /// The sweep must not outrun the reminder: an origin polled less often than
    /// the reminder interval would otherwise lose its episode between attempts
    /// and read as new every time, which is the flood it exists to stop.
    ///
    /// The gap is measured in reminder intervals rather than against the timeout:
    /// shortening the timeout would otherwise drag the poll along with it,
    /// and the margin between the two would go untested.
    #[test]
    fn an_episode_polled_more_slowly_than_the_reminder_survives_to_send_one() {
        let request = key("GET", "https://example.test/data");
        let mut limiter = FetchLogLimiter::default();

        assert_eq!(
            limiter.record(&request, 500, &FetchCompletionContext::Normal, 0),
            first(),
        );
        assert_eq!(
            limiter.record(
                &request,
                500,
                &FetchCompletionContext::Normal,
                2 * FAILURE_REMINDER_INTERVAL_MS - 1,
            ),
            reminder(),
            "an episode has to outlast the slowest poll the reminder still serves",
        );
    }

    /// The episode is held for as long as the outage lasts, so what it
    /// remembers has to stay bounded however many causes the origin invents.
    ///
    /// One cause per window, because only an admitted cause is remembered:
    /// rotated inside a single window the budget stops the set growing long
    /// before the cap does, and the bound goes untested.
    #[test]
    fn an_episode_remembers_no_more_causes_than_the_cap() {
        let request = key("GET", "https://example.test/data");
        let mut limiter = FetchLogLimiter::default();

        for (window, status) in (500..=599_u32).enumerate() {
            let now_ms = u64::try_from(window).expect("BUG: a window count fits")
                * FAILURE_REMINDER_INTERVAL_MS;
            limiter.record(&request, status, &FetchCompletionContext::Normal, now_ms);
        }

        assert_eq!(
            limiter.episodes[&request.digest()].causes.len(),
            MAX_FETCH_FAILURE_CAUSES,
        );
        assert_eq!(
            limiter.episodes[&request.digest()].causes.capacity(),
            MAX_FETCH_FAILURE_CAUSES,
            "what an episode holds is retained for months, so a reserve past \
             the cap is retained too",
        );
    }

    #[test]
    fn a_change_restarts_the_reminder_window() {
        let request = key("GET", "https://example.test/data");
        let mut limiter = FetchLogLimiter::default();

        assert_eq!(
            limiter.record(&request, 500, &FetchCompletionContext::Normal, 0),
            first(),
        );
        assert_eq!(
            limiter.record(&request, 503, &FetchCompletionContext::Normal, 1),
            changed(500),
        );
        assert_eq!(
            limiter.record(
                &request,
                503,
                &FetchCompletionContext::Normal,
                FAILURE_REMINDER_INTERVAL_MS,
            ),
            FetchLogDecision::NoLog,
            "a changed failure starts a new reminder window",
        );
        assert_eq!(
            limiter.record(
                &request,
                503,
                &FetchCompletionContext::Normal,
                FAILURE_REMINDER_INTERVAL_MS + 1,
            ),
            reminder(),
        );
    }

    #[test]
    fn a_change_during_clock_rewind_does_not_admit_an_early_reminder() {
        let request = key("GET", "https://example.test/data");
        let mut limiter = FetchLogLimiter::default();

        assert_eq!(
            limiter.record(
                &request,
                500,
                &FetchCompletionContext::Normal,
                FAILURE_REMINDER_INTERVAL_MS,
            ),
            first(),
        );
        assert_eq!(
            limiter.record(&request, 503, &FetchCompletionContext::Normal, 1),
            changed(500),
        );
        assert_eq!(
            limiter.record(
                &request,
                503,
                &FetchCompletionContext::Normal,
                FAILURE_REMINDER_INTERVAL_MS + 1,
            ),
            FetchLogDecision::NoLog,
            "a rewound clock must not move the changed failure's log anchor backwards",
        );
    }

    #[test]
    fn refusal_category_and_bounded_payload_are_part_of_the_fingerprint() {
        let request = key("GET", "https://example.test/data");
        let first_refusal = CredentialRefusal::DestinationNotUrl;
        let second_refusal = CredentialRefusal::Substitution(SubstitutionError::UnknownField {
            slot: "weather".to_owned(),
            field: "token".to_owned(),
        });
        // Every refusal settles on the same wire status, so only the refusal
        // itself can tell these three causes apart.
        let status = FetchOutcome::Refused.to_wire();
        let mut limiter = FetchLogLimiter::default();

        assert_eq!(
            limiter.record(
                &request,
                status,
                &FetchCompletionContext::CredentialRefusal(first_refusal.clone()),
                1,
            ),
            first(),
        );
        assert_eq!(
            limiter.record(
                &request,
                status,
                &FetchCompletionContext::CredentialRefusal(second_refusal.clone()),
                2,
            ),
            changed(status),
        );
        let relabelled_field = CredentialRefusal::Substitution(SubstitutionError::UnknownField {
            slot: "weather".to_owned(),
            field: "key".to_owned(),
        });
        assert_eq!(
            limiter.record(
                &request,
                status,
                &FetchCompletionContext::CredentialRefusal(relabelled_field),
                3,
            ),
            changed(status),
            "a different refusal payload must name a different failure cause",
        );
    }

    #[test]
    fn aborted_and_hermetic_results_preserve_the_current_episode() {
        let request = key("GET", "https://example.test/data");
        let mut limiter = FetchLogLimiter::default();

        assert_eq!(
            limiter.record(&request, 500, &FetchCompletionContext::Normal, 0),
            first(),
        );
        assert_eq!(
            limiter.record(
                &request,
                FetchOutcome::Aborted.to_wire(),
                &FetchCompletionContext::Normal,
                FAILURE_REMINDER_INTERVAL_MS,
            ),
            FetchLogDecision::NoLog,
        );
        assert_eq!(
            limiter.record(
                &request,
                FetchOutcome::Network.to_wire(),
                &FetchCompletionContext::HermeticRefusal,
                FAILURE_REMINDER_INTERVAL_MS,
            ),
            FetchLogDecision::NoLog,
        );
        assert_eq!(
            limiter.record(
                &request,
                500,
                &FetchCompletionContext::Normal,
                FAILURE_REMINDER_INTERVAL_MS,
            ),
            reminder(),
        );
    }

    #[test]
    fn method_url_and_limiter_instances_isolate_episodes() {
        let get = key("GET", "https://example.test/data");
        let post = key("POST", "https://example.test/data");
        let other_url = key("GET", "https://example.test/other");
        let mut first_limiter = FetchLogLimiter::default();
        let mut second_limiter = FetchLogLimiter::default();

        for request in [&get, &post, &other_url] {
            assert_eq!(
                first_limiter.record(request, 500, &FetchCompletionContext::Normal, 1),
                first(),
            );
        }
        assert_eq!(
            second_limiter.record(&get, 500, &FetchCompletionContext::Normal, 1),
            first(),
        );
    }

    #[test]
    fn a_rewound_clock_neither_regresses_activity_nor_admits_early() {
        let request = key("GET", "https://example.test/data");
        let mut limiter = FetchLogLimiter::default();

        assert_eq!(
            limiter.record(&request, 500, &FetchCompletionContext::Normal, 100),
            first(),
        );
        assert_eq!(
            limiter.record(&request, 500, &FetchCompletionContext::Normal, 50),
            FetchLogDecision::NoLog,
        );
        assert_eq!(
            limiter.record(
                &request,
                500,
                &FetchCompletionContext::Normal,
                100 + FAILURE_REMINDER_INTERVAL_MS,
            ),
            reminder(),
        );
    }

    #[test]
    fn capacity_evicts_the_least_recently_seen_episode() {
        let mut limiter = FetchLogLimiter::default();
        for i in 0..MAX_FETCH_FAILURE_EPISODES {
            let request = key("GET", &format!("https://example.test/{i}"));
            assert_eq!(
                limiter.record(
                    &request,
                    500,
                    &FetchCompletionContext::Normal,
                    100 + i as u64,
                ),
                first(),
            );
        }

        let refreshed = key("GET", "https://example.test/0");
        assert_eq!(
            limiter.record(&refreshed, 500, &FetchCompletionContext::Normal, 1_000,),
            FetchLogDecision::NoLog,
        );
        assert_eq!(
            limiter.record(&refreshed, 503, &FetchCompletionContext::Normal, 1),
            changed(500),
            "a changed failure during clock rewind must preserve recency",
        );
        let newcomer = key("GET", "https://example.test/new");
        assert_eq!(
            limiter.record(&newcomer, 500, &FetchCompletionContext::Normal, 1_001,),
            first(),
        );

        let evicted = key("GET", "https://example.test/1");
        assert_eq!(
            limiter.record(&evicted, 500, &FetchCompletionContext::Normal, 1_002,),
            first(),
            "suppressed activity must protect the refreshed episode from eviction",
        );
        assert_eq!(
            limiter.record(&refreshed, 503, &FetchCompletionContext::Normal, 1_003,),
            FetchLogDecision::NoLog,
        );
    }

    #[test]
    fn a_changed_failure_refreshes_recency() {
        let mut limiter = FetchLogLimiter::default();
        for i in 0..MAX_FETCH_FAILURE_EPISODES {
            let request = key("GET", &format!("https://example.test/{i}"));
            assert_eq!(
                limiter.record(
                    &request,
                    500,
                    &FetchCompletionContext::Normal,
                    100 + i as u64,
                ),
                first(),
            );
        }

        let refreshed = key("GET", "https://example.test/0");
        assert_eq!(
            limiter.record(&refreshed, 503, &FetchCompletionContext::Normal, 1_000,),
            changed(500),
        );
        let newcomer = key("GET", "https://example.test/new");
        assert_eq!(
            limiter.record(&newcomer, 500, &FetchCompletionContext::Normal, 1_001,),
            first(),
        );
        assert_eq!(
            limiter.record(
                &key("GET", "https://example.test/1"),
                500,
                &FetchCompletionContext::Normal,
                1_002,
            ),
            first(),
            "a changed failure must protect the request from recency eviction",
        );
    }
}
