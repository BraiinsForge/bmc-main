// Copyright (C) 2026  Braiins Systems s.r.o.

// DRM framebuffer renderer for LED semicircles.

use crate::apa102::{LED_COUNT, Led};
use std::fs::OpenOptions;
use std::io;
use std::os::unix::io::AsRawFd;

const DISPLAY_W: usize = 480;
const DISPLAY_H: usize = 1280;
const STRIP_H: usize = 40;
const BYTES_PER_PIXEL: usize = 4;
const LED_SPACING: usize = 24;
const LED_STRIP_X_OFFSET: usize = 120;
const GLOW_RADIUS: f32 = 18.0;
const GLOW_DIAMETER: usize = 36;
const HALF_GLOW_DIAMETER: usize = 18;

#[derive(Debug)]
struct GlowSprite {
    width: usize,
    height: usize,
    alpha: Vec<u8>,
}

impl GlowSprite {
    fn new() -> Self {
        let width = GLOW_DIAMETER;
        let height = STRIP_H;
        let mut alpha = vec![0_u8; width * height];
        let cx = GLOW_RADIUS;

        for y in 0..height {
            #[expect(
                clippy::cast_precision_loss,
                reason = "glow dimensions are small constants"
            )]
            let dy = (height - y) as f32;
            for x in 0..width {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "glow dimensions are small constants"
                )]
                let dx = x as f32 - cx;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist < GLOW_RADIUS {
                    let t = dist / GLOW_RADIUS;
                    alpha[y * width + x] = alpha_from_unit((-t * t * 3.0).exp());
                }
            }
        }

        Self {
            width,
            height,
            alpha,
        }
    }

    fn get_alpha(&self, x: usize, y: usize) -> u8 {
        if x < self.width && y < self.height {
            self.alpha[y * self.width + x]
        } else {
            0
        }
    }
}

#[derive(Debug)]
pub struct Renderer {
    fb: *mut u8,
    fb_len: usize,
    stride: usize,
    sprite: GlowSprite,
    drm_fd: i32,
    fb_handle: u32,
}

#[repr(C)]
struct DrmModeGetFb {
    fb_id: u32,
    width: u32,
    height: u32,
    pitch: u32,
    bpp: u32,
    depth: u32,
    handle: u32,
}

#[repr(C)]
struct DrmModeMapDumb {
    handle: u32,
    pad: u32,
    offset: u64,
}

#[repr(C)]
struct DrmModeGetCrtc {
    set_connectors_ptr: u64,
    count_connectors: u32,
    crtc_id: u32,
    fb_id: u32,
    x: u32,
    y: u32,
    gamma_size: u32,
    mode_valid: u32,
    mode: [u8; 68],
}

#[repr(C)]
struct DrmModeGetResources {
    fb_id_ptr: u64,
    crtc_id_ptr: u64,
    connector_id_ptr: u64,
    encoder_id_ptr: u64,
    count_fbs: u32,
    count_crtcs: u32,
    count_connectors: u32,
    count_encoders: u32,
    min_width: u32,
    max_width: u32,
    min_height: u32,
    max_height: u32,
}

#[repr(C)]
struct DrmClipRect {
    x1: u16,
    y1: u16,
    x2: u16,
    y2: u16,
}

#[repr(C)]
struct DrmModeDirtyFb {
    fb_id: u32,
    flags: u32,
    color: u32,
    num_clips: u32,
    clips_ptr: u64,
}

const _: () = assert!(std::mem::size_of::<DrmModeGetFb>() == 28);
const _: () = assert!(std::mem::size_of::<DrmModeMapDumb>() == 16);
const _: () = assert!(std::mem::size_of::<DrmModeGetCrtc>() == 104);
const _: () = assert!(std::mem::size_of::<DrmModeGetResources>() == 64);
const _: () = assert!(std::mem::size_of::<DrmClipRect>() == 8);
const _: () = assert!(std::mem::size_of::<DrmModeDirtyFb>() == 24);

