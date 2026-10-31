// AUTO-GENERATED FROM ../manifest.json by `bmc-widget-codegen` v0.1.0.
// Do not edit by hand. Run `just wasm::gen <widget>` after changing the manifest.

#![allow(
    dead_code,
    reason = "fields are widget-specific; not every key is used by every render path"
)]

use bmc_wasm_sdk::params as snapshot;
use bmc_wasm_sdk::params::typed::ParamRead;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sizing {
    Contain,
    Cover,
}
impl Sizing {
    /// Every variant, in manifest-declaration order. Useful when a widget
    /// wants to render a "pick one" UI or audit the enum exhaustively.
    pub const ALL: &'static [Self] = &[Self::Contain, Self::Cover];
    /// Manifest wire value for this variant.
    #[must_use]
    pub fn as_manifest_value(self) -> &'static str {
        match self {
            Self::Contain => "contain",
            Self::Cover => "cover",
        }
    }
    /// Human-readable label declared in the manifest's `enum_values`.
    #[must_use]
    pub fn as_manifest_label(self) -> &'static str {
        match self {
            Self::Contain => "Fit — whole photo",
            Self::Cover => "Fill — crop to edges",
        }
    }
    #[must_use]
    pub fn from_manifest_value(s: &str) -> Option<Self> {
        match s {
            "contain" => Some(Self::Contain),
            "cover" => Some(Self::Cover),
            _ => None,
        }
    }
}
bmc_wasm_sdk::impl_manifest_str_enum!(Sizing);
#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    pub label: Option<String>,
    pub refresh_seconds: i32,
    pub server: Option<String>,
    pub sizing: Sizing,
}
impl Params {
    /// Materialise a typed snapshot from a dynamic [`snapshot::Params`].
    #[must_use]
    pub fn from_snapshot(snap: &snapshot::Params) -> Self {
        Self {
            label: <String as ParamRead>::read_optional(snap, "label"),
            refresh_seconds: <i32 as ParamRead>::read_required(snap, "refresh_seconds"),
            server: <String as ParamRead>::read_optional(snap, "server"),
            sizing: <Sizing as ParamRead>::read_required(snap, "sizing"),
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
        if self.label != other.label {
            out.push("label");
        }
        if self.refresh_seconds != other.refresh_seconds {
            out.push("refresh_seconds");
        }
        if self.server != other.server {
            out.push("server");
        }
        if self.sizing != other.sizing {
            out.push("sizing");
        }
        out
    }
}
/// Credential slots this widget declares, one module per slot.
pub mod credentials {
    ///Immich API key — a `generic-token` account. Required — the widget cannot work until an account is bound.
    ///
    ///API key from Immich → Account Settings → API Keys
    pub mod immich {
        ///Placeholder for this slot's `token` field.
        pub const TOKEN: &str = "{{ credential.immich.token }}";
    }
}
