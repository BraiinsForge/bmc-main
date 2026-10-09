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

//! Regions where a scanout frame differs from the one the panel last received.
//!
//! A shader pass compares whole frames in cells of [`CELL`]×[`CELL`] pixels.
//! It beat a CPU compare of the mapped buffers on BMM101 in both diff time and CPU load;
//! `docs/devel/display-scanout.md` has the numbers.

use anyhow::{Context, Result, bail, ensure};
use smithay::{
    backend::{
        allocator::{Buffer, Fourcc, dmabuf::Dmabuf},
        renderer::{
            Bind, Frame as RendererFrame, ImportDma, Offscreen, Renderer,
            gles::{
                GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName, UniformType, ffi,
            },
        },
    },
    utils::{Buffer as BufferCoord, Physical, Rectangle, Size, Transform},
};

const DIFF_SHADER: &str = include_str!("frame_diff.frag");

/// Edge of the square pixel cell the comparison reports changes in.
const CELL: usize = 4;

/// Line of `frame_diff.frag` that becomes the shader's `CELL` define.
const CELL_DEFINE_MARKER: &str = "//_CELL_";

/// Bytes `ReadPixels` writes for each cell flag, one `RGBA`/`UNSIGNED_BYTE` texel.
const READBACK_TEXEL_BYTES: usize = 4;

/// Texture unit the previous frame is bound to;
/// smithay binds the drawn texture to unit 0.
const PREVIOUS_TEXTURE_UNIT: usize = 1;

/// More errors in a row than GL has distinct error flags to hold.
const MAX_STALE_GL_ERRORS: usize = 16;

/// What a page flip has to send to the panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transfer {
    /// The frame matches the one the panel holds; skip the flip.
    Unchanged,
    /// Only these regions differ.
    Regions(Vec<Rectangle<i32, Physical>>),
    /// The panel's contents are unknown, or nobody tracks them.
    WholeFrame,
}

impl Transfer {
    /// The transfer for `mask`, the diff against the frame the panel holds,
    /// or `None` when that frame is unknown.
    #[must_use]
    pub fn from_diff(mask: Option<&CellMask>) -> Self {
        match mask {
            None => Self::WholeFrame,
            Some(mask) if mask.is_empty() => Self::Unchanged,
            Some(mask) => Self::Regions(mask.rects()),
        }
    }

    /// This transfer for a plane that takes no damage clips:
    /// changed regions go out as the whole frame, and an unchanged frame still skips its flip.
    #[must_use]
    pub fn without_clips(self) -> Self {
        match self {
            Self::Unchanged => Self::Unchanged,
            Self::Regions(_) | Self::WholeFrame => Self::WholeFrame,
        }
    }
}

/// The transfer for the frame about to flip.
/// `diff` compares it with the panel's frame, or returns `None` when that frame is unknown.
/// A failed diff turns `differ` off for good,
/// so a broken GPU path costs one log line rather than one per frame.
pub fn next_transfer<D>(
    differ: &mut Option<D>,
    diff: impl FnOnce(&mut D) -> Result<Option<CellMask>>,
) -> Transfer {
    let Some(active) = differ.as_mut() else {
        return Transfer::WholeFrame;
    };
    match diff(active) {
        Ok(mask) => Transfer::from_diff(mask.as_ref()),
        Err(e) => {
            tracing::error!("Frame diff failed, sending whole frames from now on: {e:#}");
            *differ = None;
            Transfer::WholeFrame
        }
    }
}

/// What [`flip_and_advance`] did with a frame.
#[derive(Debug)]
pub enum Flip {
    /// The frame matched the panel's, so nothing went out.
    Skipped,
    /// The frame went out as its transfer asked.
    Sent,
    /// The flip with damage clips failed and the same frame went through whole,
    /// which points at the clips.
    ClipsRefused(anyhow::Error),
}

/// Send the frame as `transfer` asks through `flip`,
/// then `advance` the scanout pools once a flip went through.
///
/// An unchanged frame neither flips nor advances,
/// so the next render still compares against the panel's frame.
/// A failed flip leaves the reference on the frame the panel still holds.
/// A flip with clips that fails is retried once with the whole frame.
pub fn flip_and_advance(
    transfer: &Transfer,
    mut flip: impl FnMut(Option<&[Rectangle<i32, Physical>]>) -> Result<()>,
    advance: impl FnOnce(),
) -> Result<Flip> {
    let clips = match transfer {
        Transfer::Unchanged => return Ok(Flip::Skipped),
        Transfer::Regions(rects) => Some(rects.as_slice()),
        Transfer::WholeFrame => None,
    };
    let flipped = match flip(clips) {
        Ok(()) => Flip::Sent,
        Err(clipped) if clips.is_some() => {
            flip(None).with_context(|| {
                format!("Page flip failed without damage clips too; with them: {clipped:#}")
            })?;
            Flip::ClipsRefused(clipped)
        }
        Err(e) => return Err(e),
    };
    advance();
    Ok(flipped)
}

