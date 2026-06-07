// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::device::DeviceFamily;
use crate::model::MinerModel;

#[must_use]
pub fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_ascii_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Normalize a list of operator-supplied fragments, dropping any that are blank
/// after normalization. A blank fragment is a substring of every name, so it
/// would make a whitelist match everything (benign) but a blacklist hide the
/// whole fleet — a stray comma in `["bmm101", ""]` must not blank the view.
#[must_use]
pub fn normalized_fragments<'a>(entries: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    entries
        .into_iter()
        .map(normalize)
        .filter(|f| !f.is_empty())
        .collect()
}

/// True if any `entries` fragment is a substring of the model's normalized name
/// or id. `entries` must already be normalized — they are normalized once when
/// [`Filters`] is built, not per call, because `summarize` (hence this) runs
/// per telemetry event on a fleet that can be hundreds of devices.
#[must_use]
pub fn matches_any(model: &MinerModel, entries: &[String]) -> bool {
    let name = normalize(&model.name);
    let id = normalize(&model.id);
    entries
        .iter()
        .any(|entry| name.contains(entry.as_str()) || id.contains(entry.as_str()))
}

/// The manifest param key carrying a family's enable/disable toggle. The one
/// place the family↔key mapping lives, shared by the render filter and the
/// driver's enable/disable handling.
#[must_use]
pub fn family_enabled_key(family: DeviceFamily) -> &'static str {
    match family {
        DeviceFamily::Bos => "bos_enabled",
        DeviceFamily::Ubos => "ubos_enabled",
        DeviceFamily::Bitaxe => "axeos_enabled",
    }
}

/// `whitelist`/`blacklist` hold model-name fragments already run through
/// [`normalize`]; build them with [`normalize`] once (see the render filter
/// build) so `matches_any` need not re-normalize per device.
#[derive(Debug, Clone, PartialEq)]
pub struct Filters {
    pub whitelist: Vec<String>,
    pub blacklist: Vec<String>,
    pub bos_enabled: bool,
    pub ubos_enabled: bool,
    pub axeos_enabled: bool,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            whitelist: Vec::new(),
            blacklist: Vec::new(),
            bos_enabled: true,
            ubos_enabled: true,
            axeos_enabled: true,
        }
    }
}

impl Filters {
    fn family_enabled(&self, family: DeviceFamily) -> bool {
        match family {
            DeviceFamily::Bos => self.bos_enabled,
            DeviceFamily::Ubos => self.ubos_enabled,
            DeviceFamily::Bitaxe => self.axeos_enabled,
        }
    }

    #[must_use]
    pub fn is_visible(&self, family: DeviceFamily, model: Option<&MinerModel>) -> bool {
        if !self.family_enabled(family) {
            return false;
        }
        let Some(model) = model else {
            return true;
        };
        if !self.whitelist.is_empty() && !matches_any(model, &self.whitelist) {
            return false;
        }
        !matches_any(model, &self.blacklist)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str, name: &str) -> MinerModel {
        MinerModel {
            id: id.to_owned(),
            name: name.to_owned(),
            chip_type: None,
            chip_count: None,
            nominal_hashrate_ths: None,
        }
    }

    #[test]
    fn family_enabled_key_maps_each_family_to_its_manifest_param() {
        assert_eq!(family_enabled_key(DeviceFamily::Bos), "bos_enabled");
        assert_eq!(family_enabled_key(DeviceFamily::Ubos), "ubos_enabled");
        assert_eq!(family_enabled_key(DeviceFamily::Bitaxe), "axeos_enabled");
    }

    #[test]
    fn normalize_lowercases_and_strips_whitespace() {
        assert_eq!(
            normalize("Braiins Mini Miner BMM 101"),
            "braiinsminiminerbmm101"
        );
        assert_eq!(normalize("  NerdQAxe++ \t"), "nerdqaxe++");
    }

    #[test]
    fn normalized_fragments_drops_blank_entries() {
        // A stray empty entry (e.g. a trailing comma in `["bmm101", ""]`) must
        // not survive, or it would hide every device through the blacklist.
        let fragments = normalized_fragments(["bmm101", "", "  ", "NerdQAxe"]);
        assert_eq!(fragments, vec!["bmm101".to_owned(), "nerdqaxe".to_owned()]);
    }

    #[test]
    fn blacklist_with_only_a_blank_fragment_hides_nothing() {
        let f = Filters {
            blacklist: normalized_fragments(["bmm101", ""]),
            ..Default::default()
        };
        let axe = model("id", "NerdQAxe++");
        assert!(
            f.is_visible(DeviceFamily::Bitaxe, Some(&axe)),
            "a blank blacklist fragment must not hide unrelated models"
        );
    }

