// Copyright (C) 2026  Braiins Systems s.r.o.

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "screen code uses many SDK builders, macros, and tokens"
    )
)]
use bmc_wasm_sdk::*;

use crate::screens::parts::{
    BACK_CHIP, FRAME_H, FRAME_W, LABEL, LABEL_FONT, LINK, PAD, ROW_FONT, TITLE_FONT,
};

const QR_SIZE: f32 = 240.0;

#[derive(Debug)]
pub struct NoCredentialsData {
    pub fleet_name: String,
    /// The network the Deck is on, so a phone/PC user knows which to join.
    pub ssid: String,
    /// The Deck web-app URL — encoded in the QR and written out to type.
    pub url: String,
}

#[must_use]
pub fn no_credentials_view(data: &NoCredentialsData) -> Node {
    col(
        props!(background: BLACK, width: FRAME_W, height: FRAME_H, padding: PAD, gap: 16.0),
        [
            header(&data.fleet_name),
            center(
                props!(flex: 1.0),
                [col(
                    props!(gap: 24.0, cross_align: CrossAlign::Center),
                    [
                        canvas(
                            props!(width: QR_SIZE, height: QR_SIZE),
                            [Draw::qr(0.0, 0.0, QR_SIZE, &data.url, QrStyle::default())],
                        ),
                        text(
                            "Scan to open the Deck web app and add BOS/uBOS credentials",
                            style!(size: TITLE_FONT, color: WHITE, align: TextAlign::Center),
                        ),
                        network_hint(&data.ssid, &data.url),
                    ],
                )],
            ),
        ],
    )
}

fn header(fleet_name: &str) -> Node {
    row(
        props!(height: BACK_CHIP, cross_align: CrossAlign::Center, gap: 12.0),
        [
            text(
                fleet_name,
                style!(size: TITLE_FONT, weight: FontWeight::BOLD, color: WHITE),
            ),
            text("No credentials", style!(size: LABEL_FONT, color: LABEL)),
        ],
    )
}

/// The same-network path for a phone or PC: join this Wi-Fi, open this link.
fn network_hint(ssid: &str, url: &str) -> Node {
    col(
        props!(gap: 6.0, cross_align: CrossAlign::Center),
        [
            text(
                fmt!("On the network \u{201c}{ssid}\u{201d}"),
                style!(size: ROW_FONT, color: LABEL),
            ),
            text(url, style!(size: ROW_FONT, color: LINK)),
        ],
    )
}
