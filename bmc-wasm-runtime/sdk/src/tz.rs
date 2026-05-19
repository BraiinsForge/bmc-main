// Copyright (C) 2026  Braiins Systems s.r.o.

//! IANA timezone identifier for the formatting helpers.
//!
//! Three usage patterns:
//!
//! * **Compile-time-validated static name** — `tz!("America/Los_Angeles")`
//!   expands at macro time to a [`Tz`] borrowing a `&'static str`,
//!   after checking the name against the deck's supported list
//!   (`bmc_shared_time::timezone_variants_raw::TIMEZONE_VARIANTS_RAW`,
//!   itself sourced from openwrt/LuCI's `zoneinfo.uc`).
//!
//! * **Runtime-supplied name** — `Tz::from_runtime(operator_set_name)`
//!   stores the string as-is. No SDK-side validation; the host's
//!   `host_resolve_tz` returns a sentinel offset for unknown names
//!   and format helpers fall back to the system timezone in that case.
//!
//! * **System-tz fallthrough** — pass `None` to the formatting helpers'
//!   `opts.timezone`; they default to `system::current().timezone()`
//!   and the host's pre-applied UTC offset (`SystemTime::utc_offset_secs`).

use std::borrow::Cow;

/// IANA timezone identifier. Constructed via the [`crate::tz!`]
/// proc macro for compile-time-validated static names,
/// or [`Tz::from_runtime`] for operator-set strings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tz(Cow<'static, str>);

impl Tz {
    /// Macro-internal: construct from a static IANA name already
    /// validated at compile time by the `tz!` macro.
    /// Do not call directly.
    #[doc(hidden)]
    #[must_use]
    pub const fn from_static_validated(iana: &'static str) -> Self {
        Self(Cow::Borrowed(iana))
    }

    /// Construct from a runtime IANA name (e.g. an operator-set widget
    /// param). The string is held as-is; the host's `host_resolve_tz`
    /// validates it on first use, and format helpers fall back
    /// to the system timezone for unknown names.
    #[must_use]
    pub fn from_runtime(iana: impl Into<String>) -> Self {
        Self(Cow::Owned(iana.into()))
    }

    /// Borrow the IANA name as a `&str`.
    /// Always returns the exact string the [`Tz`]
    /// was constructed with, no normalization.
    #[must_use]
    pub fn iana(&self) -> &str {
        &self.0
    }
}
