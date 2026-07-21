// AUTO-GENERATED FROM ../manifest.json by `bmc-widget-codegen` v0.1.0.
// Do not edit by hand. Run `just wasm::gen <widget>` after changing the manifest.

#![allow(
    dead_code,
    reason = "fields are widget-specific; not every key is used by every render path"
)]

use bmc_wasm_sdk::params as snapshot;
use bmc_wasm_sdk::params::typed::ParamRead;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChartSpanMinutes {
    _15Minutes,
    _1Hour,
    _6Hours,
    _24Hours,
}
impl ChartSpanMinutes {
    /// Every variant, in manifest-declaration order. Useful when a widget
    /// wants to render a "pick one" UI or audit the enum exhaustively.
    pub const ALL: &'static [Self] = &[
        Self::_15Minutes,
        Self::_1Hour,
        Self::_6Hours,
        Self::_24Hours,
    ];
    /// Manifest wire value for this variant.
    #[must_use]
    pub fn as_manifest_value(self) -> i32 {
        match self {
            Self::_15Minutes => 15,
            Self::_1Hour => 60,
            Self::_6Hours => 360,
            Self::_24Hours => 1440,
        }
    }
    /// Human-readable label declared in the manifest's `enum_values`.
    #[must_use]
    pub fn as_manifest_label(self) -> &'static str {
        match self {
            Self::_15Minutes => "15 minutes",
            Self::_1Hour => "1 hour",
            Self::_6Hours => "6 hours",
            Self::_24Hours => "24 hours",
        }
    }
    #[must_use]
    pub fn from_manifest_value(v: i32) -> Option<Self> {
        match v {
            15 => Some(Self::_15Minutes),
            60 => Some(Self::_1Hour),
            360 => Some(Self::_6Hours),
            1440 => Some(Self::_24Hours),
            _ => None,
        }
    }
}
bmc_wasm_sdk::impl_manifest_i32_enum!(ChartSpanMinutes);
#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    pub axeos_enabled: bool,
    pub bos_password: String,
    pub chart_span_minutes: ChartSpanMinutes,
    pub fleet_name: String,
    pub ubos_password: String,
    pub ubos_username: String,
}
impl Params {
    /// Materialise a typed snapshot from a dynamic [`snapshot::Params`].
    #[must_use]
    pub fn from_snapshot(snap: &snapshot::Params) -> Self {
        Self {
            axeos_enabled: <bool as ParamRead>::read_required(snap, "axeos_enabled"),
            bos_password: <String as ParamRead>::read_required(snap, "bos_password"),
            chart_span_minutes: <ChartSpanMinutes as ParamRead>::read_required(
                snap,
                "chart_span_minutes",
            ),
            fleet_name: <String as ParamRead>::read_required(snap, "fleet_name"),
            ubos_password: <String as ParamRead>::read_required(snap, "ubos_password"),
            ubos_username: <String as ParamRead>::read_required(snap, "ubos_username"),
        }
    }
    /// Latest typed snapshot delivered for this widget instance.
    /// Cached per-thread; only re-parses when `snapshot::version()` changes
    /// since the last call.
    #[must_use]
    pub fn current() -> Self {
        thread_local! {
            static CACHE : core::cell::RefCell < Option < (u64, Params) >> = const {
            core::cell::RefCell::new(None) };
        }
        let v = snapshot::version();
        CACHE.with(|cell| {
            let mut cache = cell.borrow_mut();
            if let Some((cv, ref params)) = *cache
                && cv == v
            {
                return params.clone();
            }
            let fresh = Self::from_snapshot(&snapshot::current());
            *cache = Some((v, fresh.clone()));
            fresh
        })
    }
    /// Snapshot delivered immediately before [`current`]; `None` until at
    /// least one update has been observed (i.e. during `init` and the
    /// first `render`).
    #[must_use]
    pub fn previous() -> Option<Self> {
        let prev = snapshot::previous();
        if prev.is_empty() {
            None
        } else {
            Some(Self::from_snapshot(&prev))
        }
    }
    /// Manifest keys whose value differs between `self` and `other`.
    ///
    /// Intended for `on_params_update` diffing — pass `current()` and the
    /// inside-hook value of `previous()` to get the set of keys to react
    /// to. Field-by-field `PartialEq`; emitted in struct-field order so
    /// the result is deterministic.
    #[must_use]
    pub fn changed_keys(&self, other: &Self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.axeos_enabled != other.axeos_enabled {
            out.push("axeos_enabled");
        }
        if self.bos_password != other.bos_password {
            out.push("bos_password");
        }
        if self.chart_span_minutes != other.chart_span_minutes {
            out.push("chart_span_minutes");
        }
        if self.fleet_name != other.fleet_name {
            out.push("fleet_name");
        }
        if self.ubos_password != other.ubos_password {
            out.push("ubos_password");
        }
        if self.ubos_username != other.ubos_username {
            out.push("ubos_username");
        }
        out
    }
}
