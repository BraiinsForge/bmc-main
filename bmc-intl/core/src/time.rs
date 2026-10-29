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

//! Relative times and counts of time units, worded by CLDR's patterns:
//! declined as each language needs, as in Czech "před 5 hodinami" against "za 5 hodin".

use std::sync::{Arc, LazyLock, OnceLock};

use fixed_decimal::{Decimal, Sign};
use icu_decimal::DecimalFormatter;
use icu_decimal::options::GroupingStrategy;
use icu_experimental::relativetime::options::Numeric;
use icu_experimental::relativetime::{RelativeTimeFormatter, RelativeTimeFormatterOptions};
use icu_locale_core::LanguageIdentifier;
use icu_plurals::{PluralCategory, PluralRules};
use icu_provider::DataError;

use crate::Language;
use crate::data::{self, Data};
use crate::units::{self, Plural};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TimeUnit {
    Second,
    Minute,
    Hour,
    Day,
}

/// How much of a unit's name a relative time writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TimeWidth {
    /// `5m ago`.
    Narrow,
    /// `5 minutes ago`, Czech `před 5 minutami`.
    Long,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Past,
    Future,
}

/// As a message writes a number value: grouping waits for the device's number preference.
const UNGROUPED: GroupingStrategy = GroupingStrategy::Never;

impl TimeUnit {
    const fn cldr(self) -> &'static str {
        match self {
            Self::Second => "duration-second",
            Self::Minute => "duration-minute",
            Self::Hour => "duration-hour",
            Self::Day => "duration-day",
        }
    }
}

#[derive(Default)]
struct PerUnit<T> {
    second: T,
    minute: T,
    hour: T,
    day: T,
}

impl<T> PerUnit<T> {
    const fn get(&self, unit: TimeUnit) -> &T {
        match unit {
            TimeUnit::Second => &self.second,
            TimeUnit::Minute => &self.minute,
            TimeUnit::Hour => &self.hour,
            TimeUnit::Day => &self.day,
        }
    }
}

#[derive(Default)]
struct PerWidth<T> {
    narrow: T,
    long: T,
}

impl<T> PerWidth<T> {
    const fn get(&self, width: TimeWidth) -> &T {
        match width {
            TimeWidth::Narrow => &self.narrow,
            TimeWidth::Long => &self.long,
        }
    }
}

type RelativeFormatters = PerWidth<PerUnit<OnceLock<Option<RelativeTimeFormatter>>>>;

/// One language's time formatters, each built from its data on first use.
/// `None` where the data lacks one: a relative time or unit is then worded in English.
pub(crate) struct TimeFormats {
    data: Arc<Data>,
    language: Language,
    relative: RelativeFormatters,
    /// A pack's own, never another language's:
    /// its counts would take this one's plurals.
    units: Option<units::Loaded>,
    plurals: OnceLock<Option<PluralRules>>,
    digits: OnceLock<Option<DecimalFormatter>>,
}

impl TimeFormats {
    pub(crate) fn new(data: Arc<Data>, language: Language, units: Option<units::Loaded>) -> Self {
        Self {
            data,
            language,
            relative: RelativeFormatters::default(),
            units,
            plurals: OnceLock::new(),
            digits: OnceLock::new(),
        }
    }

    /// The base data's, which carries every formatter
    /// the markers binary builds and English's units.
    pub(crate) fn english() -> Arc<Self> {
        static ENGLISH: LazyLock<Arc<TimeFormats>> = LazyLock::new(|| {
            Arc::new(TimeFormats::new(
                Data::builtin(),
                Language::EN,
                Some(data::builtin_units()),
            ))
        });
        ENGLISH.clone()
    }

    pub(crate) fn relative(
        &self,
        count: u64,
        unit: TimeUnit,
        width: TimeWidth,
        direction: Direction,
    ) -> Option<String> {
        let sign = match direction {
            Direction::Past => Sign::Negative,
            Direction::Future => Sign::None,
        };
        let formatter = self.relative_formatter(unit, width)?;
        Some(
            formatter
                .format(Decimal::from(count).with_sign(sign))
                .to_string(),
        )
    }

    pub(crate) fn now(&self, width: TimeWidth) -> Option<String> {
        let formatter = self.relative_formatter(TimeUnit::Second, width)?;
        Some(formatter.format(Decimal::from(0_u8)).to_string())
    }

