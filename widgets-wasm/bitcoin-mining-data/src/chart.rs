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

#[must_use]
pub fn series_points(values: &[f64], width: f32, height: f32, inset: f32) -> Vec<(f32, f32)> {
    match values {
        [] => return Vec::new(),
        [_] => {
            return vec![
                (inset, height / 2.0),
                ((width - inset).max(inset), height / 2.0),
            ];
        }
        _ => {}
    }
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let range = max - min;
    let drawable_width = (width - inset * 2.0).max(0.0);
    let drawable_height = (height - inset * 2.0).max(0.0);
    let last = values.len().saturating_sub(1);
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            #[expect(
                clippy::cast_precision_loss,
                reason = "chart samples and dimensions fit comfortably in f32"
            )]
            let x = inset + drawable_width * index as f32 / last as f32;
            let y = if range > 0.0 {
                let normalized = (*value - min) / range;
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "normalized chart coordinates are bounded before conversion"
                )]
                let normalized = normalized as f32;
                inset + drawable_height * (1.0 - normalized)
            } else {
                f32::midpoint(inset, height - inset)
            };
            (x, y)
        })
        .collect()
}

#[must_use]
pub fn is_rising(values: &[f64]) -> bool {
    matches!((values.first(), values.last()), (Some(first), Some(last)) if last >= first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_series_inside_the_requested_box() {
        let points = series_points(&[10.0, 20.0, 15.0], 100.0, 50.0, 2.0);
        assert_eq!(points.first(), Some(&(2.0, 48.0)));
        assert_eq!(points.get(1), Some(&(50.0, 2.0)));
        assert_eq!(points.last(), Some(&(98.0, 25.0)));
    }

    #[test]
    fn maps_one_sample_to_a_visible_flat_line() {
        assert_eq!(
            series_points(&[10.0], 100.0, 50.0, 2.0),
            [(2.0, 25.0), (98.0, 25.0)]
        );
    }

    #[test]
    fn maps_multi_sample_flat_series_to_the_vertical_midpoint() {
        assert_eq!(
            series_points(&[5.0, 5.0, 5.0], 100.0, 50.0, 2.0),
            [(2.0, 25.0), (50.0, 25.0), (98.0, 25.0)]
        );
    }
}
