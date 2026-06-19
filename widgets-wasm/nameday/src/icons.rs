// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::manifest_params::Country;

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

const FLAG_AT: Svg = include_svg!("assets/flags/at.svg");
const FLAG_CZ: Svg = include_svg!("assets/flags/cz.svg");
const FLAG_DE: Svg = include_svg!("assets/flags/de.svg");
const FLAG_DK: Svg = include_svg!("assets/flags/dk.svg");
const FLAG_EE: Svg = include_svg!("assets/flags/ee.svg");
const FLAG_ES: Svg = include_svg!("assets/flags/es.svg");
const FLAG_FI: Svg = include_svg!("assets/flags/fi.svg");
const FLAG_FR: Svg = include_svg!("assets/flags/fr.svg");
const FLAG_HR: Svg = include_svg!("assets/flags/hr.svg");
const FLAG_HU: Svg = include_svg!("assets/flags/hu.svg");
const FLAG_IT: Svg = include_svg!("assets/flags/it.svg");
const FLAG_LT: Svg = include_svg!("assets/flags/lt.svg");
const FLAG_LV: Svg = include_svg!("assets/flags/lv.svg");
const FLAG_PL: Svg = include_svg!("assets/flags/pl.svg");
const FLAG_SE: Svg = include_svg!("assets/flags/se.svg");
const FLAG_SK: Svg = include_svg!("assets/flags/sk.svg");
const FLAG_US: Svg = include_svg!("assets/flags/us.svg");

#[must_use]
pub fn get_flag_svg(country_code: Country) -> &'static Svg {
    match country_code {
        Country::At => &FLAG_AT,
        Country::Cz => &FLAG_CZ,
        Country::De => &FLAG_DE,
        Country::Dk => &FLAG_DK,
        Country::Ee => &FLAG_EE,
        Country::Es => &FLAG_ES,
        Country::Fi => &FLAG_FI,
        Country::Fr => &FLAG_FR,
        Country::Hr => &FLAG_HR,
        Country::Hu => &FLAG_HU,
        Country::It => &FLAG_IT,
        Country::Lt => &FLAG_LT,
        Country::Lv => &FLAG_LV,
        Country::Pl => &FLAG_PL,
        Country::Se => &FLAG_SE,
        Country::Sk => &FLAG_SK,
        Country::Us => &FLAG_US,
    }
}
