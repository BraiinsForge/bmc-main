// AUTO-GENERATED FROM ../manifest.json by `bmc-widget-codegen` v0.1.0.
// Do not edit by hand. Run `just wasm::gen <widget>` after changing the manifest.

#![allow(
    dead_code,
    reason = "fields are widget-specific; not every key is used by every render path"
)]

use bmc_wasm_sdk::params as snapshot;
use bmc_wasm_sdk::params::typed::ParamRead;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Period {
    _1h,
    _3h,
    _6h,
    _12h,
    _1d,
    _3d,
    _7d,
    _14d,
    _1mo,
    _3mo,
    _6mo,
    _9mo,
    _1y,
    _2y,
    _3y,
    _5y,
    _10y,
    _25y,
    Full,
}
impl Period {
    /// Every variant, in manifest-declaration order. Useful when a widget
    /// wants to render a "pick one" UI or audit the enum exhaustively.
    pub const ALL: &'static [Self] = &[
        Self::_1h,
        Self::_3h,
        Self::_6h,
        Self::_12h,
        Self::_1d,
        Self::_3d,
        Self::_7d,
        Self::_14d,
        Self::_1mo,
        Self::_3mo,
        Self::_6mo,
        Self::_9mo,
        Self::_1y,
        Self::_2y,
        Self::_3y,
        Self::_5y,
        Self::_10y,
        Self::_25y,
        Self::Full,
    ];
    /// Manifest wire value for this variant.
    #[must_use]
    pub fn as_manifest_value(self) -> &'static str {
        match self {
            Self::_1h => "1h",
            Self::_3h => "3h",
            Self::_6h => "6h",
            Self::_12h => "12h",
            Self::_1d => "1d",
            Self::_3d => "3d",
            Self::_7d => "7d",
            Self::_14d => "14d",
            Self::_1mo => "1mo",
            Self::_3mo => "3mo",
            Self::_6mo => "6mo",
            Self::_9mo => "9mo",
            Self::_1y => "1Y",
            Self::_2y => "2Y",
            Self::_3y => "3Y",
            Self::_5y => "5Y",
            Self::_10y => "10Y",
            Self::_25y => "25Y",
            Self::Full => "full",
        }
    }
    /// Human-readable label declared in the manifest's `enum_values`.
    #[must_use]
    pub fn as_manifest_label(self) -> &'static str {
        match self {
            Self::_1h => "1 Hour",
            Self::_3h => "3 Hours",
            Self::_6h => "6 Hours",
            Self::_12h => "12 Hours",
            Self::_1d => "1 Day",
            Self::_3d => "3 Days",
            Self::_7d => "7 Days",
            Self::_14d => "14 Days",
            Self::_1mo => "1 Month",
            Self::_3mo => "3 Months",
            Self::_6mo => "6 Months",
            Self::_9mo => "9 Months",
            Self::_1y => "1 Year",
            Self::_2y => "2 Years",
            Self::_3y => "3 Years",
            Self::_5y => "5 Years",
            Self::_10y => "10 Years",
            Self::_25y => "25 Years",
            Self::Full => "All Time",
        }
    }
    #[must_use]
    pub fn from_manifest_value(s: &str) -> Option<Self> {
        match s {
            "1h" => Some(Self::_1h),
            "3h" => Some(Self::_3h),
            "6h" => Some(Self::_6h),
            "12h" => Some(Self::_12h),
            "1d" => Some(Self::_1d),
            "3d" => Some(Self::_3d),
            "7d" => Some(Self::_7d),
            "14d" => Some(Self::_14d),
            "1mo" => Some(Self::_1mo),
            "3mo" => Some(Self::_3mo),
            "6mo" => Some(Self::_6mo),
            "9mo" => Some(Self::_9mo),
            "1Y" => Some(Self::_1y),
            "2Y" => Some(Self::_2y),
            "3Y" => Some(Self::_3y),
            "5Y" => Some(Self::_5y),
            "10Y" => Some(Self::_10y),
            "25Y" => Some(Self::_25y),
            "full" => Some(Self::Full),
            _ => None,
        }
    }
}
bmc_wasm_sdk::impl_manifest_str_enum!(Period);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Sparkline,
    Candlestick,
}
impl View {
    /// Every variant, in manifest-declaration order. Useful when a widget
    /// wants to render a "pick one" UI or audit the enum exhaustively.
    pub const ALL: &'static [Self] = &[Self::Sparkline, Self::Candlestick];
    /// Manifest wire value for this variant.
    #[must_use]
    pub fn as_manifest_value(self) -> &'static str {
        match self {
            Self::Sparkline => "sparkline",
            Self::Candlestick => "candlestick",
        }
    }
    /// Human-readable label declared in the manifest's `enum_values`.
    #[must_use]
    pub fn as_manifest_label(self) -> &'static str {
        match self {
            Self::Sparkline => "Sparkline",
            Self::Candlestick => "Candlestick",
        }
    }
    #[must_use]
    pub fn from_manifest_value(s: &str) -> Option<Self> {
        match s {
            "sparkline" => Some(Self::Sparkline),
            "candlestick" => Some(Self::Candlestick),
            _ => None,
        }
    }
}
bmc_wasm_sdk::impl_manifest_str_enum!(View);
#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    pub pair: Option<String>,
    pub period: Period,
    pub view: View,
}
impl Params {
    /// Materialise a typed snapshot from a dynamic [`snapshot::Params`].
    #[must_use]
    pub fn from_snapshot(snap: &snapshot::Params) -> Self {
        Self {
            pair: <String as ParamRead>::read_optional(snap, "pair"),
            period: <Period as ParamRead>::read_required(snap, "period"),
            view: <View as ParamRead>::read_required(snap, "view"),
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
        if self.pair != other.pair {
            out.push("pair");
        }
        if self.period != other.period {
            out.push("period");
        }
        if self.view != other.view {
            out.push("view");
        }
        out
    }
}
