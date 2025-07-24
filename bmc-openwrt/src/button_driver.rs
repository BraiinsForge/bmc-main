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

//! Provides Unix implementation for button primitives

use anyhow::{Context, Result, anyhow};
use async_stream::stream;
use bmc_button::{ButtonEvent, ButtonEventStream, ButtonId, Buttons};
use futures::StreamExt;
use gpiod::EdgeDetect;
use log::warn;
use merge_streams::MergeStreams;
use netlink_sys::{
    AsyncSocket, AsyncSocketExt, SocketAddr, TokioSocket, protocols::NETLINK_KOBJECT_UEVENT,
};
use std::time::Duration;
use std::{future, process};
pub use sysfs_gpio;

#[derive(Clone, Debug)]
pub struct UEventButtons;

/// Implementation of `Buttons` trait for `UEventButton`
impl Buttons for UEventButtons {
    fn to_stream(&self) -> Result<ButtonEventStream> {
        let mut socket = TokioSocket::new(NETLINK_KOBJECT_UEVENT)?;
        let socket_addr = SocketAddr::new(process::id(), 1);
        socket.socket_mut().bind(&socket_addr)?;
        Ok(Box::new(stream! {
            loop {
                let (buf, _) = socket.recv_from_full().await?;

                // Ignore parsing errors, we don't need to handle every event
                if let Ok(uevent) = bmc_kobject::UEvent::from_netlink_packet(&buf[..]) {
                    log::debug!("new uevent: {uevent:?}");
                    let button_type = uevent
                         .button.and_then(|b| match b.as_str() {
                             "reset" => Some(ButtonId::Reset),
                             _ => None
                         });

                    // Skip unknown buttons
                    if let Some(button_type) = button_type {
                        match uevent.action {
                            Some(bmc_kobject::ActionType::Pressed) => {
                                yield Ok(ButtonEvent::Pressed(button_type))
                            }
                            Some(bmc_kobject::ActionType::Released) => {
                                yield Ok(ButtonEvent::Released(button_type))
                            }
                            _ => {}
                        }
                    }
                }
            }
        })
        .boxed())
    }
}

#[derive(Clone, Debug)]
pub struct SysfsButton {
    pub button_number: u64,
    pub button_id: ButtonId,
    pub edge: sysfs_gpio::Edge,
}

impl SysfsButton {
    const HIGH: u8 = 1;
    const LOW: u8 = 0;
    const WAIT_DURATION: Duration = Duration::from_millis(100);

    fn both_edges_stream(pin: sysfs_gpio::Pin, button_id: ButtonId) -> Result<ButtonEventStream> {
        Ok(pin
            .get_value_stream()?
            .take_while(|event| future::ready(event.is_ok()))
            .map(move |event| match event {
                Ok(Self::LOW) => Ok(ButtonEvent::Pressed(button_id.clone())),
                Ok(Self::HIGH) => Ok(ButtonEvent::Released(button_id.clone())),
                _ => Err(anyhow::anyhow!("Invalid button event")),
            })
            .boxed())
    }

    /// Custom stream for rising edge detection.
    /// In rising edge detection, we need to wait for button to be released before emitting release event,
    /// so we create custom stream that emits release event after button is released.
    fn rising_edge_stream(
        pin: sysfs_gpio::Pin,
        button_id: ButtonId,
    ) -> std::pin::Pin<
        std::boxed::Box<
            (
                dyn futures::Stream<
                        Item = std::result::Result<bmc_button::ButtonEvent, anyhow::Error>,
                    > + std::marker::Send
                    + 'static
            ),
        >,
    > {
        Box::new(stream! {
            while let Some(event) = pin.get_value_stream()?.next().await {
                log::debug!("new event: {event:?}");
                match event {
                    Ok(Self::LOW) => {
                        // Ignore immediate press events
                        if let Ok(Self::HIGH) = pin.get_value() {
                            log::debug!("Ignoring too quick press event for button: {pin:?}");
                            // End the match block to avoid emitting events
                            continue;
                        }

                        // Emit press event
                        yield Ok(ButtonEvent::Pressed(button_id.clone()));

                        // Wait for button to be released or error to occur before emitting release event
                        while let Ok(Self::LOW) = pin.get_value() {
                            tokio::time::sleep(Self::WAIT_DURATION).await;
                        }
                        // Emit release event
                        yield Ok(ButtonEvent::Released(button_id.clone()));
                    },
                    Ok(Self::HIGH) => {
                        // Ignore release events that are invalid in single edge detection without error
                        log::debug!("Ignoring release event for button: {pin:?}");
                    },
                    _ => yield Err(anyhow::anyhow!("Invalid button event")),
                }
            }
        })
        .boxed()
    }
}

#[derive(Clone, Debug)]
pub struct SysfsButtons {
    pub list: Vec<SysfsButton>,
}

