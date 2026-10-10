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

//! The tree wire format: one encoder and one decoder, byte for byte.
//!
//! Every node is `[type:u8][data...]`. Per-variant layout:
//!
//! ```text
//! Container:    [type][props][child_count:u16][children...]
//! Paragraph:    [type][props][text_style][span_count:u16][spans...]
//!   each span:  [flags:u16][extra_flags:u8][len:u16][text...]
//!               [color:u32 if has_color][size:u32 if has_size]
//! Button:       [type][id_len:u16][id...][style:u8][size:u8][icon_id:u16]
//!               [disabled:u8][stretch:u8][label_len:u16][label...]
//!               [has_skin:u8][normal nine-patch][has_pressed:u8][pressed nine-patch]
//!               [text_color:u32][pressed_text_color:u32][opaque:u8]
//!   nine-patch: [bitmap_id:u16][left:u16][top:u16][right:u16][bottom:u16]
//! Spacer:       [type][flex:f32]
//! Canvas:       [type][props][key_len:u16][key...][draw_count:u16][draws...]
//! Scroll:       [type][key_len:u16][key...][props][child_count:u16][children...]
//! Notification: [type][kind:u8][title_len:u16][title...][subtitle_len:u16][subtitle...]
//! RelTime:      [type][anchor:i64][format:u8][clamp:u8][text_style]
//! Tag:          [type][kind:u8][icon_mode:u8][icon:u16][content node]
//! Switcher:     [type][active:u8][disabled:u8][tab_count:u8]
//!   each tab:   [icon:u16][click_id_len:u16][click_id...]
//! Skeleton:     [type][kind:u8][chars:f32][font_size:f32][width:f32][height:f32][color:u32]
//! Modal:        [type][id_len:u16][id...][is_open:u8][padding:u16][backdrop_alpha:u8]
//!               [title_len:u16][title...][content_height:f32][child_count:u16]
//!               [bg_color:u32][header_color:u32][title_color:u32][max_width:u16]
//!               [children...][primary_key][primary_label][secondary_key][secondary_label]
//!               [danger:u8], each footer string length-prefixed by a u16
//! ProgressBar:  [type][key_len:u16][key...][track_h:f32][mode:u8][fraction:f32][active:u8]
//!               [fill_color:u32][track_color:u32][bg_color:u32][has_skin:u8]
//!               [track nine-patch][track_h:u16][thumb_id:u16][thumb_w:u16][thumb_h:u16]
//!               [thumb_pressed_id:u16]
//! Dimmed:       [type][brightness:f32][child]
//! ```
//!
//! Draws open with their `DRAW_*` byte; wrappers (`Centered`, `Orbit`, `Rotated`,
//! `Shadow`, `Modified`) are followed by the draw they wrap.
//!
//! The decoder validates what the renderer cannot survive — extents, discriminants, UTF-8 —
//! and records the byte ranges of every dynamic node,
//! for the static-half key the host builds over the raw bytes.
//! The encoder validates nothing: a widget's tree is its own to get right.

use core::fmt;
use std::ops::Range;

use crate::animation::{AnimProperty, ColorSpace, Easing, LoopMode};
use crate::arc::{
    ArcCap, ArcFill, ArcSegments, decode_arc_cap, decode_arc_fill, decode_arc_segments,
    encode_arc_cap, encode_arc_fill, encode_arc_segments,
};
use crate::colors::Color;
use crate::fill::{Dash, Fill, PathPaint, decode_fill, encode_fill};
use crate::ids::{BitmapId, MeshId, SvgId};
use crate::mesh::{MeshDrawArgs, MeshHighlight, MeshLighting, MeshTransform};
use crate::nodes::{
    ButtonSize, ButtonStyle, DRAW_ARC, DRAW_AUTOFIT_TEXT, DRAW_BITMAP, DRAW_CENTERED, DRAW_CIRCLE,
    DRAW_CURVED_TEXT, DRAW_ICON, DRAW_MESH, DRAW_MODIFIED, DRAW_NINE_PATCH, DRAW_ORBIT, DRAW_PATH,
    DRAW_QR, DRAW_RECT, DRAW_ROTATED, DRAW_SHADOW, DRAW_SPHERE, DRAW_TEXT, NODE_BUTTON,
    NODE_CANVAS, NODE_CENTER, NODE_COLUMN, NODE_DIMMED, NODE_MODAL, NODE_NOTIFICATION,
    NODE_PARAGRAPH, NODE_PROGRESS_BAR, NODE_RELTIME, NODE_ROW, NODE_SCROLL, NODE_SKELETON,
    NODE_SPACER, NODE_SWITCHER, NODE_TAG, TagIconMode, TagKind,
};
use crate::partition::{draw_is_dynamic, node_self_is_dynamic};
use crate::progress::ProgressKind;
use crate::relative_time::{RelTimeClamp, RelTimeFormat};
use crate::skeleton::SkeletonKind;
use crate::skin::{ButtonSkin, NinePatch, SliderSkin};
use crate::text::{ArcAnchor, ArcTextFacing, AutoFit, PropsData, SpanFlags, TextStyle};
use crate::tree::{
    AnimationDef, Draw, Interpolation, Node, NotificationKind, ProgressMode, QrStyle, SkeletonData,
    Span, SwitcherTab, TagIcon, TransitionDef,
};

/// Every node opens with its one-byte `NODE_*` type.
const NODE_TYPE_LEN: usize = 1;

// ── Encoding ────────────────────────────────────────────────────────

/// The wire bytes of a tree rooted at `root`.
#[must_use]
pub fn encode(root: &Node) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4096);
    encode_into(&mut buf, root);
    buf
}

/// Append the wire bytes of a tree rooted at `root` to `buf`.
pub fn encode_into(buf: &mut Vec<u8>, root: &Node) {
    Writer(buf).node(root);
}

struct Writer<'a>(&'a mut Vec<u8>);

