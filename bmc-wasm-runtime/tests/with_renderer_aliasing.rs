// Copyright (C) 2026  Braiins Systems s.r.o.

//! Aliasing/install/use/clear/trap-path coverage for
//! `WasmWidgetRuntime::with_renderer`.
//!
//! The substantive test bodies and a `headless_egl` module copied from
//! `tests/lifecycle.rs` are added in the constructor-cutover commit; this file
//! exists in commit 1 only so the file-creation diff is separate from the
//! cutover diff.
//!
//! Run under Miri once the body lands:
//!     MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-strict-provenance" \
//!         cargo +nightly miri test --test with_renderer_aliasing

#![cfg(target_os = "linux")]

#[test]
fn placeholder_until_constructor_cutover() {
    // Body lands in Task 2 Step 9.
}
