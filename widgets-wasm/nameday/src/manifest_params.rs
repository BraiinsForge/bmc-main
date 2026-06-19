// Copyright (C) 2026  Braiins Systems s.r.o.

// AUTO-GENERATED FROM ../manifest.json by `bmc-widget-codegen` v0.1.0.
// Do not edit by hand. Run `just wasm::gen <widget>` after changing the manifest.

#![allow(
    dead_code,
    reason = "fields are widget-specific; not every key is used by every render path"
)]

use bmc_wasm_sdk::params as snapshot;
use bmc_wasm_sdk::params::typed::ParamRead;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Country {
    At,
    Cz,
    De,
    Dk,
    Ee,
    Es,
    Fi,
    Fr,
    Hr,
    Hu,
    It,
    Lt,
    Lv,
    Pl,
    Se,
    Sk,
    Us,
}
impl Country {
    /// Every variant, in manifest-declaration order. Useful when a widget
    /// wants to render a "pick one" UI or audit the enum exhaustively.
    pub const ALL: &'static [Self] = &[
        Self::At,
        Self::Cz,
        Self::De,
        Self::Dk,
        Self::Ee,
        Self::Es,
        Self::Fi,
        Self::Fr,
        Self::Hr,
        Self::Hu,
        Self::It,
        Self::Lt,
        Self::Lv,
        Self::Pl,
        Self::Se,
        Self::Sk,
        Self::Us,
    ];
    /// Manifest wire value for this variant.
    #[must_use]
    pub fn as_manifest_value(self) -> &'static str {
        match self {
            Self::At => "at",
            Self::Cz => "cz",
            Self::De => "de",
            Self::Dk => "dk",
            Self::Ee => "ee",
            Self::Es => "es",
            Self::Fi => "fi",
            Self::Fr => "fr",
            Self::Hr => "hr",
            Self::Hu => "hu",
            Self::It => "it",
            Self::Lt => "lt",
            Self::Lv => "lv",
            Self::Pl => "pl",
            Self::Se => "se",
            Self::Sk => "sk",
            Self::Us => "us",
        }
    }
    /// Human-readable label declared in the manifest's `enum_values`.
    #[must_use]
    pub fn as_manifest_label(self) -> &'static str {
        match self {
            Self::At => "Austria",
            Self::Cz => "Czechia",
            Self::De => "Germany",
            Self::Dk => "Denmark",
            Self::Ee => "Estonia",
            Self::Es => "Spain",
            Self::Fi => "Finland",
            Self::Fr => "France",
            Self::Hr => "Croatia",
            Self::Hu => "Hungary",
            Self::It => "Italy",
            Self::Lt => "Lithuania",
            Self::Lv => "Latvia",
            Self::Pl => "Poland",
            Self::Se => "Sweden",
            Self::Sk => "Slovakia",
            Self::Us => "United States",
        }
    }
    #[must_use]
    pub fn from_manifest_value(s: &str) -> Option<Self> {
        match s {
            "at" => Some(Self::At),
            "cz" => Some(Self::Cz),
            "de" => Some(Self::De),
            "dk" => Some(Self::Dk),
            "ee" => Some(Self::Ee),
            "es" => Some(Self::Es),
            "fi" => Some(Self::Fi),
            "fr" => Some(Self::Fr),
            "hr" => Some(Self::Hr),
            "hu" => Some(Self::Hu),
            "it" => Some(Self::It),
            "lt" => Some(Self::Lt),
            "lv" => Some(Self::Lv),
            "pl" => Some(Self::Pl),
            "se" => Some(Self::Se),
            "sk" => Some(Self::Sk),
            "us" => Some(Self::Us),
            _ => None,
        }
    }
}
bmc_wasm_sdk::impl_manifest_str_enum!(Country);
#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    pub country: Country,
    pub show_date: bool,
}
impl Params {
    /// Materialise a typed snapshot from a dynamic [`snapshot::Params`].
    #[must_use]
    pub fn from_snapshot(snap: &snapshot::Params) -> Self {
        Self {
            country: <Country as ParamRead>::read_required(snap, "country"),
            show_date: <bool as ParamRead>::read_required(snap, "show_date"),
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
        if self.country != other.country {
            out.push("country");
        }
        if self.show_date != other.show_date {
            out.push("show_date");
        }
        out
    }
}
