// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::Result;
use bmc_display::proxy::{Proxy, ProxyEvent};
use core::time::Duration;
use drm::Device;
use drm::buffer::{Buffer, DrmFourcc};
use drm::control::Device as ControlDevice;
use drm::control::{self, AtomicCommitFlags, atomic, connector, crtc, property};
use evdev::{AbsoluteAxisCode, EventSummary, KeyCode, SynchronizationCode};
use slint::LogicalPosition;
use slint::platform::software_renderer::{RenderingRotation, Rgb565Pixel};
use slint::platform::{EventLoopProxy, Platform, software_renderer::MinimalSoftwareWindow};
use slint::platform::{PointerEventButton, WindowEvent};
use std::iter;
use std::rc::Rc;
use std::sync::Arc;
use tokio::sync::Notify;
use tracing::{info, trace};

const PIXEL_FORMAT: DrmFourcc = DrmFourcc::Rgb565;
const BITS_PER_PIXEL: u32 = 16;
const BYTES_PER_PIXEL: usize = 2;
const IDLE_TICK_MAX: Duration = Duration::from_millis(16);

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
        Self::open("/dev/dri/card1")
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
    event_receiver: flume::Receiver<ProxyEvent>,
    touch_activity: Arc<Notify>,
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
            .expect("BUG: Unable to request UniversalPlanes capability");
        card.set_client_capability(drm::ClientCapability::Atomic, true)
            .expect("BUG: Unable to request Atomic capability");

        // Load the information.
        let res = card
            .resource_handles()
            .expect("BUG: Could not load normal resource ids.");
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
            .expect("BUG: No connected connectors");

        // Get the first (usually best) mode
        let &mode = con
            .modes()
            .first()
            .expect("BUG: No modes found on connector");

        // Find a crtc and FB
        let crtc = crtcinfo.first().expect("BUG: No crtcs found");

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
            .expect("BUG: Could not create dumb buffer");
        // Map it and black it out.
        {
            let mut map = card
                .map_dumb_buffer(&mut db)
                .expect("BUG: Could not map dumbbuffer");
            for b in map.as_mut() {
                *b = 0;
            }
        }

        // Create an FB:
        let fb = card
            .add_framebuffer(&db, BITS_PER_PIXEL, BITS_PER_PIXEL)
            .expect("BUG: Could not create FB");

        let planes = card.plane_handles().expect("BUG: Could not list planes");
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
            .expect("BUG: Could not get props of connector")
            .as_hashmap(&card)
            .expect("BUG: Could not get a prop from connector");
        let crtc_props = card
            .get_properties(crtc.handle())
            .expect("BUG: Could not get props of crtc")
            .as_hashmap(&card)
            .expect("BUG: Could not get a prop from crtc");
        let plane_props = card
            .get_properties(plane)
            .expect("BUG: Could not get props of plane")
            .as_hashmap(&card)
            .expect("BUG: Could not get a prop from plane");

        let mut atomic_req = atomic::AtomicModeReq::new();
        atomic_req.add_property(
            con.handle(),
            con_props["CRTC_ID"].handle(),
            property::Value::CRTC(Some(crtc.handle())),
        );
        let blob = card
            .create_property_blob(&mode)
            .expect("BUG: Failed to create blob");
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

        // Defer the modeset until the first frame is rendered, so the kernel
        // splash screen on fb0 stays visible during application init.
        let mut pending_modeset = Some(atomic_req);

        #[expect(clippy::integer_division)]
        let pixel_stride = db.pitch() as usize / BYTES_PER_PIXEL;
        let mut map = card
            .map_dumb_buffer(&mut db)
            .expect("BUG: Could not map dumbbuffer");
        let (_, frame_buffer, _) = unsafe { map.align_to_mut::<Rgb565Pixel>() };

        let mut in_memory_buffer = frame_buffer.to_vec();

        let mut touch = evdev::Device::open("/dev/input/event0")
            .map_err(|e| slint::PlatformError::OtherError(Box::new(e)))?;
        touch.set_nonblocking(true).ok();

        // Event loop is inspired by official `linuxkms` implementation.
        // https://github.com/slint-ui/slint/blob/b80d5a23042c866fbc6d82d00c633bfda9057dd2/internal/backends/linuxkms/calloop_backend.rs#L249
        // Steps:
        // - update timers and animations
        // - run events
        // - redraw
        // - wait for next event:
        //   - only if animations are not active
        //   - timeout on the closest timer tick

        let mut saved_proxy_event: Option<ProxyEvent> = None;

        let mut pos = (0.0, 0.0);
        let mut pressed = false;
        let mut was_pressed = false; // track edge transitions

        // This loop condition is just a double check, in case sender is dropped before
        // sending ProxyEvent::Quit
        'outer: while !self.event_receiver.is_disconnected() {
            // Update timers and animations
            slint::platform::update_timers_and_animations();

            // Call all events invoked from other threads
            let proxy_events = {
                let saved = saved_proxy_event.take().into_iter();
                let lazy_drain = iter::once_with(|| self.event_receiver.drain()).flatten();

                saved.chain(lazy_drain)
            };

            for proxy_event in proxy_events {
                match proxy_event {
                    ProxyEvent::Event(event) => event(),
                    ProxyEvent::Quit => break 'outer,
                }
            }

            if let Ok(events) = touch.fetch_events() {
                for ev in events {
                    #[expect(clippy::wildcard_enum_match_arm)]
                    match ev.destructure() {
                        EventSummary::AbsoluteAxis(_, AbsoluteAxisCode::ABS_X, value) => {
                            #[expect(clippy::cast_precision_loss)]
                            let value = value as f32;
                            pos.0 = value;
                        }
                        EventSummary::AbsoluteAxis(_, AbsoluteAxisCode::ABS_Y, value) => {
                            #[expect(clippy::cast_precision_loss)]
                            let value = value as f32;
                            pos.1 = value;
                        }
                        EventSummary::Key(_, KeyCode::BTN_TOUCH, value) => {
                            pressed = value == 1;
                            if pressed {
                                self.touch_activity.notify_waiters();
                            }
                        }
                        EventSummary::Synchronization(_, SynchronizationCode::SYN_REPORT, _) => {
                            let logical_pos = LogicalPosition::new(pos.0, pos.1);

                            self.window.dispatch_event(WindowEvent::PointerMoved {
                                position: logical_pos,
                            });

                            if pressed && !was_pressed {
                                self.window.dispatch_event(WindowEvent::PointerPressed {
                                    position: logical_pos,
                                    button: PointerEventButton::Left,
                                });
                            } else if !pressed && was_pressed {
                                self.window.dispatch_event(WindowEvent::PointerReleased {
                                    position: logical_pos,
                                    button: PointerEventButton::Left,
                                });
                            }

                            was_pressed = pressed;
                        }
                        _ => {}
                    }
                }
            }

            // Render the display only if needed
            self.window.draw_if_needed(|renderer| {
                trace!("Rendering display");
                renderer.set_rendering_rotation(self.rotation);

                // We need to clear in_memory_buffer each time to get rid of leftovers/artifacts.
                // Not sure why, since we are using `RepaintBufferType::NewBuffer` anyway
                in_memory_buffer.fill(Rgb565Pixel::default());

                // We are rendering into in_memory_buffer first to avoid flickering on the display,
                // because looks like Slint is rendering layers (z-index and alpha?) one by one.
                // This behaviour does not work well in DRM without double buffering,
                // because immediate states are rendered as well, not only the final result
                renderer.render(&mut in_memory_buffer, pixel_stride);
                frame_buffer.copy_from_slice(&in_memory_buffer);
            });

            // Perform the DRM modeset after the first frame has been rendered into the
            // buffer, so the display transitions directly from the kernel splash to the
            // Slint UI without a black flash.
            if let Some(req) = pending_modeset.take() {
                card.atomic_commit(AtomicCommitFlags::ALLOW_MODESET, req)
                    .expect("BUG: Failed to set mode");
            }

            // Do not sleep when there are active animations (as mentioned in `duration_until_next_timer_update` docs)
            if self.window.has_active_animations() {
                std::thread::sleep(Duration::from_millis(1));
                // If there will be performance issues, we can introduce small delay, like in official `android-activity` implementation.
                // https://github.com/slint-ui/slint/blob/b80d5a23042c866fbc6d82d00c633bfda9057dd2/internal/backends/android-activity/lib.rs#L102
                continue;
            }

            let timeout = slint::platform::duration_until_next_timer_update()
                .map_or(IDLE_TICK_MAX, |t| t.min(IDLE_TICK_MAX));

            // Wait for the next event, error is ignored because it is timeout/close.
            saved_proxy_event = self.event_receiver.recv_timeout(timeout).ok();
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
    pub fn new(
        width: usize,
        height: usize,
        rotation: RenderingRotation,
        touch_activity: Arc<Notify>,
    ) -> Result<Self> {
        info!("Creating linux framebuffer platform");
        let window = MinimalSoftwareWindow::new(
            slint::platform::software_renderer::RepaintBufferType::NewBuffer,
        );

        let (event_sender, event_receiver) = flume::unbounded();

        Ok(Self {
            window,
            width,
            height,
            rotation,
            proxy: Box::new(Proxy::new(event_sender)),
            event_receiver,
            touch_activity,
        })
    }
}
