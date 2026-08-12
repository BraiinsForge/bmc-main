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

use anyhow::bail;
use bmc_net_types::wifi::{EncryptionType, WifiScanItem};
use log::warn;
use serde::{self, Deserialize};
use serde_json::json;

use crate::wifi::utils::{CommandUtils, filter_sort_by_strongest_signal};

#[derive(Deserialize, Debug, Clone, Default)]
struct WifiScanEncryptionJson {
    enabled: bool,
    #[serde(default)]
    wep: Vec<String>,
    #[serde(default)]
    wpa: Vec<i32>,
}

#[derive(Deserialize, Clone)]
struct WifiScanResultJson {
    #[serde(default)]
    ssid: String,
    #[serde(default)]
    mode: String,
    #[serde(default)]
    signal: i32,
    #[serde(default)]
    encryption: WifiScanEncryptionJson,
}

#[derive(Deserialize, Clone)]
struct WifiScanResultJsonContainer {
    results: Vec<WifiScanResultJson>,
}

pub struct WifiScanner;

impl WifiScanner {
    pub(crate) async fn wifi_scan(device: &str) -> Result<Vec<WifiScanItem>, anyhow::Error> {
        let device_ubus_param = json!({"device": device}).to_string();
        // Ensure that interface is up (Later we should ensure that uci wireless config has SSIDs disabled but radio enabled and use wifi up instead)
        if let Err(e) = CommandUtils::call_ifconfig_cmd(&[device, "up"]).await {
            warn!("Cannot put {device} interface up: {e}");
        }

        let scan_result =
            CommandUtils::call_ubus_cmd(&["call", "iwinfo", "scan", &device_ubus_param]).await?;

        let container = parse_scan_result(&scan_result)?;

        Ok(filter_sort_by_strongest_signal(process_json_entry(
            container,
        )))
    }
}

