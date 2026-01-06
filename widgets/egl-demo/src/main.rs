// Copyright (C) 2025  Braiins Systems s.r.o.
//
//! Pure EGL/GLES demo with DMA-BUF buffer sharing via GBM
//!
//! This demo uses GBM-based EGL (like flip-clock) to prove DMA-BUF works with
//! the compositor. It opens the GPU device directly, creates GBM buffer objects,
//! and exports them as DMA-BUF via the zwp_linux_dmabuf_v1 protocol.

use anyhow::{Context, Result};
use drm_fourcc::{DrmFourcc, DrmModifier};
use glow::HasContext;
use smithay::{
    backend::{
        drm::DrmDeviceFd,
        egl::{EGLContext, EGLDisplay},
    },
    reexports::gbm::{AsRaw, BufferObject, BufferObjectFlags, Device as GbmDevice},
};
use std::ffi::c_void;
use std::fs::OpenOptions;
use std::os::fd::{AsFd, OwnedFd};
use std::ptr;
use std::time::Instant;
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    protocol::{wl_buffer, wl_callback, wl_compositor, wl_registry, wl_surface},
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1, zwp_linux_dmabuf_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 240;
// const GPU_PATH: &str = "/dev/dri/renderD128";
const GPU_PATH: &str = "/dev/dri/card0";

// EGL constants
const EGL_NATIVE_PIXMAP_KHR: u32 = 0x30B0;
const EGL_NONE: i32 = 0x3038;
const EGL_NO_IMAGE: *mut c_void = ptr::null_mut();
const GL_TEXTURE_2D: u32 = 0x0DE1;

// Type aliases for EGL function pointers
type EglCreateImageKhr = unsafe extern "C" fn(
    dpy: *mut c_void,
    ctx: *mut c_void,
    target: u32,
    buffer: *mut c_void,
    attrib_list: *const i32,
) -> *mut c_void;

type GlEglImageTargetTexture2DOes = unsafe extern "C" fn(target: u32, image: *mut c_void);

/// Render buffer with GBM BO, EGLImage, OpenGL FBO, and Wayland buffer
struct RenderBuffer {
    bo: BufferObject<()>,
    #[expect(dead_code, reason = "must be kept alive for the FBO")]
    egl_image: *mut c_void,
    #[expect(dead_code, reason = "must be kept alive for the FBO")]
    texture: glow::Texture,
    fbo: glow::Framebuffer,
    /// Pre-created Wayland buffer for reuse (avoids leak)
    wl_buffer: Option<wl_buffer::WlBuffer>,
}

/// EGL state for GBM-based rendering
struct EglState {
    gbm: GbmDevice<DrmDeviceFd>,
    egl_display_raw: *mut c_void,
    #[expect(dead_code, reason = "kept alive for context lifetime")]
    egl_display: EGLDisplay,
    #[expect(dead_code, reason = "kept alive for GL operations")]
    egl_context: EGLContext,
    gl: glow::Context,
    egl_create_image: EglCreateImageKhr,
    gl_image_target_texture: GlEglImageTargetTexture2DOes,
    // Double buffering
    buffers: [Option<RenderBuffer>; 2],
    current_buffer: usize,
    width: u32,
    height: u32,
}

