// Copyright (C) 2025  Braiins Systems s.r.o.

//! Simplified animated value API.

use keyframe::CanTween;

use super::DynTween;

/// Inner state for animated values. Used by the `animated!` macro.
#[doc(hidden)]
pub struct AnimatedInner<T: CanTween + Copy> {
    tween: Option<DynTween<T>>,
    default: T,
}

impl<T: CanTween + Copy + Default> Default for AnimatedInner<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: CanTween + Copy + Default> AnimatedInner<T> {
    pub fn new() -> Self {
        Self {
            tween: None,
            default: T::default(),
        }
    }

    /// Start a new animation from `from` to `to`.
    pub fn start(&mut self, from: T, to: T, duration_ms: u32, easing: fn(f64) -> f64) {
        self.tween = Some(DynTween::new(from, to, duration_ms, easing));
    }

    /// Start animation from current value to new target.
    pub fn animate_to(&mut self, to: T, duration_ms: u32, easing: fn(f64) -> f64) {
        let from = self.get();
        self.tween = Some(DynTween::new(from, to, duration_ms, easing));
    }

    /// Advance the animation by `delta_ms` milliseconds.
    pub fn tick(&mut self, delta_ms: u32) {
        if let Some(ref mut tween) = self.tween {
            tween.tick(delta_ms);
        }
    }

    /// Get the current interpolated value.
    pub fn get(&self) -> T {
        self.tween.as_ref().map_or(self.default, |t| t.value())
    }

    /// Returns true when the animation has completed (or hasn't started).
    pub fn is_finished(&self) -> bool {
        self.tween.as_ref().map_or(true, DynTween::is_finished)
    }

    /// Reset the animation to the beginning.
    pub fn reset(&mut self) {
        if let Some(ref mut tween) = self.tween {
            tween.reset();
        }
    }

    /// Get mutable access to the underlying tween (for ping-pong etc).
    pub fn tween_mut(&mut self) -> Option<&mut DynTween<T>> {
        self.tween.as_mut()
    }

    /// Replace the underlying tween directly.
    pub fn set_tween(&mut self, tween: DynTween<T>) {
        self.tween = Some(tween);
    }
}

/// Define animated values with simplified API.
///
/// Creates a unit struct with static methods for managing animation state.
///
/// # Example
///
/// ```ignore
/// use bmc_wasm_sdk::{animated, animation::easing};
///
/// animated!(FADE: f32);
/// animated!(SCALE: f32);
///
/// fn init(_w: u32, _h: u32) {
///     FADE::start(0.0, 1.0, 600, easing::ease_out_cubic);
///     SCALE::start(0.5, 1.0, 300, easing::ease_out);
/// }
///
/// fn render(delta_ms: u32) {
///     FADE::tick(delta_ms);
///     SCALE::tick(delta_ms);
///
///     let fade = FADE::get();
///     let scale = SCALE::get();
///     // use fade, scale...
/// }
/// ```
#[macro_export]
macro_rules! animated {
    ($name:ident: $ty:ty) => {
        $crate::__animated_inner!($name, $ty);
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __animated_inner {
    ($name:ident, $ty:ty) => {
        #[allow(non_camel_case_types)]
        struct $name;

        impl $name {
            ::std::thread_local! {
                static INNER: ::std::cell::RefCell<$crate::animation::AnimatedInner<$ty>> =
                    ::std::cell::RefCell::new($crate::animation::AnimatedInner::new());
            }

            /// Start a new animation from `from` to `to`.
            #[inline]
            pub fn start(from: $ty, to: $ty, duration_ms: u32, easing: fn(f64) -> f64) {
                Self::INNER.with(|inner| inner.borrow_mut().start(from, to, duration_ms, easing));
            }

            /// Start animation from current value to new target.
            #[inline]
            pub fn animate_to(to: $ty, duration_ms: u32, easing: fn(f64) -> f64) {
                Self::INNER.with(|inner| inner.borrow_mut().animate_to(to, duration_ms, easing));
            }

            /// Advance the animation by `delta_ms` milliseconds.
            #[inline]
            pub fn tick(delta_ms: u32) {
                Self::INNER.with(|inner| inner.borrow_mut().tick(delta_ms));
            }

            /// Get the current interpolated value.
            #[inline]
            pub fn get() -> $ty {
                Self::INNER.with(|inner| inner.borrow().get())
            }

            /// Returns true when the animation has completed.
            #[inline]
            pub fn is_finished() -> bool {
                Self::INNER.with(|inner| inner.borrow().is_finished())
            }

            /// Reset the animation to the beginning.
            #[inline]
            pub fn reset() {
                Self::INNER.with(|inner| inner.borrow_mut().reset());
            }

            /// Access the inner state for advanced operations (ping-pong, retarget, etc).
            #[inline]
            pub fn with<R>(f: impl FnOnce(&mut $crate::animation::AnimatedInner<$ty>) -> R) -> R {
                Self::INNER.with(|inner| f(&mut inner.borrow_mut()))
            }
        }
    };
}
