// Copyright (C) 2026  Braiins Systems s.r.o.

//! RAII guards for GL handles owned by the mesh renderer.
//!
//! Each guard wraps a freshly-allocated GL handle (buffer, texture,
//! renderbuffer, framebuffer) and deletes it on drop unless `defuse` is
//! called first. This avoids leaks on partial-failure paths in
//! `parse_and_upload` and the FBO setup helpers — any `?` that aborts before
//! all handles are wired together drops the unwired handles instead of
//! leaking them.

use glow::HasContext;

pub(super) struct BufferGuard<'a> {
    gl: &'a glow::Context,
    buf: Option<glow::Buffer>,
}

impl<'a> BufferGuard<'a> {
    pub(super) fn new(gl: &'a glow::Context, buf: glow::Buffer) -> Self {
        Self { gl, buf: Some(buf) }
    }
    pub(super) fn defuse(mut self) -> glow::Buffer {
        self.buf.take().expect("BUG: BufferGuard already defused")
    }
}

impl Drop for BufferGuard<'_> {
    fn drop(&mut self) {
        if let Some(buf) = self.buf.take() {
            unsafe {
                self.gl.delete_buffer(buf);
            }
        }
    }
}

pub(super) struct TextureGuard<'a> {
    gl: &'a glow::Context,
    tex: Option<glow::Texture>,
}

impl<'a> TextureGuard<'a> {
    pub(super) fn new(gl: &'a glow::Context, tex: glow::Texture) -> Self {
        Self { gl, tex: Some(tex) }
    }
    pub(super) fn defuse(mut self) -> glow::Texture {
        self.tex.take().expect("BUG: TextureGuard already defused")
    }
}

impl Drop for TextureGuard<'_> {
    fn drop(&mut self) {
        if let Some(tex) = self.tex.take() {
            unsafe {
                self.gl.delete_texture(tex);
            }
        }
    }
}

pub(super) struct RenderbufferGuard<'a> {
    gl: &'a glow::Context,
    rb: Option<glow::Renderbuffer>,
}

impl<'a> RenderbufferGuard<'a> {
    pub(super) fn new(gl: &'a glow::Context, rb: glow::Renderbuffer) -> Self {
        Self { gl, rb: Some(rb) }
    }
    pub(super) fn defuse(mut self) -> glow::Renderbuffer {
        self.rb
            .take()
            .expect("BUG: RenderbufferGuard already defused")
    }
}

impl Drop for RenderbufferGuard<'_> {
    fn drop(&mut self) {
        if let Some(rb) = self.rb.take() {
            unsafe {
                self.gl.delete_renderbuffer(rb);
            }
        }
    }
}

pub(super) struct FramebufferGuard<'a> {
    gl: &'a glow::Context,
    fbo: Option<glow::Framebuffer>,
}

impl<'a> FramebufferGuard<'a> {
    pub(super) fn new(gl: &'a glow::Context, fbo: glow::Framebuffer) -> Self {
        Self { gl, fbo: Some(fbo) }
    }
    pub(super) fn defuse(mut self) -> glow::Framebuffer {
        self.fbo
            .take()
            .expect("BUG: FramebufferGuard already defused")
    }
}

impl Drop for FramebufferGuard<'_> {
    fn drop(&mut self) {
        if let Some(fbo) = self.fbo.take() {
            unsafe {
                self.gl.delete_framebuffer(fbo);
            }
        }
    }
}
