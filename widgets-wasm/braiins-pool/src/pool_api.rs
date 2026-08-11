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

//! Braiins Pool API: URLs, query building, and reply parsing.
//!
//! Parsing is pure and host-tested. Replies arrive as JSON-pointer documents
//! ([`JsonLookup`]); timestamp strings resolve through an injected parser so
//! tests run without the host's date support.

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "API code uses the SDK's macros and re-exports"
    )
)]
use bmc_wasm_sdk::*;

use crate::model::{NextPayout, Payout, PayoutKind, Rewards, Sample, Series, Source, WorkerCounts};

pub const BASE_URL: &str = "https://api.braiins.com/pool/v2";

/// The largest page the API will serve.
pub const API_PAGE_CAP: usize = 1_000;

/// Entries the widget asks for per page, well under [`API_PAGE_CAP`]:
/// a reply's parse runs on one frame's fuel, and a history slot costs about
/// 16 k of it in pointer reads — a full-cap page would trap mid-parse and
/// the reply would die with the frame. Deeper windows chain pages instead.
///
/// Doubles as the parsers' probe bound, so a page longer than this is read
/// only this far: the fuel ceiling holds even if a reply overruns what was
/// asked for, at the price of ignoring the overrun.
pub const PAGE_LIMIT: usize = 250;

const _: () = assert!(
    PAGE_LIMIT <= API_PAGE_CAP,
    "a page the API will not serve would truncate every listing"
);

/// URL construction lives on [`Source`] so call sites name the endpoint and
/// a query shape; every dynamic value passes through `form_urlencoded` and
/// the base/path join happens in exactly one place ([`Source::url`]).
impl Source {
    fn path(self) -> &'static str {
        match self {
            Self::HashrateCurrent => "/user/hashrate/current",
            Self::RewardsLatest => "/user/rewards/latest",
            Self::HashrateHistory => "/user/hashrate/history",
            Self::WorkersCurrent => "/user/workers/current",
            Self::WorkersHistory => "/user/workers/history",
            Self::Financials => "/user/financials",
            Self::PayoutsRecent => "/user/payouts/recent",
        }
    }

    /// Query-less endpoint URL.
    #[must_use]
    pub fn url(self) -> String {
        fmt!("{BASE_URL}{}", self.path())
    }

    /// URL for one page of a windowed, cursor-paginated listing:
    ///
    /// - [`Source::HashrateHistory`]
    /// - [`Source::WorkersHistory`]
    /// - [`Source::PayoutsRecent`] in its Big Chart query
    #[must_use]
    pub fn windowed_page_url(self, from: &str, to: &str, cursor: Option<&str>) -> String {
        let mut query = form_urlencoded::Serializer::new(String::new());
        query
            .append_pair("from_timestamp", from)
            .append_pair("to_timestamp", to)
            .append_pair("page_limit", &PAGE_LIMIT.to_string());
        if let Some(cursor) = cursor {
            query.append_pair("page_cursor", cursor);
        }
        fmt!("{}?{}", self.url(), query.finish())
    }

    /// URL for the Overview query of [`Source::PayoutsRecent`]:
    /// the latest page, no window.
    #[must_use]
    pub fn recent_page_url(self) -> String {
        let query = form_urlencoded::Serializer::new(String::new())
            .append_pair("page_limit", &PAGE_LIMIT.to_string())
            .finish();
        fmt!("{}?{query}", self.url())
    }
}

/// JSON-pointer lookup over a parsed reply. The wasm side backs it with
/// `JsonDoc`; host tests back it with a map-backed double.
pub trait JsonLookup {
    fn str(&self, path: &str) -> Option<String>;
    fn i64(&self, path: &str) -> Option<i64>;
    fn f64(&self, path: &str) -> Option<f64>;
    fn bool(&self, path: &str) -> Option<bool>;
}

#[cfg(target_arch = "wasm32")]
impl JsonLookup for bmc_wasm_sdk::json::JsonDoc {
    fn str(&self, path: &str) -> Option<String> {
        self.str(path)
    }

    fn i64(&self, path: &str) -> Option<i64> {
        self.i64(path)
    }

    fn f64(&self, path: &str) -> Option<f64> {
        self.f64(path)
    }

    fn bool(&self, path: &str) -> Option<bool> {
        self.bool(path)
    }
}

/// Timestamp-string → unix-seconds parser, injected so pure code never calls
/// the host directly (`host::parse_date` on wasm, a fixture map in tests).
pub type ParseDate<'a> = &'a dyn Fn(&str) -> Option<i64>;

