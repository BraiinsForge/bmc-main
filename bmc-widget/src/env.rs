// Copyright (C) 2025  Braiins Systems s.r.o.

//! Environment variable helpers for widgets.
//!
//! Widgets receive their initial configuration via environment variables set by the coordinator.

use bmc_widget_protocol::{Localization, Settings, SizeInfo, SizeType};
use serde::de::DeserializeOwned;
use std::env;

/// Environment variable names for widget configuration.
pub mod vars {
    pub const INSTANCE_ID: &str = "DECK_INSTANCE_ID";
    pub const SIZE_TYPE: &str = "DECK_SIZE_TYPE";
    pub const WIDTH: &str = "DECK_WIDTH";
    pub const HEIGHT: &str = "DECK_HEIGHT";
    pub const PARAMS: &str = "DECK_PARAMS";
    pub const TIMEZONE: &str = "DECK_TIMEZONE";
    pub const NIGHT_MODE: &str = "DECK_NIGHT_MODE";
    pub const LOCALIZATION: &str = "DECK_LOCALIZATION";
}

/// Errors that can occur when reading environment variables.
#[derive(Debug, thiserror::Error)]
pub enum EnvError {
    #[error("missing environment variable: {0}")]
    Missing(&'static str),

    #[error("invalid value for {var}: {message}")]
    Invalid { var: &'static str, message: String },

    #[error("failed to parse JSON for {var}: {source}")]
    JsonParse {
        var: &'static str,
        source: serde_json::Error,
    },
}

/// Reads the widget instance ID from `DECK_INSTANCE_ID`.
pub fn read_instance_id() -> Result<String, EnvError> {
    env::var(vars::INSTANCE_ID).map_err(|_| EnvError::Missing(vars::INSTANCE_ID))
}

/// Reads the widget size configuration from environment variables.
///
/// Reads `DECK_SIZE_TYPE`, `DECK_WIDTH`, and `DECK_HEIGHT`.
pub fn read_size() -> Result<SizeInfo, EnvError> {
    let size_type_str =
        env::var(vars::SIZE_TYPE).map_err(|_| EnvError::Missing(vars::SIZE_TYPE))?;
    let name = parse_size_type(&size_type_str)?;

    let width = env::var(vars::WIDTH)
        .map_err(|_| EnvError::Missing(vars::WIDTH))?
        .parse::<u32>()
        .map_err(|e| EnvError::Invalid {
            var: vars::WIDTH,
            message: e.to_string(),
        })?;

    let height = env::var(vars::HEIGHT)
        .map_err(|_| EnvError::Missing(vars::HEIGHT))?
        .parse::<u32>()
        .map_err(|e| EnvError::Invalid {
            var: vars::HEIGHT,
            message: e.to_string(),
        })?;

    Ok(SizeInfo {
        name,
        width,
        height,
    })
}

/// Reads and parses widget parameters from `DECK_PARAMS`.
///
/// Returns the default value if `DECK_PARAMS` is not set.
pub fn read_params<T: DeserializeOwned + Default>() -> Result<T, EnvError> {
    match env::var(vars::PARAMS) {
        Ok(json) => serde_json::from_str(&json).map_err(|e| EnvError::JsonParse {
            var: vars::PARAMS,
            source: e,
        }),
        Err(_) => Ok(T::default()),
    }
}

/// Reads initial settings from environment variables.
///
/// Reads `DECK_TIMEZONE`, `DECK_NIGHT_MODE`, and `DECK_LOCALIZATION`.
/// Missing values result in `None` for that field.
pub fn read_settings() -> Result<Settings, EnvError> {
    let timezone = env::var(vars::TIMEZONE).ok();

    let night_mode = match env::var(vars::NIGHT_MODE) {
        Ok(val) => Some(val == "1" || val.eq_ignore_ascii_case("true")),
        Err(_) => None,
    };

    let localization =
        match env::var(vars::LOCALIZATION) {
            Ok(json) => Some(serde_json::from_str::<Localization>(&json).map_err(|e| {
                EnvError::JsonParse {
                    var: vars::LOCALIZATION,
                    source: e,
                }
            })?),
            Err(_) => None,
        };

    Ok(Settings {
        timezone,
        night_mode,
        localization,
    })
}

fn parse_size_type(s: &str) -> Result<SizeType, EnvError> {
    match s.to_lowercase().as_str() {
        "small" => Ok(SizeType::Small),
        "medium" => Ok(SizeType::Medium),
        "large" => Ok(SizeType::Large),
        "full" => Ok(SizeType::Full),
        _ => Err(EnvError::Invalid {
            var: vars::SIZE_TYPE,
            message: format!("unknown size type: {s}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clear_env_vars() {
        // SAFETY: Tests run with --test-threads=1
        unsafe {
            env::remove_var(vars::INSTANCE_ID);
            env::remove_var(vars::SIZE_TYPE);
            env::remove_var(vars::WIDTH);
            env::remove_var(vars::HEIGHT);
            env::remove_var(vars::PARAMS);
            env::remove_var(vars::TIMEZONE);
            env::remove_var(vars::NIGHT_MODE);
            env::remove_var(vars::LOCALIZATION);
        }
    }

    /// All env var tests combined into one to avoid race conditions.
    /// These tests modify global state (environment variables) and must run sequentially.
    #[test]
    fn env_var_tests() {
        // Test: read_instance_id_returns_value
        clear_env_vars();
        // SAFETY: Single-threaded test
        unsafe { env::set_var(vars::INSTANCE_ID, "clock-abc123") };
        assert_eq!(
            read_instance_id().expect("BUG: env var was just set"),
            "clock-abc123"
        );

        // Test: read_instance_id_missing_returns_error
        clear_env_vars();
        assert!(matches!(
            read_instance_id(),
            Err(EnvError::Missing(vars::INSTANCE_ID))
        ));

        // Test: read_size_returns_value
        clear_env_vars();
        // SAFETY: Single-threaded test
        unsafe {
            env::set_var(vars::SIZE_TYPE, "medium");
            env::set_var(vars::WIDTH, "640");
            env::set_var(vars::HEIGHT, "240");
        }
        let size = read_size().expect("BUG: env vars were just set");
        assert_eq!(size.name, SizeType::Medium);
        assert_eq!(size.width, 640);
        assert_eq!(size.height, 240);

        // Test: read_size_invalid_type_returns_error
        clear_env_vars();
        // SAFETY: Single-threaded test
        unsafe {
            env::set_var(vars::SIZE_TYPE, "invalid");
            env::set_var(vars::WIDTH, "640");
            env::set_var(vars::HEIGHT, "240");
        }
        assert!(matches!(read_size(), Err(EnvError::Invalid { .. })));

        // Test: read_size_invalid_width_returns_error
        clear_env_vars();
        // SAFETY: Single-threaded test
        unsafe {
            env::set_var(vars::SIZE_TYPE, "small");
            env::set_var(vars::WIDTH, "not_a_number");
            env::set_var(vars::HEIGHT, "240");
        }
        assert!(matches!(read_size(), Err(EnvError::Invalid { .. })));

        // Test: read_params_returns_default_when_missing
        clear_env_vars();
        {
            #[derive(Debug, Default, serde::Deserialize, PartialEq)]
            struct TestParams {
                value: Option<String>,
            }
            let params: TestParams =
                read_params().expect("BUG: params deserialization should not fail");
            assert_eq!(params, TestParams::default());
        }

        // Test: read_params_parses_json
        clear_env_vars();
        // SAFETY: Single-threaded test
        unsafe { env::set_var(vars::PARAMS, r#"{"style":"digital"}"#) };
        {
            #[derive(Debug, Default, serde::Deserialize, PartialEq)]
            struct TestParamsStyle {
                style: String,
            }
            let params: TestParamsStyle =
                read_params().expect("BUG: params deserialization should not fail");
            assert_eq!(params.style, "digital");
        }

        // Test: read_settings_all_present
        clear_env_vars();
        // SAFETY: Single-threaded test
        unsafe {
            env::set_var(vars::TIMEZONE, "Europe/Prague");
            env::set_var(vars::NIGHT_MODE, "1");
            env::set_var(
                vars::LOCALIZATION,
                r#"{"dateFormat":"DdMmYyyyDot","timeFormat":"Hour24","numberFormat":"SpaceGroupCommaDecimal","temperatureUnit":"Celsius","firstDayOfWeek":"Monday"}"#,
            );
        }
        let settings = read_settings().expect("BUG: settings deserialization should not fail");
        assert_eq!(settings.timezone, Some("Europe/Prague".to_owned()));
        assert_eq!(settings.night_mode, Some(true));
        assert!(settings.localization.is_some());

        // Test: read_settings_none_when_missing
        clear_env_vars();
        let settings = read_settings().expect("BUG: settings deserialization should not fail");
        assert!(settings.timezone.is_none());
        assert!(settings.night_mode.is_none());
        assert!(settings.localization.is_none());

        // Test: read_settings_night_mode_false
        clear_env_vars();
        // SAFETY: Single-threaded test
        unsafe { env::set_var(vars::NIGHT_MODE, "0") };
        let settings = read_settings().expect("BUG: settings deserialization should not fail");
        assert_eq!(settings.night_mode, Some(false));
    }
}
