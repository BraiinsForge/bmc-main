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

//! Handing a finished view frame to the compositor thread.
//!
//! A view renders on its own thread into its own texture, and the compositor
//! samples that texture from the window context. Both sit in one share group,
//! so the texture name crosses freely — but the *ordering* does not: without a
//! sync point the compositor can sample texels the view's GPU work has not
//! written yet, which shows up as a torn or stale frame under load.
//!
//! The view therefore fences after flushing, and the compositor waits on that
//! fence before it samples. The wait is server-side (`glWaitSync`): it orders
//! the GPU's own queues and never blocks the UI thread, which a client wait
//! (`glClientWaitSync`) would.
//!
//! `bmc-gpu-render-lock` answers a similar question for the device, but only
//! for OpenGL ES — the testbed asks for a desktop context first
//! (`window.rs`), so its `GL_VERSION` never matches that crate's ES prefix.

/// How the compositor orders itself behind a view's rendering.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum GpuWait {
    /// `glFenceSync` + server-side `glWaitSync`: core since GL 3.2 / ES 3.0.
    FenceSync,
    /// `glFinish` on the view thread, draining its queue before the handoff.
    /// Correct but serialising, so it costs the parallelism threads buy.
    Finish,
}

impl std::str::FromStr for GpuWait {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "fence" => Ok(Self::FenceSync),
            "finish" => Ok(Self::Finish),
            other => Err(format!("expected `fence` or `finish`, got `{other}`")),
        }
    }
}

/// Pick a strategy from `GL_VERSION`.
///
/// Fence objects are core in GL 3.2 and ES 3.0, so the version alone decides;
/// the `GL_ARB_sync` extension that predates 3.2 is not consulted, since a
/// context that old cannot drive the widget renderer anyway.
pub(crate) fn gpu_wait_for_version(version: &str) -> GpuWait {
    if version_supports_core_sync(version) {
        GpuWait::FenceSync
    } else {
        tracing::warn!(
            version,
            "no core fence sync; view handoff falls back to glFinish"
        );
        GpuWait::Finish
    }
}

/// GL 3.2+ or ES 3.0+, read from the leading version number.
///
/// `GL_VERSION` is `<major>.<minor><vendor detail>` for desktop and
/// `OpenGL ES <major>.<minor><vendor detail>` for ES, so the vendor tail is
/// whatever the driver felt like appending — `4.6 (Core Profile) Mesa 25.1.8`,
/// `4.1 Metal - 90.5`, `OpenGL ES 3.2 Mesa 25.1.8`.
fn version_supports_core_sync(version: &str) -> bool {
    let (number, min_major, min_minor) = match version.strip_prefix("OpenGL ES ") {
        Some(rest) => (rest, 3, 0),
        None => (version, 3, 2),
    };
    let Some((major, minor)) = parse_version(number) else {
        return false;
    };
    (major, minor) >= (min_major, min_minor)
}

/// Leading `<major>.<minor>` of a version string, ignoring the vendor tail.
fn parse_version(version: &str) -> Option<(u32, u32)> {
    let mut parts = version.split('.');
    let major: u32 = parts.next()?.trim().parse().ok()?;
    let minor_digits: String = parts
        .next()?
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let minor: u32 = minor_digits.parse().ok()?;
    Some((major, minor))
}

#[cfg(test)]
mod tests {
    use super::{GpuWait, gpu_wait_for_version};

    #[test]
    fn a_desktop_mesa_context_fences() {
        assert_eq!(
            gpu_wait_for_version("4.6 (Core Profile) Mesa 25.1.8"),
            GpuWait::FenceSync
        );
    }

    #[test]
    fn an_apple_context_fences() {
        assert_eq!(gpu_wait_for_version("4.1 Metal - 90.5"), GpuWait::FenceSync);
    }

    #[test]
    fn an_es_three_context_fences() {
        assert_eq!(
            gpu_wait_for_version("OpenGL ES 3.2 Mesa 25.1.8"),
            GpuWait::FenceSync
        );
    }

    #[test]
    fn an_es_two_context_falls_back() {
        assert_eq!(
            gpu_wait_for_version("OpenGL ES 2.0 Mesa 25.1.8"),
            GpuWait::Finish
        );
    }

    #[test]
    fn a_desktop_context_below_three_two_falls_back() {
        assert_eq!(gpu_wait_for_version("3.1 Mesa 25.1.8"), GpuWait::Finish);
    }

    #[test]
    fn an_unparseable_version_falls_back() {
        assert_eq!(gpu_wait_for_version("garbage"), GpuWait::Finish);
    }
}
