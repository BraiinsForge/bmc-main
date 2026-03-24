// Copyright (C) 2026  Braiins Systems s.r.o.

use std::net::{IpAddr, Ipv4Addr};

use qrcode_generator::QrCodeEcc;
use slint::{Image, Rgb8Pixel, SharedPixelBuffer};

const IP_ADDR_CHAR_LIMIT: usize = 10;
const QR_CODE_URL_PREFIX: &str = "http://";
const QR_CODE_SIZE_SHORT: usize = 100;
const QR_CODE_SIZE_LONG: usize = 86;

/// Fixed IP address used in AP mode during initial setup
/// (configured in OpenWrt network layer).
pub const AP_IP: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 21));

#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "QR sizes are small constants (86, 100)"
)]
pub fn ip_as_qrcode(ip: Option<IpAddr>) -> Image {
    let qr_size = match ip {
        Some(ip) if ip.to_string().len() > IP_ADDR_CHAR_LIMIT => QR_CODE_SIZE_LONG,
        _ => QR_CODE_SIZE_SHORT,
    };

    let image_buffer: Vec<u8> = ip
        .and_then(|ip| {
            qrcode_generator::to_image_from_str(
                format!("{QR_CODE_URL_PREFIX}{ip}"),
                QrCodeEcc::Low,
                qr_size,
            )
            .ok()
        })
        .unwrap_or_else(|| vec![0; qr_size * qr_size])
        .into_iter()
        .flat_map(|x| [x, x, x])
        .collect();

    let shared_pixel_buf = SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(
        &image_buffer,
        qr_size as u32,
        qr_size as u32,
    );
    Image::from_rgb8(shared_pixel_buf)
}
