// Copyright (C) 2025  Braiins Systems s.r.o.
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

use anyhow::{anyhow, Result};
use ii_net::wifi::WifiLinkState;

use crate::wifi::utils::CommandUtils;

pub struct WifiSta;

impl WifiSta {
    fn get_value_by_key(input: &str, key: &str) -> Result<String> {
        input
            .lines()
            .find_map(|line| {
                line.trim().starts_with(key).then(|| {
                    line.split_once(':')
                        .map(|(_key, value)| value.trim().to_owned())
                })
            })
            .flatten()
            .ok_or_else(|| anyhow!("No value found for '{key}' key"))
    }

    fn get_ssid(input: &str) -> Result<String> {
        let key = "SSID";
        let value = Self::get_value_by_key(input, key)?;
        str::parse::<String>(&value)
            .map_err(|_| anyhow!("Cannot parse {value} which is a value for '{key}' key"))
    }

    fn get_signal(input: &str) -> Result<i32> {
        let key = "signal";
        let value = Self::get_value_by_key(input, key)?.replace("dBm", "");
        str::parse::<i32>(value.trim())
            .map_err(|_| anyhow!("Cannot parse {value} which is a value for '{key}' key"))
    }

    pub async fn link_details(device: &str) -> Result<WifiLinkState> {
        let link_details = CommandUtils::call_iw_cmd(&["dev", device, "link"]).await?;
        Ok(WifiLinkState {
            ssid: Self::get_ssid(&link_details)?,
            signal_level: Self::get_signal(&link_details)?,
        })
    }
}

mod tests {
    #[test]
    fn test_parser() {
        let output = r"
Connected to 80:2a:a8:5a:05:36 (on wlan0)
	SSID: ubnt-ms
	freq: 2462
	RX: 1214 bytes (14 packets)
	TX: 3528 bytes (22 packets)
	signal: -58 dBm
";

        let ssid = crate::wifi::WifiSta::get_ssid(output).expect("BUG: SSID parsing error");
        let signal_level =
            crate::wifi::WifiSta::get_signal(output).expect("BUG: Signal parsing error");

        assert_eq!(ssid, "ubnt-ms".to_owned());
        assert_eq!(signal_level, -58);
    }
}
