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

//! A GLES renderer on Mesa's surfaceless EGL, for tests that run real shaders.

use smithay::backend::{
    egl::{EGLContext, EGLDisplay, native::EGLSurfacelessDisplay},
    renderer::gles::GlesRenderer,
};

/// Set by the Nix `ci` profile, which supplies Mesa:
/// an EGL failure there is a broken profile, and a skip would pass a test that never ran.
const REQUIRE_EGL: &str = "BMC_REQUIRE_HEADLESS_EGL";

pub(super) fn headless_renderer() -> Option<GlesRenderer> {
    let init = || -> anyhow::Result<GlesRenderer> {
        // SAFETY: only smithay creates or terminates EGL displays in this test binary,
        // and it shares one tracked instance between the tests that run in parallel.
        let display = unsafe { EGLDisplay::new(EGLSurfacelessDisplay) }?;
        let context = EGLContext::new(&display)?;
        // SAFETY: the context was created just above and is current on no other thread.
        Ok(unsafe { GlesRenderer::new(context) }?)
    };
    match init() {
        Ok(renderer) => Some(renderer),
        Err(err) => {
            assert!(
                std::env::var_os(REQUIRE_EGL).is_none(),
                "{REQUIRE_EGL} is set, so skipping would pass a shader that never compiled: {err:#}"
            );
            eprintln!("skipping: headless EGL init failed, run in the `ci` dev shell: {err:#}");
            None
        }
    }
}
