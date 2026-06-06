// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::model::MinerModel;

#[must_use]
pub fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_ascii_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

#[must_use]
pub fn matches_any(model: &MinerModel, entries: &[String]) -> bool {
    let name = normalize(&model.name);
    let id = normalize(&model.id);
    entries.iter().any(|entry| {
        let entry = normalize(entry);
        name.contains(&entry) || id.contains(&entry)
    })
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Filters {
    pub whitelist: Vec<String>,
    pub blacklist: Vec<String>,
}

impl Filters {
    #[must_use]
    pub fn is_visible(&self, model: Option<&MinerModel>) -> bool {
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
    fn normalize_lowercases_and_strips_whitespace() {
        assert_eq!(
            normalize("Braiins Mini Miner BMM 101"),
            "braiinsminiminerbmm101"
        );
        assert_eq!(normalize("  NerdQAxe++ \t"), "nerdqaxe++");
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
            blacklist: Vec::new(),
        };
        let bmm = model("id", "Braiins Mini Miner BMM 101");
        let axe = model("id", "NerdQAxe++");
        assert!(f.is_visible(Some(&bmm)));
        assert!(
            !f.is_visible(Some(&axe)),
            "a non-whitelisted model is hidden"
        );
    }

    #[test]
    fn blacklist_hides_matching_models() {
        let f = Filters {
            whitelist: Vec::new(),
            blacklist: vec!["nerdqaxe".to_owned()],
        };
        let bmm = model("id", "Braiins Mini Miner BMM 101");
        let axe = model("id", "NerdQAxe++");
        assert!(f.is_visible(Some(&bmm)));
        assert!(!f.is_visible(Some(&axe)));
    }

    #[test]
    fn both_empty_shows_everything() {
        let f = Filters::default();
        let bmm = model("id", "Braiins Mini Miner BMM 101");
        assert!(f.is_visible(Some(&bmm)));
    }

    #[test]
    fn unresolved_model_is_always_visible() {
        let f = Filters {
            whitelist: vec!["bmm101".to_owned()],
            blacklist: vec!["bmm101".to_owned()],
        };
        assert!(
            f.is_visible(None),
            "a device whose model is unknown cannot be filtered out"
        );
    }

    #[test]
    fn blacklist_overrides_whitelist_for_the_same_model() {
        let f = Filters {
            whitelist: vec!["bmm".to_owned()],
            blacklist: vec!["bmm101".to_owned()],
        };
        let bmm = model("id", "Braiins Mini Miner BMM 101");
        assert!(
            !f.is_visible(Some(&bmm)),
            "a whitelisted but also blacklisted model is hidden"
        );
    }
}
