// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

//! Replace-style ephemeral asset slots.
//!
//! A `*Slot` owns a stable `&'static str` name and binds it to one
//! registration at a time. `set(data)` evicts any previous registration
//! under that name and registers the new payload; `evict()` drops the
//! current registration without re-registering.
//!
//! # When to use a Slot vs an `include_*!` macro
//!
//! - **Static asset** baked in at compile time (icons, UI bitmaps, sound
//!   effects that ship with the widget): use `include_bitmap!` /
//!   `include_svg!` / `include_mesh!` / `include_audio!` and the matching
//!   `ensure_*_registered` helper. The host dedups by the macro-emitted
//!   tag, so repeated calls are free.
//! - **Dynamic asset** whose bytes are fetched at runtime and change over
//!   time (album art, dynamically generated charts, downloaded skins):
//!   use a Slot. Each `set(bytes)` releases the previous payload's memory
//!   and GPU resources before registering the new one — without a Slot
//!   (or an equivalent manual `evict_prefix`), each variant would
//!   accumulate forever.
//!
//! Slot names participate in the host's segment-delimited namespace, so
//! `BitmapSlot::new("album_art")` and `BitmapSlot::new("album_art_thumb")`
//! coexist safely — eviction respects segment boundaries (`:`).
//!
//! Slots exist on wasm32 only; the gallery has no use for them,
//! as its static assets register through `Bitmap`, `Svg`, `Mesh` and `Audio`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use bmc_ui::{AudioId, BitmapId, MeshId, SvgId};
use bmc_wasm_protocol::ImageJobId;

use crate::host;
use crate::net::FetchBodyRef;
use crate::widget::{Receiver, RegisterError, Target};

macro_rules! define_slot {
    (
        $(#[$meta:meta])*
        $slot:ident => $id:ty, |$name:ident, $data:ident| $register:expr
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy)]
        pub struct $slot {
            name: &'static str,
        }

        impl $slot {
            #[must_use]
            pub const fn new(name: &'static str) -> Self {
                Self { name }
            }

            /// Replace the current registration under this slot's name with
            /// `data`. Any previously-registered payload (and, for audio,
            /// any in-flight playback sinks) are evicted first. Any ID returned
            /// by an earlier `set` is invalid after this call.
            #[must_use]
            pub fn set(&self, $data: &[u8]) -> Option<$id> {
                let $name = self.name;
                host::evict_prefix($name);
                $register
            }

            /// Drop the current registration under this slot's name. Any ID
            /// returned by an earlier `set` is invalid after this call.
            pub fn evict(&self) {
                host::evict_prefix(self.name);
            }

            #[must_use]
            pub const fn name(&self) -> &'static str {
                self.name
            }
        }
    };
}

define_slot! {
    /// Slot for a dynamic raster bitmap (PNG/JPEG bytes).
    BitmapSlot => BitmapId,
    |name, data| host::register_bitmap(name, data)
}

/// How a decode scales an image to a `w`×`h` box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// Shrink the image to fit within the box, keeping its aspect; a smaller image keeps its size.
    Within { w: u32, h: u32 },
    /// Scale the image to cover the box and crop the overflow.
    Cover { w: u32, h: u32 },
}

impl Fit {
    fn from_cover(max_w: u32, max_h: u32, cover: bool) -> Self {
        if cover {
            Fit::Cover { w: max_w, h: max_h }
        } else {
            Fit::Within { w: max_w, h: max_h }
        }
    }

    /// The host's box and `cover` flag for this fit.
    fn to_host(self) -> (u32, u32, bool) {
        match self {
            Fit::Within { w, h } => (w, h, false),
            Fit::Cover { w, h } => (w, h, true),
        }
    }
}

impl BitmapSlot {
    /// Decode `data` to fit within (`cover` false) or cover-crop to (`cover` true)
    /// `max_w`×`max_h`, off the render thread; `on_ready` fires when it replaces
    /// the slot's bitmap. `identity` is recorded in the per-instance asset cache
    /// so the host can restore the bitmap when a draw uses it.
    #[must_use]
    pub fn set_fit(
        &self,
        data: &[u8],
        max_w: u32,
        max_h: u32,
        cover: bool,
        identity: &[u8],
        on_ready: ImageReadyCallback,
    ) -> Option<ImageJobId> {
        set_bitmap_fit(self.name, data, max_w, max_h, cover, identity, on_ready)
    }