/// The strftime pattern producing the API's RFC 3339 UTC timestamp form
/// ("2026-08-02T19:40:56Z"). Format with `bmc_wasm_sdk::format::strftime`,
/// which the host renders in UTC.
pub const RFC3339_UTC: &str = "%Y-%m-%dT%H:%M:%SZ";

/// What a reply says about a page after this one.
#[derive(Debug, PartialEq, Eq)]
pub enum NextPage {
    /// This page completes the listing: `has_next` is false, or the reply
    /// carries no pagination at all, as a single-page listing may not.
    Done,
    Cursor(String),
    /// `has_next` with no cursor to follow. The reply contradicts itself, so
    /// the listing cannot be completed — and must not pass for a complete one.
    Malformed,
}

#[must_use]
pub fn next_page(json: &impl JsonLookup) -> NextPage {
    match json.bool("/pagination/has_next") {
        Some(true) => match json.str("/pagination/next_cursor") {
            Some(cursor) if !cursor.is_empty() => NextPage::Cursor(cursor),
            _ => NextPage::Malformed,
        },
        _ => NextPage::Done,
    }
}

#[must_use]
pub fn parse_hashrate_current(json: &impl JsonLookup) -> Option<f64> {
    json.f64("/hashrate_th_per_sec")
}

#[must_use]
pub fn parse_rewards(json: &impl JsonLookup) -> Option<Rewards> {
    Some(Rewards {
        today_btc: json.f64("/todays_reward_estimate_btc")?,
        today_usd: json.f64("/todays_reward_estimate_usd")?,
    })
}

#[must_use]
pub fn parse_workers_current(json: &impl JsonLookup) -> Option<WorkerCounts> {
    let count = |path: &str| json.i64(path).and_then(|v| usize::try_from(v).ok());
    Some(WorkerCounts {
        active: count("/active_workers")?,
        low: count("/low_workers")?,
        offline: count("/offline_workers")?,
        disabled: count("/disabled_workers")?,
    })
}

/// One page of a history endpoint. `value_field` selects the per-slot value
/// (`hashrate_th_per_sec` or `active_workers`); either parses as f64 since
/// both series chart as continuous values.
#[must_use]
pub fn parse_history_page(
    json: &impl JsonLookup,
    value_field: &str,
    date: ParseDate<'_>,
) -> Option<Series> {
    let mut samples = Vec::new();
    for i in 0..PAGE_LIMIT {
        let Some(at) = json.str(&fmt!("/slots/{i}/slot_start")) else {
            break;
        };
        let value = json.f64(&fmt!("/slots/{i}/{value_field}"))?;
        samples.push(Sample {
            at: date(&at)?,
            value,
        });
    }
    Some(Series {
        from: json.str("/from_timestamp").and_then(|s| date(&s)),
        to: json.str("/to_timestamp").and_then(|s| date(&s)),
        samples,
    })
}

/// Fold `/user/financials` across the user's financial accounts: the soonest
/// payout estimate and the furthest progress, matching how the design shows a
/// single "Next Payout In" line.
#[must_use]
pub fn parse_financials(json: &impl JsonLookup, date: ParseDate<'_>) -> NextPayout {
    let mut folded = NextPayout::default();
    for i in 0..PAGE_LIMIT {
        let estimate = json
            .str(&fmt!("/financial_accounts/{i}/next_payout_at_estimate"))
            .and_then(|s| date(&s));
        let progress = json.f64(&fmt!("/financial_accounts/{i}/next_payout_progress_pct"));
        if estimate.is_none() && progress.is_none() {
            break;
        }
        folded.estimate_at = merge(folded.estimate_at, estimate, i64::min);
        folded.progress_pct = merge(folded.progress_pct, progress, f64::max);
    }
    folded
}

fn merge<T>(a: Option<T>, b: Option<T>, pick: fn(T, T) -> T) -> Option<T> {
    match (a, b) {
        (Some(a), Some(b)) => Some(pick(a, b)),
        (a, None) => a,
        (None, b) => b,
    }
}

