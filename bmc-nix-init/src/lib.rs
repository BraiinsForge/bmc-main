// Copyright (C) 2026  Braiins Systems s.r.o.

pub mod proxy;

#[expect(warnings)]
mod generated {
    slint::include_modules!();
}
pub use generated::*;
