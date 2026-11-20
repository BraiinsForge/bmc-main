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

//! The card every tile is drawn on, the pieces of its heading, and the layouts made of them:
//! centred, and over a large reading.

use bmc_wasm_sdk::typography::{EMDASH, unbroken};
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
/// A reading's row, as a share of its font size, with room for descenders.
const READING_LINE: f32 = 1.2;
/// The line height of a time that wraps onto two lines, as a share of its font size.
const TIME_LINE: f32 = 1.1;
/// The reading's font size beside the heading, where it has the card's whole height.
const BESIDE_READING: u32 = 80;

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
/// The share of a wide card the centred block takes, left of the rows or the reading, picked by eye in the gallery.
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

/// How the layout over a reading is set on a card of one height.
#[derive(Debug, Clone, Copy, PartialEq)]
struct OverReading {
    /// The circle's diameter.
    icon: f32,
    text: TextSizes,
    /// Between the circle and the capitals of the name under it.
    ring_clearance: f32,
    /// The reading's font size, which a reading too wide for the card shrinks from.
    reading: u32,
    /// Between the heading and the reading.
    reading_gap: f32,
    line: LineRow,
}

impl OverReading {
    /// On a card one row high and narrow the reading takes the line's place, to keep its size.
    const SHORT: Self = Self {
        icon: 64.0,
        text: TextSizes::REGULAR,
        ring_clearance: 12.0,
        reading: 64,
        reading_gap: 0.0,
        line: LineRow::InPlaceOfReading,
    };
    const TALL: Self = Self {
        icon: 96.0,
        text: TextSizes::LARGE,
        ring_clearance: 20.0,
        reading: 120,
        reading_gap: 16.0,
        line: LineRow::UnderName,
    };

    fn of(size: SizeVariant) -> Self {
        match size {
            SizeVariant::Small | SizeVariant::Medium => Self::SHORT,
            SizeVariant::Large | SizeVariant::Full => Self::TALL,
        }
    }
}

/// Where the layout over a reading shows the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineRow {
    UnderName,
    /// Only a line that says why the tile cannot show the reading, such as "Loading…" or "Reconnecting…",
    /// in its row; a time on the line is left out.
    InPlaceOfReading,
}

/// What a card shows large under its heading, such as a sensor's value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading {
    Words(String),
    /// How long ago a time was, or how soon it comes, such as "in 5 minutes":
    /// the host words it in the device language and keeps it current.
    Time(SystemTime),
}

