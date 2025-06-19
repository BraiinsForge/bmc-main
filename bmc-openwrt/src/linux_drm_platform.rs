// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::Result;
use bmc_display::proxy::Proxy;
use drm::Device;
use drm::buffer::{Buffer, DrmFourcc};
use drm::control::Device as ControlDevice;
use drm::control::{self, AtomicCommitFlags, atomic, connector, crtc, property};
use slint::platform::software_renderer::{RenderingRotation, Rgb565Pixel};
use slint::platform::{EventLoopProxy, Platform, software_renderer::MinimalSoftwareWindow};
use std::rc::Rc;
use std::time::Duration;
use tracing::{debug, info};

const SLEEP_DURATION: Duration = Duration::from_secs(1);
const PIXEL_FORMAT: DrmFourcc = DrmFourcc::Rgb565;
const BITS_PER_PIXEL: u32 = 16;
const BYTES_PER_PIXEL: usize = 2;

#[derive(Debug)]
/// A simple wrapper for a device node.
pub struct Card(std::fs::File);

/// Simple helper methods for opening a `Card`.
impl Card {
    pub fn open(path: &str) -> std::result::Result<Self, std::io::Error> {
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        options.write(true);
        Ok(Card(options.open(path)?))
    }

    pub fn open_global() -> std::result::Result<Self, std::io::Error> {
        Self::open("/dev/dri/card0")
    }
}

impl std::os::unix::io::AsFd for Card {
    fn as_fd(&self) -> std::os::unix::io::BorrowedFd<'_> {
        self.0.as_fd()
    }
}

/// With `AsFd` implemented, we can now implement `drm::Device`.
impl Device for Card {}
impl ControlDevice for Card {}

#[expect(missing_debug_implementations)]
pub struct LinuxDrmPlatform {
    window: Rc<MinimalSoftwareWindow>,
    width: usize,
    height: usize,
    rotation: RenderingRotation,
    proxy: Box<Proxy>,
    event_receiver: flume::Receiver<Box<dyn FnOnce() + Send>>,
}

