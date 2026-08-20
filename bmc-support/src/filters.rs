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

use crate::constants::{BMC_CONFIG_DIR, BMC_CONFIG_LEGACY, BMC_SECRETS};
use regex::Regex;
use std::path::Path;
use tracing::warn;

/// Type alias for a credential filter function.
type FilterFn = fn(&[u8]) -> Vec<u8>;

/// Type alias for a matcher deciding whether a filter applies to a path.
type MatchFn = fn(&Path) -> bool;

/// Registry of credential censors. Each entry pairs a matcher over the
/// file path with the filter to run when it matches. Matching by
/// predicate rather than one fixed path lets a single censor cover a
/// whole file family: the BMC config, its relocated legacy copy, and
/// every timestamped backup all carry the same `api_key` secrets and
/// must all be censored.
const CREDENTIAL_FILTERS: &[(MatchFn, FilterFn)] = &[
    (is_bmc_config, censor_bmc_config),
    (is_uci_wireless, censor_uci_wireless),
];

/// Whether a path must be kept out of the archive entirely.
///
/// The secret store holds plaintext account credentials with no diagnostic value,
/// so it is excluded rather than censored — a censor would have to track every
/// credential-type field name as the catalog grows, and missing one leaks.
pub fn is_excluded(path: &Path) -> bool {
    let secrets = Path::new(BMC_SECRETS);
    let Some(stem) = secrets.file_stem().and_then(|stem| stem.to_str()) else {
        return false;
    };
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };

    // Matched on the stem: `set_extension` replaces rather than appends, so the atomic write
    // leaves `secrets.tmp` mid-rename, not `secrets.json.tmp`.
    path.parent() == secrets.parent() && name.starts_with(&format!("{stem}."))
}

/// Whether a credential filter applies to `path`, i.e. whether its content
/// must be buffered for censoring instead of streamed.
pub fn is_filtered(path: &Path) -> bool {
    CREDENTIAL_FILTERS.iter().any(|(matches, _)| matches(path))
}

/// Apply credential censoring to file content if a filter matches the path.
///
/// Returns the filtered content, or the original content unchanged if:
/// - No filter matches the path
/// - The filter fails for any reason
///
/// Filter failures are logged but never prevent archive creation.
pub fn apply(path: &Path, content: Vec<u8>) -> Vec<u8> {
    let Some(filter_fn) = CREDENTIAL_FILTERS
        .iter()
        .find_map(|(matches, f)| matches(path).then_some(f))
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

/// Match the BMC config and every copy that carries its credentials:
/// the current `/etc/bmc/config.json` with its backups, plus the
/// pre-migration `/etc/bmc_config.json`.
fn is_bmc_config(path: &Path) -> bool {
    path.parent() == Some(Path::new(BMC_CONFIG_DIR)) || path == Path::new(BMC_CONFIG_LEGACY)
}

/// Match the OpenWrt UCI wireless config, which holds the Wi-Fi key.
fn is_uci_wireless(path: &Path) -> bool {
    path == Path::new("/etc/config/wireless")
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
        let result = apply(Path::new("/etc/bmc/config.json"), content);
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            r#"{"api_key":"<CENSORED>"}"#
        );
    }

    #[test]
    fn apply_filters_legacy_bmc_config() {
        let content = br#"{"api_key":"secret"}"#.to_vec();
        let result = apply(Path::new("/etc/bmc_config.json"), content);
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            r#"{"api_key":"<CENSORED>"}"#
        );
    }

    #[test]
    fn apply_filters_bmc_config_timestamped_backup() {
        let content = br#"{"api_key":"secret"}"#.to_vec();
        let result = apply(Path::new("/etc/bmc/config.json.backup.1784028993"), content);
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            r#"{"api_key":"<CENSORED>"}"#
        );
    }

    #[test]
    fn is_bmc_config_matches_the_config_family_only() {
        // Current layout: config plus its backups.
        assert!(is_bmc_config(Path::new("/etc/bmc/config.json")));
        assert!(is_bmc_config(Path::new("/etc/bmc/config.json.backup.42")));
        assert!(is_bmc_config(Path::new("/etc/bmc/config.json.bcp")));
        // Legacy layout.
        assert!(is_bmc_config(Path::new("/etc/bmc_config.json")));
        // Unrelated neighbours must not be swept in.
        assert!(!is_bmc_config(Path::new("/etc/bmc")));
        assert!(!is_bmc_config(Path::new("/etc/bmc_config.jsonx")));
        assert!(!is_bmc_config(Path::new("/etc/hosts")));
    }

    #[test]
    fn is_excluded_matches_the_secret_store_family_only() {
        assert!(is_excluded(Path::new("/etc/bmc/secrets.json")));
        // The atomic-write temp file and the unreadable-store backup carry the same secrets.
        assert!(is_excluded(Path::new("/etc/bmc/secrets.tmp")));
        assert!(is_excluded(Path::new("/etc/bmc/secrets.json.bcp")));
        // The config beside it is collected (censored), never excluded.
        assert!(!is_excluded(Path::new("/etc/bmc/config.json")));
        assert!(!is_excluded(Path::new("/etc/bmc")));
        assert!(!is_excluded(Path::new("/etc/secrets.json")));
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
        let result = apply(Path::new("/etc/bmc/config.json"), content.clone());
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
