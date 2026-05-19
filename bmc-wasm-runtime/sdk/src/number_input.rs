// Copyright (C) 2026  Braiins Systems s.r.o.

//! CDS-style NumberInput compound component.
//!
//! Renders a labeled numeric field with +/- stepper buttons, optional suffix,
//! helper text, warning and error states. Built entirely from existing tree
//! primitives — no custom host-side rendering needed.

use crate::host::{ButtonSize, ButtonStyle};
use crate::props;

use crate::tree::{
    Draw, Node, PropsData, StyleResult, TextStyle, TreeRenderResult, canvas, col, make_button, row,
    spacer, text,
};
use bmc_wasm_protocol::{
    CrossAlign, GRAY_40, GRAY_50, GRAY_60, GRAY_80, GRAY_90, ICON_MINUS, ICON_PLUS, ICON_WARN_ALT,
    ICON_WARN_FILLED, RED_50, WHITE, YELLOW_30,
};

/// Configuration for a number input.
#[derive(Clone, Debug, Default)]
pub struct NumberInputProps {
    pub label: &'static str,
    pub suffix: &'static str,
    pub min: i32,
    pub max: i32,
    pub step: i32,
    pub helper: &'static str,
    pub warning: &'static str,
    pub error: &'static str,
    pub disabled: bool,
}

/// Build a number input from key, value, and props.
#[must_use]
#[expect(clippy::too_many_lines)]
pub fn number_input(key: &str, value: i32, p: &NumberInputProps) -> Node {
    let has_error = !p.error.is_empty();
    let has_warning = !p.warning.is_empty();
    let text_color = if p.disabled { GRAY_60 } else { WHITE };
    let border_color = if has_error {
        RED_50
    } else if has_warning {
        YELLOW_30
    } else if p.disabled {
        GRAY_80
    } else {
        GRAY_60
    };

    let mut children: Vec<Node> = Vec::new();

    // Label (optional)
    if !p.label.is_empty() {
        children.push(text(
            p.label.to_owned(),
            StyleResult(
                TextStyle {
                    size: 12,
                    color: GRAY_40,
                    ..Default::default()
                },
                PropsData::default(),
            ),
        ));
    }

    // Value text
    let value_text = if p.suffix.is_empty() {
        crate::fmt!("{value}")
    } else {
        let suffix = p.suffix;
        crate::fmt!("{value} {suffix}")
    };

    let minus_key = crate::fmt!("{key}_minus");
    let plus_key = crate::fmt!("{key}_plus");

    // 1px vertical divider — 60% of button height, dimmer color
    let divider = || col(props!(width: 1.0, height: 20.0, background: GRAY_80), []);

    // Status icon: (icon_id, color) or None
    let status = if has_error {
        Some((ICON_WARN_FILLED, RED_50))
    } else if has_warning {
        Some((ICON_WARN_ALT, YELLOW_30))
    } else {
        None
    };

    // Input row: [ text | spacer(1) | icon? | gap? | divider | − | divider | + ]
    let mut input_children: Vec<Node> = vec![
        text(
            value_text,
            StyleResult(
                TextStyle {
                    size: 14,
                    color: text_color,
                    ..Default::default()
                },
                PropsData {
                    inset_left: 12.0,
                    ..Default::default()
                },
            ),
        ),
        spacer(1.0),
    ];
    if let Some((icon_id, color)) = status {
        input_children.push(canvas(
            props!(width: 18.0, height: 18.0),
            [Draw::svg_builtin(1.0, 1.0, 14.0, 14.0, icon_id, color)],
        ));
        input_children.push(row(props!(width: 12.0), []));
    }
    input_children.push(divider());
    input_children.push(make_button(
        minus_key,
        String::new(),
        ButtonStyle::Ghost,
        ButtonSize::Small,
        Some(ICON_MINUS),
        p.disabled,
        None,
    ));
    input_children.push(divider());
    input_children.push(make_button(
        plus_key,
        String::new(),
        ButtonStyle::Ghost,
        ButtonSize::Small,
        Some(ICON_PLUS),
        p.disabled,
        None,
    ));

    // Input row + bottom border flush together (no gap)
    children.push(col(
        props!(),
        [
            row(
                props!(background: GRAY_90, cross_align: CrossAlign::Center),
                input_children,
            ),
            row(props!(height: 1.0, background: border_color), []),
        ],
    ));

    // Helper / warning / error text (optional)
    let bottom_text = if has_error {
        Some((p.error, RED_50))
    } else if has_warning {
        Some((p.warning, GRAY_50))
    } else if !p.helper.is_empty() {
        Some((p.helper, GRAY_50))
    } else {
        None
    };
    if let Some((msg, color)) = bottom_text {
        children.push(text(
            msg.to_owned(),
            StyleResult(
                TextStyle {
                    size: 12,
                    color,
                    ..Default::default()
                },
                PropsData::default(),
            ),
        ));
    }

    col(props!(gap: 4.0), children)
}

/// Handle +/- clicks for a number input. Returns `Some(new_value)` if changed.
#[must_use]
pub fn number_input_handle(
    key: &str,
    value: i32,
    props: &NumberInputProps,
    result: &TreeRenderResult,
) -> Option<i32> {
    let minus_key = crate::fmt!("{key}_minus");
    let plus_key = crate::fmt!("{key}_plus");
    let step = if props.step == 0 { 1 } else { props.step };

    if result.clicks.contains_key(&minus_key) {
        Some(value.saturating_sub(step).clamp(props.min, props.max))
    } else if result.clicks.contains_key(&plus_key) {
        Some(value.saturating_add(step).clamp(props.min, props.max))
    } else {
        None
    }
}
