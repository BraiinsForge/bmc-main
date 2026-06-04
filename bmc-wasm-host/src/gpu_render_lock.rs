// Copyright (C) 2026  Braiins Systems s.r.o.

use std::{
    ffi::OsStr,
    fs::{File, OpenOptions},
    path::PathBuf,
};

use anyhow::{Context, Result};

const DEFAULT_GPU_RENDER_LOCK_PATH: &str = "/run/bmc-gpu-render.lock";
const GPU_RENDER_LOCK_PATH_ENV: &str = "BMC_GPU_RENDER_LOCK_PATH";

pub(crate) fn lock_path_from_env_value(value: Option<&OsStr>) -> Option<PathBuf> {
    match value {
        Some(value) if value.is_empty() => None,
        Some(value) => Some(PathBuf::from(value)),
        None => Some(PathBuf::from(DEFAULT_GPU_RENDER_LOCK_PATH)),
    }
}

#[derive(Debug)]
pub(crate) struct GpuRenderLock {
    file: Option<File>,
}

impl GpuRenderLock {
    pub(crate) fn from_env() -> Result<Self> {
        let Some(path) =
            lock_path_from_env_value(std::env::var_os(GPU_RENDER_LOCK_PATH_ENV).as_deref())
        else {
            return Ok(Self { file: None });
        };

        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&path)
            .with_context(|| format!("open GPU render lock {}", path.display()))?;
        tracing::info!(
            lock_path = %path.display(),
            "using cross-process GPU render lock"
        );
        Ok(Self { file: Some(file) })
    }

    pub(crate) fn lock(&self, scope: &'static str) -> Result<GpuRenderLockGuard> {
        let Some(file) = self.file.as_ref() else {
            return Ok(GpuRenderLockGuard { file: None, scope });
        };

        let file = file
            .try_clone()
            .with_context(|| format!("clone diagnostic GPU render lock fd for {scope}"))?;
        tracing::debug!(scope, "waiting for GPU render lock");
        rustix::fs::flock(&file, rustix::fs::FlockOperation::LockExclusive)
            .with_context(|| format!("lock GPU render scope {scope}"))?;
        tracing::debug!(scope, "acquired GPU render lock");
        Ok(GpuRenderLockGuard {
            file: Some(file),
            scope,
        })
    }
}

#[must_use]
pub(crate) struct GpuRenderLockGuard {
    file: Option<File>,
    scope: &'static str,
}

impl Drop for GpuRenderLockGuard {
    fn drop(&mut self) {
        if let Some(file) = self.file.as_ref()
            && let Err(e) = rustix::fs::flock(file, rustix::fs::FlockOperation::Unlock)
        {
            tracing::warn!(
                scope = self.scope,
                error = ?e,
                "failed to release GPU render lock"
            );
        }
        tracing::debug!(scope = self.scope, "released GPU render lock");
    }
}

#[cfg(test)]
mod tests {
    use super::lock_path_from_env_value;
    use std::{ffi::OsStr, path::Path};

    #[test]
    fn defaults_missing_lock_path() {
        assert_eq!(
            lock_path_from_env_value(None).as_deref(),
            Some(Path::new("/run/bmc-gpu-render.lock"))
        );
    }

    #[test]
    fn ignores_empty_lock_path() {
        assert_eq!(lock_path_from_env_value(Some(OsStr::new(""))), None);
    }

    #[test]
    fn accepts_non_empty_lock_path() {
        let path = lock_path_from_env_value(Some(OsStr::new("/run/bmc-gpu-render.lock")));

        assert_eq!(path.as_deref(), Some(Path::new("/run/bmc-gpu-render.lock")));
    }
}