impl Writer<'_> {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }

    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    fn i64(&mut self, v: i64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    fn color(&mut self, c: Color) {
        self.u32(c.to_u32());
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }

    /// A u16 length prefix, then the bytes; `too_long` is the panic for a string past u16.
    fn str(&mut self, s: &str, too_long: &'static str) {
        let bytes = s.as_bytes();
        self.u16(u16::try_from(bytes.len()).expect(too_long));
        self.bytes(bytes);
    }

    fn svg_id(&mut self, id: Option<SvgId>) {
        self.u16(id.map_or(0, SvgId::to_wire));
    }

    fn bitmap_id(&mut self, id: Option<BitmapId>) {
        self.u16(id.map_or(0, BitmapId::to_wire));
    }

    fn mesh_id(&mut self, id: Option<MeshId>) {
        self.u16(id.map_or(0, MeshId::to_wire));
    }

    fn props(&mut self, props: &PropsData) {
        self.bytes(&props.to_bytes());
    }

    fn text_style(&mut self, style: &TextStyle) {
        self.bytes(&style.to_bytes());
    }

    fn nine_patch(&mut self, np: &NinePatch) {
        self.bitmap_id(np.bitmap_id);
        self.u16(np.left);
        self.u16(np.top);
        self.u16(np.right);
        self.u16(np.bottom);
    }

    fn container(&mut self, tag: u8, props: &PropsData, children: &[Node]) {
        self.u8(tag);
        self.props(props);
        self.u16(u16::try_from(children.len()).expect("BUG: container children exceed u16::MAX"));
        for child in children {
            self.node(child);
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "tree serialization keeps the wire format in one linear encoder"
    )]
    fn node(&mut self, node: &Node) {
        match node {
            Node::Column(props, children) => self.container(NODE_COLUMN, props, children),
            Node::Row(props, children) => self.container(NODE_ROW, props, children),
            Node::Center(props, children) => self.container(NODE_CENTER, props, children),
            Node::Paragraph {
                props,
                base_style,
                spans,
            } => {
                self.u8(NODE_PARAGRAPH);
                self.props(props);
                self.text_style(base_style);
                self.u16(u16::try_from(spans.len()).expect("BUG: text spans exceed u16::MAX"));
                for span in spans {
                    let (flags, extra_flags) = span.wire_flags();
                    self.u16(flags);
                    self.u8(extra_flags);
                    self.str(&span.text, "BUG: span text exceeds u16::MAX bytes");
                    if let Some(color) = span.color {
                        self.color(color);
                    }
                    if let Some(size) = span.size {
                        self.u32(size);
                    }
                }
            }
            Node::RelTime {
                anchor,
                format,
                clamp,
                style,
            } => {
                self.u8(NODE_RELTIME);
                self.i64(*anchor);
                self.u8(u8::from(*format));
                self.u8(u8::from(*clamp));
                self.text_style(style);
            }
            Node::Tag {
                kind,
                icon,
                content,
            } => {
                self.u8(NODE_TAG);
                self.u8(*kind as u8);
                let (mode, icon_id) = match icon {
                    TagIcon::Default => (TagIconMode::Default, None),
                    TagIcon::Hidden => (TagIconMode::Hidden, None),
                    TagIcon::Custom(id) => (TagIconMode::Custom, Some(*id)),
                };
                self.u8(mode as u8);
                self.svg_id(icon_id);
                self.node(content);
            }
            Node::Button {
                id,
                label,
                style,
                size,
                icon_id,
                disabled,
                stretch,
                skin,
            } => {
                self.u8(NODE_BUTTON);
                self.str(id, "BUG: button id exceeds u16::MAX bytes");
                self.u8(*style as u8);
                self.u8(*size as u8);
                self.svg_id(*icon_id);
                self.u8(u8::from(*disabled));
                self.u8(u8::from(*stretch));
                self.str(label, "BUG: button label exceeds u16::MAX bytes");
                if let Some(skin) = skin {
                    self.u8(1);
                    self.nine_patch(&skin.normal);
                    if let Some(pressed) = &skin.pressed {
                        self.u8(1);
                        self.nine_patch(pressed);
                    } else {
                        self.u8(0);
                    }
                    self.color(skin.text_color);
                    self.color(skin.pressed_text_color);
                    self.u8(u8::from(skin.opaque));
                } else {
                    self.u8(0);
                }
            }
            Node::Spacer { flex } => {
                self.u8(NODE_SPACER);
                self.f32(*flex);
            }
            Node::Canvas {
                props,
                touch_key,
                draws,
            } => {
                self.u8(NODE_CANVAS);
                self.props(props);
                self.str(
                    touch_key.as_deref().unwrap_or(""),
                    "BUG: touch key exceeds u16::MAX bytes",
                );
                self.u16(u16::try_from(draws.len()).expect("BUG: canvas draws exceed u16::MAX"));
                for draw in draws {
                    self.draw(draw);
                }
            }
            Node::Scroll {
                scroll_key,
                props,
                children,
            } => {
                self.u8(NODE_SCROLL);
                self.str(scroll_key, "BUG: scroll key exceeds u16::MAX bytes");
                self.props(props);
                self.u16(
                    u16::try_from(children.len()).expect("BUG: scroll children exceed u16::MAX"),
                );
                for child in children {
                    self.node(child);
                }
            }
            Node::Notification {
                kind,
                title,
                subtitle,
            } => {
                self.u8(NODE_NOTIFICATION);
                self.u8(*kind as u8);
                self.str(title, "BUG: title exceeds u16::MAX bytes");
                self.str(subtitle, "BUG: subtitle exceeds u16::MAX bytes");
            }
            Node::Switcher {
                active,
                disabled,
                tabs,
            } => {
                self.u8(NODE_SWITCHER);
                self.u8(u8::try_from(*active).expect("BUG: active tab index exceeds u8::MAX"));
                self.u8(u8::from(*disabled));
                self.u8(u8::try_from(tabs.len()).expect("BUG: switcher tabs exceed u8::MAX"));
                for tab in tabs {
                    self.svg_id(tab.icon);
                    self.str(&tab.click_id, "BUG: tab click id exceeds u16::MAX bytes");
                }
            }
            Node::Skeleton(data) => {
                self.u8(NODE_SKELETON);
                self.u8(data.kind.into());
                self.f32(data.chars);
                self.f32(data.font_size);
                self.f32(data.width);
                self.f32(data.height);
                self.color(data.color);
            }
            Node::Modal {
                modal_id,
                is_open,
                title,
                content_height,
                padding,
                backdrop_alpha,
                bg_color,
                header_color,
                title_color,
                max_width,
                body,
                footer_primary_key,
                footer_primary_label,
                footer_secondary_key,
                footer_secondary_label,
                footer_danger,
            } => {
                self.u8(NODE_MODAL);
                self.str(modal_id, "BUG: modal id exceeds u16::MAX bytes");
                self.u8(u8::from(*is_open));
                self.u16(*padding);
                self.u8(*backdrop_alpha);
                self.str(title, "BUG: modal title exceeds u16::MAX bytes");
                self.f32(*content_height);
                self.u16(
                    u16::try_from(body.len()).expect("BUG: modal body children exceed u16::MAX"),
                );
                self.color(*bg_color);
                self.color(*header_color);
                self.color(*title_color);
                self.u16(*max_width);
                for child in body {
                    self.node(child);
                }
                self.str(
                    footer_primary_key,
                    "BUG: footer primary key exceeds u16::MAX bytes",
                );
                self.str(
                    footer_primary_label,
                    "BUG: footer primary label exceeds u16::MAX bytes",
                );
                self.str(
                    footer_secondary_key,
                    "BUG: footer secondary key exceeds u16::MAX bytes",
                );
                self.str(
                    footer_secondary_label,
                    "BUG: footer secondary label exceeds u16::MAX bytes",
                );
                self.u8(u8::from(*footer_danger));
            }
            Node::ProgressBar {
                touch_key,
                track_h,
                mode,
                active,
                fill_color,
                track_color,
                bg_color,
                skin,
            } => {
                self.u8(NODE_PROGRESS_BAR);
                self.str(
                    touch_key.as_deref().unwrap_or(""),
                    "BUG: progress touch key exceeds u16::MAX bytes",
                );
                self.f32(*track_h);
                self.u8(mode.kind().into());
                self.f32(mode.fraction());
                self.u8(u8::from(*active));
                self.color(*fill_color);
                self.color(*track_color);
                self.color(*bg_color);
                if let Some(skin) = skin {
                    self.u8(1);
                    self.nine_patch(&skin.track);
                    self.u16(skin.track_h);
                    self.bitmap_id(skin.thumb_id);
                    self.u16(skin.thumb_w);
                    self.u16(skin.thumb_h);
                    self.bitmap_id(skin.thumb_pressed_id);
                } else {
                    self.u8(0);
                }
            }
            Node::Dimmed { brightness, child } => {
                self.u8(NODE_DIMMED);
                self.f32(*brightness);
                self.node(child);
            }
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "draw serialization keeps the wire format in one linear encoder"
    )]
    fn draw(&mut self, draw: &Draw) {
        match draw {
            Draw::Rect { x, y, w, h, fill } => {
                self.u8(DRAW_RECT);
                self.f32(*x);
                self.f32(*y);
                self.f32(*w);
                self.f32(*h);
                encode_fill(self.0, fill);
            }
            Draw::Circle { cx, cy, r, fill } => {
                self.u8(DRAW_CIRCLE);
                self.f32(*cx);
                self.f32(*cy);
                self.f32(*r);
                encode_fill(self.0, fill);
            }
            Draw::Arc {
                cx,
                cy,
                radius,
                start_angle,
                end_angle,
                width,
                fill,
                segments,
                cap,
            } => {
                self.u8(DRAW_ARC);
                self.f32(*cx);
                self.f32(*cy);
                self.f32(*radius);
                self.f32(*start_angle);
                self.f32(*end_angle);
                self.f32(*width);
                encode_arc_fill(self.0, fill);
                encode_arc_segments(self.0, segments);
                encode_arc_cap(self.0, *cap);
            }
            Draw::Svg {
                x,
                y,
                w,
                h,
                color,
                icon_id,
                anti_alias,
                fills,
            } => {
                self.u8(DRAW_ICON);
                self.f32(*x);
                self.f32(*y);
                self.f32(*w);
                self.f32(*h);
                self.color(*color);
                self.svg_id(*icon_id);
                self.u8(u8::from(*anti_alias));
                self.u16(
                    u16::try_from(fills.len())
                        .expect("BUG: Draw::Svg has more than u16::MAX fill overrides"),
                );
                for (id, color) in fills {
                    self.str(id, "BUG: SVG path id exceeds u16::MAX bytes");
                    self.color(*color);
                }
            }
            Draw::Bitmap {
                x,
                y,
                w,
                h,
                bitmap_id,
            } => {
                self.u8(DRAW_BITMAP);
                self.f32(*x);
                self.f32(*y);
                self.f32(*w);
                self.f32(*h);
                self.bitmap_id(*bitmap_id);
            }
            Draw::Qr {
                x,
                y,
                size,
                style,
                text,
            } => {
                self.u8(DRAW_QR);
                self.f32(*x);
                self.f32(*y);
                self.f32(*size);
                self.color(style.dark);
                self.color(style.light);
                self.u8(style.quiet_zone);
                self.str(text, "BUG: QR text exceeds u16::MAX bytes");
            }
            Draw::Centered { inner } => {
                self.u8(DRAW_CENTERED);
                self.draw(inner);
            }
            Draw::Orbit {
                radius,
                angle,
                inner,
            } => {
                self.u8(DRAW_ORBIT);
                self.f32(*radius);
                self.f32(*angle);
                self.draw(inner);
            }
            Draw::Rotated { angle, inner } => {
                self.u8(DRAW_ROTATED);
                self.f32(*angle);
                self.draw(inner);
            }
            Draw::Shadow {
                dx,
                dy,
                blur,
                color,
                inner,
            } => {
                self.u8(DRAW_SHADOW);
                self.f32(*dx);
                self.f32(*dy);
                self.f32(*blur);
                self.color(*color);
                self.draw(inner);
            }
            Draw::Modified {
                animations,
                transition,
                color_space,
                inner,
            } => {
                self.u8(DRAW_MODIFIED);
                let mut flags: u8 = 0;
                if !animations.is_empty() {
                    flags |= 0x01;
                }
                if transition.is_some() {
                    flags |= 0x02;
                }
                flags |= (*color_space as u8) << 2;
                self.u8(flags);

                if !animations.is_empty() {
                    self.u8(u8::try_from(animations.len())
                        .expect("BUG: node animations exceed u8::MAX"));
                    for anim in animations {
                        self.u8(anim.property as u8);
                        self.f32(anim.from);
                        self.f32(anim.to);
                        self.u32(anim.duration_ms);
                        self.u16(anim.delay_ms);
                        self.u8(anim.easing as u8);
                        self.u8(anim.loop_mode as u8);
                    }
                }

                if let Some(t) = transition {
                    self.u32(t.id_hash);
                    self.u32(t.duration_ms);
                    self.u8(t.easing as u8);
                }

                self.draw(inner);
            }
            Draw::Path {
                points,
                paint,
                closed,
                interpolation,
            } => {
                let mut flags: u8 = 0;
                if *closed {
                    flags |= 0x01;
                }
                if *interpolation == Interpolation::CatmullRom {
                    flags |= 0x02;
                }
                if matches!(paint, PathPaint::Fill(_)) {
                    flags |= 0x04;
                }
                if matches!(paint, PathPaint::Stroke { dash: Some(_), .. }) {
                    flags |= 0x08;
                }
                self.u8(DRAW_PATH);
                self.u8(flags);
                self.u16(u16::try_from(points.len()).expect("BUG: path points exceed u16::MAX"));
                for &(x, y) in points {
                    self.f32(x);
                    self.f32(y);
                }
                match paint {
                    PathPaint::Fill(fill) => encode_fill(self.0, fill),
                    PathPaint::Stroke { color, width, dash } => {
                        self.color(*color);
                        self.f32(*width);
                        if let Some(d) = dash {
                            self.f32(d.on);
                            self.f32(d.off);
                        }
                    }
                }
            }
            Draw::Text { x, y, text, style } => {
                self.u8(DRAW_TEXT);
                self.f32(*x);
                self.f32(*y);
                self.text_style(style);
                self.str(text, "BUG: draw text exceeds u16::MAX bytes");
            }
            Draw::CurvedText {
                cx,
                cy,
                radius,
                angle,
                anchor,
                facing,
                text,
                style,
            } => {
                self.u8(DRAW_CURVED_TEXT);
                self.f32(*cx);
                self.f32(*cy);
                self.f32(*radius);
                self.f32(*angle);
                self.u8(u8::from(*anchor));
                self.u8(u8::from(*facing));
                self.text_style(style);
                self.str(text, "BUG: Draw::CurvedText text exceeds u16::MAX bytes");
            }
            Draw::AutofitText {
                x,
                y,
                box_width,
                box_height,
                mode,
                min_size,
                max_size,
                text,
                style,
            } => {
                self.u8(DRAW_AUTOFIT_TEXT);
                self.f32(*x);
                self.f32(*y);
                self.f32(*box_width);
                self.f32(*box_height);
                self.u8(*mode as u8);
                self.u16(*min_size);
                self.u16(*max_size);
                self.text_style(style);
                self.str(text, "BUG: Draw::AutofitText text exceeds u16::MAX bytes");
            }
            Draw::Sphere {
                x,
                y,
                w,
                h,
                bitmap_id,
                atmosphere,
                center_lat,
                center_lon,
                zoom,
                light_lat,
                light_lon,
            } => {
                self.u8(DRAW_SPHERE);
                self.f32(*x);
                self.f32(*y);
                self.f32(*w);
                self.f32(*h);
                self.bitmap_id(*bitmap_id);
                self.u8(u8::from(*atmosphere));
                self.f32(*center_lat);
                self.f32(*center_lon);
                self.f32(*zoom);
                self.f32(*light_lat);
                self.f32(*light_lon);
            }
            Draw::Mesh {
                x,
                y,
                w,
                h,
                mesh_id,
                args,
            } => {
                self.u8(DRAW_MESH);
                self.f32(*x);
                self.f32(*y);
                self.f32(*w);
                self.f32(*h);
                self.mesh_id(*mesh_id);
                self.f32(args.transform.fov);
                self.f32(args.transform.distance);
                for component in args.transform.quat {
                    self.f32(component);
                }
                for component in args.transform.position {
                    self.f32(component);
                }
                self.f32(args.transform.scale);
                self.f32(args.lighting.pitch);
                self.f32(args.lighting.yaw);
                self.f32(args.lighting.ambient);
                self.f32(args.lighting.specular);
                self.f32(args.highlight.u_min);
                self.f32(args.highlight.v_min);
                self.f32(args.highlight.u_max);
                self.f32(args.highlight.v_max);
                self.f32(args.highlight.r);
                self.f32(args.highlight.g);
                self.f32(args.highlight.b);
            }
            Draw::NinePatch {
                x,
                y,
                w,
                h,
                nine_patch,
            } => {
                self.u8(DRAW_NINE_PATCH);
                self.f32(*x);
                self.f32(*y);
                self.f32(*w);
                self.f32(*h);
                self.nine_patch(nine_patch);
            }
        }
    }
}