    /// [`set_fit`](Self::set_fit) whose result runs `method` on `to`,
    /// with the box and `cover` as a [`Fit`].
    ///
    /// A refusal is [`RegisterError`] rather than `None`,
    /// or [`HandleError`](crate::HandleError) through a [`PartHandle`](crate::PartHandle),
    /// and a part dropped before the decode finishes gets no result.
    #[must_use]
    pub fn set_fit_on<T: 'static, R: Receiver<T>>(
        &self,
        to: &R,
        data: &[u8],
        fit: Fit,
        identity: &[u8],
        method: ImageReadyMethod<T>,
    ) -> R::Out<Result<ImageJobId, RegisterError>> {
        set_bitmap_fit_on(to, self.name, data, fit, identity, method)
    }

    /// Decode a callback-scoped host fetch body without copying it into WASM.
    pub fn set_fit_ref<'a>(
        &self,
        body: FetchBodyRef<'a>,
        max_w: u32,
        max_h: u32,
        cover: bool,
        identity: &[u8],
        on_ready: ImageReadyCallback,
    ) -> Result<ImageJobId, FetchBodyRef<'a>> {
        self.set_fit_ref_with(
            body,
            Fit::from_cover(max_w, max_h, cover),
            identity,
            Awaiting::Callback(on_ready),
        )
    }

    /// [`set_fit_ref`](Self::set_fit_ref) whose result runs `method` on `to`,
    /// with the box and `cover` as a [`Fit`].
    ///
    /// A refusal hands the body back in [`RegisterError`], for [`into_inner`](RegisterError::into_inner) to take out,
    /// inside [`HandleError::Register`](crate::HandleError::Register) through a [`PartHandle`](crate::PartHandle),
    /// which drops `body` while the handle is not live;
    /// a part dropped before the decode finishes gets no result.
    #[must_use = "a refused decode hands the body back"]
    pub fn set_fit_ref_on<'a, T: 'static, R: Receiver<T>>(
        &self,
        to: &R,
        body: FetchBodyRef<'a>,
        fit: Fit,
        identity: &[u8],
        method: ImageReadyMethod<T>,
    ) -> R::Out<Result<ImageJobId, RegisterError<FetchBodyRef<'a>>>> {
        to.__register(|target| {
            self.set_fit_ref_with(body, fit, identity, on_target(target, method))
                .map_err(RegisterError::new)
        })
    }

    fn set_fit_ref_with<'a>(
        &self,
        body: FetchBodyRef<'a>,
        fit: Fit,
        identity: &[u8],
        awaiting: Awaiting,
    ) -> Result<ImageJobId, FetchBodyRef<'a>> {
        let (max_w, max_h, cover) = fit.to_host();
        let job_id = host::register_bitmap_fit_ref(self.name, body, max_w, max_h, cover, identity)?;
        remember_image_callback(job_id, awaiting);
        Ok(job_id)
    }
}

fn remember_image_callback(job_id: ImageJobId, awaiting: Awaiting) {
    IMAGE_PENDING.with(|p| p.borrow_mut().insert(job_id, awaiting));
}

/// [`BitmapSlot::set_fit`] against a tag chosen at runtime, for widgets
/// holding one image per payload row rather than one per fixed position
/// in their layout — a slot's name is `&'static str`, which a tag
/// derived from a fetched URL cannot be. Tags share the host's
/// segment-delimited namespace, so give them a common prefix to stay
/// evictable together.
#[must_use]
pub fn set_bitmap_fit(
    tag: &str,
    data: &[u8],
    max_w: u32,
    max_h: u32,
    cover: bool,
    identity: &[u8],
    on_ready: ImageReadyCallback,
) -> Option<ImageJobId> {
    set_bitmap_fit_with(
        tag,
        data,
        Fit::from_cover(max_w, max_h, cover),
        identity,
        Awaiting::Callback(on_ready),
    )
}

/// [`set_bitmap_fit`] whose result runs `method` on `to`, with the box and `cover` as a [`Fit`].
///
/// A refusal is [`RegisterError`] rather than `None`,
/// or [`HandleError`](crate::HandleError) through a [`PartHandle`](crate::PartHandle),
/// and a part dropped before the decode finishes gets no result.
#[must_use]
pub fn set_bitmap_fit_on<T: 'static, R: Receiver<T>>(
    to: &R,
    tag: &str,
    data: &[u8],
    fit: Fit,
    identity: &[u8],
    method: ImageReadyMethod<T>,
) -> R::Out<Result<ImageJobId, RegisterError>> {
    to.__register(|target| {
        set_bitmap_fit_with(tag, data, fit, identity, on_target(target, method))
            .ok_or(RegisterError::new(()))
    })
}

fn set_bitmap_fit_with(
    tag: &str,
    data: &[u8],
    fit: Fit,
    identity: &[u8],
    awaiting: Awaiting,
) -> Option<ImageJobId> {
    let (max_w, max_h, cover) = fit.to_host();
    let job_id = host::register_bitmap_fit(tag, data, max_w, max_h, cover, identity)?;
    remember_image_callback(job_id, awaiting);
    Some(job_id)
}

