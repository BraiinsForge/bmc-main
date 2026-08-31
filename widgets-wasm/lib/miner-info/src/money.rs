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

//! Fiat figures, which are deliberately *not* modelled as
//! [`bmc_wasm_sdk::units`] quantities.
//!
//! Those convert through physical constants — a metre is a thousandth of a
//! kilometre and always will be. A dollar is not a fixed fraction of a euro, so
//! there is no canonical currency to store and no rate-free `as_eur` to expose.
//! Money is therefore an amount *plus the currency it is in*, carried together
//! so a figure can never be rendered behind the wrong symbol.

/// A currency the public API can quote in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Currency {
    #[default]
    Usd,
}

impl Currency {
    /// Code the public API takes in its `currency` query parameter.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Usd => "usd",
        }
    }

    /// Symbol amounts in this currency render behind.
    #[must_use]
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Usd => "$",
        }
    }
}

/// An amount of money, and which money it is.
///
/// The pair travels together, so the symbol is always read off the value rather
/// than assumed by the caller.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Money {
    pub amount: f64,
    pub currency: Currency,
}

impl Money {
    #[must_use]
    pub const fn new(amount: f64, currency: Currency) -> Self {
        Self { amount, currency }
    }
}

/// Mining revenue density in money: what one terahash per second earns in a
/// day. The fiat twin of [`bmc_wasm_sdk::units::Hashvalue`], which is a real
/// quantity because satoshis per bitcoin is a constant.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hashprice {
    pub per_terahash_day: Money,
}

impl Hashprice {
    /// Denominator appended when rendered; the currency symbol leads the value,
    /// so it is not part of this.
    pub const UNIT: &'static str = "TH/Day";

    #[must_use]
    pub const fn new(amount: f64, currency: Currency) -> Self {
        Self {
            per_terahash_day: Money::new(amount, currency),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The query and the rendered symbol are two views of one value, so they
    /// cannot be changed apart.
    #[test]
    fn a_currency_knows_both_its_query_code_and_its_symbol() {
        assert_eq!(Currency::Usd.code(), "usd");
        assert_eq!(Currency::Usd.symbol(), "$");
    }

    #[test]
    fn an_amount_carries_the_currency_it_is_in() {
        let price = Money::new(101_754.0, Currency::Usd);
        assert_eq!(price.currency.symbol(), "$");
    }
}
