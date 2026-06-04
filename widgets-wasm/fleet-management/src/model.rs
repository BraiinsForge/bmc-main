// Copyright (C) 2026  Braiins Systems s.r.o.

#[derive(Debug, Clone, PartialEq)]
pub struct MinerModel {
    pub id: String,
    pub name: String,
    pub chip_type: Option<String>,
    pub chip_count: Option<u32>,
    pub nominal_hashrate_ths: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModelAccumulator {
    pub id: Option<String>,
    pub name: Option<String>,
    pub chip_type: Option<String>,
    pub chip_count: Option<u32>,
}

impl ModelAccumulator {
    /// Build a `MinerModel` only once both the platform slug and product name
    /// are set; return `None` if either is absent. The remaining hardware
    /// fields ride along.
    #[must_use]
    pub fn into_model(self) -> Option<MinerModel> {
        let (Some(id), Some(name)) = (self.id, self.name) else {
            return None;
        };
        Some(MinerModel {
            id,
            name,
            chip_type: self.chip_type,
            chip_count: self.chip_count,
            nominal_hashrate_ths: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulator_missing_id_or_name_yields_no_model() {
        let no_name = ModelAccumulator {
            id: Some("am2-s17".to_owned()),
            chip_count: Some(76),
            ..ModelAccumulator::default()
        };
        assert_eq!(no_name.into_model(), None);

        let no_id = ModelAccumulator {
            name: Some("BMM 101".to_owned()),
            ..ModelAccumulator::default()
        };
        assert_eq!(no_id.into_model(), None);
        assert_eq!(ModelAccumulator::default().into_model(), None);
    }

    #[test]
    fn accumulator_with_id_and_name_builds_a_model_carrying_every_field() {
        let acc = ModelAccumulator {
            id: Some("stm32mp157c-ii2-bmm1".to_owned()),
            name: Some("BMM 101".to_owned()),
            chip_type: Some("BM1370".to_owned()),
            chip_count: Some(152),
        };
        let model = acc
            .into_model()
            .expect("id and name present builds a model");
        assert_eq!(model.id, "stm32mp157c-ii2-bmm1");
        assert_eq!(model.name, "BMM 101");
        assert_eq!(model.chip_type.as_deref(), Some("BM1370"));
        assert_eq!(model.chip_count, Some(152));
        assert_eq!(model.nominal_hashrate_ths, None);
    }
}
