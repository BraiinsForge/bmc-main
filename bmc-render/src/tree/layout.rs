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

//! Building the taffy tree from [`Node`]s and measuring their content.

#![expect(clippy::cast_precision_loss)]

use super::walk::TreeResult;
use super::{Draw, Node, ProgressMode, SkeletonData, Span};
use crate::components::modal::ModalInfo;
use crate::components::notification::{NotificationData, measure_notification};
use crate::components::progress_bar::ProgressBarData;
use crate::components::skeleton;
use crate::components::switcher::{SwitcherData, switcher_size};
use crate::components::tag::{TAG_PAD_VERT, TagData, resolve_icon, tag_content_padding};
use crate::components::{ButtonSize, ButtonStyle};
use crate::renderer::Renderer;
use anyhow::Result;
use bmc_ui::ButtonSkin;
use bmc_ui::{BitmapId, Color, SvgId};
use bmc_ui::{CrossAlign, Justify, PropsData, TextOverflow, TextStyle};
use taffy::prelude::*;
use taffy::{Overflow, Point};

/// Paragraph data for measurement and rendering
#[derive(Clone, Debug)]
pub(super) struct ParagraphData {
    pub(super) base_style: TextStyle,
    pub(super) spans: Vec<Span>,
}

/// Button data stored in taffy node context.
#[derive(Clone, Debug)]
pub(crate) struct ButtonContext {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) style: ButtonStyle,
    pub(super) size: ButtonSize,
    pub(super) icon_id: Option<SvgId>,
    pub(super) disabled: bool,
    pub(super) skin: Option<ButtonSkin>,
}

/// Nine-patch background image data. `bitmap_id == None` means no background.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BgNinePatch {
    pub(super) bitmap_id: Option<BitmapId>,
    pub(super) left: u16,
    pub(super) top: u16,
    pub(super) right: u16,
    pub(super) bottom: u16,
}

/// Node data attached to taffy nodes
#[derive(Clone, Default, Debug)]
pub struct NodeContext {
    /// Whether this **subtree** can change while the guest is idle — drives
    /// whether a dynamic-only walk may skip it wholesale. See
    /// [`crate::partition`].
    pub(crate) dynamic: bool,
    /// Whether this node's **own paint** can change. Deliberately not the same
    /// question: a container holding an animated child is a dynamic *subtree*
    /// but paints a static background, which must come from the cached layer
    /// rather than be repainted over it.
    pub(crate) self_dynamic: bool,
    /// The [`Node::Dimmed`] factor this node's subtree draws at,
    /// relative to its parent. `None` rather than `1.0`,
    /// because the derived `Default` would otherwise dim to black.
    pub(super) dim: Option<f32>,
    pub(super) background: Color,
    /// CSS-modeled box decoration: radius rounds background
    /// and border alike, a zero width paints no border.
    pub(super) border_radius: f32,
    pub(super) border_width: f32,
    pub(super) border_color: Color,
    pub(super) bg_nine_patch: BgNinePatch,
    pub(super) paragraph: Option<ParagraphData>,
    pub(super) button: Option<ButtonContext>,
    pub(super) draws: Vec<Draw>, // canvas draw commands
    /// Touch interaction key for interactive canvases (None = decorative)
    pub(super) touch_key: Option<String>,
    pub(super) notification: Option<NotificationData>,
    /// String key for scroll container state tracking and interaction targeting.
    pub(super) scroll_key: Option<String>,
    pub(super) progress_bar: Option<ProgressBarData>,
    pub(super) tag: Option<TagData>,
    pub(super) switcher: Option<SwitcherData>,
    pub(super) skeleton: Option<SkeletonData>,
}

