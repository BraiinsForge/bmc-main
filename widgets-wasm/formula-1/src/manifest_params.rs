// AUTO-GENERATED FROM ../manifest.json by `bmc-widget-codegen` v0.1.0.
// Do not edit by hand. Run `just wasm::gen <widget>` after changing the manifest.

#![allow(
    dead_code,
    reason = "fields are widget-specific; not every key is used by every render path"
)]

use bmc_wasm_sdk::params as snapshot;
use bmc_wasm_sdk::params::typed::ParamRead;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Driver {
    Gasly,
    Colapinto,
    Alonso,
    Stroll,
    Hulkenberg,
    Bortoleto,
    Bottas,
    Perez,
    Leclerc,
    Hamilton,
    Ocon,
    Bearman,
    Norris,
    Piastri,
    Russell,
    Antonelli,
    MaxVerstappen,
    Hadjar,
    Lawson,
    ArvidLindblad,
    Albon,
    Sainz,
}
impl Driver {
    /// Every variant, in manifest-declaration order. Useful when a widget
    /// wants to render a "pick one" UI or audit the enum exhaustively.
    pub const ALL: &'static [Self] = &[
        Self::Gasly,
        Self::Colapinto,
        Self::Alonso,
        Self::Stroll,
        Self::Hulkenberg,
        Self::Bortoleto,
        Self::Bottas,
        Self::Perez,
        Self::Leclerc,
        Self::Hamilton,
        Self::Ocon,
        Self::Bearman,
        Self::Norris,
        Self::Piastri,
        Self::Russell,
        Self::Antonelli,
        Self::MaxVerstappen,
        Self::Hadjar,
        Self::Lawson,
        Self::ArvidLindblad,
        Self::Albon,
        Self::Sainz,
    ];
    /// Manifest wire value for this variant.
    #[must_use]
    pub fn as_manifest_value(self) -> &'static str {
        match self {
            Self::Gasly => "gasly",
            Self::Colapinto => "colapinto",
            Self::Alonso => "alonso",
            Self::Stroll => "stroll",
            Self::Hulkenberg => "hulkenberg",
            Self::Bortoleto => "bortoleto",
            Self::Bottas => "bottas",
            Self::Perez => "perez",
            Self::Leclerc => "leclerc",
            Self::Hamilton => "hamilton",
            Self::Ocon => "ocon",
            Self::Bearman => "bearman",
            Self::Norris => "norris",
            Self::Piastri => "piastri",
            Self::Russell => "russell",
            Self::Antonelli => "antonelli",
            Self::MaxVerstappen => "max_verstappen",
            Self::Hadjar => "hadjar",
            Self::Lawson => "lawson",
            Self::ArvidLindblad => "arvid_lindblad",
            Self::Albon => "albon",
            Self::Sainz => "sainz",
        }
    }
    /// Human-readable label declared in the manifest's `enum_values`.
    #[must_use]
    pub fn as_manifest_label(self) -> &'static str {
        match self {
            Self::Gasly => "ALP — Pierre Gasly",
            Self::Colapinto => "ALP — Franco Colapinto",
            Self::Alonso => "AST — Fernando Alonso",
            Self::Stroll => "AST — Lance Stroll",
            Self::Hulkenberg => "AUD — Nico Hulkenberg",
            Self::Bortoleto => "AUD — Gabriel Bortoleto",
            Self::Bottas => "CAD — Valtteri Bottas",
            Self::Perez => "CAD — Sergio Perez",
            Self::Leclerc => "FER — Charles Leclerc",
            Self::Hamilton => "FER — Lewis Hamilton",
            Self::Ocon => "HAA — Esteban Ocon",
            Self::Bearman => "HAA — Oliver Bearman",
            Self::Norris => "MCL — Lando Norris",
            Self::Piastri => "MCL — Oscar Piastri",
            Self::Russell => "MER — George Russell",
            Self::Antonelli => "MER — Kimi Antonelli",
            Self::MaxVerstappen => "RBR — Max Verstappen",
            Self::Hadjar => "RBR — Isack Hadjar",
            Self::Lawson => "RCB — Liam Lawson",
            Self::ArvidLindblad => "RCB — Arvid Lindblad",
            Self::Albon => "WIL — Alexander Albon",
            Self::Sainz => "WIL — Carlos Sainz",
        }
    }
    #[must_use]
    pub fn from_manifest_value(s: &str) -> Option<Self> {
        match s {
            "gasly" => Some(Self::Gasly),
            "colapinto" => Some(Self::Colapinto),
            "alonso" => Some(Self::Alonso),
            "stroll" => Some(Self::Stroll),
            "hulkenberg" => Some(Self::Hulkenberg),
            "bortoleto" => Some(Self::Bortoleto),
            "bottas" => Some(Self::Bottas),
            "perez" => Some(Self::Perez),
            "leclerc" => Some(Self::Leclerc),
            "hamilton" => Some(Self::Hamilton),
            "ocon" => Some(Self::Ocon),
            "bearman" => Some(Self::Bearman),
            "norris" => Some(Self::Norris),
            "piastri" => Some(Self::Piastri),
            "russell" => Some(Self::Russell),
            "antonelli" => Some(Self::Antonelli),
            "max_verstappen" => Some(Self::MaxVerstappen),
            "hadjar" => Some(Self::Hadjar),
            "lawson" => Some(Self::Lawson),
            "arvid_lindblad" => Some(Self::ArvidLindblad),
            "albon" => Some(Self::Albon),
            "sainz" => Some(Self::Sainz),
            _ => None,
        }
    }
}
bmc_wasm_sdk::impl_manifest_str_enum!(Driver);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Auto,
    NextRace,
    Standings,
    Driver,
}
impl View {
    /// Every variant, in manifest-declaration order. Useful when a widget
    /// wants to render a "pick one" UI or audit the enum exhaustively.
    pub const ALL: &'static [Self] = &[Self::Auto, Self::NextRace, Self::Standings, Self::Driver];
    /// Manifest wire value for this variant.
    #[must_use]
    pub fn as_manifest_value(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::NextRace => "next_race",
            Self::Standings => "standings",
            Self::Driver => "driver",
        }
    }
    /// Human-readable label declared in the manifest's `enum_values`.
    #[must_use]
    pub fn as_manifest_label(self) -> &'static str {
        match self {
            Self::Auto => "Automatic (live session, next race, or standings)",
            Self::NextRace => "Next Race",
            Self::Standings => "Driver Standings",
            Self::Driver => "Driver Statistics",
        }
    }
    #[must_use]
    pub fn from_manifest_value(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(Self::Auto),
            "next_race" => Some(Self::NextRace),
            "standings" => Some(Self::Standings),
            "driver" => Some(Self::Driver),
            _ => None,
        }
    }
}
bmc_wasm_sdk::impl_manifest_str_enum!(View);
#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    pub driver: Driver,
    pub local_time: bool,
    pub view: View,
}
impl Params {
    /// Materialise a typed snapshot from a dynamic [`snapshot::Params`].
    #[must_use]
    pub fn from_snapshot(snap: &snapshot::Params) -> Self {
        Self {
            driver: <Driver as ParamRead>::read_required(snap, "driver"),
            local_time: <bool as ParamRead>::read_required(snap, "local_time"),
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
        if self.driver != other.driver {
            out.push("driver");
        }
        if self.local_time != other.local_time {
            out.push("local_time");
        }
        if self.view != other.view {
            out.push("view");
        }
        out
    }
}