/// One page of `/user/payouts/recent`, filtered to COMPLETED payouts — the
/// only status the widget shows (last payout amount, chart markers).
///
/// `None` when a completed payout cannot be read whole, the same rule
/// [`parse_history_page`] follows.
/// A listing short of an entry it should carry is not that listing:
/// passing it off as complete moves the last payout to an older one.
#[must_use]
pub fn parse_payouts_page(json: &impl JsonLookup, date: ParseDate<'_>) -> Option<Vec<Payout>> {
    let mut payouts = Vec::new();
    for i in 0..PAGE_LIMIT {
        let Some(status) = json.str(&fmt!("/payouts/{i}/status")) else {
            break;
        };
        // Only COMPLETED is shown, so another status is skipped rather than read:
        // a pending payout missing its amount is not a broken page.
        if status != "COMPLETED" {
            continue;
        }
        let kind = match json.str(&fmt!("/payouts/{i}/type")).as_deref() {
            Some("ONCHAIN") => Some(PayoutKind::Onchain),
            Some("LIGHTNING") => Some(PayoutKind::Lightning),
            // A rail this widget cannot name is not on-chain by default.
            _ => None,
        };
        let at = json
            .str(&fmt!("/payouts/{i}/occurred_at"))
            .and_then(|s| date(&s))?;
        let amount_btc = json.f64(&fmt!("/payouts/{i}/amount_btc"))?;
        payouts.push(Payout {
            at,
            amount_btc,
            kind,
        });
    }
    Some(payouts)
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::JsonLookup;
    use std::collections::BTreeMap;

    #[derive(Default)]
    pub(crate) struct MapJson {
        pub(crate) strings: BTreeMap<&'static str, &'static str>,
        pub(crate) ints: BTreeMap<&'static str, i64>,
        pub(crate) floats: BTreeMap<&'static str, f64>,
        pub(crate) bools: BTreeMap<&'static str, bool>,
    }

    impl JsonLookup for MapJson {
        fn str(&self, path: &str) -> Option<String> {
            self.strings.get(path).map(|s| (*s).to_owned())
        }

        fn i64(&self, path: &str) -> Option<i64> {
            self.ints.get(path).copied()
        }

        fn f64(&self, path: &str) -> Option<f64> {
            self.floats.get(path).copied()
        }

        fn bool(&self, path: &str) -> Option<bool> {
            self.bools.get(path).copied()
        }
    }

    /// Fixture date parser: strings of the form "@1234" parse to 1234.
    pub(crate) fn fixture_date(s: &str) -> Option<i64> {
        s.strip_prefix('@').and_then(|n| n.parse().ok())
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::{MapJson, fixture_date};
    use super::*;

    #[test]
    fn windowed_url_encodes_timestamps_and_cursor() {
        let url = Source::HashrateHistory.windowed_page_url(
            "2026-08-02T09:05:00Z",
            "2026-08-02T21:05:00Z",
            Some("abc+/="),
        );
        assert_eq!(
            url,
            format!(
                "https://api.braiins.com/pool/v2/user/hashrate/history\
                 ?from_timestamp=2026-08-02T09%3A05%3A00Z\
                 &to_timestamp=2026-08-02T21%3A05%3A00Z\
                 &page_limit={PAGE_LIMIT}\
                 &page_cursor=abc%2B%2F%3D"
            )
        );
    }

    #[test]
    fn recent_url_is_one_unwindowed_page() {
        assert_eq!(
            Source::PayoutsRecent.recent_page_url(),
            format!("https://api.braiins.com/pool/v2/user/payouts/recent?page_limit={PAGE_LIMIT}")
        );
    }

    #[test]
    fn a_listing_is_complete_without_has_next() {
        let mut json = MapJson::default();
        // A cursor alone says nothing: the reply has to claim another page.
        json.strings.insert("/pagination/next_cursor", "abc");
        assert_eq!(next_page(&json), NextPage::Done);
        json.bools.insert("/pagination/has_next", false);
        assert_eq!(next_page(&json), NextPage::Done);
    }

    #[test]
    fn has_next_with_a_cursor_names_the_page_to_ask_for() {
        let mut json = MapJson::default();
        json.bools.insert("/pagination/has_next", true);
        json.strings.insert("/pagination/next_cursor", "abc");
        assert_eq!(next_page(&json), NextPage::Cursor("abc".to_owned()));
    }

    /// Reading this as `Done` would commit a truncated listing as a whole one.
    #[test]
    fn has_next_without_a_usable_cursor_is_malformed() {
        let mut json = MapJson::default();
        json.bools.insert("/pagination/has_next", true);
        assert_eq!(next_page(&json), NextPage::Malformed);
        json.strings.insert("/pagination/next_cursor", "");
        assert_eq!(next_page(&json), NextPage::Malformed);
    }

    #[test]
    fn history_page_reads_slots_until_the_first_gap() {
        let mut json = MapJson::default();
        json.strings.insert("/from_timestamp", "@100");
        json.strings.insert("/to_timestamp", "@300");
        json.strings.insert("/slots/0/slot_start", "@100");
        json.floats.insert("/slots/0/hashrate_th_per_sec", 1.5);
        json.strings.insert("/slots/1/slot_start", "@200");
        json.floats.insert("/slots/1/hashrate_th_per_sec", 2.5);
        let series = parse_history_page(&json, "hashrate_th_per_sec", &fixture_date)
            .expect("BUG: fixture parses");
        assert_eq!(series.from, Some(100));
        assert_eq!(series.to, Some(300));
        assert_eq!(series.samples.len(), 2);
        assert_eq!(series.samples[1].at, 200);
        assert!((series.samples[1].value - 2.5).abs() < f64::EPSILON);
    }

    #[test]
    fn history_page_with_missing_value_is_unusable() {
        let mut json = MapJson::default();
        json.strings.insert("/slots/0/slot_start", "@100");
        assert!(parse_history_page(&json, "hashrate_th_per_sec", &fixture_date).is_none());
    }

    #[test]
    fn financials_fold_soonest_estimate_and_furthest_progress() {
        let mut json = MapJson::default();
        json.strings
            .insert("/financial_accounts/0/next_payout_at_estimate", "@500");
        json.floats
            .insert("/financial_accounts/0/next_payout_progress_pct", 40.0);
        json.strings
            .insert("/financial_accounts/1/next_payout_at_estimate", "@300");
        json.floats
            .insert("/financial_accounts/1/next_payout_progress_pct", 10.0);
        let folded = parse_financials(&json, &fixture_date);
        assert_eq!(folded.estimate_at, Some(300));
        assert_eq!(folded.progress_pct, Some(40.0));
    }

    #[test]
    fn payouts_keep_only_completed() {
        let mut json = MapJson::default();
        json.strings.insert("/payouts/0/status", "PENDING");
        json.strings.insert("/payouts/0/occurred_at", "@100");
        json.floats.insert("/payouts/0/amount_btc", 0.5);
        json.strings.insert("/payouts/1/status", "COMPLETED");
        json.strings.insert("/payouts/1/type", "LIGHTNING");
        json.strings.insert("/payouts/1/occurred_at", "@200");
        json.floats.insert("/payouts/1/amount_btc", 0.25);
        let payouts = parse_payouts_page(&json, &fixture_date)
            .expect("BUG: a page whose completed payouts read whole must parse");
        assert_eq!(payouts.len(), 1);
        assert_eq!(payouts[0].at, 200);
        assert_eq!(payouts[0].kind, Some(PayoutKind::Lightning));
    }

    /// The rail picks a chart marker; the amount and the time are the payout.
    #[test]
    fn an_unnamed_rail_keeps_the_payout_without_claiming_one() {
        let mut json = MapJson::default();
        json.strings.insert("/payouts/0/status", "COMPLETED");
        json.strings.insert("/payouts/0/type", "SIDECHAIN");
        json.strings.insert("/payouts/0/occurred_at", "@200");
        json.floats.insert("/payouts/0/amount_btc", 0.25);
        let payouts = parse_payouts_page(&json, &fixture_date)
            .expect("BUG: an unknown rail must not void the page");
        assert_eq!(payouts.len(), 1, "an unknown rail must not drop the payout");
        assert_eq!(payouts[0].at, 200);
        assert_eq!(payouts[0].kind, None, "no rail beats the wrong rail");

        json.strings.insert("/payouts/0/type", "ONCHAIN");
        let named = parse_payouts_page(&json, &fixture_date).expect("BUG: a named rail parses");
        assert_eq!(named[0].kind, Some(PayoutKind::Onchain));
    }

    #[test]
    fn a_completed_payout_missing_its_amount_voids_the_page() {
        let mut json = MapJson::default();
        json.strings.insert("/payouts/0/status", "COMPLETED");
        json.strings.insert("/payouts/0/occurred_at", "@200");
        assert_eq!(parse_payouts_page(&json, &fixture_date), None);
        json.floats.insert("/payouts/0/amount_btc", 0.25);
        assert!(parse_payouts_page(&json, &fixture_date).is_some());
    }

    #[test]
    fn an_unreadable_pending_payout_leaves_the_page_whole() {
        let mut json = MapJson::default();
        json.strings.insert("/payouts/0/status", "PENDING");
        json.strings.insert("/payouts/1/status", "COMPLETED");
        json.strings.insert("/payouts/1/occurred_at", "@200");
        json.floats.insert("/payouts/1/amount_btc", 0.25);
        let payouts = parse_payouts_page(&json, &fixture_date)
            .expect("BUG: a skipped status must not void the page");
        assert_eq!(payouts.len(), 1);
    }

    #[test]
    fn workers_current_requires_every_count() {
        let mut json = MapJson::default();
        json.ints.insert("/active_workers", 3);
        json.ints.insert("/low_workers", 1);
        json.ints.insert("/offline_workers", 2);
        assert!(parse_workers_current(&json).is_none());
        json.ints.insert("/disabled_workers", 0);
        let counts = parse_workers_current(&json).expect("BUG: all counts present");
        assert_eq!(counts.active, 3);
        assert_eq!(counts.disabled, 0);
    }
}
