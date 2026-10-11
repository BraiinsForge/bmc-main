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

use std::convert::Infallible;

use bmc_ui::MeshId;

use super::{
    MESH_ID_EXCLUSIVE_CAP, MeshRenderer, PendingMeshReservations, UploadedMesh, ZERO_MESH_ARGS,
    atlas, reserve_mesh,
};
use crate::renderer::{AssetSuspendResult, AssetTagState};
use crate::test_harness::{GlHarness, create_real_buffer, create_real_texture};
use glow::HasContext;

fn minimal_mesh() -> Vec<u8> {
    let body_offset = bmc_ui::mesh::HEADER_SIZE + atlas::AABB_SIZE;
    let mut data = vec![0_u8; body_offset];
    data[0..4].copy_from_slice(&bmc_ui::mesh::MESH_MAGIC.to_le_bytes());
    let offset = u32::try_from(body_offset)
        .expect("BUG: minimal mesh offset fits u32")
        .to_le_bytes();
    data[12..16].copy_from_slice(&offset);
    data[16..20].copy_from_slice(&offset);
    data
}

#[test]
fn pending_released_ids_survive_renderer_initialization() {
    let mut reservations = PendingMeshReservations::new(MESH_ID_EXCLUSIVE_CAP);
    let first =
        reserve_mesh(&mut reservations, "first").expect("BUG: first reservation must succeed");
    let second =
        reserve_mesh(&mut reservations, "second").expect("BUG: second reservation must succeed");
    assert_eq!(reservations.evict_prefix("first").len(), 1);

    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let gl = &harness.gl;
    let mut canvas = harness.build_canvas().expect("BUG: canvas init failed");
    let mut renderer = MeshRenderer::new_with_reservations(gl, &mut canvas, 0, &mut reservations)
        .expect("BUG: mesh renderer init must succeed");

    assert_eq!(
        renderer.register_mesh(gl, "replacement", &minimal_mesh()),
        Some(first)
    );
    assert_ne!(first, second);
    renderer.destroy(gl, &mut canvas);
}

/// Build an `UploadedMesh` whose GL handles are real, freshly allocated
/// objects (not wrapped in any draw state). Used to verify `drop_gl`
/// releases them all.
fn build_test_mesh(gl: &glow::Context, with_texture: bool, with_normal_map: bool) -> UploadedMesh {
    UploadedMesh {
        vbo: create_real_buffer(gl, glow::ARRAY_BUFFER),
        ibo: create_real_buffer(gl, glow::ELEMENT_ARRAY_BUFFER),
        index_count: 0,
        texture: with_texture.then(|| create_real_texture(gl)),
        normal_map: with_normal_map.then(|| create_real_texture(gl)),
        resident_bytes: 42,
        has_uvs: false,
        has_tangents: false,
        is_msdf: false,
        body_color: [0.0; 4],
        label_color: [0.0; 4],
    }
}

#[test]
fn drop_gl_releases_all_handles() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let gl = &harness.gl;

    let mesh = build_test_mesh(gl, true, true);
    let vbo = mesh.vbo;
    let ibo = mesh.ibo;
    let texture = mesh.texture.expect("BUG: texture should be Some");
    let normal_map = mesh.normal_map.expect("BUG: normal_map should be Some");

    unsafe {
        assert!(gl.is_buffer(vbo), "BUG: vbo should be live before drop");
        assert!(gl.is_buffer(ibo), "BUG: ibo should be live before drop");
        assert!(
            gl.is_texture(texture),
            "BUG: texture should be live before drop"
        );
        assert!(
            gl.is_texture(normal_map),
            "BUG: normal_map should be live before drop",
        );
    }

    unsafe { mesh.drop_gl(gl) };

    unsafe {
        assert!(!gl.is_buffer(vbo), "BUG: vbo leaked past drop_gl");
        assert!(!gl.is_buffer(ibo), "BUG: ibo leaked past drop_gl");
        assert!(!gl.is_texture(texture), "BUG: texture leaked past drop_gl");
        assert!(
            !gl.is_texture(normal_map),
            "BUG: normal_map leaked past drop_gl",
        );
    }
}

