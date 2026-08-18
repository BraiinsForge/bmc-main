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
    _1d,
    _7d,
    _1mo,
}
impl Period {
    /// Every variant, in manifest-declaration order. Useful when a widget
    /// wants to render a "pick one" UI or audit the enum exhaustively.
    pub const ALL: &'static [Self] = &[Self::_1h, Self::_1d, Self::_7d, Self::_1mo];
    /// Manifest wire value for this variant.
    #[must_use]
    pub fn as_manifest_value(self) -> &'static str {
        match self {
            Self::_1h => "1h",
            Self::_1d => "1d",
            Self::_7d => "7d",
            Self::_1mo => "1mo",
        }
    }
    /// Human-readable label declared in the manifest's `enum_values`.
    #[must_use]
    pub fn as_manifest_label(self) -> &'static str {
        match self {
            Self::_1h => "1 Hour",
            Self::_1d => "1 Day",
            Self::_7d => "7 Days",
            Self::_1mo => "1 Month",
        }
    }
    #[must_use]
    pub fn from_manifest_value(s: &str) -> Option<Self> {
        match s {
            "1h" => Some(Self::_1h),
            "1d" => Some(Self::_1d),
            "7d" => Some(Self::_7d),
            "1mo" => Some(Self::_1mo),
            _ => None,
        }
    }
}
bmc_wasm_sdk::impl_manifest_str_enum!(Period);
#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    pub period: Period,
    pub symbol_1: Option<String>,
    pub symbol_2: Option<String>,
    pub symbol_3: Option<String>,
    pub symbol_4: Option<String>,
    pub symbol_5: Option<String>,
    pub symbol_6: Option<String>,
    pub symbol_7: Option<String>,
    pub symbol_8: Option<String>,
}
impl Params {
    /// Materialise a typed snapshot from a dynamic [`snapshot::Params`].
    #[must_use]
    pub fn from_snapshot(snap: &snapshot::Params) -> Self {
        Self {
            period: <Period as ParamRead>::read_required(snap, "period"),
            symbol_1: <String as ParamRead>::read_optional(snap, "symbol_1"),
            symbol_2: <String as ParamRead>::read_optional(snap, "symbol_2"),
            symbol_3: <String as ParamRead>::read_optional(snap, "symbol_3"),
            symbol_4: <String as ParamRead>::read_optional(snap, "symbol_4"),
            symbol_5: <String as ParamRead>::read_optional(snap, "symbol_5"),
            symbol_6: <String as ParamRead>::read_optional(snap, "symbol_6"),
            symbol_7: <String as ParamRead>::read_optional(snap, "symbol_7"),
            symbol_8: <String as ParamRead>::read_optional(snap, "symbol_8"),
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
        if self.period != other.period {
            out.push("period");
        }
        if self.symbol_1 != other.symbol_1 {
            out.push("symbol_1");
        }
        if self.symbol_2 != other.symbol_2 {
            out.push("symbol_2");
        }
        if self.symbol_3 != other.symbol_3 {
            out.push("symbol_3");
        }
        if self.symbol_4 != other.symbol_4 {
            out.push("symbol_4");
        }
        if self.symbol_5 != other.symbol_5 {
            out.push("symbol_5");
        }
        if self.symbol_6 != other.symbol_6 {
            out.push("symbol_6");
        }
        if self.symbol_7 != other.symbol_7 {
            out.push("symbol_7");
        }
        if self.symbol_8 != other.symbol_8 {
            out.push("symbol_8");
        }
        out
    }
}