/// Which cells of a frame changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellMask {
    width: usize,
    height: usize,
    columns: usize,
    rows: usize,
    changed: Vec<bool>,
}

impl CellMask {
    #[must_use]
    pub fn new(width: usize, height: usize) -> Self {
        let columns = width.div_ceil(CELL);
        let rows = height.div_ceil(CELL);
        Self {
            width,
            height,
            columns,
            rows,
            changed: vec![false; columns * rows],
        }
    }

    pub fn mark(&mut self, column: usize, row: usize) {
        self.changed[row * self.columns + column] = true;
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.changed.contains(&true)
    }

    /// Changed cells as frame-pixel rectangles, clipped to the frame.
    ///
    /// Each row's runs of changed cells extend the rectangle above them
    /// when they span the same columns,
    /// so a block of changed cells comes out as one rectangle.
    #[must_use]
    pub fn rects(&self) -> Vec<Rectangle<i32, Physical>> {
        let mut closed = Vec::new();
        let mut open: Vec<OpenRect> = Vec::new();
        for row in 0..=self.rows {
            let runs = if row < self.rows {
                self.runs(row)
            } else {
                Vec::new()
            };
            open.retain(|rect| {
                let continues = runs.contains(&rect.run);
                if !continues {
                    closed.push(self.pixel_rect(rect.run, rect.top_row, row));
                }
                continues
            });
            for run in runs {
                if !open.iter().any(|rect| rect.run == run) {
                    open.push(OpenRect { run, top_row: row });
                }
            }
        }
        closed
    }

    fn runs(&self, row: usize) -> Vec<Run> {
        let cells = &self.changed[row * self.columns..(row + 1) * self.columns];
        let mut runs = Vec::new();
        let mut column = 0;
        while column < self.columns {
            if cells[column] {
                let first_column = column;
                while column < self.columns && cells[column] {
                    column += 1;
                }
                runs.push(Run {
                    first_column,
                    end_column: column,
                });
            } else {
                column += 1;
            }
        }
        runs
    }

    #[expect(
        clippy::cast_possible_wrap,
        clippy::cast_possible_truncation,
        reason = "panel dimensions fit in i32"
    )]
    fn pixel_rect(&self, run: Run, top_row: usize, end_row: usize) -> Rectangle<i32, Physical> {
        let x = run.first_column * CELL;
        let y = top_row * CELL;
        let right = (run.end_column * CELL).min(self.width);
        let bottom = (end_row * CELL).min(self.height);
        Rectangle::from_loc_and_size(
            (x as i32, y as i32),
            ((right - x) as i32, (bottom - y) as i32),
        )
    }
}

/// Changed cells next to each other in one row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Run {
    first_column: usize,
    /// One past the last column.
    end_column: usize,
}

/// Changed cells that [`CellMask::rects`] is still extending downwards.
struct OpenRect {
    run: Run,
    top_row: usize,
}

/// Shader pass writing one pixel per cell, 1.0 where the cell changed,
/// read back to the CPU.
pub struct FrameDiffer {
    program: GlesTexProgram,
    cells: GlesTexture,
    width: usize,
    height: usize,
    frame_size: (f32, f32),
    grid_size: Size<i32, Physical>,
    readback: Vec<u8>,
}