#[test]
fn resident_bytes_sum_only_live_mesh_payloads() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let gl = &harness.gl;
    let mut canvas = harness.build_canvas().expect("BUG: canvas init failed");
    let mut renderer = MeshRenderer::new(gl, &mut canvas, 0).expect("BUG: mesh renderer init");
    for tag in ["first", "last"] {
        renderer
            .reservations
            .register_with(tag, (), |()| {
                Ok::<_, Infallible>(build_test_mesh(gl, false, false))
            })
            .expect("BUG: an infallible load registers");
    }
    renderer.reserve("reserved");

    assert_eq!(renderer.resident_bytes(), 84);
    renderer.destroy(gl, &mut canvas);
}

#[test]
fn failed_initial_registration_does_not_leave_a_reservation() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let gl = &harness.gl;
    let mut canvas = harness.build_canvas().expect("BUG: canvas init failed");
    let mut renderer = MeshRenderer::new(gl, &mut canvas, 0).expect("BUG: mesh renderer init");

    assert_eq!(renderer.register_mesh(gl, "broken", &[]), None);
    assert_eq!(renderer.tag_state("broken"), AssetTagState::Unknown);
    assert_eq!(renderer.reserve("next").map(MeshId::to_wire), Some(1));

    renderer.destroy(gl, &mut canvas);
}

#[test]
fn failed_registration_keeps_recycled_non_tail_index_reusable() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let gl = &harness.gl;
    let mut canvas = harness.build_canvas().expect("BUG: canvas init failed");
    let mut renderer = MeshRenderer::new(gl, &mut canvas, 0).expect("BUG: mesh renderer init");
    let mesh = minimal_mesh();
    let first = renderer
        .register_mesh(gl, "first", &mesh)
        .expect("BUG: first registration must succeed");
    let recycled = renderer
        .register_mesh(gl, "middle", &mesh)
        .expect("BUG: middle registration must succeed");
    let last = renderer
        .register_mesh(gl, "last", &mesh)
        .expect("BUG: last registration must succeed");

    assert_eq!(renderer.evict_prefix(gl, "middle"), 1);
    assert_eq!(renderer.register_mesh(gl, "broken", &[]), None);
    assert_eq!(
        renderer.register_mesh(gl, "replacement", &mesh),
        Some(recycled)
    );
    assert_ne!(first, recycled);
    assert_ne!(last, recycled);

    renderer.destroy(gl, &mut canvas);
}

#[test]
fn evicting_mesh_invalidates_atlas_slot_before_id_reuse() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let gl = &harness.gl;
    let mut canvas = harness.build_canvas().expect("BUG: canvas init failed");
    let mut renderer = MeshRenderer::new(gl, &mut canvas, 0).expect("BUG: mesh renderer init");
    let mesh = minimal_mesh();
    let id = renderer
        .register_mesh(gl, "old", &mesh)
        .expect("BUG: initial registration must succeed");
    let args = ZERO_MESH_ARGS;
    assert!(renderer.slots[0].check_and_update(id, &args));
    assert!(!renderer.slots[0].check_and_update(id, &args));

    assert_eq!(renderer.evict_prefix(gl, "old"), 1);
    let replacement = renderer
        .register_mesh(gl, "new", &mesh)
        .expect("BUG: replacement registration must succeed");
    assert_eq!(replacement, id);
    assert!(renderer.slots[0].check_and_update(replacement, &args));

    renderer.destroy(gl, &mut canvas);
}

#[test]
fn drop_gl_handles_optional_textures() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let gl = &harness.gl;

    let mesh = build_test_mesh(gl, false, false);
    let vbo = mesh.vbo;
    let ibo = mesh.ibo;

    unsafe {
        assert!(gl.is_buffer(vbo), "BUG: vbo should be live before drop");
        assert!(gl.is_buffer(ibo), "BUG: ibo should be live before drop");
    }

    unsafe { mesh.drop_gl(gl) };

    unsafe {
        assert!(!gl.is_buffer(vbo), "BUG: vbo leaked past drop_gl");
        assert!(!gl.is_buffer(ibo), "BUG: ibo leaked past drop_gl");
    }
}

