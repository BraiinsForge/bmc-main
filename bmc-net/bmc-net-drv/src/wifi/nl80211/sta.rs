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

use anyhow::{Ok, Result, bail};
use futures::stream::TryStreamExt;
use ii_net::wifi::WifiLinkState;
use tracing::debug;
use wl_nl80211::{Nl80211Attr, Nl80211Handle, Nl80211StationInfo};

struct WifiInterfaceDetails {
    index: u32,
    ssid: String,
}

pub struct WifiSta;

impl WifiSta {
    async fn get_iface_details(
        handle: Nl80211Handle,
        ifname: &str,
    ) -> anyhow::Result<WifiInterfaceDetails> {
        let ifname_attr = Nl80211Attr::IfName(ifname.to_string());
        let mut interface_handle = handle.interface().get(Vec::new()).execute().await;

        let mut index = None;
        let mut ssid = None;
        while let Some(ifhandle) = interface_handle.try_next().await? {
            let attrs = ifhandle.payload.attributes;
            if attrs.iter().any(|attr| attr == &ifname_attr) {
                for attr in attrs {
                    match attr {
                        Nl80211Attr::IfIndex(i) => index = Some(i.to_owned()),
                        Nl80211Attr::Ssid(s) => ssid = Some(s),
                        _ => {}
                    }
                }
            }
        }

        debug!("Interface details: index={index:?}, ssid={ssid:?}");
        if let (Some(index), Some(ssid)) = (index, ssid) {
            return Ok(WifiInterfaceDetails { index, ssid });
        }
        bail!("Some details for interface {ifname} are missing")
    }

    pub async fn link_details(handle: Nl80211Handle, device: &str) -> Result<WifiLinkState> {
        let intf_details = Self::get_iface_details(handle.clone(), device).await?;

        let mut signal = None;
        let mut sta_msg = handle.station().dump(intf_details.index).execute().await;
        while let Some(station_handle) = sta_msg.try_next().await? {
            for attr in station_handle.payload.attributes {
                if let Nl80211Attr::StationInfo(sta_attrs) = attr {
                    for sta in sta_attrs {
                        if let Nl80211StationInfo::Signal(dbm) = sta {
                            signal = Some(dbm);
                        }
                    }
                }
            }
        }

        if let Some(signal) = signal {
            return Ok(WifiLinkState {
                ssid: intf_details.ssid,
                signal_level: signal.into(),
            });
        }
        bail!("Wifi signal not found")
    }
}
