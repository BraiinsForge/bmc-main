// AUTO-GENERATED FROM ../manifest.json by `bmc-widget-codegen` v0.1.0.
// Do not edit by hand. Run `just wasm::gen <widget>` after changing the manifest.

#![allow(
    dead_code,
    reason = "fields are widget-specific; not every key is used by every render path"
)]

use bmc_wasm_sdk::params as snapshot;
use bmc_wasm_sdk::params::typed::ParamRead;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoldSeconds {
    Off,
    _1S,
    _2S,
    _3S,
    _5S,
}
impl HoldSeconds {
    /// Every variant, in manifest-declaration order. Useful when a widget
    /// wants to render a "pick one" UI or audit the enum exhaustively.
    pub const ALL: &'static [Self] = &[Self::Off, Self::_1S, Self::_2S, Self::_3S, Self::_5S];
    /// Manifest wire value for this variant.
    #[must_use]
    pub fn as_manifest_value(self) -> i32 {
        match self {
            Self::Off => 0,
            Self::_1S => 1,
            Self::_2S => 2,
            Self::_3S => 3,
            Self::_5S => 5,
        }
    }
    /// Human-readable label declared in the manifest's `enum_values`.
    #[must_use]
    pub fn as_manifest_label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::_1S => "1 s",
            Self::_2S => "2 s",
            Self::_3S => "3 s",
            Self::_5S => "5 s",
        }
    }
    #[must_use]
    pub fn from_manifest_value(v: i32) -> Option<Self> {
        match v {
            0 => Some(Self::Off),
            1 => Some(Self::_1S),
            2 => Some(Self::_2S),
            3 => Some(Self::_3S),
            5 => Some(Self::_5S),
            _ => None,
        }
    }
}
bmc_wasm_sdk::impl_manifest_i32_enum!(HoldSeconds);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapAction {
    Toggle,
    None,
}
impl TapAction {
    /// Every variant, in manifest-declaration order. Useful when a widget
    /// wants to render a "pick one" UI or audit the enum exhaustively.
    pub const ALL: &'static [Self] = &[Self::Toggle, Self::None];
    /// Manifest wire value for this variant.
    #[must_use]
    pub fn as_manifest_value(self) -> &'static str {
        match self {
            Self::Toggle => "toggle",
            Self::None => "none",
        }
    }
    /// Human-readable label declared in the manifest's `enum_values`.
    #[must_use]
    pub fn as_manifest_label(self) -> &'static str {
        match self {
            Self::Toggle => "Toggle",
            Self::None => "Nothing",
        }
    }
    #[must_use]
    pub fn from_manifest_value(s: &str) -> Option<Self> {
        match s {
            "toggle" => Some(Self::Toggle),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}
bmc_wasm_sdk::impl_manifest_str_enum!(TapAction);
#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    pub entity_id: String,
    pub hold_seconds: HoldSeconds,
    pub name: Option<String>,
    pub tap_action: TapAction,
}
impl Params {
    /// Materialise a typed snapshot from a dynamic [`snapshot::Params`].
    #[must_use]
    pub fn from_snapshot(snap: &snapshot::Params) -> Self {
        Self {
            entity_id: <String as ParamRead>::read_required(snap, "entity_id"),
            hold_seconds: <HoldSeconds as ParamRead>::read_required(snap, "hold_seconds"),
            name: <String as ParamRead>::read_optional(snap, "name"),
            tap_action: <TapAction as ParamRead>::read_required(snap, "tap_action"),
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
        if self.entity_id != other.entity_id {
            out.push("entity_id");
        }
        if self.hold_seconds != other.hold_seconds {
            out.push("hold_seconds");
        }
        if self.name != other.name {
            out.push("name");
        }
        if self.tap_action != other.tap_action {
            out.push("tap_action");
        }
        out
    }
}
/// Credential slots this widget declares, one module per slot.
pub mod credentials {
    ///Home Assistant — a `home-assistant` account. Required — the widget cannot work until an account is bound.
    ///
    ///Add your Home Assistant under Connected Accounts first, then pick it here.
    pub mod ha {
        ///Placeholder for this slot's `token` field.
        pub const TOKEN: &str = "{{ credential.ha.token }}";
        ///Placeholder for this slot's `url` field.
        pub const URL: &str = "{{ credential.ha.url }}";
        ///Placeholder for this slot's `websocket_url` field.
        pub const WEBSOCKET_URL: &str = "{{ credential.ha.websocket_url }}";
    }
}
