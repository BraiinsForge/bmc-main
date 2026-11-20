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

//! The card every tile is drawn on, the pieces of its heading, and the centred layout made of them.

use bmc_wasm_sdk::typography::unbroken;
use bmc_wasm_sdk::*;

use crate::interaction::gesture::{Availability, Touch};
use crate::view::blocks::badge::{Badge, Disc, Feedback, Mark};
use crate::view::style::{Accent, CONTROL_HEIGHT, Icon, LABEL_COLOR, ROW_GAP, fill_insets};
use crate::view::touches::{Touches, middle};

/// The whole card, under its blocks: the host gives a finger to the smallest target under it,
/// so the card has only the touches on its heading and between its blocks.
/// Block keys must not start with `@`.
const CARD: &str = "@card";

/// Between the card's edges and what it holds.
pub const PADDING: f32 = 24.0;
/// Braiins Sans Bold's capitals, as a share of its size: the `capHeight` of its `OS/2` table.
const BOLD_CAP_HEIGHT: f32 = 0.698;
const STATE_COLOR: Color = GRAY_40;
/// Where a block looks centred: this share of the card's height above its middle.
const OPTICAL_LIFT: f32 = 0.03;
/// The frontend's `--state-inactive-color`.
const INACTIVE_COLOR: Color = Color::from_hex(0x9E_9E_9E);
/// The frontend's tile tints a pressed card at 12 % of the state colour.
const PRESSED_TINT: f32 = 0.12;
/// "5 minutes ago", as the frontend words a time on its tile.
const SINCE_FORMAT: RelTimeFormat = RelTimeFormat {
    length: RelTimeLength::Long,
    segments: RelTimeSegments::Single,
};

/// What the card says of the entity's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateDisplay {
    /// The line under the name, such as when a button was last pressed.
    pub line: Option<Line>,
    /// The colour the state wears, such as amber for a running script; none while it is inactive.
    pub color: Option<Accent>,
    /// The frontend's icon for the domain in this state, such as `mdi:palette` for a scene.
    pub icon: Icon,
    pub disc: Disc,
    pub mark: Option<Mark>,
}

/// The sizes the name and the lines under it are set in, and the rows they take.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextSizes {
    title: u32,
    pub title_height: f32,
    line: u32,
    pub line_height: f32,
}

impl TextSizes {
    /// The heading on a card one row high.
    pub const REGULAR: Self = Self {
        title: 28,
        title_height: 36.0,
        line: 22,
        line_height: 28.0,
    };
    /// Half as large again, for a layout with room to spare.
    pub const LARGE: Self = Self {
        title: 42,
        title_height: 54.0,
        line: 33,
        line_height: 42.0,
    };

    /// How far under the top of its row the name's capitals start, centred in the row as they are.
    #[must_use]
    #[expect(clippy::cast_precision_loss, reason = "a font size, exact in f32")]
    pub fn title_cap_inset(self) -> f32 {
        (self.title_height - BOLD_CAP_HEIGHT * self.title as f32) / 2.0
    }
}

/// How the centred layout is set on a card of one height.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Centred {
    /// The circle's diameter.
    icon: f32,
    text: TextSizes,
    /// Between the hold ring and the capitals of the name under it.
    ring_clearance: f32,
    hint: HintRow,
}

impl Centred {
    /// On a card one row high: the icon leaves room for the hold ring under the top edge,
    /// and one line under the name.
    const SHORT: Self = Self {
        icon: 80.0,
        text: TextSizes::REGULAR,
        ring_clearance: 20.0,
        hint: HintRow::AfterLine,
    };
    /// On a card two rows high, its icon, text and clearance scaled up together.
    const TALL: Self = Self {
        icon: 128.0,
        text: TextSizes::LARGE,
        ring_clearance: 30.0,
        hint: HintRow::UnderLine,
    };

    fn of(size: SizeVariant) -> Self {
        match size {
            SizeVariant::Small | SizeVariant::Medium => Self::SHORT,
            SizeVariant::Large | SizeVariant::Full => Self::TALL,
        }
    }
}