impl FrameDiffer {
    #[expect(
        clippy::cast_precision_loss,
        reason = "panel dimensions are exact in f32"
    )]
    pub fn new(renderer: &mut GlesRenderer, width: u32, height: u32) -> Result<Self> {
        let (width, height) = (width as usize, height as usize);
        assert!(
            DIFF_SHADER.contains(CELL_DEFINE_MARKER),
            "BUG: frame_diff.frag lost the line its CELL define replaces"
        );
        let shader = DIFF_SHADER.replace(CELL_DEFINE_MARKER, &format!("#define CELL {CELL}"));
        let program = renderer
            .compile_custom_texture_shader(
                &shader,
                &[
                    UniformName::new("previous", UniformType::_1i),
                    UniformName::new("frame_size", UniformType::_2f),
                ],
            )
            .context("Failed to compile frame diff shader")?;
        let grid = CellMask::new(width, height);
        let grid_size = Size::from((to_gl(grid.columns), to_gl(grid.rows)));
        let cells: GlesTexture = renderer
            .create_buffer(
                Fourcc::Abgr8888,
                Size::<i32, BufferCoord>::from((grid_size.w, grid_size.h)),
            )
            .context("Failed to create frame diff target")?;
        Ok(Self {
            program,
            cells,
            width,
            height,
            frame_size: (width as f32, height as f32),
            grid_size,
            readback: vec![0; grid.columns * grid.rows * READBACK_TEXEL_BYTES],
        })
    }

    /// Cells where `new` differs from `previous`. Both must be finished
    /// frames of this differ's size.
    pub fn diff(
        &mut self,
        renderer: &mut GlesRenderer,
        new: &Dmabuf,
        previous: &Dmabuf,
    ) -> Result<CellMask> {
        // Through the `previous` sampler an external texture reads as black,
        // which hides every region that turned black.
        let render_formats = renderer.egl_context().dmabuf_render_formats();
        if [new, previous]
            .iter()
            .any(|buffer| !render_formats.contains(&buffer.format()))
        {
            bail!("Scanout buffers import as external textures, which the diff cannot sample");
        }
        let new_texture = renderer
            .import_dmabuf(new, None)
            .context("Failed to import the new frame")?;
        let previous_texture = renderer
            .import_dmabuf(previous, None)
            .context("Failed to import the previous frame")?;

        let mut target = renderer
            .bind(&mut self.cells)
            .context("Failed to bind frame diff target")?;
        let mut frame = renderer
            .render(&mut target, self.grid_size, Transform::Normal)
            .context("Failed to begin frame diff")?;
        // Marks every cell changed, so a pass that draws nothing sends a whole frame.
        // A raw clear needs no shader, unlike smithay's.
        // SAFETY: plain state calls on the current context, which has `cells` bound.
        frame.with_context(|gl| unsafe {
            gl.ClearColor(1.0, 1.0, 1.0, 1.0);
            gl.Clear(ffi::COLOR_BUFFER_BIT);
        })?;
        let previous_id = previous_texture.tex_id();
        frame.with_context(|gl| bind_previous(gl, previous_id))?;
        let dst = Rectangle::from_size(self.grid_size);
        let src = Rectangle::from_size(Size::<f64, BufferCoord>::from((
            f64::from(self.frame_size.0),
            f64::from(self.frame_size.1),
        )));
        let drawn = frame.render_texture_from_to(
            &new_texture,
            src,
            dst,
            &[dst],
            // Opaque, so smithay writes the flags over the cleared cells without blending.
            &[dst],
            Transform::Normal,
            1.0,
            Some(&self.program),
            &[
                Uniform::new("previous", to_gl(PREVIOUS_TEXTURE_UNIT)),
                Uniform::new("frame_size", self.frame_size),
            ],
        );
        frame.with_context(|gl| bind_previous(gl, 0))?;
        drawn.context("Failed to draw frame diff")?;
        let _sync = frame.finish().context("Failed to finish frame diff")?;

        let (columns, rows) = (self.grid_size.w, self.grid_size.h);
        let readback = &mut self.readback;
        let read = renderer.with_context(|gl| read_cells(gl, columns, rows, readback))?;
        drop(target);
        read?;

        let mut mask = CellMask::new(self.width, self.height);
        let row_bytes = mask.columns * READBACK_TEXEL_BYTES;
        for (row, cells) in self.readback.chunks_exact(row_bytes).enumerate() {
            for (column, cell) in cells
                .as_chunks::<READBACK_TEXEL_BYTES>()
                .0
                .iter()
                .enumerate()
            {
                if cell[0] >= 128 {
                    mask.mark(column, row);
                }
            }
        }
        Ok(mask)
    }
}