fn parse_scan_result(scan_result: &str) -> anyhow::Result<WifiScanResultJsonContainer> {
    serde_json::from_str::<WifiScanResultJsonContainer>(scan_result)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Converts the scanned access points, skipping (with a warning) any single
/// entry that fails to convert rather than failing the whole scan. This keeps
/// one exotic neighbouring AP (e.g. an unrecognized encryption combination)
/// from blanking the entire network list, matching the esp32 backend.
fn process_json_entry(container: WifiScanResultJsonContainer) -> Vec<WifiScanItem> {
    container
        .results
        .into_iter()
        .filter(|item| item.mode == "Master")
        .filter_map(|item| {
            let ssid = item.ssid.clone();
            WifiScanItem::try_from(item)
                .inspect_err(|e| warn!("skipping access point {ssid:?} in scan: {e}"))
                .ok()
        })
        .collect()
}

impl TryFrom<WifiScanResultJson> for WifiScanItem {
    type Error = anyhow::Error;

    fn try_from(item: WifiScanResultJson) -> Result<Self, Self::Error> {
        Ok(WifiScanItem::new(
            item.ssid,
            item.signal,
            item.encryption.try_into()?,
        ))
    }
}

impl TryFrom<WifiScanEncryptionJson> for EncryptionType {
    type Error = anyhow::Error;

    fn try_from(mut item: WifiScanEncryptionJson) -> Result<Self, Self::Error> {
        if !item.enabled {
            return Ok(Self::None);
        }

        if !item.wep.is_empty() {
            match item.wep[0].as_str() {
                "shared" => Ok(Self::WepShared),
                "open" => Ok(Self::Wep),
                _ => {
                    let msg = format!("WEP encryption not recognized: {item:?}");
                    warn!("{msg}");
                    bail!(msg)
                }
            }
        } else if !item.wpa.is_empty() {
            item.wpa.sort_unstable();
            match item.wpa.as_slice() {
                [1] => Ok(Self::Wpa),
                [2] => Ok(Self::Wpa2),
                [3] | [1, 2, 3] => Ok(Self::Wpa3),
                [1, 2] => Ok(Self::Wpa1_2),
                [2, 3] => Ok(Self::Wpa2_3),
                _ => {
                    let msg = format!("WPA encryption not recognized: {item:?}");
                    warn!("{msg}");
                    bail!(msg)
                }
            }
        } else {
            let msg = format!("Encryption not recognized: {item:?}");
            warn!("{msg}");
            bail!(msg)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wifi::utils::{filter_empty_ssid, filter_unsupported_enc};
    use anyhow::Result;
    use bmc_net_types::wifi::SignalStrength;

    #[test]
    fn test_filter_with_empty_ssid() {
        let entries: Result<Vec<WifiScanItem>> = vec![
            WifiScanResultJson {
                mode: "Master".to_owned(),
                signal: -80,
                encryption: WifiScanEncryptionJson {
                    enabled: true,
                    wep: Vec::new(),
                    wpa: vec![2],
                },
                ssid: "test".to_owned(),
            },
            WifiScanResultJson {
                mode: "Master".to_owned(),
                signal: -50,
                encryption: WifiScanEncryptionJson {
                    enabled: true,
                    wep: Vec::new(),
                    wpa: vec![2],
                },
                ssid: String::new(),
            },
        ]
        .into_iter()
        .map(WifiScanItem::try_from)
        .collect();

        let entries: Vec<WifiScanItem> = entries
            .expect("BUG: Error in struct conversion")
            .into_iter()
            .filter(filter_empty_ssid)
            .collect();

        assert_eq!(1, entries.len());
    }

    #[test]
    fn test_filter_unsupported_enc() {
        let entries: Result<Vec<WifiScanItem>> = vec![
            WifiScanResultJson {
                mode: "Master".to_owned(),
                signal: -80,
                encryption: WifiScanEncryptionJson {
                    enabled: true,
                    wep: Vec::new(),
                    wpa: vec![2],
                },
                ssid: "test".to_owned(),
            },
            WifiScanResultJson {
                mode: "Master".to_owned(),
                signal: -50,
                encryption: WifiScanEncryptionJson {
                    enabled: true,
                    wep: Vec::new(),
                    wpa: vec![3],
                },
                ssid: "test2".to_owned(),
            },
            WifiScanResultJson {
                mode: "Master".to_owned(),
                signal: -50,
                encryption: WifiScanEncryptionJson {
                    enabled: true,
                    wep: Vec::new(),
                    wpa: vec![1, 2, 3],
                },
                ssid: "test3".to_owned(),
            },
        ]
        .into_iter()
        .map(WifiScanItem::try_from)
        .collect();

        let entries: Vec<WifiScanItem> = entries
            .expect("BUG: Error in struct conversion")
            .into_iter()
            .filter(filter_unsupported_enc)
            .collect();

        assert_eq!(1, entries.len());
    }

    #[test]
    fn test_filter_sort_by_strongest_signal() {
        let entries: Result<Vec<WifiScanItem>> = vec![
            WifiScanResultJson {
                mode: "Master".to_owned(),
                signal: -80,
                encryption: WifiScanEncryptionJson {
                    enabled: true,
                    wep: Vec::new(),
                    wpa: vec![2],
                },
                ssid: "test".to_owned(),
            },
            WifiScanResultJson {
                mode: "Master".to_owned(),
                signal: -50,
                encryption: WifiScanEncryptionJson {
                    enabled: true,
                    wep: Vec::new(),
                    wpa: vec![2],
                },
                ssid: "test".to_owned(),
            },
        ]
        .into_iter()
        .map(WifiScanItem::try_from)
        .collect();

        let entries = entries.expect("BUG: Error in struct conversion");

        assert_eq!(
            -50,
            filter_sort_by_strongest_signal(entries)[0].signal_level
        );
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "the body is one captured ubus JSON payload, not branching logic"
    )]
    fn test_parse_scanner_output() {
        let output = r#"{
                    "results": [
                        {
                            "ssid": "ubnt-ms",
                            "bssid": "80:2A:A8:5A:05:36",
                            "mode": "Master",
                            "channel": 11,
                            "signal": -73,
                            "quality": 37,
                            "quality_max": 70,
                            "encryption": {
                                "enabled": true,
                                "wpa": [
                                    1
                                ],
                                "authentication": [
                                    "psk"
                                ],
                                "ciphers": [
                                    "ccmp"
                                ]
                            }
                        },
                        {
                            "ssid": "braiins_service",
                            "bssid": "36:55:49:1F:A8:DF",
                            "mode": "Master",
                            "channel": 7,
                            "signal": -37,
                            "quality": 70,
                            "quality_max": 70,
                            "encryption": {
                                "enabled": true,
                                "wpa": [
                                    2,
                                    3
                                ],
                                "authentication": [
                                    "psk",
                                    "sae"
                                ],
                                "ciphers": [
                                    "ccmp"
                                ]
                            }
                        },
                        {
                            "ssid": "Vodafone-2EB2",
                            "bssid": "34:2C:C4:49:54:61",
                            "mode": "Master",
                            "channel": 1,
                            "signal": -84,
                            "quality": 26,
                            "quality_max": 70,
                            "encryption": {
                                "enabled": true,
                                "wpa": [
                                    3,
                                    2
                                ],
                                "authentication": [
                                    "psk"
                                ],
                                "ciphers": [
                                    "ccmp"
                                ]
                            }
                        },
                        {
                            "ssid": "braiins_servicexxx",
                            "bssid": "2A:9E:EA:66:09:9F",
                            "mode": "Master",
                            "channel": 11,
                            "signal": -33,
                            "quality": 70,
                            "quality_max": 70,
                            "encryption": {
                                "enabled": false
                            }
                        },
                        {
                            "ssid": "TestWEP",
                            "bssid": "DC:41:A9:36:E9:8A",
                            "mode": "Master",
                            "channel": 6,
                            "signal": -35,
                            "quality": 70,
                            "quality_max": 70,
                            "encryption": {
                                "enabled": true,
                                "wep": [
                                    "open",
                                    "shared"
                                ],
                                "ciphers": [
                                    "wep-40",
                                    "wep-104"
                                ]
                            }
                        },
                        {
                            "bssid": "DC:41:A9:36:E9:8A",
                            "mode": "Master",
                            "channel": 6,
                            "signal": 0,
                            "quality": 70,
                            "quality_max": 70,
                            "encryption": {
                                "enabled": true,
                                "wep": [
                                    "open",
                                    "shared"
                                ],
                                "ciphers": [
                                    "wep-40",
                                    "wep-104"
                                ]
                            }
                        }
                    ]
                }"#;

        let scan_parsed: WifiScanResultJsonContainer =
            serde_json::from_str(output).expect("BUG: Test failed!");

        assert!(scan_parsed.results.iter().any(|e| {
            e.ssid == "braiins_service"
                && EncryptionType::try_from(e.encryption.clone()).expect("BUG: Error")
                    == EncryptionType::Wpa2_3
        }));

        let cell = scan_parsed
            .clone()
            .results
            .into_iter()
            .find(|e| e.ssid == "ubnt-ms")
            .and_then(|e| WifiScanItem::try_from(e).ok())
            .expect("BUG: Failed to parse cell");
        assert_eq!(cell.ssid, "ubnt-ms");
        assert_eq!(cell.encryption_type, EncryptionType::Wpa);
        assert_eq!(cell.signal_strength(), SignalStrength::Fair);

        let cell = scan_parsed
            .clone()
            .results
            .into_iter()
            .find(|e| e.ssid == "Vodafone-2EB2")
            .and_then(|e| WifiScanItem::try_from(e).ok())
            .expect("BUG: Failed to parse cell");
        assert_eq!(cell.ssid, "Vodafone-2EB2");
        assert_eq!(cell.encryption_type, EncryptionType::Wpa2_3);
        assert_eq!(cell.signal_strength(), SignalStrength::Low);

        let cell = scan_parsed
            .clone()
            .results
            .into_iter()
            .find(|e| e.ssid == "braiins_service")
            .and_then(|e| WifiScanItem::try_from(e).ok())
            .expect("BUG: Failed to parse cell");
        assert_eq!(cell.ssid, "braiins_service");
        assert_eq!(cell.encryption_type, EncryptionType::Wpa2_3);
        assert_eq!(cell.signal_strength(), SignalStrength::Excellent);

        let cell = scan_parsed
            .clone()
            .results
            .into_iter()
            .find(|e| e.ssid == "TestWEP")
            .and_then(|e| WifiScanItem::try_from(e).ok())
            .expect("BUG: Failed to parse cell");
        assert_eq!(cell.ssid, "TestWEP");
        assert_eq!(cell.encryption_type, EncryptionType::Wep);
        assert_eq!(cell.signal_strength(), SignalStrength::Excellent);

        let cell = scan_parsed
            .clone()
            .results
            .into_iter()
            .find(|e| e.ssid.is_empty())
            .and_then(|e| WifiScanItem::try_from(e).ok())
            .expect("BUG: Failed to parse cell");
        assert_eq!(cell.ssid, ""); // Intentionally empty SSID
        assert_eq!(cell.encryption_type, EncryptionType::Wep);
        assert_eq!(cell.signal_strength(), SignalStrength::Offline);

        let res = filter_sort_by_strongest_signal(
            scan_parsed
                .clone()
                .results
                .into_iter()
                .filter_map(|e| WifiScanItem::try_from(e).ok())
                .collect::<Vec<WifiScanItem>>(),
        );
        assert_eq!(res.len(), 6);
        assert_eq!(res[0].signal_level, 0);
        assert_eq!(res[0].ssid, ""); // Intentionally empty SSID
        assert_eq!(res[1].signal_level, -33);
        assert_eq!(res[1].ssid, "braiins_servicexxx");
        assert_eq!(res[3].signal_level, -37);
    }
}
