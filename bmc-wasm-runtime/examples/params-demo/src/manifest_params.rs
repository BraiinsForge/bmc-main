// AUTO-GENERATED FROM ../manifest.json by `bmc-widget-codegen` v0.1.0.
// Do not edit by hand. Run `just wasm::gen <widget>` after changing the manifest.

#![allow(
    dead_code,
    reason = "fields are widget-specific; not every key is used by every render path"
)]

use bmc_wasm_sdk::params as snapshot;
use bmc_wasm_sdk::params::typed::ParamRead;
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DoubleEnum {
    Linear,
    Mac,
    SRgb,
}
impl DoubleEnum {
    /// Every variant, in manifest-declaration order. Useful when a widget
    /// wants to render a "pick one" UI or audit the enum exhaustively.
    pub const ALL: &'static [Self] = &[Self::Linear, Self::Mac, Self::SRgb];
    /// Manifest wire value for this variant.
    #[must_use]
    pub fn as_manifest_value(self) -> f64 {
        match self {
            Self::Linear => 1.0,
            Self::Mac => 1.8,
            Self::SRgb => 2.2,
        }
    }
    /// Human-readable label declared in the manifest's `enum_values`.
    #[must_use]
    pub fn as_manifest_label(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Mac => "Mac",
            Self::SRgb => "sRGB",
        }
    }
    #[must_use]
    pub fn from_manifest_value(v: f64) -> Option<Self> {
        if (v - 1.0).abs() < f64::EPSILON {
            return Some(Self::Linear);
        }
        if (v - 1.8).abs() < f64::EPSILON {
            return Some(Self::Mac);
        }
        if (v - 2.2).abs() < f64::EPSILON {
            return Some(Self::SRgb);
        }
        None
    }
}
bmc_wasm_sdk::impl_manifest_f64_enum!(DoubleEnum);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntegerEnum {
    One,
    Two,
    Three,
    Four,
}
impl IntegerEnum {
    /// Every variant, in manifest-declaration order. Useful when a widget
    /// wants to render a "pick one" UI or audit the enum exhaustively.
    pub const ALL: &'static [Self] = &[Self::One, Self::Two, Self::Three, Self::Four];
    /// Manifest wire value for this variant.
    #[must_use]
    pub fn as_manifest_value(self) -> i32 {
        match self {
            Self::One => 1,
            Self::Two => 2,
            Self::Three => 3,
            Self::Four => 4,
        }
    }
    /// Human-readable label declared in the manifest's `enum_values`.
    #[must_use]
    pub fn as_manifest_label(self) -> &'static str {
        match self {
            Self::One => "One",
            Self::Two => "Two",
            Self::Three => "Three",
            Self::Four => "Four",
        }
    }
    #[must_use]
    pub fn from_manifest_value(v: i32) -> Option<Self> {
        match v {
            1 => Some(Self::One),
            2 => Some(Self::Two),
            3 => Some(Self::Three),
            4 => Some(Self::Four),
            _ => None,
        }
    }
}
bmc_wasm_sdk::impl_manifest_i32_enum!(IntegerEnum);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringEnum {
    Violet,
    Green,
    Amber,
}
impl StringEnum {
    /// Every variant, in manifest-declaration order. Useful when a widget
    /// wants to render a "pick one" UI or audit the enum exhaustively.
    pub const ALL: &'static [Self] = &[Self::Violet, Self::Green, Self::Amber];
    /// Manifest wire value for this variant.
    #[must_use]
    pub fn as_manifest_value(self) -> &'static str {
        match self {
            Self::Violet => "violet",
            Self::Green => "green",
            Self::Amber => "amber",
        }
    }
    /// Human-readable label declared in the manifest's `enum_values`.
    #[must_use]
    pub fn as_manifest_label(self) -> &'static str {
        match self {
            Self::Violet => "Violet",
            Self::Green => "Green",
            Self::Amber => "Amber",
        }
    }
    #[must_use]
    pub fn from_manifest_value(s: &str) -> Option<Self> {
        match s {
            "violet" => Some(Self::Violet),
            "green" => Some(Self::Green),
            "amber" => Some(Self::Amber),
            _ => None,
        }
    }
}
bmc_wasm_sdk::impl_manifest_str_enum!(StringEnum);
#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    pub boolean_flag: bool,
    pub double_enum: DoubleEnum,
    pub double_range: f64,
    pub free_string: String,
    pub integer_enum: IntegerEnum,
    pub integer_range: i32,
    pub optional_boolean: Option<bool>,
    pub optional_double: Option<f64>,
    pub optional_integer: Option<i32>,
    pub optional_string: Option<String>,
    pub string_date: String,
    pub string_enum: StringEnum,
    pub string_uri: String,
    pub tz: String,
}
impl Params {
    /// Materialise a typed snapshot from a dynamic [`snapshot::Params`].
    #[must_use]
    pub fn from_snapshot(snap: &snapshot::Params) -> Self {
        Self {
            boolean_flag: <bool as ParamRead>::read_required(snap, "boolean_flag"),
            double_enum: <DoubleEnum as ParamRead>::read_required(snap, "double_enum"),
            double_range: <f64 as ParamRead>::read_required(snap, "double_range"),
            free_string: <String as ParamRead>::read_required(snap, "free_string"),
            integer_enum: <IntegerEnum as ParamRead>::read_required(snap, "integer_enum"),
            integer_range: <i32 as ParamRead>::read_required(snap, "integer_range"),
            optional_boolean: <bool as ParamRead>::read_optional(snap, "optional_boolean"),
            optional_double: <f64 as ParamRead>::read_optional(snap, "optional_double"),
            optional_integer: <i32 as ParamRead>::read_optional(snap, "optional_integer"),
            optional_string: <String as ParamRead>::read_optional(snap, "optional_string"),
            string_date: <String as ParamRead>::read_required(snap, "string_date"),
            string_enum: <StringEnum as ParamRead>::read_required(snap, "string_enum"),
            string_uri: <String as ParamRead>::read_required(snap, "string_uri"),
            tz: <String as ParamRead>::read_required(snap, "tz"),
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
        if self.boolean_flag != other.boolean_flag {
            out.push("boolean_flag");
        }
        if self.double_enum != other.double_enum {
            out.push("double_enum");
        }
        if self.double_range != other.double_range {
            out.push("double_range");
        }
        if self.free_string != other.free_string {
            out.push("free_string");
        }
        if self.integer_enum != other.integer_enum {
            out.push("integer_enum");
        }
        if self.integer_range != other.integer_range {
            out.push("integer_range");
        }
        if self.optional_boolean != other.optional_boolean {
            out.push("optional_boolean");
        }
        if self.optional_double != other.optional_double {
            out.push("optional_double");
        }
        if self.optional_integer != other.optional_integer {
            out.push("optional_integer");
        }
        if self.optional_string != other.optional_string {
            out.push("optional_string");
        }
        if self.string_date != other.string_date {
            out.push("string_date");
        }
        if self.string_enum != other.string_enum {
            out.push("string_enum");
        }
        if self.string_uri != other.string_uri {
            out.push("string_uri");
        }
        if self.tz != other.tz {
            out.push("tz");
        }
        out
    }
}
/// Credential slots this widget declares, one module per slot.
pub mod credentials {
    ///Media server — a `generic-userpass` account. Required — the widget cannot work until an account is bound.
    ///
    ///Two-field type — exercises more than one placeholder per slot
    pub mod media {
        ///Placeholder for this slot's `password` field.
        pub const PASSWORD: &str = "{{ credential.media.password }}";
        ///Placeholder for this slot's `username` field.
        pub const USERNAME: &str = "{{ credential.media.username }}";
    }
    ///Pool account — a `braiins-pool` account. Required — the widget cannot work until an account is bound.
    ///
    ///Egress-pinned type — its secret may only be sent to api.braiins.com
    pub mod pool {
        ///Placeholder for this slot's `token` field.
        pub const TOKEN: &str = "{{ credential.pool.token }}";
    }
    ///Backup pool account — a `braiins-pool` account. Required — the widget cannot work until an account is bound.
    ///
    ///A second slot of one type — a widget may hold more than one account of it
    pub mod pool_backup {
        ///Placeholder for this slot's `token` field.
        pub const TOKEN: &str = "{{ credential.pool_backup.token }}";
    }
    ///Weather service — a `generic-token` account. Required — the widget cannot work until an account is bound.
    ///
    ///Single-token type, no egress pin
    pub mod weather {
        ///Placeholder for this slot's `token` field.
        pub const TOKEN: &str = "{{ credential.weather.token }}";
    }
}