/// Read the `columns × rows` cell flags into `readback`.
fn read_cells(gl: &ffi::Gles2, columns: i32, rows: i32, readback: &mut [u8]) -> Result<()> {
    let texels = usize::try_from(columns * rows).expect("BUG: the cell grid size is positive");
    assert!(
        readback.len() >= texels * READBACK_TEXEL_BYTES,
        "BUG: the readback buffer holds every cell"
    );
    ensure!(
        clear_gl_errors(gl),
        "GL keeps reporting errors before the frame diff readback"
    );
    // SAFETY: plain state calls on the current context.
    let unbind = unsafe {
        gl.BindBuffer(ffi::PIXEL_PACK_BUFFER, 0);
        gl.GetError()
    };
    // Without GLES 3 or NV_pixel_buffer_object the target is invalid, and nothing can be bound to it.
    ensure!(
        matches!(unbind, ffi::NO_ERROR | ffi::INVALID_ENUM),
        "Unbinding the pixel pack buffer failed: GL error 0x{unbind:04x}"
    );
    // SAFETY: `readback` holds `columns × rows` RGBA texels and no pack buffer is bound,
    // so ReadPixels writes only inside it.
    let error = unsafe {
        gl.ReadPixels(
            0,
            0,
            columns,
            rows,
            ffi::RGBA,
            ffi::UNSIGNED_BYTE,
            readback.as_mut_ptr().cast(),
        );
        gl.GetError()
    };
    ensure!(
        error == ffi::NO_ERROR,
        "Frame diff readback failed: GL error 0x{error:04x}"
    );
    Ok(())
}

/// Clear the current context's error flags, or `false` when they keep coming.
fn clear_gl_errors(gl: &ffi::Gles2) -> bool {
    // SAFETY: GetError only reads and resets the current context's error flags.
    (0..MAX_STALE_GL_ERRORS).any(|_| unsafe { gl.GetError() } == ffi::NO_ERROR)
}

/// Bind `texture` (0 unbinds) as the shader's `previous` sampler,
/// leaving unit 0 active as smithay expects.
fn bind_previous(gl: &ffi::Gles2, texture: ffi::types::GLuint) {
    let nearest = i32::try_from(ffi::NEAREST).expect("BUG: GL_NEAREST fits in GLint");
    // SAFETY: plain state calls on the current context.
    unsafe {
        gl.ActiveTexture(
            ffi::TEXTURE0
                + ffi::types::GLenum::try_from(PREVIOUS_TEXTURE_UNIT)
                    .expect("BUG: texture unit index fits in GLenum"),
        );
        gl.BindTexture(ffi::TEXTURE_2D, texture);
        if texture != 0 {
            gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MIN_FILTER, nearest);
            gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MAG_FILTER, nearest);
        }
        gl.ActiveTexture(ffi::TEXTURE0);
    }
}

fn to_gl(value: usize) -> i32 {
    i32::try_from(value).expect("BUG: panel-sized values fit in GLint")
}

