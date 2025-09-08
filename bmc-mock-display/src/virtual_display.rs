// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::mock_backlight_driver::MockBacklightDriver;
use anyhow::{Context, Result};
use bmc_display::display_driver::DisplayBacklightDriver;
use bmc_display::proxy::ProxyEvent;
use bmc_display::{
    display_controller::{DisplayController, WindowHandle},
    display_driver::DisplayDriver,
    metadata::{DisplayMetadata, ResolutionMetadata, UsizeMetadata},
};
use minifb;
use slint::LogicalPosition;
use slint::platform::software_renderer::PremultipliedRgbaColor;
use slint::platform::{EventLoopProxy, Platform, software_renderer::MinimalSoftwareWindow};
use slint::platform::{PointerEventButton, WindowEvent};
use std::cell::RefCell;
use std::iter;
use std::rc::Rc;
use tracing::{info, trace};

const WINDOW_TITLE: &str = "BMC - display mockup";
const WINDOW_TITLE_OFF: &str = "OFF";

#[derive(Debug)]
pub struct VirtualDisplay;

impl VirtualDisplay {
    pub fn create() -> Result<(WindowHandle, DisplayDriver<MockBacklightDriver>)> {
        let brightness = UsizeMetadata::new(18, 0, 20);
        let resolution = ResolutionMetadata::new(1280, 480);
        let display_metadata = DisplayMetadata::new(brightness, resolution);

        let backlight_driver = MockBacklightDriver::new(
            false,
            u8::try_from(display_metadata.brightness.default)?,
            u8::try_from(display_metadata.brightness.max)?,
        );

        if cfg!(feature = "winit-skia") {
            info!("Using winit skia renderer");
        } else if cfg!(feature = "winit-software") {
            info!("Using winit software renderer");
        } else {
            info!("Using minifb software renderer");

            slint::platform::set_platform(Box::new(VirtualDisplayPlatform::new(
                display_metadata.resolution.width as usize,
                display_metadata.resolution.height as usize,
                backlight_driver.clone(),
            )?))
            .expect("BUG: Cannot set platform");
        }

        let (display_controller, main_window) = DisplayController::create(
            display_metadata.resolution.width,
            display_metadata.resolution.height,
        )
        .context("Cannot initialize ui")?;

        let display_driver = DisplayDriver::init(backlight_driver, display_controller)?;

        Ok((main_window, display_driver))
    }
}

#[expect(missing_debug_implementations)]
pub struct VirtualDisplayPlatform {
    window: Rc<MinimalSoftwareWindow>,
    minifb: RefCell<minifb::Window>,
    buffer: RefCell<Vec<PremultipliedRgbaColor>>,
    width: usize,
    height: usize,
    proxy: Box<bmc_display::proxy::Proxy>,
    event_receiver: flume::Receiver<ProxyEvent>,
    backlight_driver: MockBacklightDriver,
}

