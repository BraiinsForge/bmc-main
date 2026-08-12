// Copyright (C) 2024  Braiins Systems s.r.o.
//
// This file is part of Braiins Open-Source Initiative (BOSI).
//
// BOSI is free software: you can redistribute it and/or modify
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
// Please, keep in mind that we may also license BOSI or any part thereof
// under a proprietary license. For more information on the terms and conditions
// of such proprietary license or if you have any other questions, please
// contact us at opensource@braiins.com.

use anyhow::Result;
use bmc_net_types::wifi::{EncryptionType, WifiScanItem};
use tokio::time::Duration;

use crate::wifi::utils::CommandUtils;

const SCAN_TIMEOUT: Duration = Duration::from_secs(10);

/// A single parsed `iwlist` scan cell, reduced to the fields the canonical
/// [`WifiScanItem`] needs. Encryption is derived from the information elements.
#[derive(Debug, Clone, Default)]
struct WifiScanResult {
    ssid: String,
    signal_level: i32,
    information_element: Vec<String>,
}

impl From<WifiScanResult> for WifiScanItem {
    fn from(result: WifiScanResult) -> Self {
        let enc_list = result
            .information_element
            .iter()
            .map(|ie| parse_information_element(ie))
            .collect::<Vec<EncryptionType>>();

        WifiScanItem::new(
            result.ssid,
            result.signal_level,
            determine_encryption_type(&enc_list),
        )
    }
}

fn parse_information_element(ie: &str) -> EncryptionType {
    match ie {
        x if x.contains("PSK unknown") => EncryptionType::Wpa2_3, // match before Wpa3-only
        x if x.contains("unknown") => EncryptionType::Wpa3,
        x if x.contains("WPA2 Version") => EncryptionType::Wpa2,
        x if x.contains("WPA Version 1") => EncryptionType::Wpa,
        x if x.contains("WEP") => EncryptionType::WepShared,
        _ => EncryptionType::None,
    }
}

fn determine_encryption_type(list: &[EncryptionType]) -> EncryptionType {
    let mut final_type = EncryptionType::None;

    for enc_type in list {
        if (final_type == EncryptionType::Wpa && enc_type == &EncryptionType::Wpa2)
            || (final_type == EncryptionType::Wpa2 && enc_type == &EncryptionType::Wpa)
        {
            final_type = EncryptionType::Wpa1_2;
        } else if (final_type == EncryptionType::Wpa2 && enc_type == &EncryptionType::Wpa2_3)
            || (final_type == EncryptionType::Wpa2_3 && enc_type == &EncryptionType::Wpa2)
        {
            final_type = EncryptionType::Wpa2_3;
        } else if final_type == EncryptionType::Wpa2 && enc_type == &EncryptionType::Wpa3 {
            final_type = EncryptionType::Wpa3;
        } else if final_type == EncryptionType::None {
            final_type = *enc_type;
        }
    }

    final_type
}

/// Parse `iwlist scan` output into scan cells. `iwlist` has no machine-readable
/// output, so this walks the text: a `Cell` line starts an entry, `ESSID`/`Quality`
/// fill it, and `IE` lines (which span several rows) drive the encryption guess.
fn parse_scanner_output(output: &str) -> Vec<WifiScanResult> {
    let mut entries: Vec<WifiScanResult> = Vec::new();
    let mut collecting_ie = false;

    for line in output.lines() {
        let line = line.trim();

        if line.starts_with("Cell") {
            collecting_ie = false;
            entries.push(WifiScanResult::default());
        } else if let Some(entry) = entries.last_mut() {
            match line.split_once(':') {
                Some(("ESSID", ssid)) => {
                    collecting_ie = false;
                    entry.ssid = ssid.trim().trim_matches('"').to_string();
                }
                Some(("IE", ie)) => {
                    // The security header is itself an `IE:` line, so keep it
                    // rather than treating it as a mere start-of-run marker.
                    let ie = ie.trim();
                    collecting_ie = true;
                    if !ie.starts_with("Unknown") {
                        entry.information_element.push(ie.to_string());
                    }
                }
                Some((key, ie)) => {
                    if collecting_ie && !ie.trim().starts_with("Unknown") {
                        entry
                            .information_element
                            .push(format!("{key}: {}", ie.trim()));
                    } else {
                        collecting_ie = false;
                    }
                }
                None => {
                    if line.starts_with("Quality") {
                        collecting_ie = false;
                        entry.signal_level = line
                            .split_whitespace()
                            .nth(2)
                            .and_then(|part| part.split('=').nth(1))
                            .and_then(|level| level.split_whitespace().next())
                            .and_then(|level| level.trim().parse().ok())
                            .unwrap_or_default();
                    }
                }
            }
        }
    }

    entries
}

