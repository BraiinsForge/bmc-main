// Copyright (C) 2025  Braiins Systems s.r.o.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub enum NumberFormat {
    #[default]
    SpaceGroupCommaDecimal, // 1 234 567,89
    CommaGroupDotDecimal, // 1,234,567.89
    DotGroupCommaDecimal, // 1.234.567,89
    SpaceGroupDotDecimal, // 1 234 567.89
}

impl NumberFormat {
    pub fn format_number<T: FormatNumberValue + Copy>(self, number: T) -> String {
        let (group_sep, decimal_sep) = match self {
            NumberFormat::SpaceGroupCommaDecimal => (" ", ","),
            NumberFormat::CommaGroupDotDecimal => (",", "."),
            NumberFormat::DotGroupCommaDecimal => (".", ","),
            NumberFormat::SpaceGroupDotDecimal => (" ", "."),
        };

        let is_negative = number.is_negative();

        let (int_part, frac_part) = number.split_parts();

        let mut int_str = int_part.to_string();
        let mut grouped = String::new();

        // Add grouping separators
        while int_str.len() > 3 {
            let split_point = int_str.len() - 3;

            #[expect(clippy::string_slice)]
            let chunk = &int_str[split_point..];

            grouped = format!("{group_sep}{chunk}{grouped}");
            int_str.truncate(split_point);
        }

        grouped = format!("{int_str}{grouped}");

        if is_negative {
            grouped = format!("-{grouped}");
        }

        match frac_part {
            Some(f) => format!("{grouped}{decimal_sep}{f:02}"),
            None => grouped,
        }
    }
}

pub trait FormatNumberValue {
    fn split_parts(self) -> (u64, Option<u8>);
    #[expect(clippy::wrong_self_convention)]
    fn is_negative(self) -> bool;
}

impl FormatNumberValue for f64 {
    fn split_parts(self) -> (u64, Option<u8>) {
        #[expect(clippy::cast_sign_loss)]
        #[expect(clippy::cast_possible_truncation)]
        let int_part = self.trunc().abs() as u64;

        #[expect(clippy::cast_sign_loss)]
        #[expect(clippy::cast_possible_truncation)]
        let frac_part = ((self.abs().fract() * 100.0).round()) as u8;

        (int_part, Some(frac_part))
    }

    fn is_negative(self) -> bool {
        self.is_sign_negative()
    }
}

impl FormatNumberValue for u64 {
    fn split_parts(self) -> (u64, Option<u8>) {
        (self, None)
    }

    fn is_negative(self) -> bool {
        false
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_space_group_comma_decimal() {
        let number = 1234567.89;
        let format = NumberFormat::SpaceGroupCommaDecimal;
        let result = format.format_number(number);
        assert_eq!(result, "1 234 567,89");
    }

    #[test]
    fn test_comma_group_dot_decimal() {
        let number = 1234567.89;
        let format = NumberFormat::CommaGroupDotDecimal;
        let result = format.format_number(number);
        assert_eq!(result, "1,234,567.89");
    }

    #[test]
    fn test_dot_group_comma_decimal() {
        let number = 1234567.89;
        let format = NumberFormat::DotGroupCommaDecimal;
        let result = format.format_number(number);
        assert_eq!(result, "1.234.567,89");
    }

    #[test]
    fn test_space_group_dot_decimal() {
        let number = 1234567.89;
        let format = NumberFormat::SpaceGroupDotDecimal;
        let result = format.format_number(number);
        assert_eq!(result, "1 234 567.89");

        let number = 0_u64;
        let format = NumberFormat::SpaceGroupDotDecimal;
        let result = format.format_number(number);
        assert_eq!(result, "0");

        let number = 1234567_u64;
        let format = NumberFormat::SpaceGroupDotDecimal;
        let result = format.format_number(number);
        assert_eq!(result, "1 234 567");
    }
}
