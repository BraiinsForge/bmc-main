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

// bmc-virt-console: Host app for the virtual BMC device.
// Connects to the relay daemon via TCP IPC and renders a skeuomorphic
// device view with GPU-accelerated rotation.

mod ansi;
mod app;
mod device_frame;
mod fb_texture;
mod icons;
mod input;
mod led_glow;
mod log_panel;

/// Ensures only one console instance runs at a time via a lock file.
/// The lock is held for the lifetime of the process and cleaned up on drop.
struct SingleInstance {
    path: std::path::PathBuf,
}

impl SingleInstance {
    fn acquire() -> Result<Self, String> {
        let path = std::env::temp_dir().join("bmc-virt-console.lock");

        // Check if an existing instance is running
        if let Ok(contents) = std::fs::read_to_string(&path)
            && let Ok(pid) = contents.trim().parse::<u32>()
            && std::path::Path::new(&format!("/proc/{pid}")).exists()
        {
            return Err(format!(
                "another console instance is already running (PID {pid})"
            ));
        }

        // Write our PID
        std::fs::write(&path, std::process::id().to_string())
            .map_err(|e| format!("failed to write lock file: {e}"))?;

        Ok(Self { path })
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt::init();

    let _lock = match SingleInstance::acquire() {
        Ok(lock) => lock,
        Err(msg) => {
            eprintln!("{msg}");
            // TODO: focus the existing window (needs IPC or X11/Wayland activation)
            std::process::exit(0);
        }
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 800.0])
            .with_title("BMC Virtual Console"),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    eframe::run_native(
        "BMC Virtual Console",
        options,
        Box::new(|cc| Ok(Box::new(app::ConsoleApp::new(cc)))),
    )
}
