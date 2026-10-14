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

/// Lines of each top-level item whose header satisfies `header`,
/// up to the item's closing brace at column 0.
fn item_bodies(src: &str, header: impl Fn(&str) -> bool) -> Vec<Vec<&str>> {
    let mut bodies = Vec::new();
    let mut current: Option<Vec<&str>> = None;
    for line in src.lines() {
        match current.as_mut() {
            Some(body) if line == "}" => bodies.push(std::mem::take(body)),
            Some(body) => body.push(line),
            None if header(line) => current = Some(Vec::new()),
            None => {}
        }
        if line == "}" {
            current = None;
        }
    }
    bodies
}

/// The sub-traits `Renderer` gathers, read from its supertrait bound.
fn sub_traits(src: &str) -> Vec<&str> {
    let (_, rest) = src
        .split_once("pub trait Renderer:")
        .expect("BUG: Renderer must name its sub-traits as supertraits");
    let (bound, _) = rest
        .split_once('{')
        .expect("BUG: Renderer's supertrait bound must end at its body");
    bound.split('+').map(str::trim).collect()
}

/// Method names declared with a default body in the renderer sub-traits.
fn defaulted_trait_methods(src: &str) -> Vec<&str> {
    let mut traits = Vec::new();
    for name in sub_traits(src) {
        let header = format!("pub trait {name} {{");
        let found = item_bodies(src, |line| line == header);
        assert_eq!(
            found.len(),
            1,
            "BUG: sub-trait {name} must be declared once, as `{header}`"
        );
        traits.extend(found);
    }
    let mut methods = Vec::new();
    for body in traits {
        let mut pending: Option<&str> = None;
        for line in body.into_iter().map(str::trim) {
            if let Some(rest) = line.strip_prefix("fn ") {
                pending = rest.split('(').next();
            }
            let Some(name) = pending else { continue };
            // A signature may span lines; it ends in `;` when required
            // and in `{` or `{}` when it has a default body.
            if line.ends_with(';') {
                pending = None;
            } else if line.ends_with('{') || line.ends_with("{}") {
                methods.push(name);
                pending = None;
            }
        }
    }
    methods
}

fn wrapper_methods(src: &str) -> Vec<&str> {
    let impls = item_bodies(src, |line| {
        line.starts_with("impl Render") && line.contains(" for RenderTarget")
    });
    assert!(
        !impls.is_empty(),
        "BUG: RenderTarget must implement the renderer sub-traits"
    );
    impls
        .into_iter()
        .flatten()
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
