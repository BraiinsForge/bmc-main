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
use std::io::BufRead;
use tokio::process::Command;

#[derive(Debug)]
pub struct Esp32Sdio;

const CLI_COMMAND: &str = "esp32-sdio-cli";
const GET_AP_SCAN_LIST: &str = "get_ap_scan_list";

#[derive(Debug, PartialEq)]
pub struct Ap {
    pub ssid: String,
    pub rssi: i32,
    pub auth: AuthMode,
}

impl Esp32Sdio {
    pub async fn get_ap_scan_list() -> Result<Vec<Ap>> {
        let output = Command::new(CLI_COMMAND)
            .arg(GET_AP_SCAN_LIST)
            .output()
            .await?;

        let networks = output
            .stdout
            .lines()
            .filter_map(|line| {
                line.ok()
                    .and_then(|network| Self::parse_ap_scan_line(&network))
            })
            .collect::<Vec<Ap>>();

        Ok(networks)
    }

    // line is in a format: "SSID: Public-wifi rssi: -53 auth: 3"
    pub(crate) fn parse_ap_scan_line(line: &str) -> Option<Ap> {
        let parts: Vec<&str> = line.split_ascii_whitespace().collect();

        if parts.len() < 6 {
            return None;
        }

        let ssid = parts[1].to_owned();
        let rssi: i32 = parts[3].parse().ok()?;
        let auth: AuthMode = parts[5].parse::<u8>().ok()?.into();

        Some(Ap { ssid, rssi, auth })
    }
}

// source: https://github.com/espressif/esp-hosted/blob/30f4082314b6e13d869e9bdff7949fa428713337/esp_hosted_fg/docs/common/ctrl_apis.md
#[derive(Debug, PartialEq)]
pub enum AuthMode {
    Open,
    Wep,
    WpaPsk,
    Wpa2Psk,
    WpaWpa2Psk,
    Wpa2Enterprise,
    Wpa3Psk,
    Wpa2Wpa3Psk,
    Unknown,
}

impl From<u8> for AuthMode {
    fn from(value: u8) -> Self {
        match value {
            0 => AuthMode::Open,
            1 => AuthMode::Wep,
            2 => AuthMode::WpaPsk,
            3 => AuthMode::Wpa2Psk,
            4 => AuthMode::WpaWpa2Psk,
            5 => AuthMode::Wpa2Enterprise,
            6 => AuthMode::Wpa3Psk,
            7 => AuthMode::Wpa2Wpa3Psk,
            _ => AuthMode::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parsing_of_the_ap_output() {
        let output = [
            "SSID: MiniMinerTest rssi: -42 auth: 3",
            "SSID: ca-fi rssi: -53 auth: 3",
            "SSID: Braiins-Public rssi: -53 auth: 3",
            "SSID: Braiins-Backup rssi: -66 auth: 3",
            "SSID: Braiins-Public rssi: -69 auth: 1",
            "SSID: Braiins-Public rssi: -71 auth: 2",
            "SSID: Braiins-Public rssi: -75 auth: 4",
        ];

        let result = output
            .iter()
            .map(|line| Esp32Sdio::parse_ap_scan_line(line))
            .collect::<Vec<Option<Ap>>>();

        let expected = vec![
            Some(Ap {
                ssid: String::from("MiniMinerTest"),
                rssi: -42,
                auth: AuthMode::Wpa2Psk,
            }),
            Some(Ap {
                ssid: String::from("ca-fi"),
                rssi: -53,
                auth: AuthMode::Wpa2Psk,
            }),
            Some(Ap {
                ssid: String::from("Braiins-Public"),
                rssi: -53,
                auth: AuthMode::Wpa2Psk,
            }),
            Some(Ap {
                ssid: String::from("Braiins-Backup"),
                rssi: -66,
                auth: AuthMode::Wpa2Psk,
            }),
            Some(Ap {
                ssid: String::from("Braiins-Public"),
                rssi: -69,
                auth: AuthMode::Wep,
            }),
            Some(Ap {
                ssid: String::from("Braiins-Public"),
                rssi: -71,
                auth: AuthMode::WpaPsk,
            }),
            Some(Ap {
                ssid: String::from("Braiins-Public"),
                rssi: -75,
                auth: AuthMode::WpaWpa2Psk,
            }),
        ];

        assert_eq!(result, expected);
    }
}
