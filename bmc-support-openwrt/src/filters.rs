// Copyright (C) 2025  Braiins Systems s.r.o.
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

//! Credential filters for the OpenWrt board — [`SupportFilter`] implementations
//! carrying its config layout, secret store and UCI wireless knowledge.

use bmc_support::SupportFilter;
use regex::Regex;
use std::path::Path;
use tracing::warn;

/// Account secrets, deliberately **never** collected — see [`SecretsExclusion`].
const BMC_SECRETS: &str = "/etc/bmc/secrets.json";
/// Directory holding the current config and its timestamped backups.
/// Collected wholesale so `config.json.backup.<ts>` snapshots ride
/// along in the support archive.
pub const BMC_CONFIG_DIR: &str = "/etc/bmc";
/// Pre-migration config path, deliberately kept on disk for downgrade
/// safety (see `bmc::config_migration`). Collected so a bad migration
/// can still be diagnosed from the original file.
pub const BMC_CONFIG_LEGACY: &str = "/etc/bmc_config.json";

/// Keeps the secret store out of the archive.
///
/// It holds plaintext account credentials with no diagnostic value, so it is
/// excluded rather than censored — a censor would have to track every
/// credential-type field name as the catalog grows, and missing one leaks.
#[derive(Debug)]
pub struct SecretsExclusion;

impl SupportFilter for SecretsExclusion {
    fn excludes(&self, path: &Path) -> bool {
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
}

/// Censors `"api_key"` values in the BMC config and every copy that carries
/// its credentials: the current config with its backups, plus the
/// pre-migration legacy config.
#[derive(Debug)]
pub struct BmcConfigCensor;

impl SupportFilter for BmcConfigCensor {
    fn matches(&self, path: &Path) -> bool {
        path.parent() == Some(Path::new(BMC_CONFIG_DIR)) || path == Path::new(BMC_CONFIG_LEGACY)
    }

    /// Scans for the pattern `"api_key"` `:` (optional whitespace) `"<value>"`
    /// and replaces `<value>` with `<CENSORED>`.
    /// Non-UTF-8 content is returned unchanged.
    fn apply(&self, content: &[u8]) -> Vec<u8> {
        let re = Regex::new(r#""api_key"(\s*:\s*)"(?:[^"\\]|\\.)*""#).expect("BUG: invalid regex");

        let Ok(text) = std::str::from_utf8(content) else {
            warn!("credential filter skipped: file is not valid UTF-8");
            return content.to_vec();
        };

        re.replace_all(text, r#""api_key"$1"<CENSORED>""#)
            .into_owned()
            .into_bytes()
    }
}

/// Censors `option key` values in the OpenWrt UCI wireless config, which
/// holds the Wi-Fi key.
#[derive(Debug)]
pub struct UciWirelessCensor;

impl SupportFilter for UciWirelessCensor {
    fn matches(&self, path: &Path) -> bool {
        path == Path::new("/etc/config/wireless")
    }

    /// Replaces the quoted value after `option key` with `<CENSORED>`,
    /// preserving the surrounding quote characters.
    /// Non-UTF-8 content is returned unchanged.
    fn apply(&self, content: &[u8]) -> Vec<u8> {
        let re = Regex::new(r"(?m)(option key\s+)'[^']*'").expect("BUG: invalid regex");

        let Ok(text) = std::str::from_utf8(content) else {
            warn!("credential filter skipped: file is not valid UTF-8");
            return content.to_vec();
        };

        re.replace_all(text, "${1}'<CENSORED>'")
            .into_owned()
            .into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run `content` through the whole filter set the way the archive does,
    /// driving the real `bmc_support::censor` engine.
    fn apply_bmc_filters(path: &Path, content: Vec<u8>) -> Vec<u8> {
        let filters: [&dyn SupportFilter; 3] =
            [&SecretsExclusion, &BmcConfigCensor, &UciWirelessCensor];
        bmc_support::censor(&filters, path, content)
    }

    #[test]
    fn apply_filters_bmc_config() {
        let content = br#"{"api_key":"secret"}"#.to_vec();
        let result = apply_bmc_filters(Path::new("/etc/bmc/config.json"), content);
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            r#"{"api_key":"<CENSORED>"}"#
        );
    }

    #[test]
    fn apply_filters_legacy_bmc_config() {
        let content = br#"{"api_key":"secret"}"#.to_vec();
        let result = apply_bmc_filters(Path::new("/etc/bmc_config.json"), content);
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            r#"{"api_key":"<CENSORED>"}"#
        );
    }

    #[test]
    fn apply_filters_bmc_config_timestamped_backup() {
        let content = br#"{"api_key":"secret"}"#.to_vec();
        let result =
            apply_bmc_filters(Path::new("/etc/bmc/config.json.backup.1784028993"), content);
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            r#"{"api_key":"<CENSORED>"}"#
        );
    }

