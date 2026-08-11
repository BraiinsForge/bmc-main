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

//! 3D mesh demo — Suzanne viewer + Google Dice-style tray.

use std::cell::{Cell, RefCell};

#[expect(clippy::wildcard_imports)]
use bmc_wasm_sdk::math::*;
#[expect(clippy::wildcard_imports)]
use bmc_wasm_sdk::*;

static SUZANNE: Mesh = include_mesh!("assets/suzanne.glb");
static D4: Mesh = include_mesh!("assets/D4.glb");
static D6: Mesh = include_mesh!("assets/D6.glb");
static D8: Mesh = include_mesh!("assets/D8.glb");
static D10: Mesh = include_mesh!("assets/D10.glb");
static D12: Mesh = include_mesh!("assets/D12.glb");
static D20: Mesh = include_mesh!("assets/D20.glb");

#[derive(Clone, Copy, PartialEq, Eq)]
enum DieType {
    D4,
    D6,
    D8,
    D10,
    D12,
    D20,
}

impl DieType {
    fn mesh(self) -> &'static Mesh {
        match self {
            Self::D4 => &D4,
            Self::D6 => &D6,
            Self::D8 => &D8,
            Self::D10 => &D10,
            Self::D12 => &D12,
            Self::D20 => &D20,
        }
    }

    #[expect(clippy::cast_possible_truncation)]
    fn faces(self) -> u32 {
        self.mesh().face_normals.len() as u32
    }

    /// Map internal face index (1-based) to display value.
    /// All dice show `face` directly except D10 which shows `face - 1` (0-9).
    fn face_value(self, face: u8) -> u32 {
        match self {
            Self::D10 => u32::from(face) - 1,
            _ => u32::from(face),
        }
    }

    /// Camera distance — larger shapes or those that look flat need more distance.
    fn camera_distance(self) -> f32 {
        match self {
            Self::D4 => 3.2,
            Self::D6 => 3.5,
            Self::D8 => 2.8,
            Self::D10 => 2.6,
            Self::D12 => 2.5,
            Self::D20 => 2.4,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::D4 => "D4",
            Self::D6 => "D6",
            Self::D8 => "D8",
            Self::D10 => "D10",
            Self::D12 => "D12",
            Self::D20 => "D20",
        }
    }

    fn button_key(self) -> &'static str {
        match self {
            Self::D4 => "add_d4",
            Self::D6 => "add_d6",
            Self::D8 => "add_d8",
            Self::D10 => "add_d10",
            Self::D12 => "add_d12",
            Self::D20 => "add_d20",
        }
    }
}

const ALL_DIE_TYPES: [DieType; 6] = [
    DieType::D4,
    DieType::D6,
    DieType::D8,
    DieType::D10,
    DieType::D12,
    DieType::D20,
];

#[derive(Clone)]
struct DieInstance {
    die_type: DieType,
    face: u8,
    roll_from: [f32; 4],
    roll_to: [f32; 4],
    roll_elapsed_ms: u32,
}

fn new_die(die_type: DieType) -> DieInstance {
    let face = random_face(die_type.faces(), 0);
    DieInstance {
        die_type,
        face,
        roll_from: [0.0, 0.0, 0.0, 1.0],
        roll_to: target_orientation(die_type, face).into(),
        roll_elapsed_ms: ROLL_DURATION_MS,
    }
}

/// Maximum dice in the tray (limited by atlas slots — Suzanne tab uses none).
const MAX_DICE: usize = 9;

/// Roll animation duration in ms.
const ROLL_DURATION_MS: u32 = 800;

/// Double-tap window for removing a die (ms).
const DOUBLE_TAP_MS: u32 = 400;

/// How long to show the "tap again" hint (ms).
const REMOVE_HINT_MS: u32 = 2_400;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Suzanne,
    Tray,
}

thread_local! {
    static MODE: Cell<Mode> = const { Cell::new(Mode::Tray) };
    // Suzanne drag state
    static YAW: Cell<f32> = const { Cell::new(30.0) };
    static PITCH: Cell<f32> = const { Cell::new(15.0) };
    static PREV_DRAG_X: Cell<f32> = const { Cell::new(f32::NAN) };
    static PREV_DRAG_Y: Cell<f32> = const { Cell::new(f32::NAN) };
    // Dice tray state — initialized lazily in first render
    static DICE: RefCell<Vec<DieInstance>> = const { RefCell::new(Vec::new()) };
    static INITIALIZED: Cell<bool> = const { Cell::new(false) };
    // Double-tap removal: index of die pending removal and elapsed ms since first tap
    static PENDING_REMOVE_IDX: Cell<Option<usize>> = const { Cell::new(None) };
    static PENDING_REMOVE_MS: Cell<u32> = const { Cell::new(0) };
}

