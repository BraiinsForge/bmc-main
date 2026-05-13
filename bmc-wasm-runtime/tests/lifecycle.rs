// Copyright (C) 2026  Braiins Systems s.r.o.

//! Integration tests for the params/lifecycle wire (Stage E of BDK-432).
//!
//! These tests construct a real `WasmWidgetRuntime` with a hand-rolled WAT probe widget
//! and assert the behaviours documented in `docs/devlogs/BDK-432-wasm-widget-params/PLAN.md`:
//!
//! * `on_params_update` does NOT fire for the initial delivery via `RuntimeConfig::params`.
//! * `on_params_update` DOES fire for every subsequent `deliver_params_update` call.
//! * The host bumps the version counter on every install; consecutive bumps produce distinct
//!   values (the "different = changed" contract — wrapping is OK, but distinct values must hold
//!   for tests that perform a small number of pushes).
//! * Inside `on_params_update`, the snapshot the guest fetches via `host_params_snapshot`
//!   reflects the just-pushed table (not the previous one).
//! * The lifecycle guard for `host_submit_tree` traps when called from `on_params_update`.
//!
//! ## Headless GL
//!
//! `WasmWidgetRuntime::new` needs a current EGL/GL context for FemtoVG.
//! The `ci` build profile (`nix/profiles.nix`) supplies Mesa, llvmpipe
//! and surfaceless EGL inside the Nix sandbox.
//! Locally without Nix the EGL init will fail, and each test then skips
//! with a clear log line rather than spuriously failing.
//!
//! The expectation is that CI (which runs through the `ci`
//! profile) is the authoritative environment for these tests.

#![cfg(target_os = "linux")]

use std::collections::BTreeMap;

use bmc_wasm_runtime::{RuntimeConfig, WasmWidgetRuntime};
use bmc_widget_manifest::{ParamKey, ParamValue};

// ── Headless EGL setup ──────────────────────────────────────────────
// Pbuffer + surfaceless on Linux via glutin. Mirrors `bmc-wasm-runtime/src/bin/capture/run.rs`
// but stripped to the test's needs (no FBO management beyond construction).

mod headless_egl {
    use anyhow::{Context, Result};
    use glutin::api::egl::device::Device;
    use glutin::api::egl::display::Display as EglDisplay;
    use glutin::config::{ConfigSurfaceTypes, ConfigTemplateBuilder};
    use glutin::context::{ContextApi, ContextAttributesBuilder};
    use glutin::display::{Display, GetGlDisplay};
    use glutin::prelude::*;
    use glutin::surface::{PbufferSurface, Surface, SurfaceAttributesBuilder};
    use std::ffi::{CString, c_void};
    use std::num::NonZeroU32;

    /// Headless GL context tied to a pbuffer; kept alive for the lifetime of the widget
    /// runtime through `_resources` below.
    pub struct HeadlessGl {
        pub display: Display,
        pub fbo_id: u32,
        /// Ownership root for the GL resources backing this context. Never accessed after
        /// construction — the `_` prefix tells the compiler that's intentional. Held only
        /// so its `Drop` impls fire when the test ends; see [`GlResources`] for drop order.
        _resources: GlResources,
    }

    /// Ownership root for the GL resources `HeadlessGl` keeps alive. Fields drop in declaration
    /// order, which matters: the pbuffer surface must drop before the context that made it
    /// current (glutin enforces this at runtime), and texture / context-wrapped glow handles
    /// release their underlying GL state once the context goes.
    ///
    /// Every field is intentionally write-only — held to keep the GL state alive for as long
    /// as `HeadlessGl` lives, never read after construction.
    #[expect(dead_code, reason = "ownership markers — see struct doc")]
    struct GlResources {
        surface: Surface<PbufferSurface>,
        context: glutin::context::PossiblyCurrentContext,
        gl: glow::Context,
        texture: glow::Texture,
    }