    /// From the pack's narrow unit patterns: icu_experimental 0.7.0 formats a unit
    /// only with `compiled_data`, and its `DurationFormatter` panics in every constructor.
    pub(crate) fn count(&self, count: u64, unit: TimeUnit) -> Option<String> {
        let patterns = &self.units.as_ref()?.get().narrow.units;
        // Without its plural rules or digits, the pack's own `other` pattern and ASCII digits
        // still read better than English inside the pack's translated message.
        let plural = self.plural(count).unwrap_or(Plural::Other);
        let pattern = patterns
            .get(unit.cldr(), plural)
            .or_else(|| patterns.get(unit.cldr(), Plural::Other))?;
        let digits = self.digits.get_or_init(|| {
            let built = DecimalFormatter::try_new_with_buffer_provider(
                &self.data.fallback_provider(),
                (&LanguageIdentifier::from(self.language)).into(),
                UNGROUPED.into(),
            );
            self.or_warn("number", None, built)
        });
        let count = match digits {
            Some(digits) => digits.format(&Decimal::from(count)).to_string(),
            None => count.to_string(),
        };
        Some(pattern.replace("{0}", &count))
    }

    fn plural(&self, count: u64) -> Option<Plural> {
        let rules = self
            .plurals
            .get_or_init(|| {
                let built = PluralRules::try_new_cardinal_with_buffer_provider(
                    &self.data.fallback_provider(),
                    (&LanguageIdentifier::from(self.language)).into(),
                );
                self.or_warn("plural", None, built)
            })
            .as_ref()?;
        Some(match rules.category_for(count) {
            PluralCategory::Zero => Plural::Zero,
            PluralCategory::One => Plural::One,
            PluralCategory::Two => Plural::Two,
            PluralCategory::Few => Plural::Few,
            PluralCategory::Many => Plural::Many,
            PluralCategory::Other => Plural::Other,
        })
    }

    fn relative_formatter(
        &self,
        unit: TimeUnit,
        width: TimeWidth,
    ) -> Option<&RelativeTimeFormatter> {
        self.relative
            .get(width)
            .get(unit)
            .get_or_init(|| {
                self.or_warn(
                    "relative time",
                    Some(width),
                    self.build_relative(unit, width),
                )
            })
            .as_ref()
    }

    fn or_warn<T>(
        &self,
        what: &str,
        width: Option<TimeWidth>,
        built: Result<T, DataError>,
    ) -> Option<T> {
        built
            .inspect_err(|error| {
                tracing::warn!(
                    language = %self.language,
                    ?width,
                    %error,
                    "no {what} data for this language",
                );
            })
            .ok()
    }

    fn build_relative(
        &self,
        unit: TimeUnit,
        width: TimeWidth,
    ) -> Result<RelativeTimeFormatter, DataError> {
        // "now" for zero seconds; the other units count on,
        // as `Auto` makes 24 hours "yesterday" whatever the calendar says.
        let numeric = match unit {
            TimeUnit::Second => Numeric::Auto,
            TimeUnit::Minute | TimeUnit::Hour | TimeUnit::Day => Numeric::Always,
        };
        let options = RelativeTimeFormatterOptions::default()
            .with_numeric(numeric)
            .with_grouping_strategy(UNGROUPED);
        let new = match (width, unit) {
            (TimeWidth::Narrow, TimeUnit::Second) => {
                RelativeTimeFormatter::try_new_narrow_second_with_buffer_provider
            }
            (TimeWidth::Narrow, TimeUnit::Minute) => {
                RelativeTimeFormatter::try_new_narrow_minute_with_buffer_provider
            }
            (TimeWidth::Narrow, TimeUnit::Hour) => {
                RelativeTimeFormatter::try_new_narrow_hour_with_buffer_provider
            }
            (TimeWidth::Narrow, TimeUnit::Day) => {
                RelativeTimeFormatter::try_new_narrow_day_with_buffer_provider
            }
            (TimeWidth::Long, TimeUnit::Second) => {
                RelativeTimeFormatter::try_new_long_second_with_buffer_provider
            }
            (TimeWidth::Long, TimeUnit::Minute) => {
                RelativeTimeFormatter::try_new_long_minute_with_buffer_provider
            }
            (TimeWidth::Long, TimeUnit::Hour) => {
                RelativeTimeFormatter::try_new_long_hour_with_buffer_provider
            }
            (TimeWidth::Long, TimeUnit::Day) => {
                RelativeTimeFormatter::try_new_long_day_with_buffer_provider
            }
        };
        new(
            &self.data.fallback_provider(),
            (&LanguageIdentifier::from(self.language)).into(),
            options,
        )
    }
}
