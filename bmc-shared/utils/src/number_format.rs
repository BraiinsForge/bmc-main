// Copyright (C) 2025  Braiins Systems s.r.o.
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

use formato::{FormatOptions, Formato};
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum NumberFormat {
    #[default]
    SpaceGroupCommaDecimal, // 1 234 567,89
    CommaGroupDotDecimal, // 1,234,567.89
    DotGroupCommaDecimal, // 1.234.567,89
    SpaceGroupDotDecimal, // 1 234 567.89
}

impl NumberFormat {
    #[expect(clippy::needless_pass_by_value)]
    pub fn format_number<T: Formato>(self, number: T, precision: usize) -> String {
        // A non-breaking space groups thousands so a number never wraps mid-value in the UI.
        let (group_sep, decimal_sep) = match self {
            NumberFormat::SpaceGroupCommaDecimal => ("\u{00a0}", ","),
            NumberFormat::CommaGroupDotDecimal => (",", "."),
            NumberFormat::DotGroupCommaDecimal => (".", ","),
            NumberFormat::SpaceGroupDotDecimal => ("\u{00a0}", "."),
        };

        let options = FormatOptions::new()
            .with_thousands(group_sep)
            .with_decimal(decimal_sep);

        let pattern = if precision == 0 {
            "#,##0".to_owned()
        } else {
            format!("#,##0.{}", "0".repeat(precision))
        };

        number.formato_ops(&pattern, &options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_space_group_comma_decimal() {
        let number = 1234567.89;
        let format = NumberFormat::SpaceGroupCommaDecimal;
        let result = format.format_number(number, 2);
        assert_eq!(result, "1\u{00a0}234\u{00a0}567,89");
    }

    #[test]
    fn test_comma_group_dot_decimal() {
        let number = 1234567.089;
        let format = NumberFormat::CommaGroupDotDecimal;
        let result = format.format_number(number, 3);
        assert_eq!(result, "1,234,567.089");
    }

    #[test]
    fn test_dot_group_comma_decimal() {
        let number = 1234567.89;
        let format = NumberFormat::DotGroupCommaDecimal;
        let result = format.format_number(number, 2);
        assert_eq!(result, "1.234.567,89");
    }

    #[test]
    fn test_space_group_dot_decimal() {
        let number = 1234567.89;
        let format = NumberFormat::SpaceGroupDotDecimal;
        let result = format.format_number(number, 3);
        assert_eq!(result, "1\u{00a0}234\u{00a0}567.890");

        let number = 0_u64;
        let format = NumberFormat::SpaceGroupDotDecimal;
        let result = format.format_number(number, 2);
        assert_eq!(result, "0.00");

        let number = 1234567_u64;
        let format = NumberFormat::SpaceGroupDotDecimal;
        let result = format.format_number(number, 0);
        assert_eq!(result, "1\u{00a0}234\u{00a0}567");

        let number = 1234567_f64;
        let format = NumberFormat::SpaceGroupDotDecimal;
        let result = format.format_number(number, 1);
        assert_eq!(result, "1\u{00a0}234\u{00a0}567.0");
    }

    #[test]
    fn test_decimal_precision() {
        let number = 0.125672;
        let format = NumberFormat::SpaceGroupDotDecimal;
        let result = format.format_number(number, 5);
        assert_eq!(result, "0.12567");

        let number = 0.125676;
        let format = NumberFormat::SpaceGroupDotDecimal;
        let result = format.format_number(number, 5);
        assert_eq!(result, "0.12568");
    }
}
