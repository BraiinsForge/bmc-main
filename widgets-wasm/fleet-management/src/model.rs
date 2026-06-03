// Copyright (C) 2026  Braiins Systems s.r.o.

#[derive(Debug, Clone, PartialEq)]
pub struct MinerModel {
    pub id: String,
    pub name: String,
    pub chip_type: Option<String>,
    pub chip_count: Option<u32>,
    pub nominal_hashrate_ths: Option<f32>,
}

impl MinerModel {
    #[must_use]
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            chip_type: None,
            chip_count: None,
            nominal_hashrate_ths: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_model_leaves_hardware_specifics_none() {
        let m = MinerModel::new("bmm-101", "BMM 101");
        assert_eq!(m.id, "bmm-101");
        assert_eq!(m.name, "BMM 101");
        assert_eq!(m.chip_type, None);
        assert_eq!(m.chip_count, None);
        assert_eq!(m.nominal_hashrate_ths, None);
    }
}