    #[test]
    fn bmc_config_censor_matches_the_config_family_only() {
        // Current layout: config plus its backups.
        assert!(BmcConfigCensor.matches(Path::new("/etc/bmc/config.json")));
        assert!(BmcConfigCensor.matches(Path::new("/etc/bmc/config.json.backup.42")));
        assert!(BmcConfigCensor.matches(Path::new("/etc/bmc/config.json.bcp")));
        // Legacy layout.
        assert!(BmcConfigCensor.matches(Path::new("/etc/bmc_config.json")));
        // Unrelated neighbours must not be swept in.
        assert!(!BmcConfigCensor.matches(Path::new("/etc/bmc")));
        assert!(!BmcConfigCensor.matches(Path::new("/etc/bmc_config.jsonx")));
        assert!(!BmcConfigCensor.matches(Path::new("/etc/hosts")));
    }

    #[test]
    fn secrets_exclusion_matches_the_secret_store_family_only() {
        assert!(SecretsExclusion.excludes(Path::new("/etc/bmc/secrets.json")));
        // The atomic-write temp file and the unreadable-store backup carry the same secrets.
        assert!(SecretsExclusion.excludes(Path::new("/etc/bmc/secrets.tmp")));
        assert!(SecretsExclusion.excludes(Path::new("/etc/bmc/secrets.json.bcp")));
        // The config beside it is collected (censored), never excluded.
        assert!(!SecretsExclusion.excludes(Path::new("/etc/bmc/config.json")));
        assert!(!SecretsExclusion.excludes(Path::new("/etc/bmc")));
        assert!(!SecretsExclusion.excludes(Path::new("/etc/secrets.json")));
    }

    #[test]
    fn apply_filters_wireless() {
        let content = b"\toption key 'secret'\n".to_vec();
        let result = apply_bmc_filters(Path::new("/etc/config/wireless"), content);
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            "\toption key '<CENSORED>'\n"
        );
    }

    #[test]
    fn apply_no_filter_for_unknown_path() {
        let content = b"unchanged content".to_vec();
        let result = apply_bmc_filters(Path::new("/etc/hosts"), content.clone());
        assert_eq!(result, content);
    }

    #[test]
    fn apply_invalid_utf8_returns_original() {
        let content = vec![0xFF, 0xFE, 0xFD];
        let result = apply_bmc_filters(Path::new("/etc/bmc/config.json"), content.clone());
        assert_eq!(result, content);
    }

    #[test]
    fn censor_bmc_config_replaces_api_key_value() {
        let input = r#"{"authentication":{"api_key":"sk-secret-123"},"name":"pool"}"#;
        let result = BmcConfigCensor.apply(input.as_bytes());
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            r#"{"authentication":{"api_key":"<CENSORED>"},"name":"pool"}"#
        );
    }

    #[test]
    fn censor_bmc_config_replaces_multiple_api_keys() {
        let input = r#"{"accounts":[{"api_key":"secret1"},{"api_key":"secret2"}]}"#;
        let result = BmcConfigCensor.apply(input.as_bytes());
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            r#"{"accounts":[{"api_key":"<CENSORED>"},{"api_key":"<CENSORED>"}]}"#
        );
    }

    #[test]
    fn censor_bmc_config_handles_whitespace_after_colon() {
        let input = "\"api_key\" : \"my-key\"";
        let result = BmcConfigCensor.apply(input.as_bytes());
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            "\"api_key\" : \"<CENSORED>\""
        );
    }

    #[test]
    fn censor_bmc_config_no_api_key_unchanged() {
        let input = r#"{"name":"test","value":42}"#;
        let result = BmcConfigCensor.apply(input.as_bytes());
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            input
        );
    }

    #[test]
    fn censor_bmc_config_empty_input() {
        let result = BmcConfigCensor.apply(b"");
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            ""
        );
    }

    #[test]
    fn censor_bmc_config_handles_escaped_quotes_in_value() {
        let input = r#"{"api_key":"val\"ue"}"#;
        let result = BmcConfigCensor.apply(input.as_bytes());
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            r#"{"api_key":"<CENSORED>"}"#
        );
    }

    #[test]
    fn censor_uci_wireless_single_quoted_key() {
        let input = "\toption key 'mypassword'\n";
        let result = UciWirelessCensor.apply(input.as_bytes());
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            "\toption key '<CENSORED>'\n"
        );
    }

    #[test]
    fn censor_uci_wireless_preserves_other_options() {
        let input =
            "\toption ssid 'MyNetwork'\n\toption key 'secret'\n\toption encryption 'psk2'\n";
        let result = UciWirelessCensor.apply(input.as_bytes());
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            "\toption ssid 'MyNetwork'\n\toption key '<CENSORED>'\n\toption encryption 'psk2'\n"
        );
    }

    #[test]
    fn censor_uci_wireless_no_key_option() {
        let input = "\toption ssid 'MyNetwork'\n\toption encryption 'none'\n";
        let result = UciWirelessCensor.apply(input.as_bytes());
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            input
        );
    }

    #[test]
    fn censor_uci_wireless_empty_input() {
        let result = UciWirelessCensor.apply(b"");
        assert_eq!(
            std::str::from_utf8(&result).expect("BUG: result should be UTF-8"),
            ""
        );
    }
}
