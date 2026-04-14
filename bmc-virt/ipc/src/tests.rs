// Copyright (C) 2026  Braiins Systems s.r.o.

#![expect(
    clippy::wildcard_enum_match_arm,
    reason = "test assertions use _ => panic! for concise failure messages"
)]

use crate::types::*;
use crate::wire;

type R<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Round-trip a guest message through encode → decode.
fn roundtrip_guest(msg: &GuestMessage, data: Option<&[u8]>) -> R<GuestMessage> {
    let mut buf = Vec::new();
    wire::encode_guest(msg, data, &mut buf)?;
    let mut cursor = std::io::Cursor::new(buf);
    wire::decode_guest(&mut cursor)?.ok_or("unexpected EOF".into())
}

/// Round-trip a host message through encode → decode.
fn roundtrip_host(msg: &HostMessage) -> R<HostMessage> {
    let mut buf = Vec::new();
    wire::encode_host(msg, &mut buf)?;
    let mut cursor = std::io::Cursor::new(buf);
    wire::decode_host(&mut cursor)?.ok_or("unexpected EOF".into())
}

#[test]
fn frame_roundtrip() -> R {
    let pixels = vec![0xAB_u8; 480 * 4];
    let msg = GuestMessage::Frame {
        header: FrameHeader {
            seq: 42,
            width: 480,
            height: 1_280,
            stride: Stride(1_920),
            bpp: Bpp(32),
            format: crate::types::PixelFormat::Rgba8888,
            brightness: 15,
        },
        data: pixels.clone(),
    };
    let decoded = roundtrip_guest(&msg, Some(&pixels))?;
    match decoded {
        GuestMessage::Frame { header, data } => {
            assert_eq!(header.seq, 42);
            assert_eq!(header.width, 480);
            assert_eq!(header.height, 1_280);
            assert_eq!(header.stride.0, 1_920);
            assert_eq!(header.bpp.0, 32);
            assert_eq!(header.format, crate::types::PixelFormat::Rgba8888);
            assert_eq!(header.brightness, 15);
            assert_eq!(data, pixels);
        }
        _ => panic!("expected Frame"),
    }
    Ok(())
}

#[test]
fn led_roundtrip() -> R {
    let mut leds = [LedState::default(); LED_COUNT];
    leds[0] = LedState {
        brightness: 31,
        r: 255,
        g: 0,
        b: 0,
    };
    leds[9] = LedState {
        brightness: 15,
        r: 0,
        g: 0,
        b: 255,
    };
    let msg = GuestMessage::Leds(LedUpdate { seq: 7, leds });
    let decoded = roundtrip_guest(&msg, None)?;
    match decoded {
        GuestMessage::Leds(update) => {
            assert_eq!(update.seq, 7);
            assert_eq!(update.leds[0].brightness, 31);
            assert_eq!(update.leds[0].r, 255);
            assert_eq!(update.leds[9].b, 255);
        }
        _ => panic!("expected Leds"),
    }
    Ok(())
}

#[test]
fn log_roundtrip() -> R {
    let msg = GuestMessage::Log {
        source: LogSource::Syslog,
        line: "hello world".into(),
    };
    let decoded = roundtrip_guest(&msg, None)?;
    match decoded {
        GuestMessage::Log { source, line } => {
            assert_eq!(source, LogSource::Syslog);
            assert_eq!(line, "hello world");
        }
        _ => panic!("expected Log"),
    }
    Ok(())
}

#[test]
fn active_effect_roundtrip() -> R {
    let msg = GuestMessage::ActiveEffect(3);
    let decoded = roundtrip_guest(&msg, None)?;
    match decoded {
        GuestMessage::ActiveEffect(idx) => assert_eq!(idx, 3),
        _ => panic!("expected ActiveEffect"),
    }
    Ok(())
}

#[test]
fn capture_status_roundtrip() -> R {
    let msg = GuestMessage::CaptureStatus {
        state: FeatureState::Unavailable,
        reason: Some("capture failed".into()),
    };
    let decoded = roundtrip_guest(&msg, None)?;
    match decoded {
        GuestMessage::CaptureStatus { state, reason } => {
            assert_eq!(state, FeatureState::Unavailable);
            assert_eq!(reason.as_deref(), Some("capture failed"));
        }
        _ => panic!("expected CaptureStatus"),
    }
    Ok(())
}

