// Copyright (C) 2025  Braiins Systems s.r.o.

//! Animation primitives for WASM widgets.
//!
//! Built on the [`keyframe`](https://github.com/hannesmann/keyframe) crate, providing:
//! - [`Tween`]: Stateful animation that interpolates values over time
//! - [`Transform`]: 2D transforms for rotation, scaling, translation
//! - Easing functions via [`easing`] module
//!
//! # Quick Start
//!
//! ```ignore
//! use bmc_wasm_sdk::animation::{Tween, easing};
//!
//! static mut FADE: Option<Tween<f32>> = None;
//!
//! fn init(_w: u32, _h: u32) {
//!     unsafe { FADE = Some(Tween::new(0.0, 1.0, 300).with_ease(easing::EaseOut)); }
//! }
//!
//! fn render(delta_ms: u32) {
//!     let tween = unsafe { FADE.as_mut().unwrap() };
//!     tween.tick(delta_ms);
//!
//!     let alpha = tween.value();
//!     // Draw with alpha...
//!
//!     if !tween.is_finished() { request_frame(); }
//! }
//! ```

mod animated;
mod transform;
mod tween;

pub use animated::AnimatedInner;
pub use transform::{deg_to_rad, rad_to_deg, Transform};
pub use tween::{DynTween, Tween};

// Re-export keyframe primitives
pub use keyframe::{ease, CanTween, EasingFunction};

/// Predefined easing functions.
///
/// For [`Tween`] (static easing), use the struct types directly:
/// ```ignore
/// let tween = Tween::new(0.0_f32, 1.0, 300).with_ease(easing::EaseOut);
/// ```
///
/// For [`DynTween`] (dynamic easing), use the lowercase function versions:
/// ```ignore
/// let tween = DynTween::new(0.0_f32, 1.0, 300, easing::ease_out);
/// ```
///
/// Available functions:
/// - `linear` - constant speed
/// - `ease_in`, `ease_out`, `ease_in_out` - sine-based
/// - `ease_in_quad`, `ease_out_quad`, `ease_in_out_quad` - quadratic
/// - `ease_in_cubic`, `ease_out_cubic`, `ease_in_out_cubic` - cubic
pub mod easing {
    pub use keyframe::functions::*;
    use keyframe::EasingFunction;

    // Function pointer versions for DynTween
    pub fn linear(t: f64) -> f64 { Linear.y(t) }
    pub fn ease_in(t: f64) -> f64 { EaseIn.y(t) }
    pub fn ease_out(t: f64) -> f64 { EaseOut.y(t) }
    pub fn ease_in_out(t: f64) -> f64 { EaseInOut.y(t) }
    pub fn ease_in_quad(t: f64) -> f64 { EaseInQuad.y(t) }
    pub fn ease_out_quad(t: f64) -> f64 { EaseOutQuad.y(t) }
    pub fn ease_in_out_quad(t: f64) -> f64 { EaseInOutQuad.y(t) }
    pub fn ease_in_cubic(t: f64) -> f64 { EaseInCubic.y(t) }
    pub fn ease_out_cubic(t: f64) -> f64 { EaseOutCubic.y(t) }
    pub fn ease_in_out_cubic(t: f64) -> f64 { EaseInOutCubic.y(t) }
    pub fn ease_in_quart(t: f64) -> f64 { EaseInQuart.y(t) }
    pub fn ease_out_quart(t: f64) -> f64 { EaseOutQuart.y(t) }
    pub fn ease_in_out_quart(t: f64) -> f64 { EaseInOutQuart.y(t) }
    pub fn ease_in_quint(t: f64) -> f64 { EaseInQuint.y(t) }
    pub fn ease_out_quint(t: f64) -> f64 { EaseOutQuint.y(t) }
    pub fn ease_in_out_quint(t: f64) -> f64 { EaseInOutQuint.y(t) }
}