impl Platform for LinuxDrmPlatform {
    fn create_window_adapter(
        &self,
    ) -> std::result::Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }

    #[expect(clippy::too_many_lines)]
    fn run_event_loop(&self) -> std::result::Result<(), slint::PlatformError> {
        info!("Running linux DRM platform event loop");
        let card =
            Card::open_global().map_err(|e| slint::PlatformError::OtherError(Box::new(e)))?;

        card.set_client_capability(drm::ClientCapability::UniversalPlanes, true)
            .expect("Unable to request UniversalPlanes capability");
        card.set_client_capability(drm::ClientCapability::Atomic, true)
            .expect("Unable to request Atomic capability");

        // Load the information.
        let res = card
            .resource_handles()
            .expect("Could not load normal resource ids.");
        let coninfo: Vec<connector::Info> = res
            .connectors()
            .iter()
            .flat_map(|con| card.get_connector(*con, true))
            .collect();
        let crtcinfo: Vec<crtc::Info> = res
            .crtcs()
            .iter()
            .flat_map(|crtc| card.get_crtc(*crtc))
            .collect();

        // Filter each connector until we find one that's connected.
        let con = coninfo
            .iter()
            .find(|&i| i.state() == connector::State::Connected)
            .expect("No connected connectors");

        // Get the first (usually best) mode
        let &mode = con.modes().first().expect("No modes found on connector");

        // Find a crtc and FB
        let crtc = crtcinfo.first().expect("No crtcs found");

        // Create a DB
        // If buffer resolution is above display resolution, a ENOSPC (not enough GPU memory) error may
        // occur
        let mut db = card
            .create_dumb_buffer(
                #[expect(clippy::cast_possible_truncation)]
                (self.width as u32, self.height as u32),
                PIXEL_FORMAT,
                BITS_PER_PIXEL,
            )
            .expect("Could not create dumb buffer");
        // Map it and black it out.
        {
            let mut map = card
                .map_dumb_buffer(&mut db)
                .expect("Could not map dumbbuffer");
            for b in map.as_mut() {
                *b = 0;
            }
        }

        // Create an FB:
        let fb = card
            .add_framebuffer(&db, BITS_PER_PIXEL, BITS_PER_PIXEL)
            .expect("Could not create FB");

        let planes = card.plane_handles().expect("Could not list planes");
        let (better_planes, compatible_planes): (
            Vec<control::plane::Handle>,
            Vec<control::plane::Handle>,
        ) = planes
            .iter()
            .filter(|&&plane| {
                card.get_plane(plane)
                    .map(|plane_info| {
                        let compatible_crtcs = res.filter_crtcs(plane_info.possible_crtcs());
                        compatible_crtcs.contains(&crtc.handle())
                    })
                    .unwrap_or(false)
            })
            .partition(|&&plane| {
                if let Ok(props) = card.get_properties(plane) {
                    for (&id, &val) in props.iter() {
                        if let Ok(info) = card.get_property(id) {
                            if info.name().to_str().map(|x| x == "type").unwrap_or(false) {
                                return val
                                    == control::property::RawValue::from(
                                        control::PlaneType::Primary as u32,
                                    );
                            }
                        }
                    }
                }
                false
            });
        let plane = *better_planes.first().unwrap_or(&compatible_planes[0]);

        let con_props = card
            .get_properties(con.handle())
            .expect("Could not get props of connector")
            .as_hashmap(&card)
            .expect("Could not get a prop from connector");
        let crtc_props = card
            .get_properties(crtc.handle())
            .expect("Could not get props of crtc")
            .as_hashmap(&card)
            .expect("Could not get a prop from crtc");
        let plane_props = card
            .get_properties(plane)
            .expect("Could not get props of plane")
            .as_hashmap(&card)
            .expect("Could not get a prop from plane");

        let mut atomic_req = atomic::AtomicModeReq::new();
        atomic_req.add_property(
            con.handle(),
            con_props["CRTC_ID"].handle(),
            property::Value::CRTC(Some(crtc.handle())),
        );
        let blob = card
            .create_property_blob(&mode)
            .expect("Failed to create blob");
        atomic_req.add_property(crtc.handle(), crtc_props["MODE_ID"].handle(), blob);
        atomic_req.add_property(
            crtc.handle(),
            crtc_props["ACTIVE"].handle(),
            property::Value::Boolean(true),
        );
        atomic_req.add_property(
            plane,
            plane_props["FB_ID"].handle(),
            property::Value::Framebuffer(Some(fb)),
        );
        atomic_req.add_property(
            plane,
            plane_props["CRTC_ID"].handle(),
            property::Value::CRTC(Some(crtc.handle())),
        );
        atomic_req.add_property(
            plane,
            plane_props["SRC_X"].handle(),
            property::Value::UnsignedRange(0),
        );
        atomic_req.add_property(
            plane,
            plane_props["SRC_Y"].handle(),
            property::Value::UnsignedRange(0),
        );
        atomic_req.add_property(
            plane,
            plane_props["SRC_W"].handle(),
            property::Value::UnsignedRange((self.width as u64) << 16),
        );
        atomic_req.add_property(
            plane,
            plane_props["SRC_H"].handle(),
            property::Value::UnsignedRange((self.height as u64) << 16),
        );
        atomic_req.add_property(
            plane,
            plane_props["CRTC_X"].handle(),
            property::Value::SignedRange(0),
        );
        atomic_req.add_property(
            plane,
            plane_props["CRTC_Y"].handle(),
            property::Value::SignedRange(0),
        );
        atomic_req.add_property(
            plane,
            plane_props["CRTC_W"].handle(),
            property::Value::UnsignedRange(self.width as u64),
        );
        atomic_req.add_property(
            plane,
            plane_props["CRTC_H"].handle(),
            property::Value::UnsignedRange(self.height as u64),
        );

        // Set the crtc
        // On many setups, this requires root access.
        card.atomic_commit(AtomicCommitFlags::ALLOW_MODESET, atomic_req)
            .expect("Failed to set mode");

        #[expect(clippy::integer_division)]
        let pixel_stride = db.pitch() as usize / BYTES_PER_PIXEL;
        let mut map = card
            .map_dumb_buffer(&mut db)
            .expect("Could not map dumbbuffer");
        let (_, frame_buffer, _) = unsafe { map.align_to_mut::<Rgb565Pixel>() };

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
                debug!("Rendering display");
                renderer.set_rendering_rotation(self.rotation);
                renderer.render(frame_buffer, pixel_stride);
            });
            // Wait for the next event or sleep for a while, error is ignored because it is timeout
            let _ = self
                .event_receiver
                .recv_timeout(SLEEP_DURATION)
                .map(|event| event());
        }

        drop(map);
        card.destroy_framebuffer(fb)
            .map_err(|e| slint::PlatformError::OtherError(Box::new(e)))?;
        card.destroy_dumb_buffer(db)
            .map_err(|e| slint::PlatformError::OtherError(Box::new(e)))?;
        Ok(())
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(self.proxy.clone())
    }
}

impl LinuxDrmPlatform {
    pub fn new(width: usize, height: usize, rotation: RenderingRotation) -> Result<Self> {
        info!("Creating linux framebuffer platform");
        let window = MinimalSoftwareWindow::new(
            slint::platform::software_renderer::RepaintBufferType::ReusedBuffer,
        );

        let (event_sender, event_receiver) = flume::unbounded();

        Ok(Self {
            window,
            width,
            height,
            rotation,
            proxy: Box::new(Proxy::new(event_sender)),
            event_receiver,
        })
    }
}