/// Where the centred layout shows the hint, where only a hold fires the card action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HintRow {
    /// A card one row high has room for one line under the name, which the hint shares with the state.
    AfterLine,
    UnderLine,
}

/// Between the heading and the rows of blocks under it, picked by eye in the gallery.
const HEADING_GAP: f32 = 16.0;
/// The share of a wide card the centred block takes, left of the rows, picked by eye in the gallery.
const BESIDE_HEADING_SHARE: f32 = 0.42;

/// The room a card leaves for rows of blocks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rows {
    pub count: usize,
    pub width: f32,
}

/// Where a card puts rows of blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arrangement {
    /// None: a card one row high and narrow has no room for them.
    Alone,
    /// Right of the centred block, on a wide card.
    Beside,
    /// Under the icon, the name and the line, on a tall card.
    Under,
}

/// How many rows of blocks a card `width` by `height` has room for, and how wide they are.
#[must_use]
pub fn rows((width, height): (f32, f32), size: SizeVariant) -> Rows {
    Shape {
        width,
        height,
        size,
    }
    .rows()
}

/// A card's measures, which pick where it puts its blocks.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Shape {
    width: f32,
    height: f32,
    size: SizeVariant,
}

impl Shape {
    fn rows(self) -> Rows {
        let body = self.height - 2.0 * PADDING;
        let (room, width) = match self.arrangement() {
            Arrangement::Alone => (0.0, 0.0),
            Arrangement::Beside => (body, self.beside_width()),
            Arrangement::Under => (
                body - heading_height(Centred::SHORT) - HEADING_GAP,
                body_width(self.width),
            ),
        };
        Rows {
            count: rows_fitting(room),
            width,
        }
    }

    fn arrangement(self) -> Arrangement {
        let short = matches!(self.size, SizeVariant::Small | SizeVariant::Medium);
        let wide = self.width > 2.0 * self.height;
        match (short, wide) {
            (_, true) => Arrangement::Beside,
            (true, false) => Arrangement::Alone,
            (false, false) => Arrangement::Under,
        }
    }

    /// The rows' column beside the centred block, to the card's right padding.
    fn beside_width(self) -> f32 {
        (self.width * (1.0 - BESIDE_HEADING_SHARE) - PADDING).max(0.0)
    }
}

/// The card's own centred block where rows `rows_height` high fit under it in `body`, else the short card's:
/// a larger icon reads better, and the short one lines up with nothing either way.
fn heading_over(size: SizeVariant, body: f32, rows_height: f32) -> Centred {
    let own = Centred::of(size);
    if heading_height(own) + HEADING_GAP + rows_height <= body {
        own
    } else {
        Centred::SHORT
    }
}

/// The icon over the name and the line, set by `centred`, from the top of the badge to the foot of the line.
fn heading_height(centred: Centred) -> f32 {
    let Centred {
        icon,
        text,
        ring_clearance,
        ..
    } = centred;
    Badge::side(icon) + ring_clearance - text.title_cap_inset()
        + text.title_height
        + text.line_height
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a handful of rows, clamped at zero"
)]
fn rows_fitting(height: f32) -> usize {
    ((height + ROW_GAP) / (CONTROL_HEIGHT + ROW_GAP))
        .floor()
        .max(0.0) as usize
}

/// What the line under the name tells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    State(String),
    /// How long ago a time was, such as "5 minutes ago": the host words it in the device language
    /// and keeps it current.
    Since(SystemTime),
    /// Something for the user: what became of what they did, or what to fix, such as "No action set".
    Notice(String),
}

/// One frame's card, drawn by [`Self::centred`].
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub(crate) name: String,
    pub(crate) line: Option<Line>,
    /// The state's colour, or a problem's warning; none while the state is inactive or the tile cannot act.
    pub(crate) color: Option<Accent>,
    pub(crate) icon: Icon,
    /// Whether a touch on the card fires its action now; none where the entity has none.
    pub(crate) action: Option<Availability>,
    /// How far a hold of the card action has got, from 0 to 1, as a ring around the icon.
    pub(crate) hold: Option<f32>,
    /// What to tell where only a hold fires the card action, such as "Hold 2 s".
    pub(crate) hint: Option<String>,
    pub(crate) disc: Disc,
    pub(crate) mark: Option<Mark>,
    /// The whole tile's width, height and size, which pick the layout's variant and place it.
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) size: SizeVariant,
}

