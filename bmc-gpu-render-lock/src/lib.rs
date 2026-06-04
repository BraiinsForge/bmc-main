// Copyright (C) 2026  Braiins Systems s.r.o.

use std::{
    ffi::{OsStr, c_void},
    fs::{File, OpenOptions},
    path::PathBuf,
};

use anyhow::{Context, Result};

const DEFAULT_GPU_RENDER_LOCK_PATH: &str = "/run/bmc-gpu-render.lock";
const GPU_RENDER_LOCK_PATH_ENV: &str = "BMC_GPU_RENDER_LOCK_PATH";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GpuCompletionWaitStrategy {
    GlFenceSync,
    EglFenceSync,
    Finish,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GpuCompletionWaitFallbackReason {
    GlSyncEntryPointUnavailable,
    EglFenceSyncExtensionAbsent,
    GlOesEglSyncExtensionAbsent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GpuCompletionWaitSupport {
    strategy: GpuCompletionWaitStrategy,
    fallback_reason: Option<GpuCompletionWaitFallbackReason>,
}

impl GpuCompletionWaitSupport {
    #[must_use]
    pub fn strategy(self) -> GpuCompletionWaitStrategy {
        self.strategy
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GlSyncEntryPoints {
    pub fence_sync: *const c_void,
    pub client_wait_sync: *const c_void,
    pub delete_sync: *const c_void,
}

#[must_use]
pub fn gpu_completion_wait_strategy(
    gl_version: &str,
    gl_extensions: &str,
    egl_extensions: &[String],
    entry_points: GlSyncEntryPoints,
) -> GpuCompletionWaitStrategy {
    detect_gpu_completion_wait_support(gl_version, gl_extensions, egl_extensions, entry_points)
        .strategy()
}

#[must_use]
fn detect_gpu_completion_wait_support(
    gl_version: &str,
    gl_extensions: &str,
    egl_extensions: &[String],
    entry_points: GlSyncEntryPoints,
) -> GpuCompletionWaitSupport {
    if gles_version_supports_core_sync(gl_version) && entry_points.all_present() {
        return GpuCompletionWaitSupport {
            strategy: GpuCompletionWaitStrategy::GlFenceSync,
            fallback_reason: None,
        };
    }

    if egl_extensions_contain(egl_extensions, "EGL_KHR_fence_sync")
        && gl_extensions_contain(gl_extensions, "GL_OES_EGL_sync")
    {
        return GpuCompletionWaitSupport {
            strategy: GpuCompletionWaitStrategy::EglFenceSync,
            fallback_reason: None,
        };
    }

    let fallback_reason =
        if gles_version_supports_core_sync(gl_version) && !entry_points.all_present() {
            GpuCompletionWaitFallbackReason::GlSyncEntryPointUnavailable
        } else if !egl_extensions_contain(egl_extensions, "EGL_KHR_fence_sync") {
            GpuCompletionWaitFallbackReason::EglFenceSyncExtensionAbsent
        } else if !gl_extensions_contain(gl_extensions, "GL_OES_EGL_sync") {
            GpuCompletionWaitFallbackReason::GlOesEglSyncExtensionAbsent
        } else {
            GpuCompletionWaitFallbackReason::GlSyncEntryPointUnavailable
        };

    GpuCompletionWaitSupport {
        strategy: GpuCompletionWaitStrategy::Finish,
        fallback_reason: Some(fallback_reason),
    }
}

#[must_use]
pub fn detect_gpu_completion_wait_strategy(
    gl_version: &str,
    gl_extensions: &str,
    egl_extensions: &[String],
    entry_points: GlSyncEntryPoints,
) -> GpuCompletionWaitStrategy {
    let support =
        detect_gpu_completion_wait_support(gl_version, gl_extensions, egl_extensions, entry_points);
    log_gpu_completion_wait_support(gl_version, support);
    support.strategy()
}

fn log_gpu_completion_wait_support(gl_version: &str, support: GpuCompletionWaitSupport) {
    match (support.strategy, support.fallback_reason) {
        (GpuCompletionWaitStrategy::GlFenceSync, None) => tracing::info!(
            gl_version,
            "using GL fence sync for GPU render completion waits"
        ),
        (GpuCompletionWaitStrategy::EglFenceSync, None) => tracing::info!(
            gl_version,
            "using EGL fence sync for GPU render completion waits"
        ),
        (
            GpuCompletionWaitStrategy::Finish,
            Some(GpuCompletionWaitFallbackReason::GlSyncEntryPointUnavailable),
        ) => tracing::info!(
            gl_version,
            "using glFinish for GPU render completion waits: GL sync entry points are missing"
        ),
        (
            GpuCompletionWaitStrategy::Finish,
            Some(GpuCompletionWaitFallbackReason::EglFenceSyncExtensionAbsent),
        ) => tracing::info!(
            gl_version,
            "using glFinish for GPU render completion waits: EGL_KHR_fence_sync is missing"
        ),
        (
            GpuCompletionWaitStrategy::Finish,
            Some(GpuCompletionWaitFallbackReason::GlOesEglSyncExtensionAbsent),
        ) => tracing::info!(
            gl_version,
            "using glFinish for GPU render completion waits: GL_OES_EGL_sync is missing"
        ),
        (GpuCompletionWaitStrategy::Finish, None) => {
            tracing::info!(gl_version, "using glFinish for GPU render completion waits");
        }
        (
            GpuCompletionWaitStrategy::GlFenceSync | GpuCompletionWaitStrategy::EglFenceSync,
            Some(_),
        ) => tracing::info!(
            gl_version,
            strategy = ?support.strategy,
            "using GPU fence sync for GPU render completion waits"
        ),
    }
}

impl GlSyncEntryPoints {
    #[must_use]
    pub fn load_with(mut load: impl FnMut(&str) -> *const c_void) -> Self {
        Self {
            fence_sync: load("glFenceSync"),
            client_wait_sync: load("glClientWaitSync"),
            delete_sync: load("glDeleteSync"),
        }
    }

    #[must_use]
    fn all_present(self) -> bool {
        !self.fence_sync.is_null()
            && !self.client_wait_sync.is_null()
            && !self.delete_sync.is_null()
    }
}

fn gles_version_supports_core_sync(version: &str) -> bool {
    let Some(rest) = version.strip_prefix("OpenGL ES ") else {
        return false;
    };
    let Some((major, _minor)) = rest.split_once('.') else {
        return false;
    };
    matches!(major.parse::<u32>(), Ok(major) if major >= 3)
}

fn gl_extensions_contain(extensions: &str, extension: &str) -> bool {
    extensions.split_whitespace().any(|item| item == extension)
}

fn egl_extensions_contain(extensions: &[String], extension: &str) -> bool {
    extensions.iter().any(|item| item == extension)
}

#[must_use]
pub fn lock_path_from_env_value(value: Option<&OsStr>) -> Option<PathBuf> {
    match value {
        Some(value) if value.is_empty() => None,
        Some(value) => Some(PathBuf::from(value)),
        None => Some(PathBuf::from(DEFAULT_GPU_RENDER_LOCK_PATH)),
    }
}

#[derive(Debug)]
pub struct GpuRenderLock {
    file: Option<File>,
}

impl GpuRenderLock {
    pub fn from_env() -> Result<Self> {
        let Some(path) =
            lock_path_from_env_value(std::env::var_os(GPU_RENDER_LOCK_PATH_ENV).as_deref())
        else {
            return Ok(Self { file: None });
        };

        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
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

    pub fn lock(&self, scope: &'static str) -> Result<GpuRenderLockGuard> {
        let Some(file) = self.file.as_ref() else {
            return Ok(GpuRenderLockGuard { file: None, scope });
        };

        let file = file
            .try_clone()
            .with_context(|| format!("clone GPU render lock fd for {scope}"))?;
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
#[derive(Debug)]
pub struct GpuRenderLockGuard {
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
    use super::{
        GlSyncEntryPoints, GpuCompletionWaitStrategy, gpu_completion_wait_strategy,
        lock_path_from_env_value,
    };
    use std::{ffi::OsStr, path::Path, ptr};

    #[test]
    fn defaults_missing_lock_path() {
        assert_eq!(
            lock_path_from_env_value(None).as_deref(),
            Some(Path::new("/run/bmc-gpu-render.lock"))
        );
    }

    #[test]
    fn disables_lock_for_empty_lock_path() {
        assert_eq!(lock_path_from_env_value(Some(OsStr::new(""))), None);
    }

    #[test]
    fn accepts_non_empty_lock_path() {
        let path = lock_path_from_env_value(Some(OsStr::new("/run/bmc-gpu-render.lock")));

        assert_eq!(path.as_deref(), Some(Path::new("/run/bmc-gpu-render.lock")));
    }

    #[test]
    fn uses_fence_only_for_gles3_with_all_sync_entry_points() {
        let present = ptr::dangling();
        let no_egl_extensions = Vec::new();

        assert_eq!(
            gpu_completion_wait_strategy(
                "OpenGL ES 3.0 Mesa",
                "",
                &no_egl_extensions,
                GlSyncEntryPoints {
                    fence_sync: present,
                    client_wait_sync: present,
                    delete_sync: present,
                },
            ),
            GpuCompletionWaitStrategy::GlFenceSync
        );
        assert_eq!(
            gpu_completion_wait_strategy(
                "OpenGL ES 2.0 Mesa",
                "",
                &no_egl_extensions,
                GlSyncEntryPoints {
                    fence_sync: present,
                    client_wait_sync: present,
                    delete_sync: present,
                },
            ),
            GpuCompletionWaitStrategy::Finish
        );
        assert_eq!(
            gpu_completion_wait_strategy(
                "OpenGL ES 3.0 Mesa",
                "",
                &no_egl_extensions,
                GlSyncEntryPoints {
                    fence_sync: present,
                    client_wait_sync: ptr::null(),
                    delete_sync: present,
                },
            ),
            GpuCompletionWaitStrategy::Finish
        );
    }

    #[test]
    fn uses_egl_fence_for_gles2_with_egl_sync_extensions() {
        let egl_extensions = vec!["EGL_KHR_fence_sync".to_string()];

        assert_eq!(
            gpu_completion_wait_strategy(
                "OpenGL ES 2.0 Mesa",
                "GL_EXT_texture_format_BGRA8888 GL_OES_EGL_sync",
                &egl_extensions,
                GlSyncEntryPoints {
                    fence_sync: ptr::null(),
                    client_wait_sync: ptr::null(),
                    delete_sync: ptr::null(),
                },
            ),
            GpuCompletionWaitStrategy::EglFenceSync
        );
    }
}
