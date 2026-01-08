// Copyright (C) 2025  Braiins Systems s.r.o.

use std::process::Stdio;

use bmc_widget::env::vars;
use bmc_widget_protocol::SizeType;
use tokio::process::{Child, Command};

use super::WidgetInfo;
use super::coordinator::WidgetEnv;

#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    #[error("failed to spawn process: {0}")]
    SpawnProcess(std::io::Error),

    #[error("failed to serialize params: {0}")]
    SerializeParams(serde_json::Error),

    #[error("failed to serialize localization: {0}")]
    SerializeLocalization(serde_json::Error),
}

/// Configuration for spawning glibc-linked widgets via the dynamic linker.
#[derive(Debug, Clone)]
pub struct LinkerConfig {
    /// Path to the glibc dynamic linker (e.g., `/nix/store/.../ld-linux-armhf.so.3`)
    pub linker_path: String,
    /// Library search path (equivalent to `LD_LIBRARY_PATH`)
    pub library_path: String,
    /// Mesa GBM backends path (GBM_BACKENDS_PATH)
    pub gbm_backends_path: Option<String>,
    /// Mesa DRI drivers path (LIBGL_DRIVERS_PATH)
    pub libgl_drivers_path: Option<String>,
    /// EGL vendor library filenames (__EGL_VENDOR_LIBRARY_FILENAMES)
    pub egl_vendor_library: Option<String>,
}

#[derive(Debug)]
pub struct WaylandSpawner {
    linker: Option<LinkerConfig>,
}

impl WaylandSpawner {
    #[must_use]
    pub fn new() -> Self {
        Self { linker: None }
    }

    #[must_use]
    pub fn with_linker(linker: LinkerConfig) -> Self {
        Self {
            linker: Some(linker),
        }
    }

    pub fn spawn(
        &self,
        widget: &WidgetInfo,
        env: &WidgetEnv,
        xdg_runtime_dir: &str,
    ) -> Result<Child, SpawnError> {
        let params_json =
            serde_json::to_string(&env.params).map_err(SpawnError::SerializeParams)?;

        // If linker config is provided, run via the dynamic linker
        // Otherwise run the binary directly
        let mut cmd = if let Some(ref linker) = self.linker {
            let mut c = Command::new(&linker.linker_path);
            c.arg("--library-path")
                .arg(&linker.library_path)
                .arg(&widget.binary_path);

            // Mesa/EGL environment for GPU rendering
            if let Some(ref path) = linker.gbm_backends_path {
                c.env("GBM_BACKENDS_PATH", path);
            }
            if let Some(ref path) = linker.libgl_drivers_path {
                c.env("LIBGL_DRIVERS_PATH", path);
            }
            if let Some(ref path) = linker.egl_vendor_library {
                c.env("__EGL_VENDOR_LIBRARY_FILENAMES", path);
            }

            c
        } else {
            Command::new(&widget.binary_path)
        };

        // Standard Wayland environment
        cmd.env("WAYLAND_DISPLAY", &env.wayland_display)
            .env("XDG_RUNTIME_DIR", xdg_runtime_dir);

        // Widget configuration via DECK_* env vars
        cmd.env(vars::INSTANCE_ID, &env.instance_id)
            .env(vars::SIZE_TYPE, size_type_to_str(env.size_type))
            .env(vars::WIDTH, env.width.to_string())
            .env(vars::HEIGHT, env.height.to_string())
            .env(vars::PARAMS, params_json);

        // Settings
        if let Some(ref tz) = env.timezone {
            cmd.env(vars::TIMEZONE, tz);
        }
        cmd.env(vars::NIGHT_MODE, if env.night_mode { "1" } else { "0" });

        if let Some(ref loc) = env.localization {
            let loc_json = serde_json::to_string(loc).map_err(SpawnError::SerializeLocalization)?;
            cmd.env(vars::LOCALIZATION, loc_json);
        }

        cmd.stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);

        let child = cmd.spawn().map_err(SpawnError::SpawnProcess)?;

        tracing::info!(
            "widget process spawned: instance={} pid={:?}",
            env.instance_id,
            child.id()
        );

        Ok(child)
    }
}

impl Default for WaylandSpawner {
    fn default() -> Self {
        Self::new()
    }
}

fn size_type_to_str(size: SizeType) -> &'static str {
    match size {
        SizeType::Small => "small",
        SizeType::Medium => "medium",
        SizeType::Large => "large",
        SizeType::Full => "full",
    }
}
