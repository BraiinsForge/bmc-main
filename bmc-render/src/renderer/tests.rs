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

//! `RenderTarget` wraps a renderer to resolve suspended assets at draw time, so
//! every [`super::Renderer`] method has to reach the inner renderer. A defaulted method
//! the wrapper forgets to forward still compiles, silently answering with the
//! default and taking the whole feature behind it quiet — an unforwarded
//! `blit_static_layer_rects`, say, falls back to the unscissored blit and
//! repaints everything, correctly and expensively.
//!
//! This asserts the source itself: every defaulted trait method must appear in
//! the wrapper's impl.

/// Method names declared with a default body in `trait Renderer`.
fn defaulted_trait_methods(src: &str) -> Vec<&str> {
    let trait_body = src
        .split_once("pub trait Renderer")
        .expect("BUG: trait Renderer must be present")
        .1;
    let trait_body = trait_body
        .split_once("pub trait RendererAssetResolver")
        .expect("BUG: RendererAssetResolver must follow trait Renderer")
        .0;
    trait_body
        .lines()
        .filter_map(|line| line.trim().strip_prefix("fn "))
        // A default body opens on the signature line; a required method ends `;`.
        .filter(|rest| rest.trim_end().ends_with('{'))
        .filter_map(|rest| rest.split('(').next())
        .collect()
}

fn wrapper_methods(src: &str) -> Vec<&str> {
    let impl_body = src
        .split_once("impl Renderer for RenderTarget")
        .expect("BUG: RenderTarget must implement Renderer")
        .1;
    impl_body
        .lines()
        .filter_map(|line| line.trim().strip_prefix("fn "))
        .filter_map(|rest| rest.split('(').next())
        .collect()
}

#[test]
fn render_target_forwards_every_defaulted_method() {
    let wrapper = wrapper_methods(include_str!("render_target.rs"));
    let missing: Vec<&str> = defaulted_trait_methods(include_str!("mod.rs"))
        .into_iter()
        .filter(|method| !wrapper.contains(method))
        .collect();
    assert!(
        missing.is_empty(),
        "RenderTarget does not forward {missing:?}; each would silently answer \
         with the trait default instead of reaching the renderer",
    );
}