impl Reading {
    /// No value, as the frontend's tile shows one.
    #[must_use]
    pub fn none() -> Self {
        Self::Words(EMDASH.to_owned())
    }
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
        let under_icon =
            ring_clearance - text.title_cap_inset() + text.title_height + text.line_height;
        vec![
            self.space_above_icon(icon, under_icon),
            self.icon_over(centred, texts, touches),
        ]
    }

    /// The space that puts an `icon`, with what hangs `under_icon` high below it, at the optical centre.
    fn space_above_icon(&self, icon: f32, under_icon: f32) -> Node {
        // The badge keeps room around its circle for the hold ring.
        let ring_room = (Badge::side(icon) - icon) / 2.0;
        let seen = icon + ring_room + under_icon;
        let above_circle = (self.height - seen) / 2.0 - OPTICAL_LIFT * self.height;
        col(
            props!(height: (above_circle - ring_room).max(0.0)),
            Vec::<Node>::new(),
        )
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

    /// The heading with `reading` set large, to be read across the room:
    /// beside it on a card one row high and wide, else under it.
    #[must_use]
    pub fn over_reading(&self, reading: &Reading, touches: &Touches) -> Node {
        match self.size {
            SizeVariant::Medium => self.reading_beside(reading, touches),
            SizeVariant::Small | SizeVariant::Large | SizeVariant::Full => {
                self.reading_under(reading, touches)
            }
        }
    }

    /// The icon over the name and `reading`, all centred,
    /// the circle to the reading's foot at the optical centre, as [`Self::centred`] sets its block.
    fn reading_under(&self, reading: &Reading, touches: &Touches) -> Node {
        let OverReading {
            icon,
            text,
            ring_clearance,
            reading: reading_size,
            reading_gap,
            line,
        } = OverReading::of(self.size);
        let width = body_width(self.width);
        let align = TextAlign::Center;
        #[expect(clippy::cast_precision_loss, reason = "a font size, exact in f32")]
        let reading_height = reading_size as f32 * READING_LINE;
        let mut heading = vec![self.title(text, width, align)];
        let reading = match line {
            LineRow::UnderName => {
                heading.extend(self.line(text, width, align));
                Self::reading(reading, reading_size, (width, reading_height))
            }
            // Under the name, as on a taller card, in a row kept as high as the reading's,
            // so the icon and the name stay put when the reading comes.
            LineRow::InPlaceOfReading => match self.words_line(text, width, align) {
                Some(line) => col(props!(height: reading_height), [line]),
                None => Self::reading(reading, reading_size, (width, reading_height)),
            },
        };
        let line_height = match line {
            LineRow::UnderName if self.line.is_some() => text.line_height,
            LineRow::UnderName | LineRow::InPlaceOfReading => 0.0,
        };
        let cap_inset = text.title_cap_inset();
        let under_icon = ring_clearance - cap_inset
            + text.title_height
            + line_height
            + reading_gap
            + reading_height;
        self.surface(
            props!(cross_align: CrossAlign::Center),
            vec![
                self.space_above_icon(icon, under_icon),
                col(
                    props!(gap: ring_clearance - cap_inset, cross_align: CrossAlign::Center),
                    [
                        self.icon(icon, touches),
                        col(
                            props!(gap: reading_gap, cross_align: CrossAlign::Center),
                            [col(props!(), heading), reading],
                        ),
                    ],
                ),
            ],
            touches,
        )
    }

    /// The icon over the name and the line, centred in a column on the left, and `reading` centred on the right:
    /// a card one row high and wide has room for both side by side, the reading at the card's full height.
    /// Its heading is the narrow card's, so a sensor's name sits alike on both.
    fn reading_beside(&self, reading: &Reading, touches: &Touches) -> Node {
        let OverReading {
            icon,
            text,
            ring_clearance,
            ..
        } = OverReading::SHORT;
        // The columns of a card with rows of blocks beside its heading, so the headings line up.
        let right = self.shape().beside_width();
        let left = self.width - PADDING - right;
        let texts_width = body_width(left);
        let mut texts = vec![self.title(text, texts_width, TextAlign::Center)];
        texts.extend(self.line(text, texts_width, TextAlign::Center));
        let reading_height = (self.height - 2.0 * PADDING).max(0.0);
        let reading = Self::reading(reading, BESIDE_READING, (right, reading_height));
        self.surface(
            props!(),
            vec![row(
                props!(
                    width: self.width,
                    height: self.height,
                    cross_align: CrossAlign::Center
                ),
                [
                    col(
                        props!(
                            width: left,
                            gap: ring_clearance - text.title_cap_inset(),
                            cross_align: CrossAlign::Center
                        ),
                        [self.icon(icon, touches), col(props!(), texts)],
                    ),
                    reading,
                ],
            )],
            touches,
        )
    }

    /// `reading` in a `width` by `height` row, white and bold.
    /// Words too wide shrink to fit, wrapping where that leaves them larger, rather than being cut:
    /// no other line says what they are.
    fn reading(reading: &Reading, size: u32, (width, height): (f32, f32)) -> Node {
        let style = style!(
            size: size,
            weight: FontWeight::BOLD,
            color: LABEL_COLOR,
            align: TextAlign::Center,
            valign: VerticalAlign::Center
        );
        match reading {
            Reading::Words(words) => {
                // AutoFit wraps with cosmic-text's `WordOrGlyph`: a word too wide for its line
                // is split wherever the halves fit the box at a smaller size.
                // A line of room per word leaves a lone word no room for halves.
                // TODO(BDK-555): drop it once AutoFit can wrap at words alone.
                #[expect(clippy::cast_precision_loss, reason = "a few words and a font size")]
                let words_height = words
                    .split(' ')
                    .filter(|word| !word.is_empty())
                    .count()
                    .max(1) as f32
                    * size as f32
                    * READING_LINE;
                let fit_height = height.min(words_height);
                canvas(
                    props!(width: width, height: height),
                    [Draw::autofit_text(
                        0.0,
                        (height - fit_height) / 2.0,
                        width,
                        fit_height,
                        unbroken_at_joints(words),
                        style,
                    )],
                )
            }
            // TODO(BDK-1153): shrink to fit as words do, once the host can fit a time it words;
            // until then it wraps, set small enough for two lines to fit.
            Reading::Time(time) => in_row(
                relative_time_live(
                    *time,
                    SINCE_FORMAT,
                    RelTimeClamp::Auto,
                    style!(
                        size: size.min(two_lines_in(height)),
                        weight: FontWeight::BOLD,
                        color: LABEL_COLOR,
                        align: TextAlign::Center,
                        line_height: TIME_LINE,
                        max_width: whole_pixels(width)
                    )
                    .into(),
                ),
                (width, height),
                TextAlign::Center,
            ),
        }
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

    /// The line where it is a state or a notice, not a time.
    fn words_line(&self, sizes: TextSizes, width: f32, align: TextAlign) -> Option<Node> {
        match self.line.as_ref()? {
            Line::State(_) | Line::Notice(_) => self.line(sizes, width, align),
            Line::Since(_) => None,
        }
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

/// `words` breakable at spaces alone: UAX #14 also breaks after a slash or a hyphen,
/// splitting a unit such as "mmol/L" or a date.
fn unbroken_at_joints(words: &str) -> String {
    const WORD_JOINER: char = '\u{2060}';
    let mut joined = String::with_capacity(words.len());
    for character in words.chars() {
        joined.push(character);
        if matches!(character, '/' | '-') {
            joined.push(WORD_JOINER);
        }
    }
    joined
}

/// The largest font size at which two lines of a time fit `height`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a card's height is a few hundred pixels, and a negative one clamps to zero"
)]
fn two_lines_in(height: f32) -> u32 {
    (height / (2.0 * TIME_LINE)).floor().max(0.0) as u32
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

    /// The text and box height of the reading's fitted words.
    fn fitted(words: &str, height: f32) -> (String, f32) {
        let reading = Card::reading(&Reading::Words(words.to_owned()), 80, (300.0, height));
        let Node::Canvas { draws, .. } = reading else {
            panic!("BUG: words are drawn on a canvas, got {reading:?}");
        };
        match draws.as_slice() {
            [
                Draw::AutofitText {
                    text, box_height, ..
                },
            ] => (text.clone(), *box_height),
            other => panic!("BUG: words are one fitted text, got {other:?}"),
        }
    }

    #[wasm_test]
    fn a_reading_breaks_at_spaces_but_not_after_a_slash_or_a_hyphen() {
        assert_eq!(
            fitted("5,4 mmol/L on 2026-11-21", 300.0).0,
            "5,4 mmol/\u{2060}L on 2026-\u{2060}11-\u{2060}21"
        );
    }

    #[wasm_test]
    fn a_single_word_has_room_for_one_line_so_it_shrinks_rather_than_splits() {
        let line = 80.0 * READING_LINE;
        assert_eq!(fitted("Calibrating", 300.0).1, line);
        assert_eq!(
            fitted("Partly cloudy", 300.0).1,
            2.0 * line,
            "two words have room for a line each"
        );
        assert_eq!(
            fitted("Partly cloudy with rain", 300.0).1,
            300.0,
            "words beyond the box share its lines"
        );
    }
}
