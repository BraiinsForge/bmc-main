// Copyright (C) 2026  Braiins Systems s.r.o.

//! Modal dialog builder.

use bmc_wasm_protocol::Color;

use crate::fmt;
use crate::tree::Node;

/// A labeled button action for modal footers.
///
/// The interaction key is derived from the modal's key: `{modal_key}::primary`
/// or `{modal_key}::secondary`. The close button uses `{modal_key}::close`.
#[derive(Clone, Debug)]
pub struct ModalAction {
    pub label: &'static str,
}

/// Modal footer descriptor — the host renders buttons at the appropriate size.
///
/// Layout follows CDS conventions:
/// - Two buttons: `[secondary | primary]`, each 50% width
/// - One button (no secondary): `[spacer | primary]`, right-aligned 50%
/// - `danger`: primary renders as Danger style (e.g. delete confirmation)
#[derive(Clone, Debug)]
pub struct ModalFooter {
    pub primary: ModalAction,
    pub secondary: Option<ModalAction>,
    pub danger: bool,
}

/// Modal dialog configuration.
///
/// All fields default to sensible values via `Default`. Only set what you need.
#[derive(Clone, Debug, Default)]
pub struct ModalProps {
    /// Estimated total height of body content for scroll sizing.
    /// Default: 0.0
    pub height: f32,

    /// Margin around the modal content area in pixels.
    /// This creates space between the modal and screen edges where the
    /// semi-transparent backdrop is visible.
    /// Default: 48 pixels
    pub margin: u16,

    /// Backdrop opacity as 0-255 value (0 = fully transparent, 255 = fully opaque).
    /// The backdrop is the dark overlay behind the modal that dims the background content.
    /// Lower values make more of the background visible through the overlay.
    /// Default: 128 (50% opacity)
    pub backdrop_alpha: u8,

    /// Modal body background color. Default = use default (GRAY_90).
    pub bg_color: Color,

    /// Modal header background color. Default = use default (GRAY_100).
    pub header_color: Color,

    /// Modal title text color. Default = use default (GRAY_10).
    pub title_color: Color,

    /// Maximum modal width in pixels. `0` = no limit (fill available space).
    pub max_width: u16,

    /// Optional footer with primary (and optional secondary) action buttons.
    pub footer: Option<ModalFooter>,
}

impl ModalProps {
    /// Default margin around modal content (48 pixels)
    pub const DEFAULT_MARGIN: u16 = 48;
    /// Default backdrop opacity (128 = 50%)
    pub const DEFAULT_BACKDROP_ALPHA: u8 = 128;
}

/// Create a modal dialog overlay.
///
/// # Arguments
/// - `key` — unique ID for state tracking; close button gets `"{key}::close"`
/// - `open` — whether the modal is visible
/// - `title` — header title text
/// - `content` — body child nodes
/// - `props` — styling, layout, and footer configuration
///
/// # Examples
/// ```ignore
/// // Simple — no footer, default props
/// modal("about", MODAL_OPEN.get(), "About", vec![text("Hello", s)], None)
///
/// // With footer + props
/// modal("settings", SETTINGS_OPEN.get(), "Settings",
///     vec![number_input!("work", 25, label: "Work")],
///     Some(ModalProps {
///         height: 200.0,
///         footer: Some(ModalFooter {
///             primary: ModalAction { key: "save", label: "Save" },
///             secondary: None,
///             danger: false,
///         }),
///         ..Default::default()
///     }),
/// )
/// ```
pub fn modal(
    key: impl Into<String>,
    open: bool,
    title: impl Into<String>,
    content: Vec<Node>,
    props: Option<ModalProps>,
) -> Node {
    debug_assert!(
        !content.iter().any(|n| matches!(n, Node::Scroll { .. })),
        "modal: body contains a Scroll node; modal already wraps the body in its own \
         scroll container (see bmc-render/src/components/modal.rs). Remove the inner \
         scroll and pass plain content — otherwise the rendered scrollbars stack."
    );
    let props = props.unwrap_or_default();
    let modal_key: String = key.into();
    let (pk, pl, sk, sl, danger) = match &props.footer {
        Some(f) => {
            let (sk, sl) = match &f.secondary {
                Some(s) => (fmt!("{modal_key}::secondary"), s.label.to_owned()),
                None => (String::new(), String::new()),
            };
            (
                fmt!("{modal_key}::primary"),
                f.primary.label.to_owned(),
                sk,
                sl,
                f.danger,
            )
        }
        None => (
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            false,
        ),
    };
    Node::Modal {
        modal_id: modal_key,
        is_open: open,
        title: title.into(),
        content_height: props.height,
        padding: if props.margin == 0 {
            ModalProps::DEFAULT_MARGIN
        } else {
            props.margin
        },
        backdrop_alpha: if props.backdrop_alpha == 0 {
            ModalProps::DEFAULT_BACKDROP_ALPHA
        } else {
            props.backdrop_alpha
        },
        max_width: props.max_width,
        bg_color: props.bg_color,
        header_color: props.header_color,
        title_color: props.title_color,
        body: content,
        footer_primary_key: pk,
        footer_primary_label: pl,
        footer_secondary_key: sk,
        footer_secondary_label: sl,
        footer_danger: danger,
    }
}
