// Copyright (C) 2026  Braiins Systems s.r.o.

#[expect(
    clippy::wildcard_imports,
    reason = "render code uses many SDK exports and macros in one file"
)]
use bmc_wasm_sdk::*;

use crate::device::{DeviceList, family_label};

#[must_use]
pub fn view(devices: &DeviceList, _width: u32, _height: u32) -> Node {
    if devices.is_empty() {
        return col(
            props!(background: BLACK),
            [center(
                props!(flex: 1.0),
                [text(
                    "Searching for miners\u{2026}",
                    style!(size: 28, color: WHITE),
                )],
            )],
        );
    }

    let mut children: Vec<Node> = vec![text(
        fmt!("{} miners", devices.len()),
        style!(size: 28, weight: FontWeight::BOLD, color: WHITE),
    )];

    for dev in devices.iter_reachable() {
        children.push(row(
            props!(gap: 12.0, cross_align: CrossAlign::Center),
            [
                text(
                    dev.identity.name.clone(),
                    style!(size: 20, color: WHITE, flex: 1.0),
                ),
                text(
                    dev.identity.host.clone(),
                    style!(size: 20, color: GRAY_40, align: TextAlign::Right),
                ),
                text(
                    family_label(dev.identity.family),
                    style!(size: 20, color: GRAY_40, align: TextAlign::Right),
                ),
            ],
        ));
    }

    col(
        props!(background: BLACK, inset_top: 16.0, inset_left: 16.0, inset_right: 16.0, gap: 8.0),
        children,
    )
}