#[test]
fn suspending_meshes_retains_slots_and_ids_until_destructive_eviction() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let gl = &harness.gl;
    let mut canvas = harness.build_canvas().expect("BUG: canvas init failed");
    let mut renderer = MeshRenderer::new(gl, &mut canvas, 0).expect("BUG: mesh renderer init");
    let mesh = minimal_mesh();

    let id_1 = renderer
        .register_mesh(gl, "widget-1:mesh", &mesh)
        .expect("BUG: first mesh registration should succeed");
    let id_10 = renderer
        .register_mesh(gl, "widget-10:mesh", &mesh)
        .expect("BUG: second mesh registration should succeed");
    assert_eq!(
        renderer.tag_state("widget-1:mesh"),
        AssetTagState::Resident(id_1)
    );
    assert_eq!(
        renderer.tag_state("widget-10:mesh"),
        AssetTagState::Resident(id_10)
    );
    let registered = renderer
        .reservations
        .payload(id_1)
        .expect("BUG: a registered mesh is resident");
    let (vbo, ibo) = (registered.vbo, registered.ibo);

    assert_eq!(
        renderer.suspend_exact(gl, "widget-1:mesh"),
        AssetSuspendResult::Suspended(id_1)
    );
    assert_eq!(
        renderer.tag_state("widget-1:mesh"),
        AssetTagState::Suspended(id_1)
    );
    assert_eq!(
        renderer.tag_state("widget-10:mesh"),
        AssetTagState::Resident(id_10)
    );
    assert_eq!(
        renderer.suspend_exact(gl, "widget-1:mesh"),
        AssetSuspendResult::AlreadySuspended(id_1)
    );
    unsafe {
        assert!(
            !gl.is_buffer(vbo),
            "BUG: suspended mesh VBO must be released"
        );
        assert!(
            !gl.is_buffer(ibo),
            "BUG: suspended mesh IBO must be released"
        );
    }

    assert_eq!(
        renderer.register_mesh(gl, "widget-1:mesh", &mesh),
        Some(id_1)
    );

    assert_eq!(
        renderer.suspend_exact(gl, "widget-1:mesh"),
        AssetSuspendResult::Suspended(id_1)
    );
    assert_eq!(
        renderer.register_mesh(gl, "widget-1:mesh", &mesh),
        Some(id_1)
    );

    assert_eq!(
        renderer.suspend_exact(gl, "widget-1:mesh"),
        AssetSuspendResult::Suspended(id_1)
    );
    assert_eq!(renderer.register_mesh(gl, "widget-1:mesh", &[]), None);
    assert_eq!(
        renderer.tag_state("widget-1:mesh"),
        AssetTagState::Suspended(id_1)
    );
    assert_eq!(renderer.evict_prefix(gl, "widget-1:mesh"), 1);
    assert_eq!(renderer.tag_state("widget-1:mesh"), AssetTagState::Unknown);
    assert_eq!(
        renderer.suspend_exact(gl, "widget-1:mesh"),
        AssetSuspendResult::Unknown
    );

    let replacement = renderer
        .register_mesh(gl, "widget-1:mesh", &mesh)
        .expect("BUG: re-registration after eviction should succeed");
    assert_eq!(replacement, id_1);

    renderer.destroy(gl, &mut canvas);
}

#[test]
fn exact_reservation_and_suspension_preserve_mesh_id() {
    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let gl = &harness.gl;
    let mut canvas = harness.build_canvas().expect("BUG: canvas init failed");
    let mut renderer = MeshRenderer::new(gl, &mut canvas, 0).expect("BUG: mesh renderer init");
    let id = renderer
        .reserve("widget:mesh")
        .expect("BUG: mesh reservation should succeed");

    assert_eq!(
        renderer.suspend_exact(gl, "widget:mesh"),
        AssetSuspendResult::AlreadySuspended(id)
    );
    assert_eq!(
        renderer.register_mesh(gl, "widget:mesh", &minimal_mesh()),
        Some(id)
    );
    assert_eq!(
        renderer.suspend_exact(gl, "widget:mesh"),
        AssetSuspendResult::Suspended(id)
    );
    assert_eq!(
        renderer.suspend_exact(gl, "missing"),
        AssetSuspendResult::Unknown
    );
}
