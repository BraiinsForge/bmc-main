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

use std::net::Ipv4Addr;

use bmc_system_overlay::Snapshot;
use bmc_wasm_host::slot::widget_network_info;

fn online_snapshot(signal_dbm: i32) -> Snapshot {
    Snapshot {
        ipv4: Some(Ipv4Addr::new(10, 0, 0, 7)),
        station_ipv4: Some(Ipv4Addr::new(10, 0, 0, 7)),
        station_ssid: Some("deck-net".to_owned()),
        wifi_signal_dbm: Some(signal_dbm),
    }
}

#[test]
fn signal_only_change_projects_identically() {
    assert_eq!(
        widget_network_info(&online_snapshot(-50)),
        widget_network_info(&online_snapshot(-62)),
        "widgets cannot observe the signal level, so a dBm-only snapshot bump must \
         compare equal and never wake them — per-second RSSI jitter re-rendering \
         every widget is the BDK-658 bug"
    );
}

#[test]
fn ssid_or_ip_change_projects_differently() {
    let base = online_snapshot(-50);
    let ip_changed = Snapshot {
        ipv4: Some(Ipv4Addr::new(10, 0, 0, 8)),
        ..base.clone()
    };
    let ssid_changed = Snapshot {
        station_ssid: Some("other-net".to_owned()),
        ..base.clone()
    };
    assert_ne!(
        widget_network_info(&base),
        widget_network_info(&ip_changed),
        "an IP change is widget-visible and must reach the delivery hook"
    );
    assert_ne!(
        widget_network_info(&base),
        widget_network_info(&ssid_changed),
        "an SSID change is widget-visible and must reach the delivery hook"
    );
}

#[test]
fn offline_projects_empty_strings() {
    let offline = Snapshot {
        ipv4: None,
        station_ipv4: None,
        station_ssid: None,
        wifi_signal_dbm: None,
    };
    let info = widget_network_info(&offline);
    assert_eq!(
        (info.ssid.as_str(), info.ip.as_str()),
        ("", ""),
        "unknown network state is empty strings per the NetworkInfo contract, \
         matching the runtime's initial value so startup delivers no spurious update"
    );
}