    impl HeadlessGl {
        /// Return a `get_proc_address` closure suitable for handing to `WasmWidgetRuntime::new`.
        pub fn proc_address(&self) -> impl FnMut(&str) -> *const c_void + '_ {
            let display = self.display.clone();
            move |s: &str| display.get_proc_address(&CString::new(s).unwrap_or_default())
        }
    }

    /// Force Mesa onto its software (llvmpipe) path before EGL/libdrm see any context.
    /// Mirrors the `ci` Nix build profile so local and CI runs probe the same backend;
    /// also sidesteps libdrm's `pci id for fd N:` stderr noise on dev boxes with a real GPU.
    ///
    /// `std::sync::Once` is fine because EGL is only touched through this module —
    /// nothing else in the test binary loads it earlier.
    fn force_software_egl() {
        static SET_ENV: std::sync::Once = std::sync::Once::new();
        SET_ENV.call_once(|| {
            // SAFETY: single-threaded test init; env-var mutation is safe before any other
            // thread or dlopen'd library reads these. Re-entry is guarded by `Once`.
            unsafe {
                std::env::set_var("EGL_PLATFORM", "surfaceless");
                std::env::set_var("MESA_LOADER_DRIVER_OVERRIDE", "llvmpipe");
                std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");
            }
        });
    }

    /// Try to build a surfaceless EGL display + GL context at `w × h`.
    /// Returns `None` and logs a skip-reason when EGL initialization fails
    /// — the common case is running `cargo test` outside the Nix `ci` profile,
    /// which is fine for local dev but means these tests are no-ops there.
    pub fn try_init(w: u32, h: u32) -> Option<HeadlessGl> {
        match init(w, h) {
            Ok(state) => Some(state),
            Err(err) => {
                eprintln!(
                    "skipping lifecycle integration test: headless EGL init failed — {err:#}\n\
                     (this test requires the `ci` build profile from nix/profiles.nix, \
                     which supplies Mesa + llvmpipe + surfaceless EGL)"
                );
                None
            }
        }
    }

    fn init(w: u32, h: u32) -> Result<HeadlessGl> {
        force_software_egl();
        let devices: Vec<_> = Device::query_devices()
            .context("EGL device enumeration not supported (missing EGL_EXT_device_query)")?
            .collect();
        let device = devices
            .iter()
            .find(|d| d.extensions().contains("EGL_MESA_device_software"))
            .or_else(|| devices.first())
            .context("no EGL devices found — is libEGL.so.1 resolvable?")?;
        let egl_display = unsafe { EglDisplay::with_device(device, None) }
            .context("failed to create EGL display from device")?;
        let display = Display::Egl(egl_display);

        let template = ConfigTemplateBuilder::new()
            .with_alpha_size(8)
            .with_stencil_size(8)
            .with_surface_type(ConfigSurfaceTypes::PBUFFER)
            .build();
        let gl_config = unsafe { display.find_configs(template) }
            .map_err(|e| anyhow::anyhow!("failed to find GL configs: {e}"))?
            .next()
            .context("no suitable GL config found")?;

        let gl_display = gl_config.display();
        let context_attrs = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::Gles(Some(glutin::context::Version::new(2, 0))))
            .build(None);
        let gl_context = unsafe { gl_display.create_context(&gl_config, &context_attrs) }
            .context("failed to create GL context")?;

        let surface_attrs = SurfaceAttributesBuilder::<PbufferSurface>::new().build(
            NonZeroU32::new(w).expect("BUG: zero width"),
            NonZeroU32::new(h).expect("BUG: zero height"),
        );
        let surface = unsafe { gl_display.create_pbuffer_surface(&gl_config, &surface_attrs) }
            .context("failed to create pbuffer surface")?;
        let gl_context = gl_context
            .make_current(&surface)
            .context("failed to make GL context current")?;

        let gl = unsafe {
            glow::Context::from_loader_function(|s| {
                gl_display.get_proc_address(&CString::new(s).unwrap_or_default())
            })
        };

        let (fbo, texture) = create_fbo(&gl, w, h)?;
        let fbo_id = fbo.0.get();

        Ok(HeadlessGl {
            display,
            fbo_id,
            _resources: GlResources {
                surface,
                context: gl_context,
                gl,
                texture,
            },
        })
    }

    fn create_fbo(
        gl: &glow::Context,
        w: u32,
        h: u32,
    ) -> Result<(glow::Framebuffer, glow::Texture)> {
        use glow::HasContext;
        // GL constants and dimensions arrive as `u32` from glow but the GL API takes `GLint`
        // (`i32`). The values in play (texture size enums, RGBA, LINEAR) are all well under
        // `i32::MAX`, so the cast is a wire-format pass-through, not a numeric narrowing.
        #[expect(
            clippy::cast_possible_wrap,
            reason = "glow constants + texture dimensions are bounded well below i32::MAX; \
                      these are GL-API u32→GLint pass-throughs"
        )]
        unsafe {
            let texture = gl
                .create_texture()
                .map_err(|e| anyhow::anyhow!("create_texture: {e}"))?;
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                w as i32,
                h as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );

            let fbo = gl
                .create_framebuffer()
                .map_err(|e| anyhow::anyhow!("create_framebuffer: {e}"))?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );

            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if status != glow::FRAMEBUFFER_COMPLETE {
                anyhow::bail!("FBO incomplete: {status:#x}");
            }
            Ok((fbo, texture))
        }
    }
}