/// Async-decode result: the job handle, and `Some(id)` on success / `None` on failure.
pub type ImageReadyCallback = fn(ImageJobId, Option<BitmapId>);

/// An async-decode result as a method of the value it is delivered to.
pub type ImageReadyMethod<T> = fn(&mut T, ImageJobId, DecodeOutcome);

/// How a decode started through a method form ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeOutcome {
    /// The bitmap replaced the slot's previous one.
    Ready(BitmapId),
    /// The decode or its upload failed.
    Failed,
    /// The decode ended while the widget slept, and the host reports no more than that:
    /// it may have failed, or its pixels may not have been kept,
    /// so the tag need not show this image.
    /// If the widget needs the image, check the cache after `on_wake`:
    /// an older entry can still sit under the tag, and another decode may have replaced it,
    /// so note its [`crate::cache::Stat::saved_at`] before asking for the decode,
    /// take the entry only if that stamp has changed since
    /// and its [`crate::cache::Stat::metadata`] names the identity asked for,
    /// and fetch and decode it again otherwise.
    WhileDormant,
}

/// What a pending decode reports to.
enum Awaiting {
    /// A free callback, which hears nothing of a decode that finished while dormant.
    Callback(ImageReadyCallback),
    Method(Rc<dyn Fn(ImageJobId, DecodeOutcome)>),
}

thread_local! {
    static IMAGE_PENDING: RefCell<HashMap<ImageJobId, Awaiting>> = RefCell::new(HashMap::new());
}

fn on_target<T: 'static>(target: Target<T>, method: ImageReadyMethod<T>) -> Awaiting {
    Awaiting::Method(Rc::new(move |job_id, outcome| {
        target.deliver(|value| method(value, job_id, outcome));
    }))
}

/// Hand a finished decode to its callback, if the widget still awaits it.
fn deliver(job_id: ImageJobId, outcome: DecodeOutcome) {
    // Taken out before invoking (the callback may start another decode).
    let awaiting = IMAGE_PENDING.with(|p| p.borrow_mut().remove(&job_id));
    let Some(awaiting) = awaiting else {
        return;
    };
    match (awaiting, outcome) {
        (Awaiting::Method(method), _) => method(job_id, outcome),
        (Awaiting::Callback(callback), DecodeOutcome::Ready(bitmap)) => {
            callback(job_id, Some(bitmap));
        }
        (Awaiting::Callback(callback), DecodeOutcome::Failed) => callback(job_id, None),
        (Awaiting::Callback(_), DecodeOutcome::WhileDormant) => {}
    }
}