/// Build the taffy node for `node` and tag it with its static/dynamic
/// classification.
///
/// The classification rides on [`NodeContext`] because the render walk sees only
/// taffy nodes, and this is the last point where both are in scope. Recursion
/// goes through this wrapper so every node owning a context gets tagged; one
/// built without a context stays untagged and is treated as dynamic.
pub(crate) fn build_taffy_node(
    taffy: &mut TaffyTree<NodeContext>,
    node: &Node,
    now_unix_secs: i64,
    result: &mut TreeResult,
    modals: &mut Vec<ModalInfo>,
) -> Result<taffy::NodeId> {
    let id = build_taffy_node_inner(taffy, node, now_unix_secs, result, modals)?;
    // O(nodes x depth) boolean work, negligible next to the Taffy build it
    // wraps — not worth a bottom-up accumulator threaded through every arm.
    let dynamic = crate::partition::node_is_dynamic(node);
    let self_dynamic = crate::partition::node_self_is_dynamic(node);
    if let Some(ctx) = taffy.get_node_context_mut(id) {
        ctx.dynamic = dynamic;
        ctx.self_dynamic = self_dynamic;
    }
    Ok(id)
}

#[expect(clippy::too_many_lines)]
fn build_taffy_node_inner(
    taffy: &mut TaffyTree<NodeContext>,
    node: &Node,
    now_unix_secs: i64,
    result: &mut TreeResult,
    modals: &mut Vec<ModalInfo>,
) -> Result<taffy::NodeId> {
    match node {
        Node::Column(props, children)
        | Node::Row(props, children)
        | Node::Center(props, children) => {
            let child_ids: Vec<_> = children
                .iter()
                .map(|c| build_taffy_node(taffy, c, now_unix_secs, result, modals))
                .collect::<Result<_>>()?;

            let is_center = matches!(node, Node::Center(_, _));
            let flex_dir = if matches!(node, Node::Row(_, _)) {
                FlexDirection::Row
            } else {
                FlexDirection::Column
            };

            let is_abs = props.is_absolute();
            let style = Style {
                position: if is_abs {
                    taffy::Position::Absolute
                } else {
                    taffy::Position::Relative
                },
                inset: if is_abs {
                    inset_from_props(props)
                } else {
                    taffy::Rect::auto()
                },
                flex_direction: flex_dir,
                flex_wrap: if props.wrap {
                    FlexWrap::Wrap
                } else {
                    FlexWrap::NoWrap
                },
                justify_content: if is_center || props.wrap {
                    Some(JustifyContent::Center)
                } else {
                    match props.justify_content {
                        Justify::Start => None,
                        Justify::Center => Some(JustifyContent::Center),
                        Justify::End => Some(JustifyContent::End),
                        Justify::SpaceBetween => Some(JustifyContent::SpaceBetween),
                    }
                },
                align_items: match props.cross_align {
                    CrossAlign::Center => Some(AlignItems::Center),
                    CrossAlign::Start => Some(AlignItems::Start),
                    CrossAlign::End => Some(AlignItems::End),
                    CrossAlign::Stretch => {
                        if is_center {
                            Some(AlignItems::Center)
                        } else {
                            None
                        }
                    }
                },
                align_content: if props.wrap {
                    Some(AlignContent::Center)
                } else {
                    None
                },
                gap: Size {
                    width: length(props.gap),
                    height: length(props.gap),
                },
                padding: padding_uniform(props.padding),
                margin: margin_uniform(props.margin),
                size: size_from_props(props),
                max_size: max_size_from_props(props),
                flex_grow: if is_center && props.flex == 0.0 {
                    1.0
                } else {
                    props.flex
                },
                // flex > 0 acts like CSS `flex: N` (grow N, basis 0) so the item
                // starts at zero and grows into available space.
                flex_basis: if props.flex > 0.0 || (is_center && props.flex == 0.0) {
                    length(0.0_f32)
                } else {
                    Dimension::auto()
                },
                ..Default::default()
            };

            let id = taffy.new_with_children(style, &child_ids)?;
            // Attached even when nothing paints: the context is the only place
            // the static/dynamic tag can live. Pixel-neutral, because the walk
            // guards the background on `!= Color::default()` and every other
            // field on `Some`, so an otherwise-empty context draws nothing.
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    dynamic: false,
                    self_dynamic: false,
                    background: props.background,
                    border_radius: props.border_radius,
                    border_width: props.border_width,
                    border_color: props.border_color,
                    bg_nine_patch: bg_np_from_props(props),
                    ..Default::default()
                }),
            )?;
            Ok(id)
        }

        Node::Paragraph {
            props,
            base_style,
            spans,
        } => {
            let is_abs = props.is_absolute();
            // Don't pre-measure - use Taffy's measure callback with available width
            let style = Style {
                position: if is_abs {
                    taffy::Position::Absolute
                } else {
                    taffy::Position::Relative
                },
                inset: if is_abs {
                    inset_from_props(props)
                } else {
                    taffy::Rect::auto()
                },
                padding: padding_uniform(props.padding),
                margin: margin_uniform(props.margin),
                // The cap bounds the text, and taffy's max size the border box,
                // so a stretching parent can't lay the text out any wider.
                max_size: Size {
                    width: paragraph_width_cap(base_style)
                        .map_or(Dimension::auto(), |cap| length(cap + 2.0 * props.padding)),
                    height: Dimension::auto(),
                },
                flex_grow: props.flex,
                // Let measure function determine size based on available width
                ..Default::default()
            };

            let id = taffy.new_leaf(style)?;
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    dynamic: false,
                    self_dynamic: false,
                    background: props.background,
                    bg_nine_patch: bg_np_from_props(props),
                    paragraph: Some(ParagraphData {
                        base_style: *base_style,
                        spans: spans.clone(),
                    }),
                    ..Default::default()
                }),
            )?;
            Ok(id)
        }

        Node::RelTime {
            anchor,
            format,
            clamp,
            style,
        } => {
            let delta = now_unix_secs - *anchor;
            let text = crate::components::format_rel(delta, *format, *clamp);
            // A pinned label yields no delay, so it never schedules a wake.
            if let Some(delay) = crate::components::next_change_delay_ms(delta, *format, *clamp) {
                result.next_frame_delay_ms =
                    Some(result.next_frame_delay_ms.map_or(delay, |d| d.min(delay)));
            }
            let id = taffy.new_leaf(Style::default())?;
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    dynamic: false,
                    self_dynamic: false,
                    paragraph: Some(ParagraphData {
                        base_style: *style,
                        spans: vec![Span::plain(text)],
                    }),
                    ..Default::default()
                }),
            )?;
            Ok(id)
        }

        Node::Tag {
            kind,
            icon,
            content,
        } => {
            let content_id = build_taffy_node(taffy, content, now_unix_secs, result, modals)?;
            let icon = resolve_icon(*kind, *icon);
            let (pad_left, pad_right) = tag_content_padding(icon.is_some());
            let style = Style {
                flex_direction: FlexDirection::Row,
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                // Size to content, don't stretch in a column parent.
                align_self: Some(AlignSelf::FlexStart),
                flex_shrink: 0.0,
                // Pill height = content + vertical padding; corners clamp to a stadium.
                padding: taffy::Rect {
                    left: length(pad_left),
                    right: length(pad_right),
                    top: length(TAG_PAD_VERT),
                    bottom: length(TAG_PAD_VERT),
                },
                ..Default::default()
            };
            let id = taffy.new_with_children(style, &[content_id])?;
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    dynamic: false,
                    self_dynamic: false,
                    tag: Some(TagData { kind: *kind, icon }),
                    ..Default::default()
                }),
            )?;
            Ok(id)
        }

        Node::Button {
            id: btn_id,
            label,
            style: btn_style,
            size: btn_size,
            icon_id,
            disabled,
            stretch,
            skin,
        } => {
            let height = btn_size.height();

            let style = Style {
                size: Size {
                    width: Dimension::auto(),
                    height: length(height),
                },
                // Buttons should never shrink below their content size
                flex_shrink: 0.0,
                // Stretch fills the cross axis (full-width in a column); else content.
                align_self: Some(if *stretch {
                    AlignSelf::Stretch
                } else {
                    AlignSelf::FlexStart
                }),
                ..Default::default()
            };
            let id = taffy.new_leaf(style)?;
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    dynamic: false,
                    self_dynamic: false,
                    button: Some(ButtonContext {
                        id: btn_id.clone(),
                        label: label.clone(),
                        style: *btn_style,
                        size: *btn_size,
                        icon_id: *icon_id,
                        disabled: *disabled,
                        skin: *skin,
                    }),
                    ..Default::default()
                }),
            )?;
            Ok(id)
        }

        Node::Spacer { flex } => {
            let style = Style {
                flex_grow: *flex,
                ..Default::default()
            };
            Ok(taffy.new_leaf(style)?)
        }

        Node::Canvas {
            props,
            touch_key,
            draws,
        } => {
            let has_explicit_size = props.width > 0.0 || props.height > 0.0;
            let is_abs = props.is_absolute();
            let style = Style {
                position: if is_abs {
                    taffy::Position::Absolute
                } else {
                    taffy::Position::Relative
                },
                inset: if is_abs {
                    inset_from_props(props)
                } else {
                    taffy::Rect::auto()
                },
                size: size_from_props(props),
                max_size: max_size_from_props(props),
                flex_grow: props.flex,
                // Prevent taffy from shrinking canvases with explicit sizes
                flex_shrink: if has_explicit_size { 0.0 } else { 1.0 },
                padding: padding_uniform(props.padding),
                margin: margin_uniform(props.margin),
                ..Default::default()
            };
            let id = taffy.new_leaf(style)?;
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    dynamic: false,
                    self_dynamic: false,
                    background: props.background,
                    bg_nine_patch: bg_np_from_props(props),
                    draws: draws.clone(),
                    touch_key: touch_key.clone(),
                    ..Default::default()
                }),
            )?;
            Ok(id)
        }

        Node::Scroll {
            scroll_key,
            props,
            children,
        } => {
            let child_ids: Vec<_> = children
                .iter()
                .map(|c| build_taffy_node(taffy, c, now_unix_secs, result, modals))
                .collect::<Result<_>>()?;

            // Extra right padding so content doesn't sit under the scrollbar
            // overlay. CDS spacing scale is 4/8/16/24 — 8 px is the smallest
            // step that keeps content off the scrollbar visual without
            // looking padded twice on top of `props.padding`.
            let scrollbar_clearance = 8.0;
            let mut padding = padding_uniform(props.padding);
            padding.right = length(props.padding + scrollbar_clearance);

            let style = Style {
                flex_direction: FlexDirection::Column,
                gap: Size {
                    width: length(props.gap),
                    height: length(props.gap),
                },
                padding,
                margin: margin_uniform(props.margin),
                size: size_from_props(props),
                max_size: max_size_from_props(props),
                flex_grow: props.flex,
                overflow: Point {
                    x: Overflow::Hidden,
                    y: Overflow::Scroll,
                },
                ..Default::default()
            };

            let id = taffy.new_with_children(style, &child_ids)?;
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    dynamic: false,
                    self_dynamic: false,
                    background: props.background,
                    bg_nine_patch: bg_np_from_props(props),
                    scroll_key: Some(scroll_key.clone()),
                    ..Default::default()
                }),
            )?;
            Ok(id)
        }

        Node::Notification {
            kind,
            title,
            subtitle,
        } => {
            let style = Style::default();
            let id = taffy.new_leaf(style)?;
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    dynamic: false,
                    self_dynamic: false,
                    notification: Some(NotificationData {
                        kind: *kind,
                        title: title.clone(),
                        subtitle: subtitle.clone(),
                    }),
                    ..Default::default()
                }),
            )?;
            Ok(id)
        }

        Node::Switcher {
            active,
            disabled,
            tabs,
        } => {
            let data = SwitcherData {
                active: *active,
                disabled: *disabled,
                tabs: tabs.clone(),
            };
            let (sw, sh) = switcher_size(&data);
            let style = Style {
                size: Size {
                    width: length(sw),
                    height: length(sh),
                },
                ..Default::default()
            };
            let id = taffy.new_leaf(style)?;
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    switcher: Some(data),
                    ..Default::default()
                }),
            )?;
            Ok(id)
        }

        Node::Skeleton(data) => {
            let (width, height) = skeleton::layout_size(data);
            let style = Style {
                size: Size {
                    width: width.map_or_else(auto, length),
                    height: length(height),
                },
                // A `Fill` skeleton spans its row by growing: a childless
                // leaf has no content to size an `auto` width from, and a
                // percentage resolves to zero while the row's own width is
                // still indefinite.
                flex_grow: if width.is_none() { 1.0 } else { 0.0 },
                ..Default::default()
            };
            let id = taffy.new_leaf(style)?;
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    skeleton: Some(*data),
                    ..Default::default()
                }),
            )?;
            Ok(id)
        }

        Node::Dimmed { brightness, child } => {
            debug_assert!(
                (0.0..=1.0).contains(brightness),
                "BUG: Node::Dimmed only dims, got {brightness}"
            );
            let id = build_taffy_node(taffy, child, now_unix_secs, result, modals)?;
            if let Some(ctx) = taffy.get_node_context_mut(id) {
                ctx.dim = Some(ctx.dim.map_or(*brightness, |inner| inner * brightness));
            } else {
                taffy.set_node_context(
                    id,
                    Some(NodeContext {
                        dim: Some(*brightness),
                        ..Default::default()
                    }),
                )?;
            }
            Ok(id)
        }

        Node::Modal {
            modal_id,
            is_open,
            padding,
            backdrop_alpha,
            title,
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
            ..
        } => {
            // Store modal for overlay rendering
            modals.push(ModalInfo {
                modal_id: modal_id.clone(),
                is_open: *is_open,
                padding: *padding,
                backdrop_alpha: *backdrop_alpha,
                title: title.clone(),
                bg_color: *bg_color,
                header_color: *header_color,
                title_color: *title_color,
                max_width: *max_width,
                body: body.clone(),
                footer_primary_key: footer_primary_key.clone(),
                footer_primary_label: footer_primary_label.clone(),
                footer_secondary_key: footer_secondary_key.clone(),
                footer_secondary_label: footer_secondary_label.clone(),
                footer_danger: *footer_danger,
            });

            // Modal doesn't participate in normal layout — hidden from flex/gap.
            let style = Style {
                display: Display::None,
                ..Default::default()
            };
            Ok(taffy.new_leaf(style)?)
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
            // When skinned, use the skin's track height for layout
            let effective_h = skin.as_ref().map_or(*track_h, |s| f32::from(s.track_h));
            let dot_radius = effective_h * 2.0;
            let bar_height = if skin.is_some() {
                // Skinned: thumb may be taller than track
                let thumb_h = skin.as_ref().map_or(0.0, |s| f32::from(s.thumb_h));
                effective_h.max(thumb_h)
            } else if matches!(mode, ProgressMode::Meter(_)) {
                // No drag thumb to budget for — the meter is just its track.
                effective_h
            } else {
                dot_radius * 2.0 + effective_h
            };
            let style = Style {
                size: Size {
                    width: Dimension::auto(),
                    height: length(bar_height),
                },
                flex_grow: 1.0,
                ..Default::default()
            };
            let id = taffy.new_leaf(style)?;
            taffy.set_node_context(
                id,
                Some(NodeContext {
                    dynamic: false,
                    self_dynamic: false,
                    touch_key: touch_key.clone(),
                    progress_bar: Some(ProgressBarData {
                        track_h: *track_h,
                        mode: *mode,
                        active: *active,
                        fill_color: *fill_color,
                        track_color: *track_color,
                        bg_color: *bg_color,
                        skin: *skin,
                    }),
                    ..Default::default()
                }),
            )?;
            Ok(id)
        }
    }
}

