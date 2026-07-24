// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

#[derive(Debug, Clone, PartialEq)]
pub struct MinerModel {
    pub id: String,
    pub name: String,
    pub chip_type: Option<String>,
    pub chip_count: Option<usize>,
    pub nominal_hashrate_ths: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModelAccumulator {
    pub id: Option<String>,
    pub name: Option<String>,
    pub chip_type: Option<String>,
    pub chip_count: Option<usize>,
}

/// Nameplate hashrate (TH/s) for models whose API omits a nominal (today uBOS).
/// Keyed on product name; a miss leaves the model nominal-less, so `is_ok`
/// falls back to the floor.
fn catalog_nominal_ths(name: &str) -> Option<f32> {
    match name {
        // 4× BM1370.
        "Braiins Forge Miner x4" => Some(4.8),
        _ => None,
    }
}

impl ModelAccumulator {
    /// Build a `MinerModel` once both id (platform slug) and product name are
    /// set, else `None`; the catalog supplies the nominal for API-less families.
    #[must_use]
    pub fn into_model(self) -> Option<MinerModel> {
        let (Some(id), Some(name)) = (self.id, self.name) else {
            return None;
        };
        let nominal_hashrate_ths = catalog_nominal_ths(&name);
        Some(MinerModel {
            id,
            name,
            chip_type: self.chip_type,
            chip_count: self.chip_count,
            nominal_hashrate_ths,
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
            .expect("BUG: id and name present builds a model");
        assert_eq!(model.id, "stm32mp157c-ii2-bmm1");
        assert_eq!(model.name, "BMM 101");
        assert_eq!(model.chip_type.as_deref(), Some("BM1370"));
        assert_eq!(model.chip_count, Some(152));
        assert_eq!(model.nominal_hashrate_ths, None);
    }

    #[test]
    fn into_model_stamps_catalog_nominal_for_a_known_ubos_model() {
        let acc = ModelAccumulator {
            id: Some("Braiins Forge Miner x4".to_owned()),
            name: Some("Braiins Forge Miner x4".to_owned()),
            ..ModelAccumulator::default()
        };
        let model = acc
            .into_model()
            .expect("BUG: id and name present builds a model");
        assert_eq!(model.nominal_hashrate_ths, Some(4.8));
    }

    #[test]
    fn into_model_leaves_an_uncataloged_model_without_a_nominal() {
        let acc = ModelAccumulator {
            id: Some("mystery".to_owned()),
            name: Some("Mystery Miner".to_owned()),
            ..ModelAccumulator::default()
        };
        let model = acc
            .into_model()
            .expect("BUG: id and name present builds a model");
        assert_eq!(model.nominal_hashrate_ths, None);
    }
}