/// Pick a random face (1..=max), avoiding `prev` to prevent repeats.
///
/// Returns `prev` when `max == 0` — meshes that ship without `face_normals`
/// have no faces to roll. Loud-logs at `warn` so a missing-extras bug in a
/// new mesh asset is obvious instead of silently degrading to a still die.
#[expect(
    clippy::cast_possible_truncation,
    reason = "(r % max) is bounded by max (≤ 20 for any shipped die mesh), which fits in u8"
)]
fn random_face(max: u32, prev: u8) -> u8 {
    if max == 0 {
        log_warn!("random_face: max == 0 (mesh missing face_normals?); freezing on prev face");
        return prev;
    }
    let r = random_u32();
    let face = (r % max) as u8 + 1;
    if face == prev && max > 1 {
        face % max as u8 + 1
    } else {
        face
    }
}

/// Compute orientation that rotates a face normal to point toward the camera (+Z)
/// with text reading upright on screen.
///
/// Face normals are read from `mesh.face_normals` (embedded from GLB extras).
/// Builds a tangent frame R = [tangent_x, tangent_y, normal], returns R^{-1}
/// which maps: normal → +Z (toward camera), tangent_y → +Y (screen up).
fn target_orientation(die_type: DieType, face: u8) -> Quat {
    let normals = die_type.mesh().face_normals;
    if normals.is_empty() {
        return Quat::IDENTITY;
    }
    let n = normals[(face - 1) as usize];
    let normal = Vec3::from_array(n);

    // Tangent frame: project world-up onto face plane for consistent text orientation.
    // Must match D20.py / D6.py tangent frame so text appears upright on screen.
    let world_up = if normal.dot(Vec3::Y).abs() > 0.95 {
        Vec3::X // fallback when face is near Y axis
    } else {
        Vec3::Y // glTF Y-up = Blender Z-up after export
    };
    let tangent_y = (world_up - normal * normal.dot(world_up)).normalize();
    let tangent_x = tangent_y.cross(normal).normalize();
    let tangent_y = normal.cross(tangent_x).normalize(); // re-orthogonalize

    // R maps +X→tangent_x, +Y→tangent_y, +Z→normal.
    // We want the inverse: normal→+Z, tangent_y→+Y.
    let face_rot = Quat::from_mat3(&Mat3::from_cols(tangent_x, tangent_y, normal))
        .conjugate()
        .normalize();

    match die_type {
        // D6: tilt slightly so the cube looks 3D when face-on
        DieType::D6 => {
            let tilt = Quat::from_euler(EulerRot::YXZ, -0.23, 0.16, 0.0);
            (tilt * face_rot).normalize()
        }
        // D4: significant tilt — face-on tetrahedron looks flat
        DieType::D4 => {
            let tilt = Quat::from_euler(EulerRot::YXZ, -0.35, 0.25, 0.0);
            (tilt * face_rot).normalize()
        }
        // D10: slight tilt for depth
        DieType::D10 => {
            let tilt = Quat::from_euler(EulerRot::YXZ, -0.20, 0.15, 0.0);
            (tilt * face_rot).normalize()
        }
        // D12: slight tilt for depth
        DieType::D12 => {
            let tilt = Quat::from_euler(EulerRot::YXZ, -0.18, 0.12, 0.0);
            (tilt * face_rot).normalize()
        }
        // D8, D20: no tilt needed
        DieType::D8 | DieType::D20 => face_rot,
    }
}

/// Compute a UV-rect highlight for the rolled face in an atlas grid.
fn highlight_for_face(die_type: DieType, face: u8) -> Option<Highlight> {
    // D6 pips are obvious — no highlight needed
    let (cols, rows) = match die_type {
        DieType::D4 => (2.0, 2.0),
        DieType::D8 => (3.0, 3.0),
        DieType::D10 => (5.0, 3.0),
        DieType::D12 => (4.0, 3.0),
        DieType::D20 => (5.0, 5.0), // 5×5 atlas (rows 0-3 used, row 4 dead zone)
        DieType::D6 => return None,
    };
    let cell = f32::from(face - 1);
    let col = cell % cols;
    let row = (cell / cols).floor();
    // No V-flip: scripts already flip V for glTF convention, and the OpenGL texture
    // upload (no flip) + glTF UVs (V=0 at top) cancel out, so the shader's v_uv
    // matches the glTF-authored values directly.
    let u_min = col / cols;
    let u_max = (col + 1.0) / cols;
    let v_min = row / rows;
    let v_max = (row + 1.0) / rows;
    Some(Highlight {
        uv_rect: [u_min, v_min, u_max, v_max],
        // Gold tint
        color: [1.0, 0.85, 0.2],
    })
}