// ── Probe widget fixtures (hand-rolled WAT) ─────────────────────────

/// Test widget that counts lifecycle invocations and records observations
/// on each `on_params_update` call. Exports observation getters used by the tests.
///
/// SDK version reported: (0, 1, 0) — matches `bmc_wasm_protocol::SDK_VERSION`
/// at the time of writing. If the host's major bumps, this fixture needs to bump too.
fn probe_widget_wat() -> &'static str {
    // Packed SDK version: major=0 | minor=1<<16 | patch=0<<32 = 0x10000 = 65536.
    // Update this constant in lockstep with `bmc_wasm_protocol::SDK_VERSION` (asserted below).
    r#"
    (module
      (import "env" "host_params_version" (func $host_params_version (result i64)))
      (import "env" "host_params_snapshot"
        (func $host_params_snapshot (param i32 i32) (result i32)))

      (memory (export "memory") 1)

      (global $update_count (mut i32) (i32.const 0))
      (global $render_count (mut i32) (i32.const 0))
      (global $init_count (mut i32) (i32.const 0))
      (global $last_version_in_update (mut i64) (i64.const 0))
      (global $last_snapshot_len_in_update (mut i32) (i32.const 0))

      ;; Required by the host. Returns packed SDK_VERSION = (0, 1, 0).
      (func (export "__bmc_sdk_version") (result i64)
        i64.const 65536)

      ;; Required by the host. Body is intentionally trivial — these tests don't render.
      (func (export "render") (param i32))

      ;; Optional. Counts calls so the test can assert init runs exactly once.
      (func (export "init") (param i32 i32)
        global.get $init_count
        i32.const 1
        i32.add
        global.set $init_count)

      ;; Optional. Counts calls; records the version + snapshot length observed at call time.
      ;; The probe fetches the snapshot into the very start of guest memory (offset 0); these
      ;; tests don't read the snapshot bytes, only the length the host reports.
      (func (export "on_params_update")
        global.get $update_count
        i32.const 1
        i32.add
        global.set $update_count

        call $host_params_version
        global.set $last_version_in_update

        i32.const 0      ;; out_ptr
        i32.const 4096   ;; out_cap — plenty for the small test snapshots
        call $host_params_snapshot
        global.set $last_snapshot_len_in_update)

      (func (export "init_count") (result i32) global.get $init_count)
      (func (export "render_count") (result i32) global.get $render_count)
      (func (export "update_count") (result i32) global.get $update_count)
      (func (export "last_version_in_update") (result i64)
        global.get $last_version_in_update)
      (func (export "last_snapshot_len_in_update") (result i32)
        global.get $last_snapshot_len_in_update))
    "#
}

/// Misbehaving widget: calls `host_submit_tree` from `on_params_update`.
/// Used to assert the lifecycle guard traps the call.
fn misbehaving_submit_tree_wat() -> &'static str {
    r#"
    (module
      (import "env" "host_submit_tree"
        (func $host_submit_tree (param i32 i32 i32 i32)))

      (memory (export "memory") 1)

      (func (export "__bmc_sdk_version") (result i64)
        i64.const 65536)

      (func (export "render") (param i32))

      ;; Calls host_submit_tree with a zero-length tree. The guard should trap before the
      ;; runtime parses any bytes, so the call never actually mutates renderer state.
      (func (export "on_params_update")
        i32.const 0
        i32.const 0
        i32.const 0
        i32.const 0
        call $host_submit_tree))
    "#
}

// ── Helpers ─────────────────────────────────────────────────────────

fn key(s: &str) -> ParamKey {
    ParamKey::try_new(s.to_owned()).expect("BUG: test key must be valid")
}

fn build_runtime(
    wat: &str,
    gl: &headless_egl::HeadlessGl,
    initial_params: BTreeMap<ParamKey, ParamValue>,
) -> WasmWidgetRuntime {
    let wasm = wat::parse_str(wat).expect("BUG: probe WAT must parse");
    let mut proc = gl.proc_address();
    let config = RuntimeConfig {
        params: initial_params,
        ..RuntimeConfig::default()
    };
    unsafe { WasmWidgetRuntime::new(&wasm, &mut proc, 320, 240, gl.fbo_id, config) }
        .expect("BUG: probe runtime must construct")
}