impl Platform for VirtualDisplayPlatform {
    fn create_window_adapter(
        &self,
    ) -> std::result::Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }

    fn run_event_loop(&self) -> std::result::Result<(), slint::PlatformError> {
        info!("Running virtual display event loop");
        let mut saved_proxy_event: Option<ProxyEvent> = None;
        let mut was_pressed = false;

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

            // This is a workaround to detect close window event as soon as possible.
            // `update` and `update_with_buffer` shouldn't be typically called both in one cycle
            {
                let mut minifb = self.minifb.borrow_mut();
                minifb.update();

                if !minifb.is_open() {
                    break 'outer;
                }

                // inside your event loop:
                if let Some((x, y)) = minifb.get_mouse_pos(minifb::MouseMode::Discard) {
                    let pos = LogicalPosition::new(x, y);

                    let pressed = minifb.get_mouse_down(minifb::MouseButton::Left);

                    // always send move (Slint uses this for drags)
                    self.window
                        .dispatch_event(WindowEvent::PointerMoved { position: pos });

                    if pressed && !was_pressed {
                        // just pressed
                        self.window.dispatch_event(WindowEvent::PointerPressed {
                            position: pos,
                            button: PointerEventButton::Left,
                        });
                    } else if !pressed && was_pressed {
                        // just released
                        self.window.dispatch_event(WindowEvent::PointerReleased {
                            position: pos,
                            button: PointerEventButton::Left,
                        });
                    }

                    was_pressed = pressed;
                }
            }

            // Render the display only if needed
            self.window.draw_if_needed(|renderer| {
                trace!("Rendering display");

                // TODO: Implement proper rendering using `TargetPixel` trait
                renderer.render(&mut self.buffer.borrow_mut(), self.width);

                let brightness = self
                    .backlight_driver
                    .brightness()
                    .expect("BUG: Cannot get current brightness");
                let max_brightness = self.backlight_driver.max_brightness();
                let brightness = f32::from(brightness) / f32::from(max_brightness);

                let title = if brightness <= 0.0 {
                    WINDOW_TITLE_OFF
                } else {
                    WINDOW_TITLE
                };

                let mut minifb = self.minifb.borrow_mut();
                minifb.set_title(title);

                Self::update_minifb(
                    &mut minifb,
                    &self.buffer.borrow(),
                    self.width,
                    self.height,
                    brightness,
                );
            });

            // Do not sleep when there are active animations (as mentioned in `duration_until_next_timer_update` docs)
            if self.window.has_active_animations() {
                // If there will be performance issues, we can introduce small delay, like in official `android-activity` implementation.
                // https://github.com/slint-ui/slint/blob/b80d5a23042c866fbc6d82d00c633bfda9057dd2/internal/backends/android-activity/lib.rs#L102
                continue;
            }

            // Wait for the next event, error is ignored because it is timeout/close.
            saved_proxy_event = match slint::platform::duration_until_next_timer_update() {
                Some(timeout) => self.event_receiver.recv_timeout(timeout).ok(),
                None => self.event_receiver.recv().ok(),
            };
        }

        Ok(())
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(self.proxy.clone())
    }
}

impl VirtualDisplayPlatform {
    pub fn new(width: usize, height: usize, backlight_driver: MockBacklightDriver) -> Result<Self> {
        info!("Creating virtual display platform");
        let window = MinimalSoftwareWindow::new(
            slint::platform::software_renderer::RepaintBufferType::ReusedBuffer,
        );

        let (event_sender, event_receiver) = flume::unbounded();

        let buffer = vec![PremultipliedRgbaColor::default(); width * height];
        let mut minifb = minifb::Window::new(
            WINDOW_TITLE,
            width,
            height,
            minifb::WindowOptions::default(),
        )
        .context("BUG: You are likely missing some X11 drivers, see README for more info!")?;

        Self::update_minifb(&mut minifb, &buffer, width, height, 1.0);

        Ok(Self {
            window,
            minifb: RefCell::new(minifb),
            buffer: RefCell::new(buffer),
            width,
            height,
            proxy: Box::new(bmc_display::proxy::Proxy::new(event_sender)),
            event_receiver,
            backlight_driver,
        })
    }

    fn update_minifb(
        minifb: &mut minifb::Window,
        buffer: &[PremultipliedRgbaColor],
        width: usize,
        height: usize,
        brightness: f32,
    ) {
        let buffer = buffer
            .iter()
            .map(|c| {
                #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let r = (f32::from(c.red) * brightness) as u32;
                #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let g = (f32::from(c.green) * brightness) as u32;
                #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let b = (f32::from(c.blue) * brightness) as u32;
                // Alpha is ignored because minifb does not support it
                (r << 16) | (g << 8) | b
            })
            .collect::<Vec<_>>();

        minifb
            .update_with_buffer(&buffer, width, height)
            .expect("BUG: Cannot update minifb window");
    }
}