#[test]
fn controls_status_roundtrip() -> R {
    let msg = GuestMessage::ControlsStatus {
        state: FeatureState::Waiting,
        reason: None,
    };
    let decoded = roundtrip_guest(&msg, None)?;
    match decoded {
        GuestMessage::ControlsStatus { state, reason } => {
            assert_eq!(state, FeatureState::Waiting);
            assert_eq!(reason, None);
        }
        _ => panic!("expected ControlsStatus"),
    }
    Ok(())
}

#[test]
fn touch_down_roundtrip() -> R {
    let msg = HostMessage::Input(InputEvent::TouchDown { x: 100, y: 200 });
    let decoded = roundtrip_host(&msg)?;
    match decoded {
        HostMessage::Input(InputEvent::TouchDown { x, y }) => {
            assert_eq!(x, 100);
            assert_eq!(y, 200);
        }
        _ => panic!("expected TouchDown"),
    }
    Ok(())
}

#[test]
fn touch_move_roundtrip() -> R {
    let msg = HostMessage::Input(InputEvent::TouchMove { x: 300, y: 400 });
    let decoded = roundtrip_host(&msg)?;
    match decoded {
        HostMessage::Input(InputEvent::TouchMove { x, y }) => {
            assert_eq!(x, 300);
            assert_eq!(y, 400);
        }
        _ => panic!("expected TouchMove"),
    }
    Ok(())
}

#[test]
fn touch_up_roundtrip() -> R {
    let msg = HostMessage::Input(InputEvent::TouchUp);
    let decoded = roundtrip_host(&msg)?;
    match decoded {
        HostMessage::Input(InputEvent::TouchUp) => {}
        _ => panic!("expected TouchUp"),
    }
    Ok(())
}

#[test]
fn button_press_roundtrip() -> R {
    let msg = HostMessage::Input(InputEvent::ButtonPress {
        button: buttons::LED_EFFECT_SET,
        data: 3,
    });
    let decoded = roundtrip_host(&msg)?;
    match decoded {
        HostMessage::Input(InputEvent::ButtonPress { button, data }) => {
            assert_eq!(button, buttons::LED_EFFECT_SET);
            assert_eq!(data, 3);
        }
        _ => panic!("expected ButtonPress"),
    }
    Ok(())
}

#[test]
fn gpio_button_roundtrip() -> R {
    for pressed in [true, false] {
        let msg = HostMessage::GpioButton { pressed };
        let decoded = roundtrip_host(&msg)?;
        match decoded {
            HostMessage::GpioButton {
                pressed: decoded_pressed,
            } => assert_eq!(decoded_pressed, pressed),
            _ => panic!("expected GpioButton"),
        }
    }
    Ok(())
}

#[test]
fn multiple_messages_in_stream() -> R {
    let mut buf = Vec::new();
    let log1 = GuestMessage::Log {
        source: LogSource::BmcLog,
        line: "first".into(),
    };
    let log2 = GuestMessage::Log {
        source: LogSource::Dmesg,
        line: "second".into(),
    };
    wire::encode_guest(&log1, None, &mut buf)?;
    wire::encode_guest(&log2, None, &mut buf)?;

    let mut cursor = std::io::Cursor::new(buf);
    let decoded1 = wire::decode_guest(&mut cursor)?.ok_or("expected log1")?;
    let decoded2 = wire::decode_guest(&mut cursor)?.ok_or("expected log2")?;
    // EOF
    assert!(wire::decode_guest(&mut cursor)?.is_none());

    match decoded1 {
        GuestMessage::Log { source, line } => {
            assert_eq!(source, LogSource::BmcLog);
            assert_eq!(line, "first");
        }
        _ => panic!("expected Log"),
    }
    match decoded2 {
        GuestMessage::Log { source, line } => {
            assert_eq!(source, LogSource::Dmesg);
            assert_eq!(line, "second");
        }
        _ => panic!("expected Log"),
    }
    Ok(())
}

#[test]
fn eof_returns_none() -> R {
    let buf: Vec<u8> = Vec::new();
    let mut cursor = std::io::Cursor::new(buf);
    assert!(wire::decode_guest(&mut cursor)?.is_none());
    assert!(wire::decode_host(&mut cursor)?.is_none());
    Ok(())
}
