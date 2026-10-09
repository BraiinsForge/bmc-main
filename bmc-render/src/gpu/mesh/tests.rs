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

use super::{
    MeshRenderer, MeshReservations, UploadedMesh, ZERO_MESH_ARGS, atlas, release_mesh_index,
};
use crate::renderer::{AssetSuspendResult, AssetTagState};
use crate::test_harness::{GlHarness, create_real_buffer, create_real_texture};
use glow::HasContext;

fn minimal_mesh() -> Vec<u8> {
    let body_offset = bmc_wasm_protocol::mesh::HEADER_SIZE + atlas::AABB_SIZE;
    let mut data = vec![0_u8; body_offset];
    data[0..4].copy_from_slice(&bmc_wasm_protocol::mesh::MESH_MAGIC.to_le_bytes());
    let offset = u32::try_from(body_offset)
        .expect("BUG: minimal mesh offset fits u32")
        .to_le_bytes();
    data[12..16].copy_from_slice(&offset);
    data[16..20].copy_from_slice(&offset);
    data
}

#[test]
fn pending_reservations_reuse_evicted_slots() {
    let mut reservations = MeshReservations::default();
    let first = reservations
        .reserve("widget-a:mesh")
        .expect("BUG: first reservation must fit");
    let second = reservations
        .reserve("widget-b:mesh")
        .expect("BUG: second reservation must fit");

    assert_eq!(reservations.evict_prefix("widget-a"), 1);
    assert_eq!(
        reservations.reserve("widget-c:mesh"),
        Some(first),
        "an evicted slot must be reused before allocating past the live second slot"
    );
    assert_ne!(first, second);
}

#[test]
#[should_panic(expected = "BUG: released mesh index must not already be free")]
fn releasing_a_mesh_index_twice_fails_loudly() {
    let mut free_indices = Vec::new();

    release_mesh_index(&mut free_indices, 0, 1);
    release_mesh_index(&mut free_indices, 0, 1);
}

#[test]
#[should_panic(expected = "BUG: released mesh index must have been issued")]
fn releasing_an_unissued_mesh_index_fails_loudly() {
    release_mesh_index(&mut Vec::new(), 1, 1);
}

#[test]
fn pending_free_indices_survive_renderer_initialization() {
    let mut reservations = MeshReservations::default();
    let first = reservations
        .reserve("first")
        .expect("BUG: first reservation must succeed");
    let second = reservations
        .reserve("second")
        .expect("BUG: second reservation must succeed");
    assert_eq!(reservations.evict_prefix("first"), 1);

    let harness = GlHarness::new().expect("BUG: headless GL setup failed");
    let gl = &harness.gl;
    let mut canvas = harness.build_canvas().expect("BUG: canvas init failed");
    let mut renderer = MeshRenderer::new_with_reservations(gl, &mut canvas, 0, reservations)
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
    renderer
        .meshes
        .push(Some(build_test_mesh(gl, false, false)));
    renderer.meshes.push(None);
    renderer
        .meshes
        .push(Some(build_test_mesh(gl, false, false)));

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
    assert!(renderer.meshes.is_empty());

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

    assert!(renderer.evict(gl, "middle"));
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

    assert!(renderer.evict(gl, "old"));
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
    let registered_slots = renderer.meshes.len();
    let vbo = renderer.meshes[0]
        .as_ref()
        .expect("BUG: registered mesh must occupy its slot")
        .vbo;
    let ibo = renderer.meshes[0]
        .as_ref()
        .expect("BUG: registered mesh must occupy its slot")
        .ibo;

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
    assert_eq!(renderer.meshes.len(), registered_slots);

    assert_eq!(
        renderer.suspend_exact(gl, "widget-1:mesh"),
        AssetSuspendResult::Suspended(id_1)
    );
    assert_eq!(
        renderer.register_mesh(gl, "widget-1:mesh", &mesh),
        Some(id_1)
    );
    assert_eq!(renderer.meshes.len(), registered_slots);

    assert_eq!(
        renderer.suspend_exact(gl, "widget-1:mesh"),
        AssetSuspendResult::Suspended(id_1)
    );
    assert_eq!(renderer.register_mesh(gl, "widget-1:mesh", &[]), None);
    assert_eq!(
        renderer.tag_state("widget-1:mesh"),
        AssetTagState::Suspended(id_1)
    );
    assert_eq!(renderer.meshes.len(), registered_slots);
    assert!(renderer.evict(gl, "widget-1:mesh"));
    assert_eq!(renderer.tag_state("widget-1:mesh"), AssetTagState::Unknown);
    assert_eq!(
        renderer.suspend_exact(gl, "widget-1:mesh"),
        AssetSuspendResult::Unknown
    );

    let replacement = renderer
        .register_mesh(gl, "widget-1:mesh", &mesh)
        .expect("BUG: re-registration after eviction should succeed");
    assert_eq!(replacement, id_1);
    assert_eq!(renderer.meshes.len(), registered_slots);

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