/// Ease-out cubic: decelerating curve for the roll animation.
fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

/// Compute orientation for a die instance at the current moment.
#[expect(clippy::cast_precision_loss)]
fn die_orientation(die: &DieInstance) -> Orientation {
    if die.die_type.faces() == 0 {
        return Orientation::from_euler(15.0, 30.0, 0.0);
    }
    if die.roll_elapsed_ms < ROLL_DURATION_MS {
        let t = (die.roll_elapsed_ms as f32 / ROLL_DURATION_MS as f32).min(1.0);
        let eased = ease_out_cubic(t);
        let from = Quat::from_array(die.roll_from);
        let to = Quat::from_array(die.roll_to);
        from.slerp(to, eased).into()
    } else {
        target_orientation(die.die_type, die.face).into()
    }
}

/// Start a roll animation on a die instance.
fn roll_die(die: &mut DieInstance) {
    let prev_face = die.face;
    let new_face = random_face(die.die_type.faces(), prev_face);
    die.roll_from = target_orientation(die.die_type, prev_face).into();
    die.roll_to = target_orientation(die.die_type, new_face).into();
    die.face = new_face;
    die.roll_elapsed_ms = 0;
}

/// Re-render in response to touch — the host no longer renders on touch by
/// itself, so an interactive widget must ask for the frame here.
#[unsafe(no_mangle)]
pub extern "C" fn on_touch() {
    request_frame();
}

#[unsafe(no_mangle)]
pub extern "C" fn render(delta_ms: u32) {
    let WidgetSize {
        width: w,
        height: h,
        ..
    } = widget_size();
    let mode = MODE.get();

    // Seed initial tray with one D6
    if !INITIALIZED.get() {
        INITIALIZED.set(true);
        DICE.with_borrow_mut(|dice| {
            dice.push(new_die(DieType::D6));
        });
    }

    match mode {
        Mode::Suzanne => render_suzanne(w, h, delta_ms),
        Mode::Tray => render_tray(w, h, delta_ms),
    }

    request_frame_after(16);
}

#[expect(clippy::cast_precision_loss)]
fn render_suzanne(w: u32, h: u32, _delta_ms: u32) {
    let size = WidgetSize::from_dimensions(w, h);

    // Drag to rotate
    if let Some(hit) = host::get_touch_drag("viewport") {
        let cur_x = hit.frac_x();
        let cur_y = hit.y / hit.height;
        let prev_x = PREV_DRAG_X.get();
        let prev_y = PREV_DRAG_Y.get();

        if !prev_x.is_nan() {
            YAW.set(YAW.get() + (cur_x - prev_x) * 360.0);
            PITCH.set(PITCH.get() + (cur_y - prev_y) * 180.0);
        }

        PREV_DRAG_X.set(cur_x);
        PREV_DRAG_Y.set(cur_y);
        request_frame();
    } else {
        PREV_DRAG_X.set(f32::NAN);
        PREV_DRAG_Y.set(f32::NAN);
    }

    let viewport_h = size.height as f32;
    let orientation = Orientation::from_euler(PITCH.get(), YAW.get(), 0.0);

    let draws = [Draw::centered(
        Draw::mesh(
            0.0,
            0.0,
            viewport_h,
            viewport_h,
            &SUZANNE,
            MeshView {
                fov: 35.0,
                distance: 3.5,
                orientation,
                scale: 1.0,
                light: Some(LightAngles {
                    pitch: 45.0,
                    yaw: -30.0,
                }),
                ..Default::default()
            },
        )
        .transition("mesh", 400, Easing::EaseOut),
    )];

    let layout: Vec<Node> = vec![
        // Full-screen viewport
        touchable("viewport", props!(flex: 1.0), draws),
        // Floating mode tabs — top-left
        mode_tabs_overlay(Mode::Suzanne),
    ];

    let result = render_ui(w, h, col(props!(background: GRAY_100), layout));
    handle_mode_tabs(&result);
}