    #[test]
    fn matches_any_finds_substring_in_name() {
        let m = model("stm32mp157c-ii2-bmm1", "Braiins Mini Miner BMM 101");
        assert!(
            matches_any(&m, &["bmm101".to_owned()]),
            "bmm101 must match Braiins Mini Miner BMM 101 by name"
        );
    }

    #[test]
    fn matches_any_finds_substring_in_id() {
        let m = model("stm32mp157c-ii2-bmm1", "Braiins Mini Miner");
        assert!(
            matches_any(&m, &["ii2".to_owned()]),
            "an entry matching the id must count as a match"
        );
    }

    #[test]
    fn matches_any_rejects_an_unrelated_fragment() {
        let m = model("stm32mp157c-ii2-bmm1", "Braiins Mini Miner BMM 101");
        assert!(!matches_any(&m, &["nerdqaxe".to_owned()]));
    }

    #[test]
    fn whitelist_non_empty_restricts_to_matching_models() {
        let f = Filters {
            whitelist: vec!["bmm101".to_owned()],
            ..Default::default()
        };
        let bmm = model("id", "Braiins Mini Miner BMM 101");
        let axe = model("id", "NerdQAxe++");
        assert!(f.is_visible(DeviceFamily::Bos, Some(&bmm)));
        assert!(
            !f.is_visible(DeviceFamily::Bos, Some(&axe)),
            "a non-whitelisted model is hidden"
        );
    }

    #[test]
    fn blacklist_hides_matching_models() {
        let f = Filters {
            blacklist: vec!["nerdqaxe".to_owned()],
            ..Default::default()
        };
        let bmm = model("id", "Braiins Mini Miner BMM 101");
        let axe = model("id", "NerdQAxe++");
        assert!(f.is_visible(DeviceFamily::Bos, Some(&bmm)));
        assert!(!f.is_visible(DeviceFamily::Bitaxe, Some(&axe)));
    }

    #[test]
    fn both_empty_shows_everything() {
        let f = Filters::default();
        let bmm = model("id", "Braiins Mini Miner BMM 101");
        assert!(f.is_visible(DeviceFamily::Bos, Some(&bmm)));
    }

    #[test]
    fn unresolved_model_is_always_visible() {
        let f = Filters {
            whitelist: vec!["bmm101".to_owned()],
            blacklist: vec!["bmm101".to_owned()],
            ..Default::default()
        };
        assert!(
            f.is_visible(DeviceFamily::Bos, None),
            "a device whose model is unknown cannot be filtered out"
        );
    }

    #[test]
    fn blacklist_overrides_whitelist_for_the_same_model() {
        let f = Filters {
            whitelist: vec!["bmm".to_owned()],
            blacklist: vec!["bmm101".to_owned()],
            ..Default::default()
        };
        let bmm = model("id", "Braiins Mini Miner BMM 101");
        assert!(
            !f.is_visible(DeviceFamily::Bos, Some(&bmm)),
            "a whitelisted but also blacklisted model is hidden"
        );
    }

    #[test]
    fn default_enables_every_family() {
        let f = Filters::default();
        let bmm = model("id", "Braiins Mini Miner BMM 101");
        assert!(f.is_visible(DeviceFamily::Bos, Some(&bmm)));
        assert!(f.is_visible(DeviceFamily::Ubos, Some(&bmm)));
        assert!(f.is_visible(DeviceFamily::Bitaxe, Some(&bmm)));
        assert!(
            f.is_visible(DeviceFamily::Bos, None),
            "an unknown-model device of an enabled family is visible"
        );
    }

    #[test]
    fn disabled_family_hides_its_devices_regardless_of_model_lists() {
        // A disabled family hides its devices even when the model would pass
        // the whitelist and dodge the blacklist.
        let f = Filters {
            whitelist: vec!["bmm101".to_owned()],
            bos_enabled: false,
            ..Default::default()
        };
        let bmm = model("id", "Braiins Mini Miner BMM 101");
        assert!(
            !f.is_visible(DeviceFamily::Bos, Some(&bmm)),
            "a disabled family hides a whitelisted model"
        );
        assert!(
            !f.is_visible(DeviceFamily::Bos, None),
            "a disabled family hides even unknown-model devices"
        );
        assert!(
            f.is_visible(DeviceFamily::Bitaxe, Some(&bmm)),
            "another enabled family is unaffected"
        );
    }

    #[test]
    fn enabled_family_still_respects_whitelist_and_blacklist() {
        let f = Filters {
            whitelist: vec!["bmm101".to_owned()],
            blacklist: vec!["nerdqaxe".to_owned()],
            ..Default::default()
        };
        let bmm = model("id", "Braiins Mini Miner BMM 101");
        let axe = model("id", "NerdQAxe++");
        assert!(f.is_visible(DeviceFamily::Bos, Some(&bmm)));
        assert!(
            !f.is_visible(DeviceFamily::Bos, Some(&axe)),
            "a non-whitelisted model stays hidden inside an enabled family"
        );
    }
}