/// Whitespace a line may break at. Non-breaking spaces glue their
/// neighbors into one word, exactly as the wrapper treats them.
fn is_breaking_whitespace(c: char) -> bool {
    c.is_whitespace() && !matches!(c, '\u{A0}' | '\u{2007}' | '\u{202F}')
}

/// Split spans into words for min-content probing. A word ends only at
/// breaking whitespace — it may cross span boundaries, so each word is
/// a list of styled fragments preserving its parts' span styles.
pub(super) fn split_min_content_words(spans: &[Span]) -> Vec<Vec<Span>> {
    let mut words: Vec<Vec<Span>> = Vec::new();
    let mut word: Vec<Span> = Vec::new();
    for span in spans {
        for (i, fragment) in span.text.split(is_breaking_whitespace).enumerate() {
            if i > 0 && !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            if !fragment.is_empty() {
                word.push(Span {
                    text: fragment.to_owned(),
                    ..span.clone()
                });
            }
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// Min-content width of a paragraph: the widest single word measured
/// unwrapped. Wrapping at this width breaks at word boundaries only,
/// so both axes of the resulting measurement match CSS min-content.
pub(crate) fn min_content_paragraph_width(
    renderer: &mut dyn Renderer,
    base_style: &TextStyle,
    spans: &[Span],
) -> f32 {
    let mut min_width = 0.0_f32;
    for word in split_min_content_words(spans) {
        let (w, _) = renderer.measure_paragraph(base_style, &word, None);
        min_width = min_width.max(w);
    }
    min_width
}

/// The width a paragraph's own style caps it at, whatever box it lands in.
fn paragraph_width_cap(style: &TextStyle) -> Option<f32> {
    (style.max_width > 0).then_some(style.max_width as f32)
}

/// Compute taffy layout with the standard measure function for paragraphs,
/// notifications, and buttons.
pub(crate) fn compute_taffy_layout(
    taffy: &mut TaffyTree<NodeContext>,
    root_id: taffy::NodeId,
    renderer: &mut dyn Renderer,
) -> Result<()> {
    taffy.compute_layout_with_measure(
        root_id,
        Size::MAX_CONTENT,
        |known_dimensions, available_space, _node_id, node_context, _style| {
            if let (Some(w), Some(h)) = (known_dimensions.width, known_dimensions.height) {
                return Size {
                    width: w,
                    height: h,
                };
            }

            if let Some(ctx) = node_context {
                if let Some(ref para) = ctx.paragraph {
                    let style = &para.base_style;
                    // A min-content probe wraps at the widest single word.
                    //
                    // Probing at max_width 0 instead breaks per glyph into an absurdly tall tower
                    // whose height becomes the min-size floor of every ancestor, freezing flex containers
                    // at bogus minimums; probing unwrapped inflates the min width to the whole line,
                    // so shrinkable panels blow out their row instead of wrapping.
                    let available_width = known_dimensions.width.or(match available_space.width {
                        AvailableSpace::Definite(w) => Some(w),
                        AvailableSpace::MinContent if style.text_overflow == TextOverflow::Wrap => {
                            Some(min_content_paragraph_width(renderer, style, &para.spans))
                        }
                        // A line that is cut rather than wrapped can give up
                        // its whole width, which is what lets it shrink in a row.
                        AvailableSpace::MinContent => Some(0.0),
                        AvailableSpace::MaxContent => None,
                    });
                    let max_width = match (available_width, paragraph_width_cap(style)) {
                        (Some(w), Some(cap)) => Some(w.min(cap)),
                        (w, cap) => w.or(cap),
                    };
                    let (w, h) = match style.text_overflow {
                        TextOverflow::Wrap | TextOverflow::Ellipsis => {
                            renderer.measure_paragraph(style, &para.spans, max_width)
                        }
                        // The width only aligns an unwrapped line,
                        // so one unbounded shaping serves every box.
                        TextOverflow::Clip => {
                            let (natural, h) = renderer.measure_paragraph(style, &para.spans, None);
                            (max_width.map_or(natural, |b| natural.min(b)), h)
                        }
                    };
                    return Size {
                        width: known_dimensions.width.unwrap_or(w),
                        height: known_dimensions.height.unwrap_or(h),
                    };
                }

                if let Some(ref notif) = ctx.notification {
                    return measure_notification(
                        notif,
                        known_dimensions,
                        available_space,
                        renderer,
                    );
                }

                if let Some(ref btn) = ctx.button {
                    let sz = btn.size;
                    let h = sz.height();
                    let w = if btn.icon_id.is_some() && btn.label.is_empty() {
                        h
                    } else if btn.icon_id.is_some() {
                        let text_w = renderer.measure_text(&btn.label, sz.font_size());
                        sz.h_padding()
                            + sz.icon_size()
                            + sz.icon_text_gap()
                            + text_w
                            + sz.h_padding()
                    } else {
                        let text_w = renderer.measure_text(&btn.label, sz.font_size());
                        text_w + sz.h_padding() * 2.0
                    };
                    return Size {
                        width: known_dimensions.width.unwrap_or(w),
                        height: known_dimensions.height.unwrap_or(h),
                    };
                }
            }

            Size::ZERO
        },
    )?;
    Ok(())
}

fn padding_uniform(v: f32) -> taffy::Rect<LengthPercentage> {
    taffy::Rect {
        top: length(v),
        right: length(v),
        bottom: length(v),
        left: length(v),
    }
}

fn margin_uniform(v: f32) -> taffy::Rect<LengthPercentageAuto> {
    taffy::Rect {
        top: LengthPercentageAuto::length(v),
        right: LengthPercentageAuto::length(v),
        bottom: LengthPercentageAuto::length(v),
        left: LengthPercentageAuto::length(v),
    }
}

fn size_from_props(props: &PropsData) -> Size<Dimension> {
    Size {
        width: if props.width == 0.0 {
            Dimension::auto()
        } else {
            length(props.width)
        },
        height: if props.height == 0.0 {
            Dimension::auto()
        } else {
            length(props.height)
        },
    }
}

fn max_size_from_props(props: &PropsData) -> Size<Dimension> {
    Size {
        width: if props.max_width == 0.0 {
            Dimension::auto()
        } else {
            length(props.max_width)
        },
        height: if props.max_height == 0.0 {
            Dimension::auto()
        } else {
            length(props.max_height)
        },
    }
}

/// Convert `PropsData` inset fields to Taffy `position` and `inset`.
/// Finite values become `LengthPercentageAuto::Length`, NAN becomes `Auto`.
fn inset_from_props(props: &PropsData) -> taffy::Rect<LengthPercentageAuto> {
    fn inset_val(v: f32) -> LengthPercentageAuto {
        if v.is_finite() {
            LengthPercentageAuto::length(v)
        } else {
            LengthPercentageAuto::auto()
        }
    }
    taffy::Rect {
        top: inset_val(props.inset_top),
        right: inset_val(props.inset_right),
        bottom: inset_val(props.inset_bottom),
        left: inset_val(props.inset_left),
    }
}

fn bg_np_from_props(props: &PropsData) -> BgNinePatch {
    BgNinePatch {
        bitmap_id: props.bg_np_id,
        left: props.bg_np_left,
        top: props.bg_np_top,
        right: props.bg_np_right,
        bottom: props.bg_np_bottom,
    }
}