#[cfg(test)]
mod tests {
    use super::{CELL, CellMask, Flip, Transfer, flip_and_advance, next_transfer};
    use anyhow::Result;
    use smithay::utils::{Physical, Rectangle};

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
        Rectangle::from_loc_and_size((x, y), (w, h))
    }

    #[test]
    fn an_unknown_panel_frame_takes_the_whole_frame() {
        assert_eq!(Transfer::from_diff(None), Transfer::WholeFrame);
    }

    #[test]
    fn an_unchanged_frame_skips_the_transfer() {
        let mask = CellMask::new(480, 320);
        assert_eq!(Transfer::from_diff(Some(&mask)), Transfer::Unchanged);
    }

    #[test]
    fn a_changed_frame_sends_only_its_changed_cells() {
        let mut mask = CellMask::new(480, 320);
        mask.mark(25, 14);
        assert_eq!(
            Transfer::from_diff(Some(&mask)),
            Transfer::Regions(vec![rect(100, 56, 4, 4)])
        );
    }

    #[test]
    fn regions_go_out_whole_where_the_plane_takes_no_clips() {
        let regions = Transfer::Regions(vec![rect(100, 56, 4, 4)]);
        assert_eq!(regions.without_clips(), Transfer::WholeFrame);
    }

    #[test]
    fn an_unchanged_frame_skips_its_flip_without_clips_too() {
        assert_eq!(Transfer::Unchanged.without_clips(), Transfer::Unchanged);
    }

    type Clips = Option<Vec<Rectangle<i32, Physical>>>;

    /// Flip `transfer` through a page flip that fails on the calls listed in `failing`.
    /// Returns the outcome, the clips each call carried and whether the pools advanced.
    fn flip_with(transfer: &Transfer, failing: &[usize]) -> (Result<Flip>, Vec<Clips>, bool) {
        let mut calls = Vec::new();
        let mut advanced = false;
        let outcome = flip_and_advance(
            transfer,
            |clips| {
                let call = calls.len();
                calls.push(clips.map(<[_]>::to_vec));
                if failing.contains(&call) {
                    anyhow::bail!("call {call} refused");
                }
                Ok(())
            },
            || advanced = true,
        );
        (outcome, calls, advanced)
    }

    #[test]
    fn an_unchanged_frame_neither_flips_nor_advances() {
        let (outcome, calls, advanced) = flip_with(&Transfer::Unchanged, &[]);
        assert!(matches!(outcome, Ok(Flip::Skipped)), "{outcome:?}");
        assert!(calls.is_empty(), "nothing goes to the panel");
        assert!(
            !advanced,
            "the back buffers stay, so the next frame compares against the panel's"
        );
    }

    #[test]
    fn changed_regions_flip_with_their_clips_then_advance() {
        let regions = vec![rect(100, 56, 4, 4)];
        let (outcome, calls, advanced) = flip_with(&Transfer::Regions(regions.clone()), &[]);
        assert!(matches!(outcome, Ok(Flip::Sent)), "{outcome:?}");
        assert_eq!(calls, vec![Some(regions)]);
        assert!(advanced);
    }

    #[test]
    fn a_whole_frame_flips_without_clips_then_advances() {
        let (outcome, calls, advanced) = flip_with(&Transfer::WholeFrame, &[]);
        assert!(matches!(outcome, Ok(Flip::Sent)), "{outcome:?}");
        assert_eq!(calls, vec![None]);
        assert!(advanced);
    }

    #[test]
    fn a_failed_flip_keeps_the_panel_frame_as_the_reference() {
        let (outcome, _, advanced) = flip_with(&Transfer::WholeFrame, &[0]);
        assert!(outcome.is_err(), "{outcome:?}");
        assert!(!advanced, "the panel still holds the front buffer's frame");
    }

    #[test]
    fn refused_clips_resend_the_whole_frame() {
        let regions = vec![rect(100, 56, 4, 4)];
        let (outcome, calls, advanced) = flip_with(&Transfer::Regions(regions.clone()), &[0]);
        assert!(matches!(outcome, Ok(Flip::ClipsRefused(_))), "{outcome:?}");
        assert_eq!(calls, vec![Some(regions), None]);
        assert!(advanced, "the whole frame reached the panel");
    }

    #[test]
    fn a_flip_failing_with_and_without_clips_keeps_the_reference() {
        let (outcome, calls, advanced) =
            flip_with(&Transfer::Regions(vec![rect(0, 0, 4, 4)]), &[0, 1]);
        assert!(outcome.is_err(), "{outcome:?}");
        assert_eq!(calls.len(), 2, "the whole frame is tried once");
        assert!(!advanced);
    }

    #[test]
    fn a_failed_diff_sends_whole_frames_from_then_on() {
        let mut differ = Some(());

        let transfer = next_transfer(&mut differ, |()| anyhow::bail!("GPU lost"));
        assert_eq!(transfer, Transfer::WholeFrame);
        assert!(differ.is_none(), "a failed diff turns the differ off");

        let transfer = next_transfer(&mut differ, |()| -> Result<Option<CellMask>> {
            panic!("a differ that is off must not diff again")
        });
        assert_eq!(transfer, Transfer::WholeFrame);
    }

    #[test]
    fn a_block_of_cells_becomes_one_rectangle() {
        let mut mask = CellMask::new(480, 320);
        for row in 2..5 {
            for column in 10..13 {
                mask.mark(column, row);
            }
        }
        assert_eq!(mask.rects(), vec![rect(40, 8, 12, 12)]);
    }

    #[test]
    fn disjoint_changes_stay_separate_rectangles() {
        let mut mask = CellMask::new(480, 320);
        mask.mark(0, 0);
        mask.mark(119, 79);
        assert_eq!(
            mask.rects(),
            vec![rect(0, 0, 4, 4), rect(476, 316, 4, 4)],
            "merging far-apart changes is the driver's call, not the mask's"
        );
    }

    #[test]
    fn differently_wide_rows_split_into_separate_rectangles() {
        let mut mask = CellMask::new(480, 320);
        mask.mark(1, 0);
        mask.mark(1, 1);
        mask.mark(2, 1);
        let mut rects = mask.rects();
        rects.sort_by_key(|r| (r.loc.y, r.loc.x));
        assert_eq!(rects, vec![rect(4, 0, 4, 4), rect(4, 4, 8, 4)]);
    }

    #[test]
    fn edge_cells_are_clipped_to_the_frame() {
        let width = 2 * CELL + 1;
        let height = CELL + 2;
        let mut mask = CellMask::new(width, height);
        mask.mark(2, 1);
        assert_eq!(mask.rects(), vec![rect(8, 4, 1, 2)]);
    }
}