/// Whether the host has still to report this decode.
///
/// Both `__on_image_ready` and `__on_image_dropped` clear the entry, so `false`
/// says no callback is coming: either it already ran, or the decode finished
/// while the widget was dormant, which a free callback never hears.
#[must_use]
pub fn image_decode_is_pending(job_id: ImageJobId) -> bool {
    IMAGE_PENDING.with(|p| p.borrow().contains_key(&job_id))
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use bmc_ui::BitmapId;
    use bmc_wasm_protocol::ImageJobId;

    use super::{DecodeOutcome, deliver};
    use crate::widget;

    /// Host entry point: dispatch a finished decode to its `set_fit` callback.
    #[unsafe(no_mangle)]
    pub extern "C" fn __on_image_ready(job_id: u32, bitmap_id: u32) {
        widget::export(|| {
            if let Some(job_id) = ImageJobId::from_wire(job_id) {
                deliver(
                    job_id,
                    BitmapId::from_ffi(bitmap_id)
                        .map_or(DecodeOutcome::Failed, DecodeOutcome::Ready),
                );
            }
        });
    }

    /// Host entry point for a decode that finished while dormant.
    #[unsafe(no_mangle)]
    pub extern "C" fn __on_image_dropped(job_id: u32) {
        widget::export(|| {
            if let Some(job_id) = ImageJobId::from_wire(job_id) {
                deliver(job_id, DecodeOutcome::WhileDormant);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use bmc_ui::BitmapId;
    use bmc_wasm_protocol::ImageJobId;

    use std::cell::RefCell;

    use super::{
        Awaiting, DecodeOutcome, deliver, image_decode_is_pending, on_target,
        remember_image_callback,
    };
    use crate::wasm_test;
    use crate::widget::Part;

    struct Gallery {
        results: Vec<DecodeOutcome>,
    }

    impl Gallery {
        fn on_ready(&mut self, _job_id: ImageJobId, outcome: DecodeOutcome) {
            self.results.push(outcome);
        }
    }

    fn gallery_awaiting(wire: u32) -> Part<Gallery> {
        let part = Part::new(Gallery {
            results: Vec::new(),
        });
        remember_image_callback(job(wire), on_target(part.target(), Gallery::on_ready));
        part
    }

    thread_local! {
        static CALLBACK_RESULTS: RefCell<Vec<Option<BitmapId>>> = const { RefCell::new(Vec::new()) };
    }

    fn record_callback(_job_id: ImageJobId, bitmap: Option<BitmapId>) {
        CALLBACK_RESULTS.with_borrow_mut(|results| results.push(bitmap));
    }

    fn callback_results() -> Vec<Option<BitmapId>> {
        CALLBACK_RESULTS.with_borrow(Clone::clone)
    }

    fn job(wire: u32) -> ImageJobId {
        ImageJobId::from_wire(wire).expect("BUG: test ids are non-zero")
    }

    #[wasm_test]
    fn a_result_from_the_host_runs_as_a_method_of_the_part() {
        let part = gallery_awaiting(4);

        crate::widget::as_host(|| super::wasm::__on_image_ready(4, 7));

        assert_eq!(
            part.with(|value| value.results.clone()),
            vec![DecodeOutcome::Ready(
                BitmapId::from_ffi(7).expect("BUG: 7 is a valid bitmap id")
            )]
        );
    }

    #[wasm_test]
    fn a_failed_decode_reaches_the_method_as_failed() {
        let part = gallery_awaiting(6);

        crate::widget::as_host(|| super::wasm::__on_image_ready(6, 0));

        assert_eq!(
            part.with(|value| value.results.clone()),
            vec![DecodeOutcome::Failed]
        );
    }

    #[wasm_test]
    fn a_decode_finished_while_dormant_reaches_the_method() {
        let part = gallery_awaiting(8);

        crate::widget::as_host(|| super::wasm::__on_image_dropped(8));

        assert_eq!(
            part.with(|value| value.results.clone()),
            vec![DecodeOutcome::WhileDormant],
            "the method learns its decode will not report a bitmap, without polling"
        );
    }

    #[wasm_test]
    fn a_decode_finished_while_dormant_runs_no_free_callback() {
        remember_image_callback(job(9), Awaiting::Callback(record_callback));

        crate::widget::as_host(|| super::wasm::__on_image_dropped(9));

        assert_eq!(callback_results(), vec![]);
        assert!(!image_decode_is_pending(job(9)));
    }

    #[wasm_test]
    fn a_result_from_the_host_reaches_a_free_callback() {
        remember_image_callback(job(10), Awaiting::Callback(record_callback));

        crate::widget::as_host(|| super::wasm::__on_image_ready(10, 7));

        assert_eq!(callback_results(), vec![BitmapId::from_ffi(7)]);
    }

    #[wasm_test]
    fn a_failed_decode_reaches_a_free_callback_as_none() {
        remember_image_callback(job(11), Awaiting::Callback(record_callback));

        crate::widget::as_host(|| super::wasm::__on_image_ready(11, 0));

        assert_eq!(
            callback_results(),
            vec![None],
            "an image widget learns its decode failed only from this None"
        );
    }

    #[wasm_test]
    fn a_result_ends_the_pending_decode() {
        let _part = gallery_awaiting(2);

        deliver(job(2), DecodeOutcome::Failed);

        assert!(!image_decode_is_pending(job(2)));
    }

    #[wasm_test]
    fn a_dropped_part_still_clears_its_decode() {
        let part = gallery_awaiting(3);

        drop(part);
        deliver(job(3), DecodeOutcome::Failed);

        assert!(
            !image_decode_is_pending(job(3)),
            "the decode is reported once"
        );
    }
}

define_slot! {
    /// Slot for a dynamic raster bitmap rendered with nearest-neighbor
    /// filtering (pixel-art, 9-patch).
    BitmapNearestSlot => BitmapId,
    |name, data| host::register_bitmap_nearest(name, data)
}

define_slot! {
    /// Slot for a dynamic icon (compact binary SVG-path representation
    /// from the `include_svg!` macro's wire format).
    IconSlot => SvgId,
    |name, data| host::register_svg(name, data)
}

define_slot! {
    /// Slot for a dynamic mesh (compact binary mesh format from the
    /// `include_mesh!` macro's wire format).
    MeshSlot => MeshId,
    |name, data| host::register_mesh(name, data)
}

define_slot! {
    /// Slot for a dynamic audio sample (WAV/OGG/MP3 bytes).
    AudioSlot => AudioId,
    |name, data| host::register_audio(data, name)
}