impl Card {
    /// The icon over the name and its line, all centred, the circle to the line at the optical centre.
    /// A hint on a row of its own hangs under the line,
    /// so a tile with a hint holds its icon and name level with one without.
    #[must_use]
    pub fn centred(&self, touches: &Touches) -> Node {
        self.surface(
            props!(cross_align: CrossAlign::Center),
            self.centred_block(self.width, touches),
            touches,
        )
    }

    /// The centred layout's block in a column `width` wide and as high as the card, with the space above it.
    fn centred_block(&self, width: f32, touches: &Touches) -> Vec<Node> {
        let centred = Centred::of(self.size);
        let Centred {
            icon,
            text,
            ring_clearance,
            hint,
        } = centred;
        let width = body_width(width);
        let align = TextAlign::Center;
        let mut texts = vec![self.title(text, width, align)];
        match hint {
            HintRow::AfterLine => texts.extend(self.line_and_hint(text, width, align)),
            HintRow::UnderLine => {
                texts.extend(self.line(text, width, align));
                texts.extend(self.hint(text, width, align));
            }
        }
        // The badge keeps room around its circle for the hold ring.
        let ring_room = (Badge::side(icon) - icon) / 2.0;
        let cap_inset = text.title_cap_inset();
        // From the top of the circle to the foot of the line.
        let seen =
            icon + ring_room + ring_clearance + text.title_height + text.line_height - cap_inset;
        let above_circle = (self.height - seen) / 2.0 - OPTICAL_LIFT * self.height;
        vec![
            col(
                props!(height: (above_circle - ring_room).max(0.0)),
                Vec::<Node>::new(),
            ),
            self.icon_over(centred, texts, touches),
        ]
    }

    /// The centred block with the first of `rows` beside or under it, as many as [`rows`] has room for,
    /// each one control high; the centred layout where none fit.
    #[must_use]
    pub fn with_rows(&self, mut rows: Vec<Node>, touches: &Touches) -> Node {
        let shape = self.shape();
        rows.truncate(shape.rows().count);
        let none = rows.is_empty();
        #[expect(clippy::cast_precision_loss, reason = "a handful of rows")]
        let rows_height = rows.len() as f32 * (CONTROL_HEIGHT + ROW_GAP) - ROW_GAP;
        let rows = col(props!(gap: ROW_GAP), rows);
        match (shape.arrangement(), none) {
            (Arrangement::Alone, _) | (Arrangement::Beside | Arrangement::Under, true) => {
                self.centred(touches)
            }
            (Arrangement::Beside, false) => {
                let left = self.width - PADDING - shape.beside_width();
                self.surface(
                    props!(),
                    vec![row(
                        props!(width: self.width, height: self.height),
                        [
                            col(
                                props!(width: left, cross_align: CrossAlign::Center),
                                self.centred_block(left, touches),
                            ),
                            col(
                                props!(
                                    width: shape.beside_width(),
                                    justify_content: Justify::Center
                                ),
                                [rows],
                            ),
                        ],
                    )],
                    touches,
                )
            }
            (Arrangement::Under, false) => {
                let centred = heading_over(self.size, self.height - 2.0 * PADDING, rows_height);
                let content = heading_height(centred) + HEADING_GAP + rows_height;
                let top = ((self.height - content) / 2.0).max(PADDING);
                self.surface(
                    props!(cross_align: CrossAlign::Center),
                    vec![
                        col(props!(height: top), Vec::<Node>::new()),
                        col(
                            props!(width: body_width(self.width), gap: HEADING_GAP),
                            [self.heading(centred, touches), rows],
                        ),
                    ],
                    touches,
                )
            }
        }
    }

