// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::constants::BMC_CONFIG;
use regex::Regex;
use std::path::Path;
use tracing::warn;

/// Type alias for a credential filter function.
type FilterFn = fn(&[u8]) -> Vec<u8>;

/// Registry of files that need credential censoring.
/// Each entry maps an absolute file path to a filter function.
const CREDENTIAL_FILTERS: &[(&str, FilterFn)] = &[
    (BMC_CONFIG, censor_bmc_config),
    ("/etc/config/wireless", censor_uci_wireless),
];

/// Apply credential censoring to file content if a filter is registered for the path.
///
/// Returns the filtered content, or the original content unchanged if:
/// - No filter is registered for the path
/// - The filter fails for any reason
///
/// Filter failures are logged but never prevent archive creation.
pub fn apply(path: &Path, content: Vec<u8>) -> Vec<u8> {
    let Some(filter_fn) = CREDENTIAL_FILTERS
        .iter()
        .find_map(|(p, f)| (path == Path::new(p)).then_some(f))
    else {
        return content;
    };

    std::panic::catch_unwind(|| filter_fn(&content)).unwrap_or_else(|_| {
        warn!(
            path = %path.display(),
            "credential filter panicked, including unfiltered content"
        );
        content
    })
}

/// Censor `"api_key"` values in BMC config JSON.
///
/// Scans for the pattern `"api_key"` `:` (optional whitespace) `"<value>"`
/// and replaces `<value>` with `<CENSORED>`.
/// Non-UTF-8 content is returned unchanged.
fn censor_bmc_config(content: &[u8]) -> Vec<u8> {
    let re = Regex::new(r#""api_key"(\s*:\s*)"(?:[^"\\]|\\.)*""#).expect("BUG: invalid regex");

    let Ok(text) = std::str::from_utf8(content) else {
        warn!("credential filter skipped: file is not valid UTF-8");
        return content.to_vec();
    };

    re.replace_all(text, r#""api_key"$1"<CENSORED>""#)
        .into_owned()
        .into_bytes()
}

/// Censor `option key` values in OpenWrt UCI wireless config.
///
/// Scans line by line for `option key` followed by a quoted or unquoted value,
/// replaces only the value content with `<CENSORED>`, preserving the surrounding
/// quote characters. Unquoted values are wrapped in single quotes.
/// Non-UTF-8 content is returned unchanged.
fn censor_uci_wireless(content: &[u8]) -> Vec<u8> {
    let re = Regex::new(r"(?m)(option key\s+)'[^']*'").expect("BUG: invalid regex");

    let Ok(text) = std::str::from_utf8(content) else {
        warn!("credential filter skipped: file is not valid UTF-8");
        return content.to_vec();
    };

    re.replace_all(text, "${1}'<CENSORED>'")
        .into_owned()
        .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn apply_filters_bmc_config() {
        let content = br#"{"api_key":"secret"}"#.to_vec();
        let result = apply(Path::new("/etc/bmc_config.json"), content);
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            r#"{"api_key":"<CENSORED>"}"#
        );
    }

    #[test]
    fn apply_filters_wireless() {
        let content = b"\toption key 'secret'\n".to_vec();
        let result = apply(Path::new("/etc/config/wireless"), content);
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            "\toption key '<CENSORED>'\n"
        );
    }

    #[test]
    fn apply_no_filter_for_unknown_path() {
        let content = b"unchanged content".to_vec();
        let result = apply(Path::new("/etc/hosts"), content.clone());
        assert_eq!(result, content);
    }

    #[test]
    fn apply_invalid_utf8_returns_original() {
        let content = vec![0xFF, 0xFE, 0xFD];
        let result = apply(Path::new("/etc/bmc_config.json"), content.clone());
        assert_eq!(result, content);
    }

    #[test]
    fn censor_bmc_config_replaces_api_key_value() {
        let input = r#"{"authentication":{"api_key":"sk-secret-123"},"name":"pool"}"#;
        let result = censor_bmc_config(input.as_bytes());
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            r#"{"authentication":{"api_key":"<CENSORED>"},"name":"pool"}"#
        );
    }

    #[test]
    fn censor_bmc_config_replaces_multiple_api_keys() {
        let input = r#"{"accounts":[{"api_key":"secret1"},{"api_key":"secret2"}]}"#;
        let result = censor_bmc_config(input.as_bytes());
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            r#"{"accounts":[{"api_key":"<CENSORED>"},{"api_key":"<CENSORED>"}]}"#
        );
    }

    #[test]
    fn censor_bmc_config_handles_whitespace_after_colon() {
        let input = "\"api_key\" : \"my-key\"";
        let result = censor_bmc_config(input.as_bytes());
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            "\"api_key\" : \"<CENSORED>\""
        );
    }

    #[test]
    fn censor_bmc_config_no_api_key_unchanged() {
        let input = r#"{"name":"test","value":42}"#;
        let result = censor_bmc_config(input.as_bytes());
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            input
        );
    }

    #[test]
    fn censor_bmc_config_empty_input() {
        let result = censor_bmc_config(b"");
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            ""
        );
    }

    #[test]
    fn censor_bmc_config_handles_escaped_quotes_in_value() {
        let input = r#"{"api_key":"val\"ue"}"#;
        let result = censor_bmc_config(input.as_bytes());
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            r#"{"api_key":"<CENSORED>"}"#
        );
    }

    #[test]
    fn censor_uci_wireless_single_quoted_key() {
        let input = "\toption key 'mypassword'\n";
        let result = censor_uci_wireless(input.as_bytes());
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            "\toption key '<CENSORED>'\n"
        );
    }

    #[test]
    fn censor_uci_wireless_preserves_other_options() {
        let input =
            "\toption ssid 'MyNetwork'\n\toption key 'secret'\n\toption encryption 'psk2'\n";
        let result = censor_uci_wireless(input.as_bytes());
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            "\toption ssid 'MyNetwork'\n\toption key '<CENSORED>'\n\toption encryption 'psk2'\n"
        );
    }

    #[test]
    fn censor_uci_wireless_no_key_option() {
        let input = "\toption ssid 'MyNetwork'\n\toption encryption 'none'\n";
        let result = censor_uci_wireless(input.as_bytes());
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            input
        );
    }

    #[test]
    fn censor_uci_wireless_empty_input() {
        let result = censor_uci_wireless(b"");
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            ""
        );
    }
}