impl EglState {
    fn new(width: u32, height: u32) -> Result<Self> {
        tracing::info!(
            "Initializing GBM-based EGL for {}x{} rendering",
            width,
            height
        );

        // Open GPU device directly
        let gpu_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(GPU_PATH)
            .context("Failed to open GPU device")?;

        let gpu_fd = DrmDeviceFd::new(OwnedFd::from(gpu_file).into());
        tracing::debug!("Opened GPU device: {}", GPU_PATH);

        // Create GBM device
        let gbm = GbmDevice::new(gpu_fd).context("Failed to create GBM device")?;
        tracing::debug!("GBM device created");

        // Create EGL display from GBM
        let egl_display = unsafe { EGLDisplay::new(gbm.clone()) }
            .context("Failed to create EGL display from GBM")?;

        let egl_display_raw = egl_display.get_display_handle().handle.cast_mut();
        tracing::info!("EGL display created: {:?}", egl_display_raw);

        // Create EGL context
        let egl_context = EGLContext::new(&egl_display).context("Failed to create EGL context")?;
        tracing::info!("EGL context created");

        // Make context current (surfaceless)
        let _ = unsafe { egl_context.make_current() };

        // Load EGL extension: eglCreateImageKHR
        let egl_create_image: EglCreateImageKhr = unsafe {
            let proc = smithay::backend::egl::get_proc_address("eglCreateImageKHR");
            if proc.is_null() {
                anyhow::bail!("eglCreateImageKHR not available");
            }
            std::mem::transmute(proc)
        };

        // Load GL extension: glEGLImageTargetTexture2DOES
        let gl_image_target_texture: GlEglImageTargetTexture2DOes = unsafe {
            let proc = smithay::backend::egl::get_proc_address("glEGLImageTargetTexture2DOES");
            if proc.is_null() {
                anyhow::bail!("glEGLImageTargetTexture2DOES not available");
            }
            std::mem::transmute(proc)
        };

        // Create glow context
        let gl = unsafe {
            glow::Context::from_loader_function(|symbol| {
                smithay::backend::egl::get_proc_address(symbol)
            })
        };

        // Log OpenGL ES info
        let version = unsafe { gl.get_parameter_string(glow::VERSION) };
        let renderer = unsafe { gl.get_parameter_string(glow::RENDERER) };
        tracing::info!("OpenGL ES: {} ({})", version, renderer);

        Ok(Self {
            gbm,
            egl_display_raw,
            egl_display,
            egl_context,
            gl,
            egl_create_image,
            gl_image_target_texture,
            buffers: [None, None],
            current_buffer: 0,
            width,
            height,
        })
    }

    #[expect(clippy::cast_possible_wrap, reason = "GL constants fit in i32")]
    fn allocate_buffer(&mut self) -> Result<RenderBuffer> {
        use smithay::reexports::gbm::Format;

        tracing::debug!("Allocating {}x{} GBM buffer", self.width, self.height);

        // Create GBM buffer object
        let bo = self
            .gbm
            .create_buffer_object::<()>(
                self.width,
                self.height,
                Format::Xrgb8888,
                BufferObjectFlags::RENDERING | BufferObjectFlags::LINEAR,
            )
            .context("Failed to create GBM buffer object")?;

        tracing::debug!(
            "GBM BO created: {}x{}, stride={}",
            bo.width(),
            bo.height(),
            bo.stride()
        );

        // Create EGLImage from GBM BO
        let attribs = [EGL_NONE];
        let egl_image = unsafe {
            (self.egl_create_image)(
                self.egl_display_raw,
                ptr::null_mut(), // EGL_NO_CONTEXT
                EGL_NATIVE_PIXMAP_KHR,
                bo.as_raw() as *mut c_void,
                attribs.as_ptr(),
            )
        };

        if egl_image == EGL_NO_IMAGE {
            anyhow::bail!("Failed to create EGLImage from GBM BO");
        }

        // Create OpenGL texture
        let texture = unsafe {
            self.gl
                .create_texture()
                .map_err(|e| anyhow::anyhow!("Failed to create texture: {}", e))?
        };

        // Bind EGLImage to texture
        unsafe {
            self.gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            (self.gl_image_target_texture)(GL_TEXTURE_2D, egl_image);

            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
        }

        // Create framebuffer object
        let fbo = unsafe {
            self.gl
                .create_framebuffer()
                .map_err(|e| anyhow::anyhow!("Failed to create framebuffer: {}", e))?
        };

        // Attach texture to framebuffer
        unsafe {
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            self.gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );

            let status = self.gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if status != glow::FRAMEBUFFER_COMPLETE {
                anyhow::bail!("Framebuffer incomplete: 0x{:x}", status);
            }
        }

