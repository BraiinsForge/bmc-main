// Copyright (C) 2025  Braiins Systems s.r.o.

use std::net::IpAddr;

use qrcode_generator::QrCodeEcc;
use slint::{Image, Rgb8Pixel, SharedPixelBuffer};

// QR code for IP address longer than this limit will use different size
const IP_ADDR_CHAR_LIMIT: usize = 10;
const QR_CODE_URL_PREFIX: &str = "http://";
// NOTE: Shorter string should have more points in QR code in order to have the same size as the QR code generated from longer string
const QR_CODE_SIZE_SHORT: u32 = 100;
const QR_CODE_SIZE_LONG: u32 = 86;

pub(crate) fn ip_as_qrcode(ip: Option<IpAddr>) -> Image {
    let qr_size = match ip {
        Some(ip) if ip.to_string().len() > IP_ADDR_CHAR_LIMIT => QR_CODE_SIZE_LONG,
        _ => QR_CODE_SIZE_SHORT,
    };

    let image_buffer: Vec<u8> = ip
        .and_then(|ip| {
            qrcode_generator::to_image_from_str(
                format!("{QR_CODE_URL_PREFIX}{ip}"),
                QrCodeEcc::Low,
                qr_size as usize,
            )
            .ok()
        })
        .unwrap_or_else(|| vec![0; (qr_size * qr_size) as usize])
        // Extend image buffer to fit to the 3 Bytes per RGB pixel image
        .into_iter()
        .flat_map(|x| vec![x, x, x])
        .collect();

    let shared_pixel_buf =
        SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(&image_buffer, qr_size, qr_size);
    Image::from_rgb8(shared_pixel_buf)
}