// ── Tests ───────────────────────────────────────────────────────────

#[test]
fn sdk_version_constant_matches_fixture_assumption() {
    // Hardwires the WAT fixture's packed version literal to the host's `SDK_VERSION`.
    // If this assertion fires, the fixture's `i64.const` needs updating.
    let (major, minor, patch) = WasmWidgetRuntime::host_sdk_version();
    let packed = u64::from(major) | (u64::from(minor) << 16) | (u64::from(patch) << 32);
    assert_eq!(
        packed, 65_536,
        "probe WAT fixtures hardcode `i64.const 65536` for `__bmc_sdk_version`; \
         host SDK version drifted to ({major}, {minor}, {patch}) — update fixtures."
    );
}

#[test]
fn initial_params_via_config_do_not_fire_on_params_update() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };

    let mut initial = BTreeMap::new();
    initial.insert(key("foo"), ParamValue::String("bar".into()));
    let mut runtime = build_runtime(probe_widget_wat(), &gl, initial);

    assert_eq!(
        runtime.call_export_i32("init_count"),
        Some(1),
        "init must run exactly once on construction"
    );
    assert_eq!(
        runtime.call_export_i32("update_count"),
        Some(0),
        "RuntimeConfig::params is the initial delivery — on_params_update must NOT fire for it"
    );
}

#[test]
fn deliver_params_update_fires_hook_and_advances_version() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };

    let mut runtime = build_runtime(probe_widget_wat(), &gl, BTreeMap::new());

    let mut delivery = BTreeMap::new();
    delivery.insert(key("foo"), ParamValue::String("hello".into()));
    let hook_ran = runtime.deliver_params_update(delivery);

    assert!(
        hook_ran,
        "deliver_params_update must invoke the exported hook"
    );
    assert_eq!(runtime.call_export_i32("update_count"), Some(1));

    let first_version = runtime
        .call_export_i64("last_version_in_update")
        .expect("widget exports last_version_in_update");
    assert!(
        first_version > 0,
        "version counter must have advanced past initial 0; got {first_version}"
    );

    let snapshot_len = runtime
        .call_export_i32("last_snapshot_len_in_update")
        .expect("widget exports last_snapshot_len_in_update");
    assert!(
        snapshot_len > 4,
        "snapshot inside on_params_update must reflect the just-pushed table \
         (count header + at least one entry); got {snapshot_len} bytes"
    );

    // A second delivery must fire the hook again with a distinct version.
    let mut delivery2 = BTreeMap::new();
    delivery2.insert(key("foo"), ParamValue::String("world".into()));
    let hook_ran2 = runtime.deliver_params_update(delivery2);
    assert!(hook_ran2);
    assert_eq!(runtime.call_export_i32("update_count"), Some(2));

    let second_version = runtime
        .call_export_i64("last_version_in_update")
        .expect("widget exports last_version_in_update");
    assert_ne!(
        second_version, first_version,
        "consecutive deliveries must produce distinct version values \
         (different = changed); got {first_version} both times"
    );
}

#[test]
fn host_submit_tree_traps_outside_render() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };

    let mut runtime = build_runtime(misbehaving_submit_tree_wat(), &gl, BTreeMap::new());

    let hook_ran = runtime.deliver_params_update(BTreeMap::new());
    assert!(
        !hook_ran,
        "calling host_submit_tree from on_params_update must trap the guard, \
         which surfaces as `deliver_params_update` returning false"
    );
}

#[test]
fn widget_without_hook_is_silently_fine() {
    let Some(gl) = headless_egl::try_init(320, 240) else {
        return;
    };

    // Probe widget without an `on_params_update` export.
    let wat = r#"
        (module
          (memory (export "memory") 1)
          (func (export "__bmc_sdk_version") (result i64) i64.const 65536)
          (func (export "render") (param i32)))
    "#;
    let mut runtime = build_runtime(wat, &gl, BTreeMap::new());

    let hook_ran = runtime.deliver_params_update(BTreeMap::new());
    assert!(
        !hook_ran,
        "absent `on_params_update` export must be silently fine — return value `false` \
         signals 'no hook', not a trap"
    );
}