/// Bring the interface up and run `iwlist scan`, returning parsed scan cells.
pub(super) async fn wifi_scan(device: &str) -> Result<Vec<WifiScanItem>> {
    CommandUtils::call_ifconfig_cmd(&[device, "up"]).await?;
    let output = CommandUtils::call_iwlist_cmd(&[device, "scan"], SCAN_TIMEOUT.as_secs()).await?;

    Ok(parse_scanner_output(&output)
        .into_iter()
        .map(WifiScanItem::from)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_net_types::wifi::SignalStrength;

    #[test]
    fn test_parse_scanner_output() {
        let output = r#"wlan0     Scan completed :
          Cell 01 - Address: D2:60:A3:68:02:4A
                    Quality=56/70  Signal level=-54 dBm
                    Encryption key:on
                    ESSID:"Braiins AP"
                    IE: IEEE 802.11i/WPA2 Version 1
                        Authentication Suites (2) : PSK unknown (8)
          Cell 02 - Address: F4:1E:57:2F:3E:38
                    Quality=39/70  Signal level=-69 dBm
                    ESSID:"Bitcoin AP"
                    IE: Unknown: 0706435A04010D14
                    IE: WPA Version 1
                        Authentication Suites (1) : PSK
                    IE: IEEE 802.11i/WPA2 Version 1
                        Authentication Suites (1) : PSK
          Cell 03 - Address: C0:49:EF:16:09:15
                    Quality=36/70  Signal level=-74 dBm
                    Encryption key:off
                    ESSID:"Nakamoto""#;

        let items: Vec<WifiScanItem> = parse_scanner_output(output)
            .into_iter()
            .map(WifiScanItem::from)
            .collect();

        let braiins = items
            .iter()
            .find(|i| i.ssid == "Braiins AP")
            .expect("BUG: Braiins AP not parsed");
        assert_eq!(braiins.encryption_type, EncryptionType::Wpa2_3);
        assert_eq!(braiins.signal_strength(), SignalStrength::Excellent);

        let bitcoin = items
            .iter()
            .find(|i| i.ssid == "Bitcoin AP")
            .expect("BUG: Bitcoin AP not parsed");
        assert_eq!(bitcoin.encryption_type, EncryptionType::Wpa1_2);
        assert_eq!(bitcoin.signal_strength(), SignalStrength::Fair);

        let nakamoto = items
            .iter()
            .find(|i| i.ssid == "Nakamoto")
            .expect("BUG: Nakamoto not parsed");
        assert_eq!(nakamoto.encryption_type, EncryptionType::None);
    }

    /// A plain WPA2-PSK cell whose RSN element is the very first `IE:` line and
    /// whose sub-lines carry no encryption keyword must not be read as open.
    #[test]
    fn test_parse_leading_security_information_element() {
        let output = r#"wlan0     Scan completed :
          Cell 01 - Address: 0A:1B:2C:3D:4E:5F
                    Quality=48/70  Signal level=-62 dBm
                    Encryption key:on
                    ESSID:"Satoshi AP"
                    IE: IEEE 802.11i/WPA2 Version 1
                        Group Cipher : CCMP
                        Pairwise Ciphers (1) : CCMP
                        Authentication Suites (1) : PSK
          Cell 02 - Address: 1A:2B:3C:4D:5E:6F
                    Quality=40/70  Signal level=-70 dBm
                    Encryption key:on
                    ESSID:"Hal AP"
                    IE: WPA Version 1
                        Group Cipher : TKIP
                        Pairwise Ciphers (1) : TKIP
                        Authentication Suites (1) : PSK"#;

        let items: Vec<WifiScanItem> = parse_scanner_output(output)
            .into_iter()
            .map(WifiScanItem::from)
            .collect();

        let satoshi = items
            .iter()
            .find(|i| i.ssid == "Satoshi AP")
            .expect("BUG: Satoshi AP not parsed");
        assert_eq!(satoshi.encryption_type, EncryptionType::Wpa2);

        let hal = items
            .iter()
            .find(|i| i.ssid == "Hal AP")
            .expect("BUG: Hal AP not parsed");
        assert_eq!(hal.encryption_type, EncryptionType::Wpa);
    }
}
