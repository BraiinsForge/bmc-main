// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::Result;
use bmc_display::proxy::Proxy;
use memmap2::MmapOptions;
use std::fs::OpenOptions;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;
use tracing::{debug, info};

use slint::platform::software_renderer::{PremultipliedRgbaColor, TargetPixel};
use slint::platform::{EventLoopProxy, Platform, software_renderer::MinimalSoftwareWindow};

const SLEEP_DURATION: Duration = Duration::from_secs(1);
const FRAME_BUFFER_PATH: &str = "/dev/fb0";
const BYTES_PER_PIXEL: usize = 2;

pub struct LinuxFbPlatform {
    window: Rc<MinimalSoftwareWindow>,
    width: usize,
    height: usize,
    proxy: Box<Proxy>,
    event_receiver: flume::Receiver<Box<dyn FnOnce() + Send>>,
}

impl Platform for LinuxFbPlatform {
    fn create_window_adapter(
        &self,
    ) -> std::result::Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }

    fn run_event_loop(&self) -> std::result::Result<(), slint::PlatformError> {
        info!("Running linux frame buffer platform event loop");
        let fb_path = Path::new(FRAME_BUFFER_PATH);
        let fb_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(fb_path)
            .map_err(|e| {
                slint::PlatformError::from(format!("Unable to open linux framebuffer file, {}", e))
            })?;
        let mut fb_mmap = unsafe {
            MmapOptions::new()
                .len(BYTES_PER_PIXEL * self.width * self.height)
                .map_mut(&fb_file)
                .map_err(|e| {
                    slint::PlatformError::from(format!(
                        "Unable to memory map linux framebuffer, {}",
                        e
                    ))
                })?
        };
        // FIXME: swapped red and blue colors on display
        let (_, frame_buffer, _) = unsafe { fb_mmap.align_to_mut::<Bgr565Pixel>() };
        let mut first_start = true;
        // Check if we should terminate the event loop
        // HACK: the loop should be terminated immediately
        // after the quit_loop is set to true not after SLEEP_DURATION
        while !self
            .proxy
            .quit_loop
            .load(std::sync::atomic::Ordering::Acquire)
        {
            // Call all events invoked from other threads
            self.event_receiver.drain().for_each(|event| event());

            // Update timers and animations
            slint::platform::update_timers_and_animations();

            // Render the display only if needed
            self.window.draw_if_needed(|renderer| {
                // In MiniMiner we show boot screen during boot. We want to first think
                // that will render the screen with actual data. So we skip the first rendering
                // of the display and render it only after the first request for a redraw.
                if first_start {
                    first_start = false;
                    return;
                }
                debug!("Rendering display");
                renderer.render(frame_buffer, self.width);
            });
            // Wait for the next event or sleep for a while, error is ignored because it is timeout
            let _ = self
                .event_receiver
                .recv_timeout(SLEEP_DURATION)
                .map(|event| event());
        }
        Ok(())
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(self.proxy.clone())
    }
}

impl LinuxFbPlatform {
    pub fn new(width: usize, height: usize) -> Result<Self> {
        info!("Creating linux framebuffer platform");
        let window = MinimalSoftwareWindow::new(
            slint::platform::software_renderer::RepaintBufferType::ReusedBuffer,
        );

        let (event_sender, event_receiver) = flume::unbounded();

        Ok(Self {
            window,
            width,
            height,
            proxy: Box::new(Proxy::new(event_sender)),
            event_receiver,
        })
    }
}

#[repr(transparent)]
#[derive(Clone, Copy)]
struct Bgr565Pixel(pub u16);

impl Bgr565Pixel {
    const B_MASK: u16 = 0b1111_1000_0000_0000;
    const G_MASK: u16 = 0b0000_0111_1110_0000;
    const R_MASK: u16 = 0b0000_0000_0001_1111;
}

impl TargetPixel for Bgr565Pixel {
    fn blend(&mut self, color: PremultipliedRgbaColor) {
        let a = (u8::MAX - color.alpha) as u32;
        // convert to 5 bits
        let a = (a + 4) >> 3;

        // 00000ggg_ggg00000_bbbbb000_000rrrrr
        let expanded = (self.0 & (Self::B_MASK | Self::R_MASK)) as u32
            | (((self.0 & Self::G_MASK) as u32) << 16);

        // gggggggg_000bbbbb_bbb000rr_rrrrrr00
        let c =
            ((color.blue as u32) << 13) | ((color.green as u32) << 24) | ((color.red as u32) << 2);
        // gggggg00_000bbbbb_000000rr_rrr00000
        let c = c & 0b11111100_00011111_00000011_11100000;

        let res = expanded * a + c;

        self.0 = ((res >> 21) as u16 & Self::G_MASK)
            | ((res >> 5) as u16 & (Self::B_MASK | Self::R_MASK));
    }

    fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self(((b as u16 & 0b11111000) << 8) | ((g as u16 & 0b11111100) << 3) | (r as u16 >> 3))
    }
}