// Encode a DRM read-write ioctl number. The kernel ABI defines these as
// 32-bit values, but libc::Ioctl is i32 on musl/aarch64 and u64 on glibc.
// We compute in Ioctl-width to avoid platform-dependent cast lints.
const fn drm_iowr(nr: libc::Ioctl, size: libc::Ioctl) -> libc::Ioctl {
    (3 << 30) | (size << 16) | ((b'd' as libc::Ioctl) << 8) | nr
}

// All DRM structs are small (<64 bytes), so size_of always fits in Ioctl.
const DRM_IOCTL_MODE_GETRESOURCES: libc::Ioctl = drm_iowr(
    0xA0,
    std::mem::size_of::<DrmModeGetResources>() as libc::Ioctl,
);
const DRM_IOCTL_MODE_GETCRTC: libc::Ioctl =
    drm_iowr(0xA1, std::mem::size_of::<DrmModeGetCrtc>() as libc::Ioctl);
const DRM_IOCTL_MODE_GETFB: libc::Ioctl =
    drm_iowr(0xAD, std::mem::size_of::<DrmModeGetFb>() as libc::Ioctl);
const DRM_IOCTL_MODE_MAP_DUMB: libc::Ioctl =
    drm_iowr(0xB3, std::mem::size_of::<DrmModeMapDumb>() as libc::Ioctl);
const DRM_IOCTL_MODE_DIRTYFB: libc::Ioctl =
    drm_iowr(0xB1, std::mem::size_of::<DrmModeDirtyFb>() as libc::Ioctl);

