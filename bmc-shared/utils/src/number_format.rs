// Copyright (C) 2025  Braiins Systems s.r.o.

use formato::{FormatOptions, Formato};
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Serialize, Deserialize, Default)]
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
        let (group_sep, decimal_sep) = match self {
            NumberFormat::SpaceGroupCommaDecimal => (" ", ","),
            NumberFormat::CommaGroupDotDecimal => (",", "."),
            NumberFormat::DotGroupCommaDecimal => (".", ","),
            NumberFormat::SpaceGroupDotDecimal => (" ", "."),
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
        assert_eq!(result, "1 234 567,89");
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
        assert_eq!(result, "1 234 567.890");

        let number = 0_u64;
        let format = NumberFormat::SpaceGroupDotDecimal;
        let result = format.format_number(number, 2);
        assert_eq!(result, "0.00");

        let number = 1234567_u64;
        let format = NumberFormat::SpaceGroupDotDecimal;
        let result = format.format_number(number, 0);
        assert_eq!(result, "1 234 567");

        let number = 1234567_f64;
        let format = NumberFormat::SpaceGroupDotDecimal;
        let result = format.format_number(number, 1);
        assert_eq!(result, "1 234 567.0");
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
