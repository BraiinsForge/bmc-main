// AUTO-GENERATED FROM ../manifest.json by `bmc-widget-codegen` v0.1.0.
// Do not edit by hand. Run `just wasm::gen <widget>` after changing the manifest.

#![allow(
    dead_code,
    reason = "fields are widget-specific; not every key is used by every render path"
)]

use bmc_wasm_sdk::params as snapshot;
use bmc_wasm_sdk::params::typed::ParamRead;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frames {
    Guest,
    Host,
}
impl Frames {
    /// Every variant, in manifest-declaration order. Useful when a widget
    /// wants to render a "pick one" UI or audit the enum exhaustively.
    pub const ALL: &'static [Self] = &[Self::Guest, Self::Host];
    /// Manifest wire value for this variant.
    #[must_use]
    pub fn as_manifest_value(self) -> &'static str {
        match self {
            Self::Guest => "guest",
            Self::Host => "host",
        }
    }
    /// Human-readable label declared in the manifest's `enum_values`.
    #[must_use]
    pub fn as_manifest_label(self) -> &'static str {
        match self {
            Self::Guest => "Guest",
            Self::Host => "Host",
        }
    }
    #[must_use]
    pub fn from_manifest_value(s: &str) -> Option<Self> {
        match s {
            "guest" => Some(Self::Guest),
            "host" => Some(Self::Host),
            _ => None,
        }
    }
}
bmc_wasm_sdk::impl_manifest_str_enum!(Frames);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scene {
    Full,
    Layered,
}
impl Scene {
    /// Every variant, in manifest-declaration order. Useful when a widget
    /// wants to render a "pick one" UI or audit the enum exhaustively.
    pub const ALL: &'static [Self] = &[Self::Full, Self::Layered];
    /// Manifest wire value for this variant.
    #[must_use]
    pub fn as_manifest_value(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Layered => "layered",
        }
    }
    /// Human-readable label declared in the manifest's `enum_values`.
    #[must_use]
    pub fn as_manifest_label(self) -> &'static str {
        match self {
            Self::Full => "Full surface",
            Self::Layered => "Layered",
        }
    }
    #[must_use]
    pub fn from_manifest_value(s: &str) -> Option<Self> {
        match s {
            "full" => Some(Self::Full),
            "layered" => Some(Self::Layered),
            _ => None,
        }
    }
}
bmc_wasm_sdk::impl_manifest_str_enum!(Scene);
#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    pub frames: Frames,
    pub scene: Scene,
}
impl Params {
    /// Materialise a typed snapshot from a dynamic [`snapshot::Params`].
    #[must_use]
    pub fn from_snapshot(snap: &snapshot::Params) -> Self {
        Self {
            frames: <Frames as ParamRead>::read_required(snap, "frames"),
            scene: <Scene as ParamRead>::read_required(snap, "scene"),
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
        if self.frames != other.frames {
            out.push("frames");
        }
        if self.scene != other.scene {
            out.push("scene");
        }
        out
    }
}