        tracing::debug!("FBO created with EGLImage-backed texture");

        Ok(RenderBuffer {
            bo,
            egl_image,
            texture,
            fbo,
            wl_buffer: None,
        })
    }

    fn begin_frame(&mut self) -> Result<()> {
        let idx = self.current_buffer;
        if self.buffers[idx].is_none() {
            self.buffers[idx] = Some(self.allocate_buffer()?);
        }

        let fbo = self.buffers[idx]
            .as_ref()
            .expect("BUG: buffer should exist after allocation")
            .fbo;

        unsafe {
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            #[expect(clippy::cast_possible_wrap, reason = "dimensions fit in i32")]
            self.gl
                .viewport(0, 0, self.width as i32, self.height as i32);
        }
        Ok(())
    }

    /// Complete the frame and return the buffer index that was rendered to
    fn end_frame(&mut self) -> usize {
        // Simple glFinish to ensure GPU is done before sharing buffer
        unsafe {
            self.gl.finish();
        }

        let idx = self.current_buffer;

        // Swap double buffers for next frame
        self.current_buffer = 1 - self.current_buffer;

        idx
    }

    /// Get the wl_buffer for a given buffer index, creating it if needed
    fn get_or_create_wl_buffer(
        &mut self,
        idx: usize,
        linux_dmabuf: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
        qh: &QueueHandle<WaylandState>,
    ) -> Result<&wl_buffer::WlBuffer> {
        let buffer = self.buffers[idx]
            .as_mut()
            .expect("BUG: buffer should exist after begin_frame");

        if buffer.wl_buffer.is_none() {
            // Create wl_buffer from DMA-BUF (only once per buffer)
            let fd = buffer
                .bo
                .fd()
                .context("Failed to get DMA-BUF fd from GBM BO")?;

            let params = linux_dmabuf.create_params(qh, ());

            let modifier: u64 = DrmModifier::Linear.into();
            let modifier_hi = (modifier >> 32) as u32;
            let modifier_lo = (modifier & 0xFFFF_FFFF) as u32;

            params.add(
                fd.as_fd(),
                0, // plane index
                0, // offset
                buffer.bo.stride(),
                modifier_hi,
                modifier_lo,
            );

            #[expect(clippy::cast_possible_wrap, reason = "buffer dimensions fit in i32")]
            let wl_buf = params.create_immed(
                self.width as i32,
                self.height as i32,
                DrmFourcc::Xrgb8888 as u32,
                zwp_linux_buffer_params_v1::Flags::empty(),
                qh,
                (),
            );

            // Destroy params - it's no longer needed after create_immed
            params.destroy();

            tracing::info!("Created wl_buffer for buffer index {}", idx);
            buffer.wl_buffer = Some(wl_buf);
        }

        Ok(buffer
            .wl_buffer
            .as_ref()
            .expect("BUG: wl_buffer should exist after creation"))
    }

    fn gl(&self) -> &glow::Context {
        &self.gl
    }
}

/// Wayland state
struct WaylandState {
    running: bool,
    compositor: Option<wl_compositor::WlCompositor>,
    xdg_wm_base: Option<xdg_wm_base::XdgWmBase>,
    linux_dmabuf: Option<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
    surface: Option<wl_surface::WlSurface>,
    xdg_surface: Option<xdg_surface::XdgSurface>,
    xdg_toplevel: Option<xdg_toplevel::XdgToplevel>,
    configured: bool,
    needs_render: bool,
    frame_count: u32,
}

