// Copyright (C) 2026  Braiins Systems s.r.o.

//! `RelativeTimeLive` node builder.

use bmc_wasm_protocol::{RelTimeClamp, RelTimeFormat};

use crate::host::SystemTime;
use crate::tree::{Node, TextStyle};

/// Self-updating relative-time label anchored at `anchor`; the host formats
/// `now - anchor` against its clock. `clamp` defaults to sign-based direction.
#[must_use]
pub fn relative_time_live(anchor: SystemTime, format: RelTimeFormat, style: TextStyle) -> Node {
    Node::RelTime {
        anchor: anchor.unix_secs,
        format,
        clamp: RelTimeClamp::Auto,
        style,
    }
}

#[cfg(test)]
mod tests {
    use bmc_wasm_protocol::{NODE_RELTIME, RelTimeClamp, RelTimeFormat};

    use crate::host::SystemTime;
    use crate::tree::{TextStyle, TreeBuffer};

    #[test]
    fn wire_leads_with_type_anchor_format_clamp() {
        let mut buf = TreeBuffer::new();
        buf.write_relative_time(
            1_700_000_000,
            RelTimeFormat::Short,
            RelTimeClamp::Auto,
            &TextStyle::default(),
        );
        let bytes = buf.into_bytes();
        assert_eq!(bytes[0], NODE_RELTIME);
        assert_eq!(&bytes[1..9], &1_700_000_000_i64.to_le_bytes());
        assert_eq!(bytes[9], u8::from(RelTimeFormat::Short));
        assert_eq!(bytes[10], u8::from(RelTimeClamp::Auto));
    }

    #[test]
    fn builder_carries_anchor_seconds_and_auto_clamp() {
        let node = super::relative_time_live(
            SystemTime { unix_secs: 42 },
            RelTimeFormat::Short,
            TextStyle::default(),
        );
        assert!(matches!(
            node,
            crate::tree::Node::RelTime {
                anchor: 42,
                clamp: RelTimeClamp::Auto,
                ..
            }
        ));
    }
}
