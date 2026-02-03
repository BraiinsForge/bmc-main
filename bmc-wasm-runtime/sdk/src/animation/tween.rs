// Copyright (C) 2025  Braiins Systems s.r.o.

//! Tween state management for animations.

use keyframe::{functions::Linear, CanTween, EasingFunction};

/// A stateful animation that interpolates from one value to another over time.
///
/// The easing function type is part of the Tween type. For dynamic easing changes,
/// use [`DynTween`] instead.
///
/// # Example
/// ```ignore
/// use bmc_wasm_sdk::animation::{Tween, easing};
///
/// let mut tween = Tween::new(0.0_f32, 1.0, 300).with_ease(easing::EaseOut);
///
/// // In render loop:
/// tween.tick(delta_ms);
/// let alpha = tween.value();
/// if !tween.is_finished() { request_frame(); }
/// ```
pub struct Tween<T: CanTween, E: EasingFunction = Linear> {
    from: T,
    to: T,
    duration_ms: u32,
    delay_ms: u32,
    elapsed_ms: u32,
    easing: E,
}

impl<T: CanTween + Copy> Tween<T, Linear> {
    /// Create a new tween from `from` to `to` over `duration_ms` milliseconds.
    /// Uses linear easing by default. Use `with_ease()` for other easing functions.
    pub fn new(from: T, to: T, duration_ms: u32) -> Self {
        Self {
            from,
            to,
            duration_ms,
            delay_ms: 0,
            elapsed_ms: 0,
            easing: Linear,
        }
    }
}

impl<T: CanTween + Copy, E: EasingFunction> Tween<T, E> {
    /// Set the easing function. Use functions from `keyframe::functions`.
    ///
    /// # Example
    /// ```ignore
    /// let tween = Tween::new(0.0_f32, 1.0, 300).with_ease(easing::EaseOutCubic);
    /// ```
    pub fn with_ease<E2: EasingFunction>(self, easing: E2) -> Tween<T, E2> {
        Tween {
            from: self.from,
            to: self.to,
            duration_ms: self.duration_ms,
            delay_ms: self.delay_ms,
            elapsed_ms: self.elapsed_ms,
            easing,
        }
    }

    /// Add a delay before the animation starts.
    pub fn delay(mut self, delay_ms: u32) -> Self {
        self.delay_ms = delay_ms;
        self
    }

    /// Advance the animation by `delta_ms` milliseconds.
    pub fn tick(&mut self, delta_ms: u32) {
        self.elapsed_ms = self.elapsed_ms.saturating_add(delta_ms);
    }

    /// Get the current interpolated value.
    pub fn value(&self) -> T {
        if self.elapsed_ms < self.delay_ms {
            return self.from;
        }

        let active_elapsed = self.elapsed_ms - self.delay_ms;

        if self.duration_ms == 0 || active_elapsed >= self.duration_ms {
            return self.to;
        }

        let t = active_elapsed as f64 / self.duration_ms as f64;
        let eased_t = self.easing.y(t);
        T::ease(self.from, self.to, eased_t)
    }

    /// Get the current progress as 0.0 to 1.0.
    pub fn progress(&self) -> f32 {
        if self.elapsed_ms < self.delay_ms {
            return 0.0;
        }

        let active_elapsed = self.elapsed_ms - self.delay_ms;

        if self.duration_ms == 0 {
            return 1.0;
        }

        (active_elapsed as f32 / self.duration_ms as f32).min(1.0)
    }

    /// Returns true when the animation has completed.
    pub fn is_finished(&self) -> bool {
        self.elapsed_ms >= self.delay_ms + self.duration_ms
    }

    /// Reset the animation to the beginning.
    pub fn reset(&mut self) {
        self.elapsed_ms = 0;
    }

    /// Set new target value, keeping current value as start.
    /// Useful for smooth re-targeting mid-animation.
    pub fn retarget(&mut self, new_to: T) {
        self.from = self.value();
        self.to = new_to;
        self.elapsed_ms = self.delay_ms; // Skip delay on retarget
    }
}

/// Type-erased easing function for dynamic tween usage.
type BoxedEasing = fn(f64) -> f64;

/// A tween with dynamic (runtime-selectable) easing function.
///
/// Use this when you need to change easing functions at runtime,
/// or when storing tweens in collections.
///
/// # Example
/// ```ignore
/// use bmc_wasm_sdk::animation::{DynTween, easing};
///
/// let mut tween = DynTween::new(0.0_f32, 1.0, 300, easing::EaseOut);
/// // Can reassign with different easing:
/// tween = DynTween::new(1.0, 0.0, 300, easing::EaseIn);
/// ```
pub struct DynTween<T: CanTween> {
    from: T,
    to: T,
    duration_ms: u32,
    delay_ms: u32,
    elapsed_ms: u32,
    easing: BoxedEasing,
}

impl<T: CanTween + Copy> DynTween<T> {
    /// Create a new dynamic tween with the specified easing function.
    ///
    /// Pass an easing function from the `easing` module, e.g. `easing::EaseOut`.
    pub fn new(from: T, to: T, duration_ms: u32, easing: fn(f64) -> f64) -> Self {
        Self {
            from,
            to,
            duration_ms,
            delay_ms: 0,
            elapsed_ms: 0,
            easing,
        }
    }

    /// Create a new tween with linear easing.
    pub fn linear(from: T, to: T, duration_ms: u32) -> Self {
        Self::new(from, to, duration_ms, |t| t)
    }

    /// Add a delay before the animation starts.
    pub fn delay(mut self, delay_ms: u32) -> Self {
        self.delay_ms = delay_ms;
        self
    }

    /// Advance the animation by `delta_ms` milliseconds.
    pub fn tick(&mut self, delta_ms: u32) {
        self.elapsed_ms = self.elapsed_ms.saturating_add(delta_ms);
    }

    /// Get the current interpolated value.
    pub fn value(&self) -> T {
        if self.elapsed_ms < self.delay_ms {
            return self.from;
        }

        let active_elapsed = self.elapsed_ms - self.delay_ms;

        if self.duration_ms == 0 || active_elapsed >= self.duration_ms {
            return self.to;
        }

        let t = active_elapsed as f64 / self.duration_ms as f64;
        let eased_t = (self.easing)(t);
        T::ease(self.from, self.to, eased_t)
    }

    /// Get the current progress as 0.0 to 1.0.
    pub fn progress(&self) -> f32 {
        if self.elapsed_ms < self.delay_ms {
            return 0.0;
        }

        let active_elapsed = self.elapsed_ms - self.delay_ms;

        if self.duration_ms == 0 {
            return 1.0;
        }

        (active_elapsed as f32 / self.duration_ms as f32).min(1.0)
    }

    /// Returns true when the animation has completed.
    pub fn is_finished(&self) -> bool {
        self.elapsed_ms >= self.delay_ms + self.duration_ms
    }

    /// Reset the animation to the beginning.
    pub fn reset(&mut self) {
        self.elapsed_ms = 0;
    }

    /// Set new target value, keeping current value as start.
    pub fn retarget(&mut self, new_to: T) {
        self.from = self.value();
        self.to = new_to;
        self.elapsed_ms = self.delay_ms;
    }
}
