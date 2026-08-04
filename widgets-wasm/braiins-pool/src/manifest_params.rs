// AUTO-GENERATED FROM ../manifest.json by `bmc-widget-codegen` v0.1.0.
// Do not edit by hand. Run `just wasm::gen <widget>` after changing the manifest.

#![allow(
    dead_code,
    reason = "fields are widget-specific; not every key is used by every render path"
)]

use bmc_wasm_sdk::params as snapshot;
use bmc_wasm_sdk::params::typed::ParamRead;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChartFrame {
    Hours4,
    Hours12,
    Hours24,
    Days7,
}
impl ChartFrame {
    /// Every variant, in manifest-declaration order. Useful when a widget
    /// wants to render a "pick one" UI or audit the enum exhaustively.
    pub const ALL: &'static [Self] = &[Self::Hours4, Self::Hours12, Self::Hours24, Self::Days7];
    /// Manifest wire value for this variant.
    #[must_use]
    pub fn as_manifest_value(self) -> &'static str {
        match self {
            Self::Hours4 => "hours_4",
            Self::Hours12 => "hours_12",
            Self::Hours24 => "hours_24",
            Self::Days7 => "days_7",
        }
    }
    /// Human-readable label declared in the manifest's `enum_values`.
    #[must_use]
    pub fn as_manifest_label(self) -> &'static str {
        match self {
            Self::Hours4 => "4 hours",
            Self::Hours12 => "12 hours",
            Self::Hours24 => "24 hours",
            Self::Days7 => "7 days",
        }
    }
    #[must_use]
    pub fn from_manifest_value(s: &str) -> Option<Self> {
        match s {
            "hours_4" => Some(Self::Hours4),
            "hours_12" => Some(Self::Hours12),
            "hours_24" => Some(Self::Hours24),
            "days_7" => Some(Self::Days7),
            _ => None,
        }
    }
}
bmc_wasm_sdk::impl_manifest_str_enum!(ChartFrame);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Overview,
    BigChart,
}
impl Style {
    /// Every variant, in manifest-declaration order. Useful when a widget
    /// wants to render a "pick one" UI or audit the enum exhaustively.
    pub const ALL: &'static [Self] = &[Self::Overview, Self::BigChart];
    /// Manifest wire value for this variant.
    #[must_use]
    pub fn as_manifest_value(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::BigChart => "big_chart",
        }
    }
    /// Human-readable label declared in the manifest's `enum_values`.
    #[must_use]
    pub fn as_manifest_label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::BigChart => "Big Chart",
        }
    }
    #[must_use]
    pub fn from_manifest_value(s: &str) -> Option<Self> {
        match s {
            "overview" => Some(Self::Overview),
            "big_chart" => Some(Self::BigChart),
            _ => None,
        }
    }
}
bmc_wasm_sdk::impl_manifest_str_enum!(Style);
#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    pub chart_frame: ChartFrame,
    pub style: Style,
    pub worker_states: bool,
}
impl Params {
    /// Materialise a typed snapshot from a dynamic [`snapshot::Params`].
    #[must_use]
    pub fn from_snapshot(snap: &snapshot::Params) -> Self {
        Self {
            chart_frame: <ChartFrame as ParamRead>::read_required(snap, "chart_frame"),
            style: <Style as ParamRead>::read_required(snap, "style"),
            worker_states: <bool as ParamRead>::read_required(snap, "worker_states"),
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
        if self.chart_frame != other.chart_frame {
            out.push("chart_frame");
        }
        if self.style != other.style {
            out.push("style");
        }
        if self.worker_states != other.worker_states {
            out.push("worker_states");
        }
        out
    }
}
/// Credential slots this widget declares, one module per slot.
pub mod credentials {
    ///Pool account — a `braiins-pool` account. Required — the widget cannot work until an account is bound.
    ///
    ///Used to fetch your hashrate, worker, and payout stats from Braiins Pool
    pub mod pool {
        ///Placeholder for this slot's `token` field.
        pub const TOKEN: &str = "{{ credential.pool.token }}";
    }
}