#[expect(
    clippy::too_many_lines,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::integer_division,
    reason = "main function for demo"
)]
fn main() -> Result<()> {
    const TARGET_FRAME_TIME: std::time::Duration = std::time::Duration::from_millis(16); // ~60 FPS max

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    tracing::info!("Starting GBM-based EGL DMA-BUF demo");
    tracing::info!("Target resolution: {}x{}", WIDTH, HEIGHT);

    // Connect to Wayland
    let conn = Connection::connect_to_env().context("Failed to connect to Wayland")?;
    tracing::info!("Connected to Wayland");

    let mut queue = conn.new_event_queue();
    let qh = queue.handle();

    let display = conn.display();
    display.get_registry(&qh, ());

    let mut state = WaylandState {
        running: true,
        compositor: None,
        xdg_wm_base: None,
        linux_dmabuf: None,
        surface: None,
        xdg_surface: None,
        xdg_toplevel: None,
        configured: false,
        needs_render: false,
        frame_count: 0,
    };

    // Roundtrip to get globals
    queue
        .roundtrip(&mut state)
        .context("Failed to roundtrip for globals")?;

    // Verify required globals
    let compositor = state.compositor.as_ref().context("No wl_compositor")?;
    let xdg_wm_base = state.xdg_wm_base.as_ref().context("No xdg_wm_base")?;
    let linux_dmabuf = state
        .linux_dmabuf
        .clone()
        .context("No zwp_linux_dmabuf_v1")?;

    // Create surface
    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = xdg_wm_base.get_xdg_surface(&surface, &qh, ());
    let xdg_toplevel = xdg_surface.get_toplevel(&qh, ());

    xdg_toplevel.set_title("EGL DMA-BUF Demo".to_owned());
    xdg_toplevel.set_app_id("bmc-egl-demo".to_owned());
    surface.commit();

    state.surface = Some(surface);
    state.xdg_surface = Some(xdg_surface);
    state.xdg_toplevel = Some(xdg_toplevel);

    // Wait for configure
    queue
        .roundtrip(&mut state)
        .context("Failed to roundtrip for configure")?;
    tracing::info!("Window configured");

    // Initialize GBM-based EGL
    let mut egl = EglState::new(WIDTH, HEIGHT)?;
    tracing::info!("GBM-based EGL initialized");

    // Request first frame callback
    if let Some(ref surface) = state.surface {
        surface.frame(&qh, ());
        surface.commit();
    }

    // Animation state
    let mut ball_x: f32 = 320.0;
    let mut ball_y: f32 = 120.0;
    let mut velocity_x: f32 = 5.0;
    let mut velocity_y: f32 = 3.5;

    // FPS tracking
    let mut fps_frame_count = 0_u32;
    let mut last_fps_time = Instant::now();

    // Frame rate limiting to prevent thermal throttling
    let mut last_frame_time = Instant::now();

    tracing::info!("Starting render loop...");

    while state.running {
        // Dispatch Wayland events
        queue
            .blocking_dispatch(&mut state)
            .context("Wayland dispatch failed")?;

        if state.needs_render {
            state.needs_render = false;

            // Frame rate limiting - sleep if we're rendering too fast
            let elapsed_since_last_frame = last_frame_time.elapsed();
            if elapsed_since_last_frame < TARGET_FRAME_TIME {
                std::thread::sleep(TARGET_FRAME_TIME - elapsed_since_last_frame);
            }
            last_frame_time = Instant::now();

            // Update animation
            ball_x += velocity_x;
            ball_y += velocity_y;

            let max_x = WIDTH as f32 - 20.0;
            let max_y = HEIGHT as f32 - 20.0;

            if ball_x <= 20.0 || ball_x >= max_x {
                velocity_x = -velocity_x;
                ball_x = ball_x.clamp(20.0, max_x);
            }
            if ball_y <= 20.0 || ball_y >= max_y {
                velocity_y = -velocity_y;
                ball_y = ball_y.clamp(20.0, max_y);
            }

            // Begin frame
            egl.begin_frame()?;

            let gl = egl.gl();

            // Render with OpenGL ES - minimal: clear + one ball
            unsafe {
                // Clear background
                gl.clear_color(0.05, 0.05, 0.1, 1.0);
                gl.clear(glow::COLOR_BUFFER_BIT);

                // Draw single ball (white)
                gl.enable(glow::SCISSOR_TEST);

                let ball_size = 40_i32;
                #[expect(clippy::cast_possible_truncation)]
                let ball_x_i32 = ball_x as i32;
                #[expect(clippy::cast_possible_truncation)]
                let ball_y_i32 = ball_y as i32;
                let height_i32 = HEIGHT as i32;

                gl.scissor(
                    ball_x_i32 - ball_size / 2,
                    height_i32 - ball_y_i32 - ball_size / 2,
                    ball_size,
                    ball_size,
                );
                gl.clear_color(1.0, 1.0, 1.0, 1.0);
                gl.clear(glow::COLOR_BUFFER_BIT);

                gl.disable(glow::SCISSOR_TEST);
            }

            // End frame and get buffer index
            let buffer_idx = egl.end_frame();

            // Get or create wl_buffer (reuses existing buffer, avoids leak)
            let buffer = egl.get_or_create_wl_buffer(buffer_idx, &linux_dmabuf, &qh)?;

            // Attach buffer to surface
            if let Some(ref surface) = state.surface {
                surface.attach(Some(buffer), 0, 0);
                #[expect(clippy::cast_possible_wrap, reason = "dimensions fit in i32")]
                surface.damage_buffer(0, 0, WIDTH as i32, HEIGHT as i32);
                surface.frame(&qh, ());
                surface.commit();
            }

            // FPS calculation
            fps_frame_count += 1;
            let elapsed = last_fps_time.elapsed();
            if elapsed.as_secs() >= 1 {
                tracing::info!("FPS: {} (DMA-BUF/GBM path)", fps_frame_count);
                fps_frame_count = 0;
                last_fps_time = Instant::now();
            }

            state.frame_count = state.frame_count.wrapping_add(1);
        }
    }

    Ok(())
}