// ── Decoding ────────────────────────────────────────────────────────

/// A decoded tree, with what the host needs to key its static half.
#[derive(Debug)]
pub struct Decoded {
    pub root: Node,
    /// Byte ranges of the dynamic half, in wire order, sorted and disjoint:
    /// every [`node_self_is_dynamic`] node past its type byte, and in each canvas
    /// its draw count plus every draw from the first [`draw_is_dynamic`] one on.
    pub dynamic_ranges: Vec<Range<usize>>,
    /// How many bytes the tree occupied; anything after is trailing data.
    pub consumed: usize,
}

/// Why a byte sequence is not a tree. `at` is the offset the reader rejected.
#[derive(Debug, Clone, PartialEq)]
pub enum DecodeError {
    Empty,
    Truncated {
        at: usize,
    },
    InvalidExtent {
        value: f32,
        at: usize,
    },
    InvalidBrightness {
        value: f32,
        at: usize,
    },
    InvalidUtf8 {
        at: usize,
    },
    InvalidFill {
        at: usize,
    },
    InvalidArcFill {
        at: usize,
    },
    InvalidArcSegments {
        at: usize,
    },
    InvalidArcCap {
        at: usize,
    },
    InvalidProps {
        at: usize,
    },
    InvalidTextStyle {
        at: usize,
    },
    InvalidDiscriminant {
        what: &'static str,
        value: u8,
        at: usize,
    },
    UnknownNode {
        tag: u8,
        at: usize,
    },
    UnknownDraw {
        tag: u8,
        at: usize,
    },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("empty tree data"),
            Self::Truncated { at } => write!(f, "unexpected end of tree data at {at}"),
            Self::InvalidExtent { value, at } => {
                write!(f, "invalid extent {value} in tree at {at}")
            }
            Self::InvalidBrightness { value, at } => {
                write!(f, "invalid brightness {value} in tree at {at}")
            }
            Self::InvalidUtf8 { at } => write!(f, "invalid UTF-8 in tree at {at}"),
            Self::InvalidFill { at } => write!(f, "invalid or truncated fill in tree at {at}"),
            Self::InvalidArcFill { at } => {
                write!(f, "invalid or truncated arc fill in tree at {at}")
            }
            Self::InvalidArcSegments { at } => {
                write!(f, "invalid or truncated arc segments in tree at {at}")
            }
            Self::InvalidArcCap { at } => {
                write!(f, "invalid or truncated arc cap in tree at {at}")
            }
            Self::InvalidProps { at } => write!(f, "invalid or truncated props in tree at {at}"),
            Self::InvalidTextStyle { at } => {
                write!(f, "invalid or truncated text style in tree at {at}")
            }
            Self::InvalidDiscriminant { what, value, at } => {
                write!(f, "invalid {what} {value} in tree at {at}")
            }
            Self::UnknownNode { tag, at } => write!(f, "unknown node type {tag} at {at}"),
            Self::UnknownDraw { tag, at } => write!(f, "unknown draw command {tag} at {at}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Decode the tree at the start of `data`.
///
/// Trailing bytes are not an error; [`Decoded::consumed`] tells where the tree ended.
pub fn decode(data: &[u8]) -> Result<Decoded, DecodeError> {
    if data.is_empty() {
        return Err(DecodeError::Empty);
    }
    let mut reader = Reader::new(data);
    let root = reader.node()?;
    Ok(Decoded {
        root,
        dynamic_ranges: reader.dynamic_ranges,
        consumed: reader.pos,
    })
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    dynamic_ranges: Vec<Range<usize>>,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            dynamic_ranges: Vec::new(),
        }
    }

    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|end| *end <= self.data.len())
            .ok_or(DecodeError::Truncated { at: self.pos })?;
        let v = &self.data[self.pos..end];
        self.pos = end;
        Ok(v)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let (array, _) = self
            .data
            .get(self.pos..)
            .and_then(<[u8]>::split_first_chunk::<N>)
            .ok_or(DecodeError::Truncated { at: self.pos })?;
        self.pos += N;
        Ok(*array)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        let [byte] = self.array()?;
        Ok(byte)
    }

    fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn i64(&mut self) -> Result<i64, DecodeError> {
        Ok(i64::from_le_bytes(self.array()?))
    }

    /// Deliberately does not reject NaN: the protocol spends it as a sentinel
    /// (`PropsData::inset_*` for "auto", `Sphere`'s `light_lat`/`light_lon` for "unlit"),
    /// so refusing it here rejects valid trees.
    /// A NaN that reaches a rotation range is caught in the host's damage tracking,
    /// which fails open to the whole canvas rather than inverting its sweep to nothing.
    fn f32(&mut self) -> Result<f32, DecodeError> {
        Ok(f32::from_bits(self.u32()?))
    }

    /// A finite, non-negative `f32`, for a width, height or radius.
    ///
    /// femtovg renders a negative extent as mirrored geometry
    /// while damage tracking computes `right < x` and reports nothing,
    /// so the old pixels are never restored under target preservation.
    fn extent(&mut self) -> Result<f32, DecodeError> {
        let value = self.f32()?;
        // Finite and non-negative both, since an extent has no sentinel reading
        // the way the coordinates above do:
        // NaN and either infinity are as meaningless here as a negative width.
        if value.is_finite() && value >= 0.0 {
            Ok(value)
        } else {
            Err(DecodeError::InvalidExtent {
                value,
                at: self.pos,
            })
        }
    }

    fn color(&mut self) -> Result<Color, DecodeError> {
        Ok(Color::from_raw(self.u32()?))
    }

    fn fill(&mut self) -> Result<Fill, DecodeError> {
        let at = self.pos;
        decode_fill(self.data, &mut self.pos).ok_or(DecodeError::InvalidFill { at })
    }

    fn arc_fill(&mut self) -> Result<ArcFill, DecodeError> {
        let at = self.pos;
        decode_arc_fill(self.data, &mut self.pos).ok_or(DecodeError::InvalidArcFill { at })
    }

    fn arc_segments(&mut self) -> Result<ArcSegments, DecodeError> {
        let at = self.pos;
        decode_arc_segments(self.data, &mut self.pos).ok_or(DecodeError::InvalidArcSegments { at })
    }

    fn arc_cap(&mut self) -> Result<ArcCap, DecodeError> {
        let at = self.pos;
        decode_arc_cap(self.data, &mut self.pos).ok_or(DecodeError::InvalidArcCap { at })
    }

    /// Decode an `Option<SvgId>`. Wire zero lifts to `None`.
    fn svg_id(&mut self) -> Result<Option<SvgId>, DecodeError> {
        Ok(SvgId::from_wire(self.u16()?))
    }

    /// Decode an `Option<BitmapId>`. Wire zero lifts to `None`.
    fn bitmap_id(&mut self) -> Result<Option<BitmapId>, DecodeError> {
        Ok(BitmapId::from_wire(self.u16()?))
    }

    /// Decode an `Option<MeshId>`. Wire zero lifts to `None`.
    fn mesh_id(&mut self) -> Result<Option<MeshId>, DecodeError> {
        Ok(MeshId::from_wire(self.u16()?))
    }

    fn nine_patch(&mut self) -> Result<NinePatch, DecodeError> {
        Ok(NinePatch {
            bitmap_id: self.bitmap_id()?,
            left: self.u16()?,
            top: self.u16()?,
            right: self.u16()?,
            bottom: self.u16()?,
        })
    }

    fn props(&mut self) -> Result<PropsData, DecodeError> {
        let at = self.pos;
        let props =
            PropsData::from_bytes(&self.data[at..]).ok_or(DecodeError::InvalidProps { at })?;
        self.pos += PropsData::SIZE;
        Ok(props)
    }

    fn text_style(&mut self) -> Result<TextStyle, DecodeError> {
        let at = self.pos;
        let style =
            TextStyle::from_bytes(&self.data[at..]).ok_or(DecodeError::InvalidTextStyle { at })?;
        self.pos += TextStyle::SIZE;
        Ok(style)
    }

    fn string(&mut self, len: u16) -> Result<String, DecodeError> {
        let at = self.pos;
        let bytes = self.bytes(usize::from(len))?;
        String::from_utf8(bytes.to_vec()).map_err(|_| DecodeError::InvalidUtf8 { at })
    }

    /// A u16 length prefix, then that many bytes of UTF-8.
    fn prefixed_string(&mut self) -> Result<String, DecodeError> {
        let len = self.u16()?;
        self.string(len)
    }

    /// A one-byte discriminant, `what` naming it in the error.
    fn discriminant<T: TryFrom<u8>>(&mut self, what: &'static str) -> Result<T, DecodeError> {
        let at = self.pos;
        let value = self.u8()?;
        T::try_from(value).map_err(|_| DecodeError::InvalidDiscriminant { what, value, at })
    }

    fn span(&mut self) -> Result<Span, DecodeError> {
        let flags = SpanFlags::unpack(self.u16()?, self.u8()?);
        let text = self.prefixed_string()?;

        let color = flags.has_color.then(|| self.color()).transpose()?;
        let size = flags.has_size.then(|| self.u32()).transpose()?;

        Ok(Span {
            text,
            weight: flags.weight,
            color,
            size,
            italic: flags.italic,
            underline: flags.underline,
            strikethrough: flags.strikethrough,
        })
    }

    fn node(&mut self) -> Result<Node, DecodeError> {
        let start = self.pos;
        let ranges_before = self.dynamic_ranges.len();
        let node = self.node_body()?;
        if node_self_is_dynamic(&node) {
            // A modal's body is dynamic along with it,
            // so its one range replaces any recorded inside.
            self.dynamic_ranges.truncate(ranges_before);
            // The type byte stays keyed: a progress bar takes height where a modal takes none,
            // so swapping one for the other moves the static siblings.
            self.dynamic_ranges.push(start + NODE_TYPE_LEN..self.pos);
        }
        Ok(node)
    }

    fn children(&mut self) -> Result<Vec<Node>, DecodeError> {
        let count = self.u16()?;
        let mut children = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            children.push(self.node()?);
        }
        Ok(children)
    }

    #[expect(clippy::too_many_lines)]
    fn node_body(&mut self) -> Result<Node, DecodeError> {
        let at = self.pos;
        let node_type = self.u8()?;

        match node_type {
            NODE_COLUMN => {
                let props = self.props()?;
                Ok(Node::Column(props, self.children()?))
            }
            NODE_ROW => {
                let props = self.props()?;
                Ok(Node::Row(props, self.children()?))
            }
            NODE_CENTER => {
                let props = self.props()?;
                Ok(Node::Center(props, self.children()?))
            }
            NODE_PARAGRAPH => {
                let props = self.props()?;
                let base_style = self.text_style()?;
                let span_count = self.u16()?;
                let mut spans = Vec::with_capacity(usize::from(span_count));
                for _ in 0..span_count {
                    spans.push(self.span()?);
                }
                Ok(Node::Paragraph {
                    props,
                    base_style,
                    spans,
                })
            }
            NODE_BUTTON => {
                let id = self.prefixed_string()?;
                let style = ButtonStyle::from(u32::from(self.u8()?));
                let size = ButtonSize::from(self.u8()?);
                let icon_id = self.svg_id()?;
                let disabled = self.u8()? != 0;
                let stretch = self.u8()? != 0;
                let label = self.prefixed_string()?;
                // Trailing optional skin payload
                let skin = if self.remaining() > 0 && self.u8()? != 0 {
                    let normal = self.nine_patch()?;
                    let pressed = if self.u8()? != 0 {
                        Some(self.nine_patch()?)
                    } else {
                        None
                    };
                    let text_color = self.color()?;
                    let pressed_text_color = self.color()?;
                    let opaque = self.u8()? != 0;
                    Some(ButtonSkin {
                        normal,
                        pressed,
                        text_color,
                        pressed_text_color,
                        opaque,
                    })
                } else {
                    None
                };
                Ok(Node::Button {
                    id,
                    label,
                    style,
                    size,
                    icon_id,
                    disabled,
                    stretch,
                    skin,
                })
            }
            NODE_SPACER => {
                let flex = self.extent()?;
                Ok(Node::Spacer { flex })
            }
            NODE_CANVAS => {
                let props = self.props()?;
                let touch_key = Some(self.prefixed_string()?).filter(|key| !key.is_empty());
                // The count includes dynamic draws,
                // which may come and go under an unchanged static half.
                let count_start = self.pos;
                let draw_count = self.u16()?;
                self.dynamic_ranges.push(count_start..self.pos);
                let mut draws = Vec::with_capacity(usize::from(draw_count));
                let mut first_dynamic = None;
                for _ in 0..draw_count {
                    let start = self.pos;
                    let draw = self.draw()?;
                    if first_dynamic.is_none() && draw_is_dynamic(&draw) {
                        first_dynamic = Some(start);
                    }
                    draws.push(draw);
                }
                // Every draw from the first dynamic one on is `Band::Dynamic` or `Band::Above`.
                if let Some(start) = first_dynamic {
                    self.dynamic_ranges.push(start..self.pos);
                }
                Ok(Node::Canvas {
                    props,
                    touch_key,
                    draws,
                })
            }
            NODE_MODAL => {
                let modal_id = self.prefixed_string()?;
                let is_open = self.u8()? != 0;
                let padding = self.u16()?;
                let backdrop_alpha = self.u8()?;
                let title = self.prefixed_string()?;
                let content_height = self.f32()?;
                let child_count = self.u16()?;
                let bg_color = self.color()?;
                let header_color = self.color()?;
                let title_color = self.color()?;
                let max_width = self.u16()?;
                let mut body = Vec::with_capacity(usize::from(child_count));
                for _ in 0..child_count {
                    body.push(self.node()?);
                }
                let footer_primary_key = self.prefixed_string()?;
                let footer_primary_label = self.prefixed_string()?;
                let footer_secondary_key = self.prefixed_string()?;
                let footer_secondary_label = self.prefixed_string()?;
                let footer_danger = self.u8()? != 0;
                Ok(Node::Modal {
                    modal_id,
                    is_open,
                    padding,
                    backdrop_alpha,
                    title,
                    content_height,
                    bg_color,
                    header_color,
                    title_color,
                    max_width,
                    body,
                    footer_primary_key,
                    footer_primary_label,
                    footer_secondary_key,
                    footer_secondary_label,
                    footer_danger,
                })
            }
            NODE_SCROLL => {
                let scroll_key = self.prefixed_string()?;
                let props = self.props()?;
                let children = self.children()?;
                Ok(Node::Scroll {
                    scroll_key,
                    props,
                    children,
                })
            }
            NODE_NOTIFICATION => {
                let kind = NotificationKind::from(self.u8()?);
                let title = self.prefixed_string()?;
                let subtitle = self.prefixed_string()?;
                Ok(Node::Notification {
                    kind,
                    title,
                    subtitle,
                })
            }
            NODE_RELTIME => {
                let anchor = self.i64()?;
                let format: RelTimeFormat = self.discriminant("relative-time format")?;
                let clamp: RelTimeClamp = self.discriminant("relative-time clamp")?;
                let style = self.text_style()?;
                Ok(Node::RelTime {
                    anchor,
                    format,
                    clamp,
                    style,
                })
            }
            NODE_TAG => {
                let kind: TagKind = self.discriminant("tag kind")?;
                let icon_mode: TagIconMode = self.discriminant("tag icon mode")?;
                let custom = self.svg_id()?;
                let icon = match icon_mode {
                    TagIconMode::Hidden => TagIcon::Hidden,
                    TagIconMode::Custom => custom.map_or(TagIcon::Hidden, TagIcon::Custom),
                    TagIconMode::Default => TagIcon::Default,
                };
                let content = Box::new(self.node()?);
                Ok(Node::Tag {
                    kind,
                    icon,
                    content,
                })
            }
            NODE_SWITCHER => {
                let active = usize::from(self.u8()?);
                let disabled = self.u8()? != 0;
                let tab_count = self.u8()?;
                let mut tabs = Vec::with_capacity(usize::from(tab_count));
                for _ in 0..tab_count {
                    let icon = self.svg_id()?;
                    let click_id = self.prefixed_string()?;
                    tabs.push(SwitcherTab { icon, click_id });
                }
                Ok(Node::Switcher {
                    active,
                    disabled,
                    tabs,
                })
            }
            NODE_SKELETON => Ok(Node::Skeleton(SkeletonData {
                kind: self.discriminant::<SkeletonKind>("skeleton kind")?,
                chars: self.f32()?,
                font_size: self.extent()?,
                width: self.extent()?,
                height: self.extent()?,
                color: self.color()?,
            })),
            NODE_PROGRESS_BAR => {
                let touch_key = Some(self.prefixed_string()?).filter(|key| !key.is_empty());
                let track_h = self.extent()?;
                let kind: ProgressKind = self.discriminant("progress kind")?;
                let fraction = self.f32()?;
                let active = self.u8()? != 0;
                let fill_color = self.color()?;
                let track_color = self.color()?;
                let bg_color = self.color()?;
                let skin = if self.u8()? != 0 {
                    Some(SliderSkin {
                        track: self.nine_patch()?,
                        track_h: self.u16()?,
                        thumb_id: self.bitmap_id()?,
                        thumb_w: self.u16()?,
                        thumb_h: self.u16()?,
                        thumb_pressed_id: self.bitmap_id()?,
                    })
                } else {
                    None
                };
                Ok(Node::ProgressBar {
                    touch_key,
                    track_h,
                    mode: ProgressMode::from_wire(kind, fraction),
                    active,
                    fill_color,
                    track_color,
                    bg_color,
                    skin,
                })
            }
            NODE_DIMMED => {
                let value_at = self.pos;
                let brightness = self.f32()?;
                if !(brightness.is_finite() && (0.0..=1.0).contains(&brightness)) {
                    return Err(DecodeError::InvalidBrightness {
                        value: brightness,
                        at: value_at,
                    });
                }
                let child = Box::new(self.node()?);
                Ok(Node::Dimmed { brightness, child })
            }
            tag => Err(DecodeError::UnknownNode { tag, at }),
        }
    }

    #[expect(clippy::too_many_lines)]
    fn draw(&mut self) -> Result<Draw, DecodeError> {
        let at = self.pos;
        let draw_type = self.u8()?;
        match draw_type {
            DRAW_RECT => {
                let x = self.f32()?;
                let y = self.f32()?;
                let w = self.extent()?;
                let h = self.extent()?;
                let fill = self.fill()?;
                Ok(Draw::Rect { x, y, w, h, fill })
            }
            DRAW_CIRCLE => {
                let cx = self.f32()?;
                let cy = self.f32()?;
                let r = self.extent()?;
                let fill = self.fill()?;
                Ok(Draw::Circle { cx, cy, r, fill })
            }
            DRAW_ARC => {
                let cx = self.f32()?;
                let cy = self.f32()?;
                let radius = self.extent()?;
                let start_angle = self.f32()?;
                let end_angle = self.f32()?;
                let width = self.extent()?;
                let fill = self.arc_fill()?;
                let segments = self.arc_segments()?;
                let cap = self.arc_cap()?;
                Ok(Draw::Arc {
                    cx,
                    cy,
                    radius,
                    start_angle,
                    end_angle,
                    width,
                    fill,
                    segments,
                    cap,
                })
            }
            DRAW_ICON => {
                let x = self.f32()?;
                let y = self.f32()?;
                let w = self.extent()?;
                let h = self.extent()?;
                let color = self.color()?;
                let icon_id = self.svg_id()?;
                let anti_alias = self.u8()? != 0;
                let fill_count = self.u16()?;
                let mut fills = Vec::with_capacity(usize::from(fill_count));
                for _ in 0..fill_count {
                    let id = self.prefixed_string()?;
                    let color = self.color()?;
                    fills.push((id, color));
                }
                Ok(Draw::Svg {
                    x,
                    y,
                    w,
                    h,
                    color,
                    icon_id,
                    anti_alias,
                    fills,
                })
            }
            DRAW_BITMAP => {
                let x = self.f32()?;
                let y = self.f32()?;
                let w = self.extent()?;
                let h = self.extent()?;
                let bitmap_id = self.bitmap_id()?;
                Ok(Draw::Bitmap {
                    x,
                    y,
                    w,
                    h,
                    bitmap_id,
                })
            }
            DRAW_QR => {
                let x = self.f32()?;
                let y = self.f32()?;
                let size = self.f32()?;
                let dark = self.color()?;
                let light = self.color()?;
                let quiet_zone = self.u8()?;
                let text = self.prefixed_string()?;
                Ok(Draw::Qr {
                    x,
                    y,
                    size,
                    style: QrStyle {
                        dark,
                        light,
                        quiet_zone,
                    },
                    text,
                })
            }
            DRAW_CENTERED => {
                let inner = self.draw()?;
                Ok(Draw::Centered {
                    inner: Box::new(inner),
                })
            }
            DRAW_ORBIT => {
                let radius = self.extent()?;
                let angle = self.f32()?;
                let inner = self.draw()?;
                Ok(Draw::Orbit {
                    radius,
                    angle,
                    inner: Box::new(inner),
                })
            }
            DRAW_ROTATED => {
                let angle = self.f32()?;
                let inner = self.draw()?;
                Ok(Draw::Rotated {
                    angle,
                    inner: Box::new(inner),
                })
            }
            DRAW_SHADOW => {
                let dx = self.f32()?;
                let dy = self.f32()?;
                let blur = self.f32()?;
                let color = self.color()?;
                let inner = self.draw()?;
                Ok(Draw::Shadow {
                    dx,
                    dy,
                    blur,
                    color,
                    inner: Box::new(inner),
                })
            }
            DRAW_MODIFIED => {
                let flags = self.u8()?;
                let has_animations = flags & 0x01 != 0;
                let has_transition = flags & 0x02 != 0;
                let color_space =
                    ColorSpace::from_u8((flags >> 2) & 0x03).unwrap_or(ColorSpace::Oklab);

                let animations = if has_animations {
                    let count = self.u8()?;
                    let mut anims = Vec::with_capacity(usize::from(count));
                    for _ in 0..count {
                        let property =
                            self.animation_field("animation property", AnimProperty::from_u8)?;
                        let from = self.f32()?;
                        let to = self.f32()?;
                        let duration_ms = self.u32()?;
                        let delay_ms = self.u16()?;
                        let easing = self.animation_field("easing", Easing::from_u8)?;
                        let loop_mode = self.animation_field("loop mode", LoopMode::from_u8)?;
                        anims.push(AnimationDef {
                            property,
                            from,
                            to,
                            duration_ms,
                            delay_ms,
                            easing,
                            loop_mode,
                        });
                    }
                    anims
                } else {
                    Vec::new()
                };

                let transition = if has_transition {
                    let id_hash = self.u32()?;
                    let duration_ms = self.u32()?;
                    let easing = self.animation_field("transition easing", Easing::from_u8)?;
                    Some(TransitionDef {
                        id_hash,
                        duration_ms,
                        easing,
                    })
                } else {
                    None
                };

                let inner = self.draw()?;
                Ok(Draw::Modified {
                    animations,
                    transition,
                    color_space,
                    inner: Box::new(inner),
                })
            }
            DRAW_PATH => {
                let flags = self.u8()?;
                let closed = flags & 0x01 != 0;
                let interpolation = if flags & 0x02 != 0 {
                    Interpolation::CatmullRom
                } else {
                    Interpolation::Linear
                };
                let fill = flags & 0x04 != 0;
                let dashed = flags & 0x08 != 0;
                let point_count = self.u16()?;
                let mut points = Vec::with_capacity(usize::from(point_count));
                for _ in 0..point_count {
                    let x = self.f32()?;
                    let y = self.f32()?;
                    points.push((x, y));
                }
                let paint = if fill {
                    PathPaint::Fill(self.fill()?)
                } else {
                    let color = self.color()?;
                    let width = self.extent()?;
                    let dash = if dashed {
                        // Both figures are read either way, so a rejected pattern
                        // doesn't desync the stream — it just draws solid.
                        Dash::new(self.f32()?, self.f32()?)
                    } else {
                        None
                    };
                    PathPaint::Stroke { color, width, dash }
                };
                Ok(Draw::Path {
                    points,
                    paint,
                    closed,
                    interpolation,
                })
            }
            DRAW_SPHERE => {
                let x = self.f32()?;
                let y = self.f32()?;
                let w = self.extent()?;
                let h = self.extent()?;
                let bitmap_id = self.bitmap_id()?;

                let flags = self.u8()?;
                let atmosphere = flags & 0x01 != 0;
                let center_lat = self.f32()?;
                let center_lon = self.f32()?;
                let zoom = self.f32()?;
                let light_lat = self.f32()?;
                let light_lon = self.f32()?;
                Ok(Draw::Sphere {
                    x,
                    y,
                    w,
                    h,
                    bitmap_id,
                    atmosphere,
                    center_lat,
                    center_lon,
                    zoom,
                    light_lat,
                    light_lon,
                })
            }
            DRAW_MESH => {
                let x = self.f32()?;
                let y = self.f32()?;
                let w = self.extent()?;
                let h = self.extent()?;
                let mesh_id = self.mesh_id()?;

                let transform = MeshTransform {
                    fov: self.f32()?,
                    distance: self.f32()?,
                    quat: [self.f32()?, self.f32()?, self.f32()?, self.f32()?],
                    position: [self.f32()?, self.f32()?, self.f32()?],
                    scale: self.f32()?,
                };
                let lighting = MeshLighting {
                    pitch: self.f32()?,
                    yaw: self.f32()?,
                    ambient: self.f32()?,
                    specular: self.f32()?,
                };
                let highlight = MeshHighlight {
                    u_min: self.f32()?,
                    v_min: self.f32()?,
                    u_max: self.f32()?,
                    v_max: self.f32()?,
                    r: self.f32()?,
                    g: self.f32()?,
                    b: self.f32()?,
                };
                Ok(Draw::Mesh {
                    x,
                    y,
                    w,
                    h,
                    mesh_id,
                    args: MeshDrawArgs {
                        transform,
                        lighting,
                        highlight,
                    },
                })
            }
            DRAW_TEXT => {
                let x = self.f32()?;
                let y = self.f32()?;
                let style = self.text_style()?;
                let text = self.prefixed_string()?;
                Ok(Draw::Text { x, y, text, style })
            }
            DRAW_CURVED_TEXT => {
                let cx = self.f32()?;
                let cy = self.f32()?;
                let radius = self.extent()?;
                let angle = self.f32()?;
                let anchor: ArcAnchor = self.discriminant("arc anchor")?;
                let facing: ArcTextFacing = self.discriminant("arc text facing")?;
                let style = self.text_style()?;
                let text = self.prefixed_string()?;
                Ok(Draw::CurvedText {
                    cx,
                    cy,
                    radius,
                    angle,
                    anchor,
                    facing,
                    text,
                    style,
                })
            }
            DRAW_NINE_PATCH => {
                let x = self.f32()?;
                let y = self.f32()?;
                let w = self.extent()?;
                let h = self.extent()?;
                let nine_patch = self.nine_patch()?;
                Ok(Draw::NinePatch {
                    x,
                    y,
                    w,
                    h,
                    nine_patch,
                })
            }
            DRAW_AUTOFIT_TEXT => {
                let x = self.f32()?;
                let y = self.f32()?;
                let box_width = self.extent()?;
                let box_height = self.extent()?;
                let mode = AutoFit::from_u8(self.u8()?);
                let min_size = self.u16()?;
                let max_size = self.u16()?;
                let style = self.text_style()?;
                let text = self.prefixed_string()?;
                Ok(Draw::AutofitText {
                    x,
                    y,
                    box_width,
                    box_height,
                    mode,
                    min_size,
                    max_size,
                    text,
                    style,
                })
            }
            tag => Err(DecodeError::UnknownDraw { tag, at }),
        }
    }

    /// A one-byte animation enum read through its `from_u8`, `what` naming it in the error.
    fn animation_field<T>(
        &mut self,
        what: &'static str,
        from_u8: fn(u8) -> Option<T>,
    ) -> Result<T, DecodeError> {
        let at = self.pos;
        let value = self.u8()?;
        from_u8(value).ok_or(DecodeError::InvalidDiscriminant { what, value, at })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arc::ArcFill;
    use crate::colors::{BLACK, WHITE};
    use crate::components::text::text;
    use crate::fill::FILL_LINEAR;
    use crate::nodes::{BUTTON_SIZE_NORMAL, DROP_SHADOW_BLUR_MAX, TagKind};
    use crate::relative_time::{RelTimeLength, RelTimeSegments};
    use crate::text::FontWeight;
    use crate::tree::{DropShadow, canvas, col, dimmed, make_button, row, spacer};

    fn draw_only(bytes: &[u8]) -> Result<Draw, DecodeError> {
        Reader::new(bytes).draw()
    }

    // ── Boundary validation, moved from the host's decoder ──────────

    /// A negative extent paints mirrored geometry
    /// that damage tracking reports nothing for — see `Reader::extent`.
    #[test]
    fn a_negative_extent_is_refused_at_the_boundary() {
        let mut data = vec![DRAW_RECT];
        data.extend_from_slice(&100.0_f32.to_le_bytes());
        data.extend_from_slice(&0.0_f32.to_le_bytes());
        data.extend_from_slice(&(-50.0_f32).to_le_bytes());
        data.extend_from_slice(&10.0_f32.to_le_bytes());
        let err = draw_only(&data).expect_err("a negative width must not reach the renderer");
        assert!(
            matches!(err, DecodeError::InvalidExtent { .. }),
            "wrong rejection: {err}"
        );
    }

    #[test]
    fn a_negative_gap_is_refused_at_the_boundary() {
        let props = PropsData {
            gap: -1.0,
            ..PropsData::default()
        };
        let bytes = props.to_bytes();
        let err = Reader::new(&bytes)
            .props()
            .expect_err("a negative gap must not reach layout");
        assert!(
            matches!(err, DecodeError::InvalidProps { at: 0 }),
            "wrong rejection: {err}"
        );
    }

    #[test]
    fn a_negative_spacer_flex_is_refused_at_the_boundary() {
        let mut data = vec![NODE_SPACER];
        data.extend_from_slice(&(-1.0_f32).to_le_bytes());
        let err = decode(&data).expect_err("a negative flex must not reach layout");
        assert!(
            matches!(err, DecodeError::InvalidExtent { .. }),
            "wrong rejection: {err}"
        );
    }

    /// `f32::min`/`max` ignore NaN, so one in a rotation range leaves the sweep
    /// at `+INF`/`-INF` and the draw reports no damage while still painting —
    /// permanently, since a transition keeps it in its recorded `from`.
    #[test]
    fn a_non_finite_extent_is_refused_at_the_boundary() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut data = vec![DRAW_RECT];
            data.extend_from_slice(&0.0_f32.to_le_bytes());
            data.extend_from_slice(&0.0_f32.to_le_bytes());
            data.extend_from_slice(&bad.to_le_bytes());
            data.extend_from_slice(&10.0_f32.to_le_bytes());
            let err = draw_only(&data).expect_err("a non-finite extent must not reach the tree");
            assert!(
                matches!(err, DecodeError::InvalidExtent { .. }),
                "wrong rejection for {bad}: {err}"
            );
        }
    }

    #[test]
    fn rect_round_trips_a_radial_fill() {
        let mut data = vec![DRAW_RECT];
        data.extend_from_slice(&1.0_f32.to_le_bytes());
        data.extend_from_slice(&2.0_f32.to_le_bytes());
        data.extend_from_slice(&3.0_f32.to_le_bytes());
        data.extend_from_slice(&4.0_f32.to_le_bytes());
        let red = Color::from_rgb(0xFF, 0, 0);
        let blue = Color::from_rgb(0, 0, 0xFF);
        encode_fill(&mut data, &Fill::radial(red, blue));

        let cmd = draw_only(&data).expect("BUG: test buffer encodes a valid DRAW_RECT");
        let Draw::Rect { fill, .. } = cmd else {
            panic!("expected Rect, got {cmd:?}");
        };
        assert_eq!(fill, Fill::radial(red, blue));
    }

    #[test]
    fn circle_round_trips_a_linear_fill() {
        let mut data = vec![DRAW_CIRCLE];
        data.extend_from_slice(&5.0_f32.to_le_bytes());
        data.extend_from_slice(&6.0_f32.to_le_bytes());
        data.extend_from_slice(&7.0_f32.to_le_bytes());
        let a = Color::from_rgb(1, 2, 3);
        let b = Color::from_rgb(4, 5, 6);
        encode_fill(&mut data, &Fill::linear(0.0, a, b));

        let Draw::Circle { cx, cy, r, fill } =
            draw_only(&data).expect("BUG: test buffer encodes a valid DRAW_CIRCLE")
        else {
            panic!("expected Circle");
        };
        assert_eq!((cx, cy, r), (5.0, 6.0, 7.0));
        assert_eq!(fill, Fill::linear(0.0, a, b));
    }

    #[test]
    fn path_fill_round_trips_a_linear_fill() {
        let mut data = vec![DRAW_PATH];
        data.push(0x04);
        data.extend_from_slice(&2_u16.to_le_bytes());
        for (x, y) in [(0.0_f32, 0.0_f32), (10.0, 10.0)] {
            data.extend_from_slice(&x.to_le_bytes());
            data.extend_from_slice(&y.to_le_bytes());
        }
        let a = Color::from_rgb(1, 2, 3);
        let b = Color::from_rgb(4, 5, 6);
        encode_fill(&mut data, &Fill::linear(0.0, a, b));

        let Draw::Path { paint, .. } =
            draw_only(&data).expect("BUG: test buffer encodes a valid DRAW_PATH")
        else {
            panic!("expected Path");
        };
        assert_eq!(paint, PathPaint::Fill(Fill::linear(0.0, a, b)));
    }

    #[test]
    fn path_stroke_round_trips_a_dash_pattern() {
        let mut data = vec![DRAW_PATH];
        data.push(0x08); // stroke (not fill) + dashed
        data.extend_from_slice(&2_u16.to_le_bytes());
        for (x, y) in [(0.0_f32, 0.0_f32), (10.0, 0.0)] {
            data.extend_from_slice(&x.to_le_bytes());
            data.extend_from_slice(&y.to_le_bytes());
        }
        let raw = 0x1122_3344_u32;
        data.extend_from_slice(&raw.to_le_bytes()); // color
        data.extend_from_slice(&3.0_f32.to_le_bytes()); // width
        data.extend_from_slice(&6.0_f32.to_le_bytes()); // dash on
        data.extend_from_slice(&4.0_f32.to_le_bytes()); // dash off

        let Draw::Path { paint, .. } =
            draw_only(&data).expect("BUG: test buffer encodes a valid dashed DRAW_PATH")
        else {
            panic!("expected Path");
        };
        assert_eq!(
            paint,
            PathPaint::Stroke {
                color: Color::from_raw(raw),
                width: 3.0,
                dash: Some(Dash { on: 6.0, off: 4.0 }),
            }
        );
    }

    #[test]
    fn qr_round_trips_geometry_style_and_text() {
        let mut data = vec![DRAW_QR];
        data.extend_from_slice(&1.0_f32.to_le_bytes()); // x
        data.extend_from_slice(&2.0_f32.to_le_bytes()); // y
        data.extend_from_slice(&40.0_f32.to_le_bytes()); // size
        data.extend_from_slice(&BLACK.to_u32().to_le_bytes()); // dark
        data.extend_from_slice(&WHITE.to_u32().to_le_bytes()); // light
        data.push(3); // quiet_zone
        data.extend_from_slice(&2_u16.to_le_bytes()); // text len
        data.extend_from_slice(b"hi");

        let Draw::Qr {
            x,
            y,
            size,
            style,
            text,
        } = draw_only(&data).expect("BUG: test buffer encodes a valid DRAW_QR")
        else {
            panic!("expected Qr");
        };
        assert_eq!((x, y, size), (1.0, 2.0, 40.0));
        assert_eq!(
            (style.dark.to_u32(), style.light.to_u32()),
            (BLACK.to_u32(), WHITE.to_u32())
        );
        assert_eq!(style.quiet_zone, 3);
        assert_eq!(text, "hi");
    }

    #[test]
    fn autofit_text_decodes_geometry_mode_bounds_style_and_text() {
        let style = TextStyle {
            size: 40,
            ..TextStyle::default()
        };
        let mut data = vec![DRAW_AUTOFIT_TEXT];
        data.extend_from_slice(&10.0_f32.to_le_bytes());
        data.extend_from_slice(&20.0_f32.to_le_bytes());
        data.extend_from_slice(&100.0_f32.to_le_bytes());
        data.extend_from_slice(&50.0_f32.to_le_bytes());
        data.push(AutoFit::ShrinkAndGrow as u8);
        data.extend_from_slice(&14_u16.to_le_bytes());
        data.extend_from_slice(&64_u16.to_le_bytes());
        data.extend_from_slice(&style.to_bytes());
        data.extend_from_slice(&5_u16.to_le_bytes());
        data.extend_from_slice(b"HELLO");

        let Draw::AutofitText {
            x,
            y,
            box_width,
            box_height,
            mode,
            min_size,
            max_size,
            text,
            style: s,
        } = draw_only(&data).expect("BUG: decode")
        else {
            panic!("BUG: expected AutofitText");
        };
        assert_eq!((x, y, box_width, box_height), (10.0, 20.0, 100.0, 50.0));
        assert_eq!(mode, AutoFit::ShrinkAndGrow);
        assert_eq!((min_size, max_size), (14, 64));
        assert_eq!(text, "HELLO");
        assert_eq!(s.size, 40);
    }

    #[test]
    fn read_draw_decodes_shadow() {
        let mut bytes = vec![DRAW_SHADOW];
        bytes.extend_from_slice(&2.0_f32.to_le_bytes());
        bytes.extend_from_slice(&3.0_f32.to_le_bytes());
        bytes.extend_from_slice(&7.5_f32.to_le_bytes());
        bytes.extend_from_slice(&0x1122_3344_u32.to_le_bytes());
        bytes.push(DRAW_RECT);
        for v in [1.0_f32, 1.0, 4.0, 5.0] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        encode_fill(&mut bytes, &Fill::Solid(Color::from_raw(0xFF00_FF00)));

        let draw = draw_only(&bytes).expect("BUG: test buffer encodes a valid DRAW_SHADOW");
        let Draw::Shadow {
            dx,
            dy,
            blur,
            color,
            inner,
        } = draw
        else {
            panic!("expected Draw::Shadow, got {draw:?}");
        };
        assert_eq!((dx, dy, blur), (2.0, 3.0, 7.5));
        assert_eq!(color, Color::from_raw(0x1122_3344));
        assert!(
            matches!(*inner, Draw::Rect { .. }),
            "shadow inner should decode as the wrapped rect",
        );
    }

    #[test]
    fn read_draw_decodes_arc() {
        let red = Color::from_rgb(0xFF, 0x00, 0x00);
        let teal = Color::from_rgb(0x00, 0x80, 0x80);
        let expected_fill = ArcFill::gradient(red, teal);
        let expected_segments = ArcSegments::Explicit(vec![(0.0, 0.5), (0.6, 1.5)]);

        let mut data = vec![DRAW_ARC];
        for value in [1.0_f32, 2.0, 30.0, 0.0, 1.5, 6.0] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        encode_arc_fill(&mut data, &expected_fill);
        encode_arc_segments(&mut data, &expected_segments);
        encode_arc_cap(&mut data, ArcCap::Butt);

        let Draw::Arc {
            cx,
            cy,
            radius,
            start_angle,
            end_angle,
            width,
            fill,
            segments,
            cap,
        } = draw_only(&data).expect("BUG: test buffer encodes a valid DRAW_ARC")
        else {
            panic!("BUG: expected Draw::Arc");
        };

        assert_eq!((cx, cy, radius), (1.0, 2.0, 30.0));
        assert_eq!((start_angle, end_angle, width), (0.0, 1.5, 6.0));
        assert_eq!(fill, expected_fill);
        assert_eq!(segments, expected_segments);
        assert_eq!(cap, ArcCap::Butt);
    }

    /// A span's colour and size are both an optional `u32`.
    ///
    /// Only their order on the wire tells them apart, so what pins
    /// that order down is decoding a span that carries both.
    #[test]
    fn a_span_decodes_its_colour_beside_its_size() {
        const COLOR_RAW: u32 = 0x08BD_BAFF;
        const SIZE: u32 = 24;
        // Literal words, not the packer's: weight 700 in bits 0–11,
        // has_weight 12, has_color 13, underline 15; has_size is extra bit 1.
        const FLAGS: u16 = 0x2BC | 0x1000 | 0x2000 | 0x8000;
        const EXTRA: u8 = 0x02;
        let text = "PH/s";

        let mut data = Vec::new();
        data.extend_from_slice(&FLAGS.to_le_bytes());
        data.push(EXTRA);
        data.extend_from_slice(
            &u16::try_from(text.len())
                .expect("BUG: fits u16")
                .to_le_bytes(),
        );
        data.extend_from_slice(text.as_bytes());
        data.extend_from_slice(&COLOR_RAW.to_le_bytes());
        data.extend_from_slice(&SIZE.to_le_bytes());

        let span = Reader::new(&data)
            .span()
            .expect("BUG: test buffer encodes a valid span");

        assert_eq!(span.text, text);
        assert_eq!(span.weight, Some(FontWeight::BOLD));
        assert_eq!(
            span.color,
            Some(Color::from_raw(COLOR_RAW)),
            "the colour read back as something else — the two u32s swapped"
        );
        assert_eq!(
            span.size,
            Some(SIZE),
            "the size read back as something else — the two u32s swapped"
        );
        assert!(span.underline);
        assert!(!span.italic);
        assert!(!span.strikethrough);
    }

    /// A span written before spans carried a size decodes as it always did:
    /// no size bit, so nothing is read past its own record.
    ///
    /// That is the compatibility the SDK version promises
    /// — a new host reads every old widget; the reverse is not owed.
    /// The bytes are literal, as that writer emitted them, with every flag
    /// it knew set: a packer that moved a bit would still round-trip itself.
    #[test]
    fn an_old_widgets_span_decodes_unchanged_on_a_new_host() {
        const NEXT_RECORD: u32 = 0xDEAD_BEEF;
        const COLOR_RAW: u32 = 0x1234_5678;
        // weight 700 in bits 0–11, has_weight 12, has_color 13; italic 14 and
        // underline 15 left clear, so a flag that strays onto them is seen.
        // The extra byte was strikethrough alone, in bit 0.
        const OLD_FLAGS: u16 = 0x2BC | 0x1000 | 0x2000;
        const OLD_EXTRA: u8 = 0x01;
        let text = "500,0";

        let mut data = Vec::new();
        data.extend_from_slice(&OLD_FLAGS.to_le_bytes());
        data.push(OLD_EXTRA);
        data.extend_from_slice(
            &u16::try_from(text.len())
                .expect("BUG: fits u16")
                .to_le_bytes(),
        );
        data.extend_from_slice(text.as_bytes());
        data.extend_from_slice(&COLOR_RAW.to_le_bytes());
        data.extend_from_slice(&NEXT_RECORD.to_le_bytes());

        let mut reader = Reader::new(&data);
        let span = reader
            .span()
            .expect("BUG: test buffer encodes a valid span");

        assert_eq!(span.text, text);
        assert_eq!(span.weight, Some(FontWeight::BOLD));
        assert_eq!(span.color, Some(Color::from_raw(COLOR_RAW)));
        assert!(span.strikethrough);
        assert!(
            !span.italic && !span.underline,
            "a flag read from a bit it was never on"
        );
        assert_eq!(
            span.size, None,
            "a span that never announced a size must not be handed one"
        );
        assert_eq!(
            reader.u32().expect("BUG: the next record follows the span"),
            NEXT_RECORD,
            "the reader ran past the span's record into the next one"
        );
    }

    #[test]
    fn curved_text_decodes_geometry_enums_style_and_text() {
        let style = TextStyle {
            size: 18,
            color: Color::from_rgb(9, 8, 7),
            ..TextStyle::default()
        };
        let mut data = vec![DRAW_CURVED_TEXT];
        for value in [1.0_f32, 2.0, 44.0, std::f32::consts::PI] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        data.push(u8::from(ArcAnchor::Center));
        data.push(u8::from(ArcTextFacing::Outward));
        data.extend_from_slice(&style.to_bytes());
        data.extend_from_slice(&5_u16.to_le_bytes());
        data.extend_from_slice(b"HELLO");

        let Draw::CurvedText {
            cx,
            cy,
            radius,
            angle,
            anchor,
            facing,
            text,
            style: decoded_style,
        } = draw_only(&data).expect("BUG: test buffer encodes a valid DRAW_CURVED_TEXT")
        else {
            panic!("expected CurvedText");
        };

        assert_eq!(
            (
                cx.to_bits(),
                cy.to_bits(),
                radius.to_bits(),
                angle.to_bits()
            ),
            (
                1.0_f32.to_bits(),
                2.0_f32.to_bits(),
                44.0_f32.to_bits(),
                std::f32::consts::PI.to_bits(),
            ),
        );
        assert_eq!(anchor, ArcAnchor::Center);
        assert_eq!(facing, ArcTextFacing::Outward);
        assert_eq!(text, "HELLO");
        assert_eq!(decoded_style.color, style.color);
        assert_eq!(decoded_style.size, style.size);
    }

    #[test]
    fn a_relative_time_node_decodes_from_the_wire() {
        let mut bytes = vec![NODE_RELTIME];
        bytes.extend_from_slice(&1_700_000_000_i64.to_le_bytes());
        bytes.push(u8::from(RelTimeFormat {
            length: RelTimeLength::Short,
            segments: RelTimeSegments::Single,
        }));
        bytes.push(u8::from(RelTimeClamp::ElapsedOnly));
        bytes.extend_from_slice(&TextStyle::default().to_bytes());

        let node = decode(&bytes).expect("BUG: RelTime should decode").root;
        assert!(matches!(
            node,
            Node::RelTime {
                anchor: 1_700_000_000,
                format: RelTimeFormat {
                    length: RelTimeLength::Short,
                    segments: RelTimeSegments::Single,
                },
                clamp: RelTimeClamp::ElapsedOnly,
                ..
            }
        ));
    }

    fn tag_wire(kind: TagKind, icon_mode: u8, icon: u16) -> Vec<u8> {
        let mut bytes = vec![NODE_TAG, kind as u8, icon_mode];
        bytes.extend_from_slice(&icon.to_le_bytes());
        bytes.push(NODE_SPACER);
        bytes.extend_from_slice(&1.0_f32.to_le_bytes());
        bytes
    }

    #[test]
    fn each_tag_icon_mode_decodes_to_its_icon() {
        let icon = SvgId::from_wire(7).expect("BUG: nonzero");
        let decoded = |mode: TagIconMode, raw: u16| {
            let Node::Tag { icon, .. } = decode(&tag_wire(TagKind::Warning, mode as u8, raw))
                .expect("BUG: Tag should decode")
                .root
            else {
                panic!("expected a tag");
            };
            icon
        };
        assert_eq!(decoded(TagIconMode::Default, 0), TagIcon::Default);
        assert_eq!(decoded(TagIconMode::Hidden, 0), TagIcon::Hidden);
        assert_eq!(decoded(TagIconMode::Custom, 7), TagIcon::Custom(icon));
        assert_eq!(
            decoded(TagIconMode::Custom, 0),
            TagIcon::Hidden,
            "a custom icon that is absent draws nothing, as the host always did"
        );
    }

    #[test]
    fn each_tag_icon_encodes_its_mode() {
        let encoded = |icon: TagIcon| {
            encode(&Node::Tag {
                kind: TagKind::Warning,
                icon,
                content: Box::new(Node::Spacer { flex: 1.0 }),
            })
        };
        let icon = SvgId::from_wire(7).expect("BUG: nonzero");
        assert_eq!(
            encoded(TagIcon::Default),
            tag_wire(TagKind::Warning, TagIconMode::Default as u8, 0)
        );
        assert_eq!(
            encoded(TagIcon::Hidden),
            tag_wire(TagKind::Warning, TagIconMode::Hidden as u8, 0)
        );
        assert_eq!(
            encoded(TagIcon::Custom(icon)),
            tag_wire(TagKind::Warning, TagIconMode::Custom as u8, 7)
        );
    }

    #[test]
    fn unknown_icon_mode_is_rejected() {
        // Strict: an unknown mode fails decoding, never silently defaults.
        let err = decode(&tag_wire(TagKind::Info, 99, 0)).expect_err("unknown mode");
        assert!(matches!(
            err,
            DecodeError::InvalidDiscriminant {
                what: "tag icon mode",
                value: 99,
                ..
            }
        ));
    }

    // ── Round trips ─────────────────────────────────────────────────

    #[expect(clippy::too_many_lines, reason = "one of every draw kind, spelled out")]
    fn every_draw() -> Vec<Draw> {
        let red = Color::from_rgb(0xFF, 0, 0);
        let blue = Color::from_rgb(0, 0, 0xFF);
        let style = TextStyle {
            size: 22,
            weight: FontWeight::BOLD,
            ..TextStyle::default()
        };
        vec![
            Draw::rect(1.0, 2.0, 3.0, 4.0, Fill::linear(90.0, red, blue)),
            Draw::circle(5.0, 6.0, 7.0, red),
            Draw::arc(
                1.0,
                2.0,
                3.0,
                0.25,
                1.5,
                4.0,
                ArcFill::gradient(red, blue),
                ArcSegments::Explicit(vec![(0.0, 0.25), (0.5, 1.0)]),
                ArcCap::Butt,
            ),
            Draw::centered(Draw::orbit(
                10.0,
                0.5,
                Draw::rotated(1.0, Draw::circle(0.0, 0.0, 1.0, red)),
            )),
            Draw::rect(0.0, 0.0, 10.0, 10.0, red).with_drop_shadow(DropShadow {
                dx: 1.0,
                dy: 2.0,
                blur: DROP_SHADOW_BLUR_MAX,
                color: Color::from_rgba(0, 0, 0, 0x80),
            }),
            Draw::svg_id(0.0, 0.0, 16.0, 16.0, SvgId::from_wire(3), red)
                .with_anti_alias()
                .fill("rim", blue),
            Draw::bitmap_id(0.0, 0.0, 8.0, 8.0, BitmapId::from_wire(0xA55A)),
            Draw::qr(1.0, 2.0, 40.0, "https://example.test", QrStyle::default()),
            Draw::rect(0.0, 0.0, 1.0, 1.0, red)
                .animate(
                    AnimProperty::Rotate,
                    0.0,
                    1.0,
                    1000,
                    Easing::EaseOutBack,
                    LoopMode::PingPong,
                )
                .animate_color(red, blue, 500, Easing::Linear, LoopMode::Once)
                .transition_with_color_space(
                    "hour-hand",
                    300,
                    Easing::EaseInOut,
                    ColorSpace::Oklch,
                ),
            Draw::path(
                vec![(0.0, 0.0), (10.0, 10.0)],
                2.0,
                red,
                true,
                Interpolation::CatmullRom,
            ),
            Draw::dashed_path(vec![(0.0, 0.0), (10.0, 0.0)], 1.0, red, 6.0, 4.0),
            Draw::fill_path(
                vec![(0.0, 0.0), (10.0, 0.0), (5.0, 10.0)],
                Fill::radial(red, blue),
                true,
            ),
            Draw::text(10.0, 20.0, "Hello", style),
            Draw::curved_text(
                195.0,
                195.0,
                120.0,
                1.0,
                ArcAnchor::End,
                ArcTextFacing::Inward,
                "MINING",
                style,
            ),
            Draw::autofit_text_ranged(
                4.0,
                8.0,
                200.0,
                80.0,
                "Many names",
                style,
                AutoFit::ShrinkAndGrow,
                14,
                64,
            ),
            Draw::nine_patch_id(
                0,
                0,
                100,
                40,
                NinePatch::from_id(BitmapId::from_wire(2).expect("BUG: nonzero"), 4, 4, 4, 4),
            ),
            Draw::sphere_id(
                0.0,
                0.0,
                400.0,
                400.0,
                BitmapId::from_wire(5),
                50.0,
                14.0,
                1.8,
                Some((20.0, 30.0)),
                true,
            ),
            Draw::sphere_id(0.0, 0.0, 400.0, 400.0, None, 50.0, 14.0, 1.8, None, false),
            Draw::mesh_id(
                0.0,
                0.0,
                200.0,
                200.0,
                MeshId::from_wire(9),
                crate::mesh::MeshView::default(),
            ),
        ]
    }

    #[expect(clippy::too_many_lines, reason = "one of every node kind, spelled out")]
    fn every_node() -> Node {
        let skin = ButtonSkin {
            normal: NinePatch::from_id(BitmapId::from_wire(1).expect("BUG: nonzero"), 1, 2, 3, 4),
            pressed: Some(NinePatch::from_id(
                BitmapId::from_wire(2).expect("BUG: nonzero"),
                4,
                3,
                2,
                1,
            )),
            text_color: WHITE,
            pressed_text_color: BLACK,
            opaque: true,
        };
        let slider_skin = SliderSkin {
            track: NinePatch::from_id(BitmapId::from_wire(3).expect("BUG: nonzero"), 2, 2, 2, 2),
            track_h: 12,
            thumb_id: BitmapId::from_wire(4),
            thumb_w: 16,
            thumb_h: 16,
            thumb_pressed_id: None,
        };
        let progress =
            |mode: ProgressMode, key: Option<&str>, skin: Option<SliderSkin>| Node::ProgressBar {
                touch_key: key.map(str::to_owned),
                track_h: 4.0,
                mode,
                active: true,
                fill_color: WHITE,
                track_color: BLACK,
                bg_color: Color::default(),
                skin,
            };
        col(
            PropsData::default(),
            [
                row(
                    PropsData {
                        gap: 8.0,
                        ..PropsData::default()
                    },
                    [spacer(1.0), spacer(2)],
                ),
                Node::Center(
                    PropsData::default(),
                    vec![text("centred", TextStyle::default())],
                ),
                Node::Paragraph {
                    props: PropsData::default(),
                    base_style: TextStyle::default(),
                    spans: vec![
                        Span::plain("plain "),
                        Span {
                            text: "styled".to_owned(),
                            weight: Some(FontWeight::BOLD),
                            color: Some(WHITE),
                            size: Some(24),
                            italic: true,
                            underline: true,
                            strikethrough: true,
                        },
                    ],
                },
                make_button(
                    "ok",
                    "OK",
                    ButtonStyle::Ghost,
                    ButtonSize::Large,
                    SvgId::from_wire(3),
                    true,
                    Some(skin),
                )
                .stretch(),
                make_button(
                    "plain",
                    "Plain",
                    ButtonStyle::Primary,
                    ButtonSize::Normal,
                    None,
                    false,
                    None,
                ),
                canvas(PropsData::default(), every_draw()),
                Node::Canvas {
                    props: PropsData::default(),
                    touch_key: Some("tap".to_owned()),
                    draws: Vec::new(),
                },
                Node::Notification {
                    kind: NotificationKind::Warning,
                    title: "Title".to_owned(),
                    subtitle: "Sub".to_owned(),
                },
                Node::RelTime {
                    anchor: 1_700_000_000,
                    format: RelTimeFormat {
                        length: RelTimeLength::Long,
                        segments: RelTimeSegments::Double,
                    },
                    clamp: RelTimeClamp::RemainingOnly,
                    style: TextStyle::default(),
                },
                Node::Tag {
                    kind: TagKind::Error,
                    icon: TagIcon::Custom(SvgId::from_wire(8).expect("BUG: nonzero")),
                    content: Box::new(text("tagged", TextStyle::default())),
                },
                Node::Tag {
                    kind: TagKind::Info,
                    icon: TagIcon::Default,
                    content: Box::new(spacer(0)),
                },
                Node::Switcher {
                    active: 1,
                    disabled: false,
                    tabs: vec![
                        SwitcherTab {
                            icon: SvgId::from_wire(1),
                            click_id: "a".to_owned(),
                        },
                        SwitcherTab {
                            icon: None,
                            click_id: "b".to_owned(),
                        },
                    ],
                },
                Node::Skeleton(SkeletonData {
                    kind: SkeletonKind::Heading,
                    chars: 12.0,
                    font_size: 24.0,
                    width: 0.0,
                    height: 0.0,
                    color: WHITE,
                }),
                Node::Scroll {
                    scroll_key: "list".to_owned(),
                    props: PropsData {
                        height: 100.0,
                        ..PropsData::default()
                    },
                    children: vec![spacer(1.0)],
                },
                Node::Modal {
                    modal_id: "m".to_owned(),
                    is_open: true,
                    title: "Modal".to_owned(),
                    content_height: 120.0,
                    padding: 16,
                    backdrop_alpha: 200,
                    bg_color: BLACK,
                    header_color: WHITE,
                    title_color: WHITE,
                    max_width: 300,
                    body: vec![spacer(1.0), progress(ProgressMode::Meter(0.5), None, None)],
                    footer_primary_key: "ok".to_owned(),
                    footer_primary_label: "OK".to_owned(),
                    footer_secondary_key: "cancel".to_owned(),
                    footer_secondary_label: "Cancel".to_owned(),
                    footer_danger: true,
                },
                progress(
                    ProgressMode::Slider(0.25),
                    Some("volume"),
                    Some(slider_skin),
                ),
                progress(ProgressMode::Indeterminate, None, None),
                dimmed(0.5, dimmed(0.25, spacer(1.0))),
            ],
        )
    }

    #[test]
    fn every_node_and_draw_round_trips_byte_for_byte() {
        let first = encode(&every_node());
        let decoded = decode(&first).expect("BUG: an encoded tree decodes");
        assert_eq!(decoded.consumed, first.len(), "the tree must be read whole");
        assert_eq!(encode(&decoded.root), first);
    }

    #[test]
    fn trailing_bytes_are_left_unread() {
        let mut bytes = encode(&spacer(1.0));
        let tree_len = bytes.len();
        bytes.extend_from_slice(&[0xAA, 0xBB]);
        let decoded = decode(&bytes).expect("BUG: a spacer decodes");
        assert_eq!(decoded.consumed, tree_len);
    }

    #[test]
    fn a_dimmed_node_wears_its_own_tag() {
        let bytes = encode(&dimmed(0.5, spacer(1.0)));
        assert_eq!(bytes[0], NODE_DIMMED);
        assert_eq!(&bytes[1..5], &0.5_f32.to_le_bytes());
        assert_eq!(bytes[5], NODE_SPACER);
    }

    #[test]
    fn a_dimmed_brightness_outside_the_unit_range_is_refused() {
        for bad in [-0.1_f32, 1.5, f32::NAN, f32::INFINITY] {
            let mut bytes = vec![NODE_DIMMED];
            bytes.extend_from_slice(&bad.to_le_bytes());
            bytes.push(NODE_SPACER);
            bytes.extend_from_slice(&1.0_f32.to_le_bytes());
            let err = decode(&bytes).expect_err("a dim that brightens or is not a number");
            assert!(
                matches!(err, DecodeError::InvalidBrightness { at: 1, .. }),
                "wrong rejection for {bad}: {err}"
            );
        }
    }

    /// The host used to read the two button bytes raw and convert at render time;
    /// decoding straight into the enums must agree on every byte value.
    #[test]
    fn button_enums_decode_as_the_host_converted_them() {
        for byte in 0..=u8::MAX {
            let mut bytes = vec![NODE_BUTTON, 0, 0, byte, byte, 0, 0, 0, 0, 0, 0];
            bytes.push(0);
            let Node::Button { style, size, .. } =
                decode(&bytes).expect("BUG: a bare button decodes").root
            else {
                panic!("expected a button");
            };
            assert_eq!(style, ButtonStyle::from(u32::from(byte)));
            assert_eq!(size, ButtonSize::from(byte));
        }
        assert_eq!(ButtonSize::from(BUTTON_SIZE_NORMAL), ButtonSize::Normal);
    }

    /// A button written before buttons carried a skin decodes only when it ends the stream:
    /// the skin flag is read whenever a byte follows.
    #[test]
    fn a_button_without_a_skin_trailer_decodes() {
        let bytes = vec![NODE_BUTTON, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        let Node::Button { skin, .. } = decode(&bytes).expect("BUG: decodes").root else {
            panic!("expected a button");
        };
        assert!(skin.is_none());
    }

    #[test]
    fn progress_fraction_survives_only_where_the_mode_carries_one() {
        let bar = |mode| Node::ProgressBar {
            touch_key: None,
            track_h: 4.0,
            mode,
            active: false,
            fill_color: WHITE,
            track_color: BLACK,
            bg_color: BLACK,
            skin: None,
        };
        for mode in [
            ProgressMode::Slider(0.75),
            ProgressMode::Meter(0.25),
            ProgressMode::Indeterminate,
        ] {
            let Node::ProgressBar { mode: decoded, .. } =
                decode(&encode(&bar(mode))).expect("BUG: decodes").root
            else {
                panic!("expected a progress bar");
            };
            assert_eq!(decoded, mode);
        }
    }

    #[test]
    fn decode_names_what_it_refuses() {
        let cases: Vec<(Vec<u8>, DecodeError)> = vec![
            (Vec::new(), DecodeError::Empty),
            (vec![NODE_COLUMN], DecodeError::InvalidProps { at: 1 }),
            (vec![0xFE], DecodeError::UnknownNode { tag: 0xFE, at: 0 }),
            (
                {
                    let mut bytes = vec![NODE_CANVAS];
                    bytes.extend_from_slice(&PropsData::default().to_bytes());
                    bytes.extend_from_slice(&0_u16.to_le_bytes());
                    bytes.extend_from_slice(&1_u16.to_le_bytes());
                    bytes.push(0x3F);
                    bytes
                },
                DecodeError::UnknownDraw {
                    tag: 0x3F,
                    at: 1 + PropsData::SIZE + 4,
                },
            ),
            (
                {
                    let mut bytes = vec![NODE_RELTIME];
                    bytes.extend_from_slice(&0_i64.to_le_bytes());
                    bytes.push(0xFF);
                    bytes
                },
                DecodeError::InvalidDiscriminant {
                    what: "relative-time format",
                    value: 0xFF,
                    at: 9,
                },
            ),
            (
                vec![NODE_BUTTON, 1, 0, 0xFF],
                DecodeError::InvalidUtf8 { at: 3 },
            ),
            (
                {
                    let mut bytes = vec![NODE_SPACER];
                    bytes.extend_from_slice(&1.0_f32.to_le_bytes()[..2]);
                    bytes
                },
                DecodeError::Truncated { at: 1 },
            ),
        ];
        for (bytes, expected) in cases {
            let err = decode(&bytes).expect_err("the bytes are not a tree");
            assert_eq!(err, expected, "for {bytes:02X?}");
        }
    }

    #[test]
    fn rect_serializes_linear_fill_after_geometry() {
        let red = Color::from_rgb(0xFF, 0, 0);
        let blue = Color::from_rgb(0, 0, 0xFF);
        let mut bytes = Vec::new();
        Writer(&mut bytes).draw(&Draw::rect(
            1.0,
            2.0,
            3.0,
            4.0,
            Fill::linear(90.0, red, blue),
        ));
        assert_eq!(bytes[0], DRAW_RECT);
        assert_eq!(
            bytes[17], FILL_LINEAR,
            "fill block must follow 16 bytes of geometry"
        );
    }

    #[test]
    #[should_panic(expected = "BUG: Draw::CurvedText text exceeds u16::MAX bytes")]
    fn curved_text_past_the_wire_length_is_a_bug() {
        let text = "x".repeat(usize::from(u16::MAX) + 1);
        let draw = Draw::curved_text(
            0.0,
            0.0,
            10.0,
            0.0,
            ArcAnchor::Center,
            ArcTextFacing::Outward,
            text,
            TextStyle::default(),
        );
        Writer(&mut Vec::new()).draw(&draw);
    }

    #[test]
    fn dynamic_ranges_cover_the_host_driven_nodes() {
        let tree = col(
            PropsData::default(),
            [
                spacer(1.0),
                Node::RelTime {
                    anchor: 0,
                    format: RelTimeFormat {
                        length: RelTimeLength::Short,
                        segments: RelTimeSegments::Single,
                    },
                    clamp: RelTimeClamp::Auto,
                    style: TextStyle::default(),
                },
            ],
        );
        let bytes = encode(&tree);
        let decoded = decode(&bytes).expect("BUG: decodes");
        let [range] = decoded.dynamic_ranges.as_slice() else {
            panic!("one dynamic node, one range: {:?}", decoded.dynamic_ranges);
        };
        let rel_time_start = bytes.len() - (1 + 8 + 1 + 1 + TextStyle::SIZE);
        assert_eq!(*range, rel_time_start + NODE_TYPE_LEN..bytes.len());
    }
}