#[expect(clippy::too_many_lines)]
fn render_tray(w: u32, h: u32, delta_ms: u32) {
    // Advance roll animations
    let mut any_rolling = false;
    DICE.with_borrow_mut(|dice| {
        for die in dice.iter_mut() {
            if die.roll_elapsed_ms < ROLL_DURATION_MS {
                die.roll_elapsed_ms = die.roll_elapsed_ms.saturating_add(delta_ms);
                any_rolling = true;
            }
        }
    });
    if any_rolling {
        request_frame();
    }

    // Advance pending-removal timer
    let pending_idx = PENDING_REMOVE_IDX.get();
    if pending_idx.is_some() {
        let ms = PENDING_REMOVE_MS.get().saturating_add(delta_ms);
        PENDING_REMOVE_MS.set(ms);
        if ms >= REMOVE_HINT_MS {
            PENDING_REMOVE_IDX.set(None);
        }
        request_frame();
    }

    // Handle double-tap-to-remove on individual dice
    let dice_snapshot: Vec<DieInstance> = DICE.with_borrow(Clone::clone);
    let mut removed = false;
    for (i, _die) in dice_snapshot.iter().enumerate() {
        let key = fmt!("die_{}", i);
        if host::get_touch_click(&key).is_some() {
            if pending_idx == Some(i) && PENDING_REMOVE_MS.get() < DOUBLE_TAP_MS {
                // Second tap within window — remove
                DICE.with_borrow_mut(|dice| {
                    if i < dice.len() {
                        dice.remove(i);
                    }
                });
                PENDING_REMOVE_IDX.set(None);
                removed = true;
            } else {
                // First tap — start pending
                PENDING_REMOVE_IDX.set(Some(i));
                PENDING_REMOVE_MS.set(0);
            }
            request_frame();
            break;
        }
    }

    // Re-snapshot after potential removal
    let dice_snapshot: Vec<DieInstance> = if removed {
        DICE.with_borrow(Clone::clone)
    } else {
        dice_snapshot
    };

    let total: u32 = dice_snapshot
        .iter()
        .map(|d| d.die_type.face_value(d.face))
        .sum();
    let n = dice_snapshot.len();

    // Die cell size by count — chosen so flex-wrap produces balanced rows.
    let cols = match n {
        0..=1 => 1,
        2 | 4 => 2,
        7 | 8 => 4,
        _ => 3,
    };
    #[expect(
        clippy::cast_precision_loss,
        reason = "pixel dimensions are converted to float layout math"
    )]
    let area_w = w as f32;
    #[expect(
        clippy::cast_precision_loss,
        reason = "pixel dimensions are converted to float layout math"
    )]
    let area_h = (h as f32).max(60.0);
    #[expect(
        clippy::cast_precision_loss,
        reason = "column counts are converted to float layout math"
    )]
    let cols_f = cols as f32;
    let cell_w = area_w / cols_f;
    let rows = n.max(1).div_ceil(cols);
    #[expect(
        clippy::cast_precision_loss,
        reason = "row counts are converted to float layout math"
    )]
    let rows_f = rows as f32;
    let die_size = cell_w.min(area_h / rows_f).max(60.0);

    // Build die nodes
    let mut die_nodes: Vec<Node> = Vec::with_capacity(n);
    for (i, die) in dice_snapshot.iter().enumerate() {
        let rolling = die.roll_elapsed_ms < ROLL_DURATION_MS;
        let orientation = die_orientation(die);
        let highlight = if rolling {
            None
        } else {
            highlight_for_face(die.die_type, die.face)
        };

        let draws = [Draw::centered(Draw::mesh(
            0.0,
            0.0,
            die_size,
            die_size,
            die.die_type.mesh(),
            MeshView {
                fov: 35.0,
                distance: die.die_type.camera_distance(),
                orientation,
                scale: 1.0,
                light: Some(LightAngles {
                    pitch: 45.0,
                    yaw: -30.0,
                }),
                highlight,
                ..Default::default()
            },
        ))];

        let key = fmt!("die_{}", i);
        die_nodes.push(touchable(
            &key,
            props!(width: cell_w, height: die_size),
            draws,
        ));
    }

    let can_add = n < MAX_DICE;
    let any_rolling_now = dice_snapshot
        .iter()
        .any(|d| d.roll_elapsed_ms < ROLL_DURATION_MS);

    // Full-screen dice grid (or empty placeholder)
    let dice_area: Node = if die_nodes.is_empty() {
        center(
            props!(flex: 1.0),
            [text("Tap + to add dice", style!(size: 16, color: GRAY_60))],
        )
    } else {
        row(props!(flex: 1.0, wrap: true), die_nodes)
    };

    // Build add-die buttons (bottom row, no Roll — that's top-right)
    let mut add_buttons: Vec<Node> = Vec::with_capacity(ALL_DIE_TYPES.len());
    for dt in ALL_DIE_TYPES {
        add_buttons.push(make_button(
            dt.button_key().into(),
            dt.label().into(),
            ButtonStyle::Secondary,
            ButtonSize::Small,
            None,
            !can_add,
            None,
        ));
    }

    // "Tap again to remove" hint — shown briefly after first tap on a die
    let show_remove_hint =
        PENDING_REMOVE_IDX.get().is_some() && PENDING_REMOVE_MS.get() < REMOVE_HINT_MS;

    let mut layout: Vec<Node> = vec![
        dice_area,
        // Floating overlays — all absolute
        mode_tabs_overlay(Mode::Tray),
        // Roll + Sum badge — top-right
        row(
            props!(
                gap: 4.0,
                padding: 4.0,
                cross_align: CrossAlign::Center,
                inset_top: 4.0,
                inset_right: 4.0
            ),
            [
                make_button(
                    "roll_all".into(),
                    "Roll".into(),
                    ButtonStyle::Primary,
                    ButtonSize::Small,
                    None,
                    any_rolling_now || n == 0,
                    None,
                ),
                row(
                    props!(
                        height: 32.0,
                        padding: 8.0,
                        cross_align: CrossAlign::Center,
                        background: BLACK.with_alpha(0.6)
                    ),
                    [text(
                        fmt!("\u{03A3} {}", total),
                        style!(size: 14, color: WHITE, weight: FontWeight::BOLD),
                    )],
                ),
            ],
        ),
        // Add-die buttons — bottom-left, tight spacing
        row(
            props!(
                gap: 2.0,
                padding: 4.0,
                cross_align: CrossAlign::Center,
                inset_bottom: 4.0,
                inset_left: 4.0
            ),
            add_buttons,
        ),
    ];

    // "Tap again" hint — absolutely positioned, centered on screen
    if show_remove_hint {
        layout.push(center(
            props!(
                inset_top: 0.0,
                inset_bottom: 0.0,
                inset_left: 0.0,
                inset_right: 0.0
            ),
            [col(
                props!(
                    padding: 12.0,
                    background: BLACK.with_alpha(0.75)
                ),
                [text(
                    "Tap again to remove",
                    style!(size: 20, color: WHITE, weight: FontWeight::BOLD),
                )],
            )],
        ));
    }

    let result = render_ui(w, h, col(props!(background: GRAY_100), layout));

    handle_mode_tabs(&result);

    // Handle add-die clicks
    for dt in ALL_DIE_TYPES {
        if result.clicks.contains_key(dt.button_key()) && can_add {
            DICE.with_borrow_mut(|dice| dice.push(new_die(dt)));
            request_frame();
        }
    }
    if result.clicks.contains_key("roll_all") && !any_rolling_now && n > 0 {
        DICE.with_borrow_mut(|dice| {
            for die in dice.iter_mut() {
                if die.die_type.faces() > 0 {
                    roll_die(die);
                }
            }
        });
        request_frame();
    }
}

/// Floating mode tabs — absolute top-left overlay with semi-transparent background.
fn mode_tabs_overlay(active: Mode) -> Node {
    let tab = |id: &str, label: &str, target: Mode| -> Node {
        let style = if active == target {
            ButtonStyle::Primary
        } else {
            ButtonStyle::Tertiary
        };
        make_button(
            id.into(),
            label.into(),
            style,
            ButtonSize::Small,
            None,
            false,
            None,
        )
    };

    row(
        props!(
            gap: 4.0,
            padding: 4.0,
            inset_top: 4.0,
            inset_left: 4.0
        ),
        [
            tab("suzanne", "Suzanne", Mode::Suzanne),
            tab("tray", "Tray", Mode::Tray),
        ],
    )
}

fn handle_mode_tabs(result: &TreeRenderResult) {
    if result.clicks.contains_key("suzanne") && MODE.get() != Mode::Suzanne {
        MODE.set(Mode::Suzanne);
        YAW.set(30.0);
        PITCH.set(15.0);
        request_frame();
    }
    if result.clicks.contains_key("tray") && MODE.get() != Mode::Tray {
        MODE.set(Mode::Tray);
        request_frame();
    }
}
