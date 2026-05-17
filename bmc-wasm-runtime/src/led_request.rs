// Copyright (C) 2026  Braiins Systems s.r.o.

//! Widget-perspective LED requests emitted by the wasm runtime.
//!
//! `LedEffectKind` and its protocol-aligned discriminants live in
//! `bmc-led`; this module just adds the request id type, the
//! per-guest allocator, and the `LedRequest` enum that the runtime
//! publishes on the host-side channel.

use std::time::Duration;

use bmc_led::data::{LedEffectKind as LedEffect, LedScope, Rgb};

/// Widget-allocated identifier for an LED request.
///
/// 0 is the stop-all sentinel and is never produced by the allocator.
///
/// The Wayland-protocol side defines the same type and sentinel value
/// in `bmc_widget_protocol`; they must stay in sync. The `widgets/wasm`
/// crate, which depends on both, enforces this at compile time via a
/// `const _ : () = assert!(..)`.
pub type LedRequestId = u32;

/// Reserved value of [`LedRequestId`] — denotes "stop everything I
/// own" on `LedRequest::Stop` and is invalid as an allocation.
pub const LED_REQUEST_ID_ALL: LedRequestId = 0;

/// One widget-perspective LED request emitted by the runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedRequest {
    SetEffect {
        request_id: LedRequestId,
        effect: LedEffect,
        color: Rgb,
        period_ms: u32,
        /// `None` = endless, `Some(n)` = temporary for `n` ms.
        duration: Option<Duration>,
        scope: LedScope,
    },
    /// `request_id == LED_REQUEST_ID_ALL` cancels every request the
    /// emitting guest owns; any other value cancels just that one.
    Stop { request_id: LedRequestId },
}

/// Per-guest monotonic allocator. 0 is reserved; the counter wraps
/// to 1 after `u32::MAX` so the emitted value is always non-zero.
#[derive(Debug)]
pub struct LedRequestIdAllocator {
    next: u32,
}

impl Default for LedRequestIdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl LedRequestIdAllocator {
    #[must_use]
    pub const fn new() -> Self {
        Self { next: 1 }
    }

    #[must_use]
    pub fn alloc(&mut self) -> LedRequestId {
        if self.next == 0 {
            self.next = 1;
        }
        let id = self.next;
        self.next = self.next.wrapping_add(1);
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocator_starts_at_one() {
        let mut a = LedRequestIdAllocator::new();
        assert_eq!(a.alloc(), 1);
        assert_eq!(a.alloc(), 2);
    }

    #[test]
    fn allocator_skips_zero_on_wrap() {
        let mut a = LedRequestIdAllocator::new();
        a.next = u32::MAX;
        assert_eq!(a.alloc(), u32::MAX);
        // wrapping_add gives 0; alloc must skip it
        assert_eq!(a.alloc(), 1);
    }
}