// === Wayland Protocol Implementations ===

impl Dispatch<wl_registry::WlRegistry, ()> for WaylandState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        (): &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    let compositor = registry.bind::<wl_compositor::WlCompositor, _, _>(
                        name,
                        version.min(6),
                        qh,
                        (),
                    );
                    tracing::debug!("Bound wl_compositor v{}", version.min(6));
                    state.compositor = Some(compositor);
                }
                "xdg_wm_base" => {
                    let xdg =
                        registry.bind::<xdg_wm_base::XdgWmBase, _, _>(name, version.min(6), qh, ());
                    tracing::debug!("Bound xdg_wm_base v{}", version.min(6));
                    state.xdg_wm_base = Some(xdg);
                }
                "zwp_linux_dmabuf_v1" => {
                    let dmabuf = registry.bind::<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, _, _>(
                        name,
                        version.min(4),
                        qh,
                        (),
                    );
                    tracing::debug!("Bound zwp_linux_dmabuf_v1 v{}", version.min(4));
                    state.linux_dmabuf = Some(dmabuf);
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &wl_compositor::WlCompositor,
        _: wl_compositor::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &wl_surface::WlSurface,
        _: wl_surface::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for WaylandState {
    fn event(
        _: &mut Self,
        xdg: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            xdg.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for WaylandState {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            state.configured = true;
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Close = event {
            state.running = false;
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.needs_render = true;
        }
    }
}

impl Dispatch<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
        _: zwp_linux_dmabuf_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1,
        event: zwp_linux_buffer_params_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_linux_buffer_params_v1::Event::Failed = event {
            tracing::error!("DMA-BUF buffer creation failed");
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            // Buffer reuse: don't destroy - the buffer is stored in RenderBuffer
            // and will be reused for future frames. Double-buffering ensures
            // we never write to a buffer the compositor is still using.
        }
    }
}