impl Renderer {
    pub fn new(drm_path: &str) -> io::Result<Self> {
        let drm_file = OpenOptions::new().read(true).write(true).open(drm_path)?;
        let drm_fd = drm_file.as_raw_fd();
        std::mem::forget(drm_file);

        let mut res = unsafe { std::mem::zeroed::<DrmModeGetResources>() };
        if unsafe { libc::ioctl(drm_fd, DRM_IOCTL_MODE_GETRESOURCES, &mut res) } < 0 {
            return Err(io::Error::last_os_error());
        }

        let mut crtc_ids = vec![0_u32; res.count_crtcs as usize];
        res.crtc_id_ptr = crtc_ids.as_mut_ptr() as u64;
        if unsafe { libc::ioctl(drm_fd, DRM_IOCTL_MODE_GETRESOURCES, &mut res) } < 0 {
            return Err(io::Error::last_os_error());
        }

        let mut fb_id = 0_u32;
        for &crtc_id in &crtc_ids {
            let mut crtc = unsafe { std::mem::zeroed::<DrmModeGetCrtc>() };
            crtc.crtc_id = crtc_id;
            if unsafe { libc::ioctl(drm_fd, DRM_IOCTL_MODE_GETCRTC, &mut crtc) } >= 0
                && crtc.fb_id != 0
            {
                fb_id = crtc.fb_id;
                break;
            }
        }

        if fb_id == 0 {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no active framebuffer found on DRM device",
            ));
        }

        let mut fb_info = unsafe { std::mem::zeroed::<DrmModeGetFb>() };
        fb_info.fb_id = fb_id;
        if unsafe { libc::ioctl(drm_fd, DRM_IOCTL_MODE_GETFB, &mut fb_info) } < 0 {
            return Err(io::Error::last_os_error());
        }

        let stride = fb_info.pitch as usize;
        let fb_len = stride * fb_info.height as usize;

        eprintln!(
            "LED renderer: fb {}x{} pitch={} bpp={} handle={}",
            fb_info.width, fb_info.height, fb_info.pitch, fb_info.bpp, fb_info.handle
        );

        let mut map_dumb = DrmModeMapDumb {
            handle: fb_info.handle,
            pad: 0,
            offset: 0,
        };
        if unsafe { libc::ioctl(drm_fd, DRM_IOCTL_MODE_MAP_DUMB, &mut map_dumb) } < 0 {
            return Err(io::Error::last_os_error());
        }

        let fb = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                fb_len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                drm_fd,
                #[expect(
                    clippy::cast_possible_wrap,
                    reason = "DRM offset won't exceed i64::MAX"
                )]
                (map_dumb.offset as i64),
            )
        };

        if fb == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }

        Ok(Self {
            fb: fb.cast::<u8>(),
            fb_len,
            stride,
            sprite: GlowSprite::new(),
            drm_fd,
            fb_handle: fb_id,
        })
    }

    pub fn render(&mut self, leds: &[Led; LED_COUNT]) {
        let any_on = leds.iter().any(|l| !l.is_off());

        for y in DISPLAY_H..DISPLAY_H + STRIP_H {
            let row_start = y * self.stride;
            if row_start + DISPLAY_W * BYTES_PER_PIXEL <= self.fb_len {
                unsafe {
                    std::ptr::write_bytes(self.fb.add(row_start), 0, DISPLAY_W * BYTES_PER_PIXEL);
                }
            }
        }

        if !any_on {
            self.dirty_fb();
            return;
        }

        for (i, led) in leds.iter().enumerate() {
            if led.is_off() {
                continue;
            }

            let cx = LED_STRIP_X_OFFSET + i * LED_SPACING + 12;
            let sprite_x_start = cx.saturating_sub(HALF_GLOW_DIAMETER);
            let r = scale_channel(led.r, led.brightness);
            let g = scale_channel(led.g, led.brightness);
            let b = scale_channel(led.b, led.brightness);

            for sy in 0..self.sprite.height {
                let fb_y = DISPLAY_H + sy;
                for sx in 0..self.sprite.width {
                    let fb_x = sprite_x_start + sx;
                    if fb_x >= DISPLAY_W {
                        continue;
                    }

                    let alpha = self.sprite.get_alpha(sx, sy);
                    if alpha == 0 {
                        continue;
                    }

                    let offset = fb_y * self.stride + fb_x * BYTES_PER_PIXEL;
                    if offset + 3 >= self.fb_len {
                        continue;
                    }

                    unsafe {
                        let ptr = self.fb.add(offset);
                        *ptr = apply_alpha(b, alpha);
                        *ptr.add(1) = apply_alpha(g, alpha);
                        *ptr.add(2) = apply_alpha(r, alpha);
                        *ptr.add(3) = 0;
                    }
                }
            }
        }

        self.dirty_fb();
    }

    fn dirty_fb(&self) {
        let clip = DrmClipRect {
            x1: 0,
            #[expect(
                clippy::cast_possible_truncation,
                reason = "display constants fit in u16"
            )]
            y1: DISPLAY_H as u16,
            #[expect(
                clippy::cast_possible_truncation,
                reason = "display constants fit in u16"
            )]
            x2: DISPLAY_W as u16,
            #[expect(
                clippy::cast_possible_truncation,
                reason = "display constants fit in u16"
            )]
            y2: (DISPLAY_H + STRIP_H) as u16,
        };
        let mut dirty = DrmModeDirtyFb {
            fb_id: self.fb_handle,
            flags: 0,
            color: 0,
            num_clips: 1,
            clips_ptr: std::ptr::from_ref(&clip).addr() as u64,
        };
        unsafe {
            libc::ioctl(self.drm_fd, DRM_IOCTL_MODE_DIRTYFB, &mut dirty);
        }
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "alpha values are clamped into the 0..=255 byte range"
)]
fn alpha_from_unit(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn scale_channel(channel: u8, brightness: u8) -> u8 {
    #[expect(
        clippy::integer_division,
        reason = "APA102 brightness uses a 5-bit integer scale"
    )]
    let scaled = (u16::from(channel) * u16::from(brightness)) / 31;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "255*31/31 = 255, fits in u8"
    )]
    let result = scaled as u8;
    result
}

fn apply_alpha(channel: u8, alpha: u8) -> u8 {
    #[expect(
        clippy::integer_division,
        reason = "framebuffer alpha blending is intentionally quantized to 8-bit channels"
    )]
    let scaled = (u16::from(channel) * u16::from(alpha)) / 255;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "255*255/255 = 255, fits in u8"
    )]
    let result = scaled as u8;
    result
}