    fn shape(&self) -> Shape {
        Shape {
            width: self.width,
            height: self.height,
            size: self.size,
        }
    }

    /// A centred block set by `centred` over rows of blocks, the hint after its line.
    fn heading(&self, centred: Centred, touches: &Touches) -> Node {
        let text = centred.text;
        let width = body_width(self.width);
        let align = TextAlign::Center;
        let mut lines = vec![self.title(text, width, align)];
        lines.extend(self.line_and_hint(text, width, align));
        self.icon_over(centred, lines, touches)
    }

    /// The icon set by `centred` over `texts`, its gap measured to the name's capitals.
    fn icon_over(&self, centred: Centred, texts: Vec<Node>, touches: &Touches) -> Node {
        col(
            props!(
                gap: centred.ring_clearance - centred.text.title_cap_inset(),
                cross_align: CrossAlign::Center
            ),
            [self.icon(centred.icon, touches), col(props!(), texts)],
        )
    }

    /// `children` laid out by `props` on the black card, over its touch target and under its tint.
    fn surface(&self, props: PropsData, children: Vec<Node>, touches: &Touches) -> Node {
        let mut layers = vec![touchable(CARD, fill_insets(), Vec::<Draw>::new())];
        layers.extend(children);
        if self.feedback(touches) == Feedback::Pressed {
            layers.push(col(
                PropsData {
                    background: self.color().with_alpha(PRESSED_TINT),
                    ..fill_insets()
                },
                Vec::<Node>::new(),
            ));
        }
        col(
            PropsData {
                background: BLACK,
                ..props
            },
            layers,
        )
    }

    /// The name, a title row high.
    fn title(&self, sizes: TextSizes, width: f32, align: TextAlign) -> Node {
        one_line(
            &self.name,
            (width, sizes.title_height),
            sizes.title,
            FontWeight::BOLD,
            LABEL_COLOR,
            align,
        )
    }

    /// The line under the name, a line row high; none where there is none.
    fn line(&self, sizes: TextSizes, width: f32, align: TextAlign) -> Option<Node> {
        Some(in_row(
            self.line_text(sizes, width, align)?,
            (width, sizes.line_height),
            align,
        ))
    }

    /// The line's words, at most `width` wide.
    fn line_text(&self, sizes: TextSizes, width: f32, align: TextAlign) -> Option<Node> {
        let style = |color| one_line_style(width, sizes.line, FontWeight::REGULAR, color, align);
        Some(match self.line.as_ref()? {
            Line::State(words) => text(words, style(STATE_COLOR)),
            Line::Since(time) => relative_time_live(
                *time,
                SINCE_FORMAT,
                RelTimeClamp::ElapsedOnly,
                style(STATE_COLOR),
            ),
            Line::Notice(words) => text(words, style(Accent::Warning.color())),
        })
    }

    /// The hint, a line row high, where only a hold fires the card action.
    fn hint(&self, sizes: TextSizes, width: f32, align: TextAlign) -> Option<Node> {
        Some(one_line(
            self.hint.as_deref()?,
            (width, sizes.line_height),
            sizes.line,
            FontWeight::REGULAR,
            STATE_COLOR,
            align,
        ))
    }

    /// The state with the hint after it, for a layout with room for one line under the name.
    /// The hint stays whole, and a state too long for both gives way to it.
    /// A notice stands alone: it tells what became of a touch.
    fn line_and_hint(&self, sizes: TextSizes, width: f32, align: TextAlign) -> Option<Node> {
        let (Some(Line::State(_) | Line::Since(_)), Some(hint)) = (&self.line, &self.hint) else {
            return self
                .line(sizes, width, align)
                .or_else(|| self.hint(sizes, width, align));
        };
        let joined = row(
            props!(max_width: width, cross_align: CrossAlign::Center),
            [
                self.line_text(sizes, width, align)?,
                text(
                    unbroken(fmt!(" · {}", hint.as_str())),
                    style!(size: sizes.line, weight: FontWeight::REGULAR, color: STATE_COLOR),
                ),
            ],
        );
        Some(in_row(joined, (width, sizes.line_height), align))
    }