/// Implementation of `Buttons` trait for `SysfsButton`
impl Buttons for SysfsButton {
    fn to_stream(&self) -> Result<ButtonEventStream> {
        let button_id = self.button_id.clone();
        let pin = sysfs_gpio::Pin::new(self.button_number);
        pin.set_edge(self.edge)
            .context(format!("Can't set edge for button: {self:?}"))?;
        match self.edge {
            sysfs_gpio::Edge::BothEdges => Self::both_edges_stream(pin, button_id),
            sysfs_gpio::Edge::RisingEdge => Ok(Self::rising_edge_stream(pin, button_id)),
            sysfs_gpio::Edge::FallingEdge | sysfs_gpio::Edge::NoInterrupt => {
                Err(anyhow!("Falling edge or no interrupt not supported"))
            }
        }
    }
}

/// Implementation of `Buttons` trait for `SysfsButtons`
impl Buttons for SysfsButtons {
    fn to_stream(&self) -> Result<ButtonEventStream> {
        let list = self.list.clone();
        Ok(list
            .into_iter()
            .map(|button| button.to_stream())
            .collect::<Result<Vec<_>>>()?
            .merge()
            .boxed())
    }
}

#[derive(Clone, Debug)]
pub struct GpiodButton {
    pub button_name: &'static str,
    pub button_id: ButtonId,
}

#[derive(Clone, Debug)]
pub struct GpiodButtons {
    pub list: Vec<GpiodButton>,
}

/// Implementation of `Buttons` trait for `GpiodButtons`
impl Buttons for GpiodButtons {
    fn to_stream(&self) -> Result<ButtonEventStream> {
        // Find chip and line_id for all buttons
        let chip_line_vec = self
            .list
            .iter()
            .map(|button| button.button_name)
            .map(bmc_gpio::PinGpiod::find_pin)
            .collect::<Result<Vec<_>>>()?;

        // Map button id to  chip and line
        let chip_line_vec = chip_line_vec
            .into_iter()
            .zip(self.list.clone())
            .collect::<Vec<_>>();

        // Check that chip is the same for all buttons
        let chip = &chip_line_vec[0].0.0;
        assert!(
            chip_line_vec
                .iter()
                .all(|((chip, _), _)| chip.name() == chip.name()),
            "BUG: All buttons must be on the same chip"
        );

        // Get line_id for all buttons
        let lines = chip_line_vec
            .iter()
            .map(|((_, line_id), _)| *line_id)
            .collect::<Vec<_>>();

        // Request lines for all buttons
        let options = gpiod::Options::input(&lines).edge(EdgeDetect::Both);
        let mut lines = chip.request_lines(options)?;

        Ok(Box::new(stream! {
            while let Some(event) = lines.next() {
                if let Ok(event) = event {
                    // Get line_id from BitId
                    let event_line_id = lines.lines().get(event.line as usize);
                    let Some(event_line_id) = event_line_id else {
                        warn!("Can not find button for the line_id: {event:?}");
                        continue;
                    };

                    // Get button_id from line_id
                    let button_id = chip_line_vec
                        .iter()
                        .find(|((_, line_id), _)| line_id == event_line_id)
                        .map(|(_, button)| button.button_id.clone());
                    let Some(button_id) = button_id else {
                        warn!("Can not find button for the line_id: {event:?}");
                        continue;
                    };

                    yield Ok(match event.edge {
                        gpiod::Edge::Rising => ButtonEvent::Released(button_id),
                        gpiod::Edge::Falling => ButtonEvent::Pressed(button_id),
                    })
                }
            }
        })
        .boxed())
    }
}

// Test for UeventButton
#[cfg(test)]
mod tests {
    use bmc_gpio::PinGpiod;
    use futures::StreamExt;
    use gpiod::{Chip, EdgeDetect, Options};

    /// Test for sysfs_gpio button driver
    #[tokio::test]
    #[ignore]
    async fn test_sysfs_gpio() {
        loop {
            let button_number = 26;
            let pin = sysfs_gpio::Pin::new(button_number);
            pin.set_edge(sysfs_gpio::Edge::BothEdges)
                .expect("BUG: Can't set edge");
            let mut pin_stream = pin.get_value_stream().expect("BUG: Can't get value stream");
            while let Some(event) = pin_stream.next().await {
                println!("event: {event:?}");
            }
        }
    }

    /// Test for gpiod
    #[test]
    #[ignore]
    fn test_gpiod() {
        println!("list:");
        for device in Chip::list_devices().expect("BUG: Can't list devices") {
            println!("device: {device:?}");
            let chip = Chip::new(device).expect("BUG: Can't create chip");
            // Loop through all lines
            let chiplabel = chip.label();
            for line in 0..chip.num_lines() {
                println!("line: {line:?} chiplabel: {chiplabel:?}");
                // Check if the name of the current line is same as defined PinIndex
                if let Ok(name) = chip.line_info(line) {
                    println!("chiplabel {chiplabel} line: {line}, name: {name}");
                }
            }
        }
        println!("list end");

        let (chip, line_id) = PinGpiod::find_pin("USR_BTN").expect("BUG: Can't find pin");

        println!("chip_id: {chip}, line_id: {line_id}");

        let options = Options::input([line_id]).edge(EdgeDetect::Both);
        let mut inputs = chip
            .request_lines(options)
            .expect("BUG: Can't request lines");

        println!("starting loop");
        loop {
            let event = inputs.read_event().expect("BUG: Can't read event");
            println!("event: {event:?}");
        }
    }
}
