// Copyright (C) 2025  Braiins Systems s.r.o.

use std::time::{Duration, Instant};

use clap::Parser;
use slint::{ComponentHandle, LogicalSize, Timer, TimerMode, WindowSize};

#[allow(warnings)]
mod generated {
    slint::include_modules!();
}
use generated::FemtoVgDemo;

const WIDTH: u32 = 640;
const HEIGHT: u32 = 240;
const BALL_RADIUS: f32 = 20.0;

#[derive(Parser)]
#[command(name = "femtovg-demo", about = "FemtoVG GPU-accelerated demo widget")]
struct Args {
    #[arg(long, help = "Run in standalone mode")]
    standalone: bool,
}

struct AnimationState {
    ball_x: f32,
    ball_y: f32,
    velocity_x: f32,
    velocity_y: f32,
    frame_count: u32,
    last_fps_update: Instant,
    current_fps: i32,
}

impl Default for AnimationState {
    fn default() -> Self {
        Self {
            ball_x: 320.0,
            ball_y: 120.0,
            velocity_x: 3.5,
            velocity_y: 2.8,
            frame_count: 0,
            last_fps_update: Instant::now(),
            current_fps: 0,
        }
    }
}

impl AnimationState {
    fn update(&mut self) {
        self.update_fps();
        self.update_ball_physics();
    }

    fn update_fps(&mut self) {
        self.frame_count += 1;
        let elapsed = self.last_fps_update.elapsed();
        if elapsed >= Duration::from_secs(1) {
            #[expect(clippy::cast_precision_loss)]
            let fps = self.frame_count as f32 / elapsed.as_secs_f32();
            #[expect(clippy::cast_possible_truncation)]
            {
                self.current_fps = fps.round() as i32;
            }
            self.frame_count = 0;
            self.last_fps_update = Instant::now();
        }
    }

    fn update_ball_physics(&mut self) {
        self.ball_x += self.velocity_x;
        self.ball_y += self.velocity_y;

        #[expect(clippy::cast_precision_loss)]
        let max_x = WIDTH as f32 - BALL_RADIUS;
        #[expect(clippy::cast_precision_loss)]
        let max_y = HEIGHT as f32 - BALL_RADIUS;

        if self.ball_x <= BALL_RADIUS || self.ball_x >= max_x {
            self.velocity_x = -self.velocity_x;
            self.ball_x = self.ball_x.clamp(BALL_RADIUS, max_x);
        }
        if self.ball_y <= BALL_RADIUS || self.ball_y >= max_y {
            self.velocity_y = -self.velocity_y;
            self.ball_y = self.ball_y.clamp(BALL_RADIUS, max_y);
        }
    }

    fn apply_to_ui(&self, ui: &FemtoVgDemo) {
        ui.set_ball_x(self.ball_x);
        ui.set_ball_y(self.ball_y);
        ui.set_fps(self.current_fps);
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let _args = Args::parse();

    // Print renderer info
    eprintln!("Slint backend: {:?}", std::env::var("SLINT_BACKEND"));
    eprintln!("Display: {:?}", std::env::var("WAYLAND_DISPLAY"));

    let ui = FemtoVgDemo::new()?;

    #[expect(clippy::cast_precision_loss)]
    ui.window().set_size(WindowSize::Logical(LogicalSize::new(
        WIDTH as f32,
        HEIGHT as f32,
    )));

    let mut state = AnimationState::default();

    let timer = Timer::default();
    let ui_handle = ui.as_weak();

    timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
        let Some(ui) = ui_handle.upgrade() else {
            return;
        };
        state.update();
        state.apply_to_ui(&ui);
    });

    ui.run()
}