    /// The entity's badge, `diameter` across: it takes [`Badge::side`] of `diameter`.
    fn icon(&self, diameter: f32, touches: &Touches) -> Node {
        Badge {
            icon: self.icon,
            diameter,
            color: self.color(),
            feedback: self.feedback(touches),
            progress: self.hold,
            disc: self.disc,
            mark: self.mark,
        }
        .view()
    }

    fn feedback(&self, touches: &Touches) -> Feedback {
        match (self.action, touch(touches).held) {
            (Some(Availability::Blocked), _) => Feedback::Inert,
            (None, _) | (Some(Availability::Ready), false) => Feedback::Resting,
            (Some(Availability::Ready), true) => Feedback::Pressed,
        }
    }

    fn color(&self) -> Color {
        self.color.map_or(INACTIVE_COLOR, Accent::color)
    }
}

/// The touches of a finger on the card outside its blocks.
#[must_use]
pub fn hold() -> Touches {
    Touches::hold(CARD, middle(PADDING, PADDING))
}

/// How a finger is on the card itself, outside its blocks.
#[must_use]
pub fn touch(touches: &Touches) -> Touch {
    Touch {
        held: touches.is_held(CARD),
        released: touches.released(CARD).is_some(),
    }
}

fn body_width(width: f32) -> f32 {
    (width - 2.0 * PADDING).max(0.0)
}

/// A text's `max_width`, which the style takes in whole pixels.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a card's width is a few hundred pixels, and a negative one clamps to zero"
)]
fn whole_pixels(width: f32) -> u32 {
    width.floor() as u32
}

/// `content` on one line in a `width` by `height` box, cut with "…" rather than shrunk or broken,
/// so the names and lines of neighbouring tiles keep one size.
fn one_line(
    content: &str,
    (width, height): (f32, f32),
    size: u32,
    weight: FontWeight,
    color: Color,
    align: TextAlign,
) -> Node {
    in_row(
        text(content, one_line_style(width, size, weight, color, align)),
        (width, height),
        align,
    )
}

/// A text style that keeps to one line `width` wide, as [`one_line`] sets it.
fn one_line_style(
    width: f32,
    size: u32,
    weight: FontWeight,
    color: Color,
    align: TextAlign,
) -> TextStyle {
    style!(
        size: size,
        weight: weight,
        color: color,
        align: align,
        max_width: whole_pixels(width),
        text_overflow: TextOverflow::Ellipsis
    )
    .into()
}

