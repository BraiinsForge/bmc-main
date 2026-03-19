// Copyright (C) 2026  Braiins Systems s.r.o.
//
// Simplified virtual display for the init mock binary.
// Adapted from bmc-mock-display/src/virtual_display.rs.

use anyhow::{Context, Result};
use bmc_nix_init::proxy::{Proxy, ProxyEvent};
use slint::LogicalPosition;
use slint::platform::software_renderer::PremultipliedRgbaColor;
use slint::platform::{EventLoopProxy, Platform, software_renderer::MinimalSoftwareWindow};
use slint::platform::{PointerEventButton, WindowEvent};
use std::cell::RefCell;
use std::iter;
use std::rc::Rc;
use tracing::info;

const WINDOW_TITLE: &str = "bmc-nix-init (mock)";

pub struct VirtualDisplayPlatform {
    window: Rc<MinimalSoftwareWindow>,
    minifb: RefCell<minifb::Window>,
    buffer: RefCell<Vec<PremultipliedRgbaColor>>,
    width: usize,
    height: usize,
    proxy: Box<Proxy>,
    event_receiver: flume::Receiver<ProxyEvent>,
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
            slint::platform::update_timers_and_animations();

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

            {
                let mut minifb = self.minifb.borrow_mut();
                minifb.update();

                if !minifb.is_open() {
                    break 'outer;
                }

                if let Some((x, y)) = minifb.get_mouse_pos(minifb::MouseMode::Discard) {
                    let pos = LogicalPosition::new(x, y);
                    let pressed = minifb.get_mouse_down(minifb::MouseButton::Left);

                    self.window
                        .dispatch_event(WindowEvent::PointerMoved { position: pos });

                    if pressed && !was_pressed {
                        self.window.dispatch_event(WindowEvent::PointerPressed {
                            position: pos,
                            button: PointerEventButton::Left,
                        });
                    } else if !pressed && was_pressed {
                        self.window.dispatch_event(WindowEvent::PointerReleased {
                            position: pos,
                            button: PointerEventButton::Left,
                        });
                    }

                    was_pressed = pressed;
                }
            }

            self.window.draw_if_needed(|renderer| {
                renderer.render(&mut self.buffer.borrow_mut(), self.width);

                let buffer = self
                    .buffer
                    .borrow()
                    .iter()
                    .map(|c| {
                        let r = u32::from(c.red);
                        let g = u32::from(c.green);
                        let b = u32::from(c.blue);
                        (r << 16) | (g << 8) | b
                    })
                    .collect::<Vec<_>>();

                self.minifb
                    .borrow_mut()
                    .update_with_buffer(&buffer, self.width, self.height)
                    .expect("BUG: cannot update minifb window");
            });

            if self.window.has_active_animations() {
                continue;
            }

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
    pub fn new(width: usize, height: usize) -> Result<Self> {
        info!("Creating virtual display platform");
        let window = MinimalSoftwareWindow::new(
            slint::platform::software_renderer::RepaintBufferType::ReusedBuffer,
        );

        let (event_sender, event_receiver) = flume::unbounded();

        let buffer = vec![PremultipliedRgbaColor::default(); width * height];
        let minifb = minifb::Window::new(
            WINDOW_TITLE,
            width,
            height,
            minifb::WindowOptions::default(),
        )
        .context("BUG: missing X11 drivers, see README for more info")?;

        Ok(Self {
            window,
            minifb: RefCell::new(minifb),
            buffer: RefCell::new(buffer),
            width,
            height,
            proxy: Box::new(Proxy::new(event_sender)),
            event_receiver,
        })
    }
}
