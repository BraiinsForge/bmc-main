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

use std::collections::HashMap;
use std::path::PathBuf;
use std::str::{FromStr, from_utf8};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
/// KObject action types
///
/// See kobject_action in include/linux/kobject.h
pub enum ActionType {
    /// A new kobject is added
    Add,
    /// A kobject is removed
    Remove,
    /// the kobject changed its internal state
    ///
    /// the `env` contains kobject-specific information.
    Change,
    /// the kobject is reparented as a result of `kobject_move`
    ///
    /// the `env` contains `DEVPATH_OLD=<oldpath>`.
    Move,
    /// The device is back online after successful `device_offline`.
    Online,
    /// The device is ready to be hot-removed.
    Offline,
    /// The device is bound to a driver.
    Bind,
    /// The device is not bound to its driver anymore.
    Unbind,
    /// Button pressed
    Pressed,
    /// Button released
    Released,
}

impl FromStr for ActionType {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use ActionType::{
            Add, Bind, Change, Move, Offline, Online, Pressed, Released, Remove, Unbind,
        };
        match s {
            "add" => Ok(Add),
            "remove" => Ok(Remove),
            "change" => Ok(Change),
            "move" => Ok(Move),
            "online" => Ok(Online),
            "offline" => Ok(Offline),
            "bind" => Ok(Bind),
            "unbind" => Ok(Unbind),
            "pressed" => Ok(Pressed),
            "released" => Ok(Released),
            _ => anyhow::bail!("Unexpected action: {}", s),
        }
    }
}

/// Linux kernel userspace event
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UEvent {
    /// Action happening
    pub action: Option<ActionType>,
    /// Complete Kernel Object path
    pub devpath: Option<PathBuf>,
    /// SubSystem originating the event
    pub subsystem: Option<String>,
    /// Arguments
    pub env: HashMap<String, String>,
    /// Sequence number
    pub seq: Option<u64>,
    /// Button identification
    pub button: Option<String>,
}

/// Parse key=value strings as UEvent, some fields may be missing
fn parse_uevent_iter<'a>(iter: impl Iterator<Item = &'a str>) -> anyhow::Result<UEvent> {
    let mut action = None;
    let mut devpath = None;
    let mut subsystem = None;
    let mut env = HashMap::new();
    let mut button = None;
    let mut seq = None;

    for f in iter {
        if let Some((key, value)) = f.split_once('=') {
            match key {
                "ACTION" => action = Some(value.parse::<ActionType>()?),
                "DEVPATH" => devpath = Some(value.parse::<PathBuf>()?),
                "SUBSYSTEM" => subsystem = Some(value.to_owned()),
                "BUTTON" => button = Some(value.to_owned()),
                "SEQNUM" => seq = Some(value.parse::<u64>()?),
                _ => {}
            }
            let _ = env.insert(key.into(), value.into());
        }
    }

    Ok(UEvent {
        action,
        devpath,
        subsystem,
        env,
        seq,
        button,
    })
}

impl FromStr for UEvent {
    type Err = anyhow::Error;

    /// Parse a netlink string with space as separator into a UEvent
    fn from_str(pkt: &str) -> Result<Self, Self::Err> {
        let lines = from_utf8(pkt.as_ref())?.split(' ');
        parse_uevent_iter(lines)
    }
}

impl UEvent {
    /// Parse a netlink packet into a UEvent with '0' as separator
    pub fn from_netlink_packet(pkt: &[u8]) -> anyhow::Result<UEvent> {
        let lines = from_utf8(pkt)?.split('\0');
        parse_uevent_iter(lines)
    }
}

// test of the paring
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_uevent() {
        let uevent = "change@/devices/virtual/thermal/thermal_zone7 ACTION=change DEVPATH=/devices/virtual/thermal/thermal_zone7 SUBSYSTEM=thermal NAME=x86_pkg_temp TEMP=100000 TRIP=0 EVENT=0 SEQNUM=22552";
        let uevent = UEvent::from_str(uevent).expect("BUG: parsing failed");
        assert_eq!(uevent.action, Some(ActionType::Change));
        let uevent = "pressed@HOME=/PATH=/sbin:/bin:/usr/sbin:/usr/bin SUBSYSTEM=button ACTION=pressed BUTTON=BTN_0 SEEN=33 SEQNUM=1205";
        let uevent = UEvent::from_str(uevent).expect("BUG: parsing failed");
        assert_eq!(uevent.action, Some(ActionType::Pressed));
    }

    #[test]
    fn test_captured_buffer() {
        let packet: Vec<u8> = vec![
            112, 114, 101, 115, 115, 101, 100, 64, 0, 72, 79, 77, 69, 61, 47, 0, 80, 65, 84, 72,
            61, 47, 115, 98, 105, 110, 58, 47, 98, 105, 110, 58, 47, 117, 115, 114, 47, 115, 98,
            105, 110, 58, 47, 117, 115, 114, 47, 98, 105, 110, 0, 83, 85, 66, 83, 89, 83, 84, 69,
            77, 61, 98, 117, 116, 116, 111, 110, 0, 65, 67, 84, 73, 79, 78, 61, 112, 114, 101, 115,
            115, 101, 100, 0, 66, 85, 84, 84, 79, 78, 61, 66, 84, 78, 95, 48, 0, 83, 69, 69, 78,
            61, 49, 51, 48, 0, 83, 69, 81, 78, 85, 77, 61, 49, 50, 48, 53, 0,
        ];

        let uevent =
            UEvent::from_netlink_packet(&packet[..packet.len()]).expect("BUG: parsing failed");
        assert_eq!(uevent.action, Some(ActionType::Pressed));
    }
}