/// A line of text in a `width` by `height` row, centred upright and set along it by `align`.
fn in_row(line: Node, (width, height): (f32, f32), align: TextAlign) -> Node {
    let cross_align = match align {
        TextAlign::Left => CrossAlign::Start,
        TextAlign::Center => CrossAlign::Center,
        TextAlign::Right => CrossAlign::End,
    };
    col(
        props!(
            width: width,
            height: height,
            justify_content: Justify::Center,
            cross_align: cross_align
        ),
        [line],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::icons::CHECK;
    use bmc_wasm_sdk::wasm_test;

    fn card(line: Option<Line>, hint: Option<&str>) -> Card {
        Card {
            name: "Gate".to_owned(),
            line,
            color: None,
            icon: Icon(&CHECK),
            action: None,
            hold: None,
            hint: hint.map(str::to_owned),
            disc: Disc::Tinted,
            mark: None,
            width: 317.0,
            height: 238.0,
            size: SizeVariant::Small,
        }
    }

    /// Every string the tree would draw, in tree order.
    fn texts(node: &Node) -> Vec<String> {
        let mut out = Vec::new();
        collect_texts(node, &mut out);
        out
    }

    fn collect_texts(node: &Node, out: &mut Vec<String>) {
        match node {
            Node::Column(_, children) | Node::Row(_, children) | Node::Center(_, children) => {
                for child in children {
                    collect_texts(child, out);
                }
            }
            Node::Paragraph { spans, .. } => {
                out.push(spans.iter().map(|span| span.text.as_str()).collect());
            }
            _ => {}
        }
    }

    #[wasm_test]
    fn with_room_for_one_line_the_hint_follows_the_state_but_not_a_notice() {
        let under_name = |line, hint| {
            card(line, hint)
                .line_and_hint(TextSizes::REGULAR, 269.0, TextAlign::Center)
                .map(|node| texts(&node))
        };
        let state = || Some(Line::State("Last pressed 5 min ago".to_owned()));
        assert_eq!(
            under_name(state(), Some("Hold 2 s")),
            Some(vec![
                "Last pressed 5 min ago".to_owned(),
                unbroken(" · Hold 2 s")
            ])
        );
        assert_eq!(
            under_name(None, Some("Hold 2 s")),
            Some(vec!["Hold 2 s".to_owned()])
        );
        assert_eq!(
            under_name(
                Some(Line::Notice("Released too early".to_owned())),
                Some("Hold 2 s")
            ),
            Some(vec!["Released too early".to_owned()])
        );
        assert_eq!(
            under_name(state(), None),
            Some(vec!["Last pressed 5 min ago".to_owned()])
        );
        assert_eq!(under_name(None, None), None);
    }

    #[wasm_test]
    fn rows_past_the_room_are_left_out_from_the_last() {
        let row = |label| {
            text(
                label,
                one_line_style(300.0, 20, FontWeight::REGULAR, LABEL_COLOR, TextAlign::Left),
            )
        };
        let tall = Card {
            width: 638.0,
            height: 480.0,
            size: SizeVariant::Large,
            ..card(None, None)
        };
        let drawn = texts(&tall.with_rows(
            ["row 1", "row 2", "row 3", "row 4"].map(row).to_vec(),
            &Touches::default(),
        ));
        assert_eq!(drawn, ["Gate", "row 1", "row 2", "row 3"]);
    }

    #[wasm_test]
    fn a_wide_card_puts_its_rows_beside_and_a_short_narrow_one_has_none() {
        let count = |width, height, size| rows((width, height), size).count;
        assert_eq!(count(317.0, 238.0, SizeVariant::Small), 0, "1x1");
        assert_eq!(
            count(476.0, 238.0, SizeVariant::Medium),
            0,
            "twice as wide as high is not wide"
        );
        assert_eq!(count(477.0, 238.0, SizeVariant::Medium), 2, "wider is");
        assert_eq!(count(638.0, 238.0, SizeVariant::Medium), 2, "2x1");
        assert_eq!(count(1280.0, 480.0, SizeVariant::Full), 5, "fullscreen");
    }

    #[wasm_test]
    fn rows_under_the_heading_keep_the_cards_own_icon_while_they_fit_under_it() {
        assert_eq!(
            rows((638.0, 480.0), SizeVariant::Large).count,
            3,
            "the short heading leaves room for three rows"
        );
        let body = 480.0 - 2.0 * PADDING;
        let height = |count: f32| count * (CONTROL_HEIGHT + ROW_GAP) - ROW_GAP;
        assert_eq!(
            heading_over(SizeVariant::Large, body, height(2.0)),
            Centred::TALL
        );
        assert_eq!(
            heading_over(SizeVariant::Large, body, height(3.0)),
            Centred::SHORT
        );
    }

    #[wasm_test]
    fn the_names_capitals_start_inside_its_row_at_every_size() {
        for sizes in [TextSizes::REGULAR, TextSizes::LARGE] {
            let inset = sizes.title_cap_inset();
            assert!(
                inset > 0.0 && inset < sizes.title_height / 2.0,
                "{sizes:?}: {inset}"
            );
        }
    }

    #[wasm_test]
    fn a_finger_on_the_card_is_on_it_apart_from_its_blocks() {
        let hit = middle(400.0, 400.0);
        assert_eq!(
            touch(&Touches::tap(CARD, hit)),
            Touch {
                held: false,
                released: true
            }
        );
        assert_eq!(
            touch(&Touches::hold(CARD, hit)),
            Touch {
                held: true,
                released: false
            }
        );
        assert_eq!(touch(&Touches::tap("power", hit)), Touch::default());
    }
}
