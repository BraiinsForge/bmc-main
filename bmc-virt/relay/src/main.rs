// Copyright (C) 2026  Braiins Systems s.r.o.

// bmc-virt-relay: Guest daemon that captures the compositor display and LED state,
// sending them to the host console app via a TCP-based IPC protocol.
//
// Usage: bmc-virt-relay [FPS]
//   FPS: framebuffer capture rate (default: 30)

mod capture;
mod commands;
mod logs;
mod touch;

use bmc_virt_ipc::protocol::DEFAULT_PORT;
use bmc_virt_ipc::{
    Bpp, FB_HEIGHT, FB_WIDTH, FeatureState, FrameHeader, GuestEndpoint, HostMessage, InputEvent,
    LED_COUNT, LedState, LedUpdate, NotifyLevel, Stride,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

const DEFAULT_FPS: u32 = 60;
const DEFAULT_TOUCH_DEVICE: &str = "/dev/input/event0";
const DEFAULT_SPI_CAPTURE: &str = "/proc/bmc_virt_spi0";

/// File read by the madplay shim to override the app's volume setting.
/// Empty or absent = use app's own volume. Contains 0–100 = override.
const VOLUME_OVERRIDE_PATH: &str = "/root/bmc_volume_override";

/// Fake backlight sysfs directory and files, created by the VM's init scripts.
const BACKLIGHT_DIR: &str = "/tmp/fake-backlight/display-bl";
const BACKLIGHT_BRIGHTNESS_PATH: &str = "/tmp/fake-backlight/display-bl/brightness";
const BACKLIGHT_MAX_PATH: &str = "/tmp/fake-backlight/display-bl/max_brightness";

#[expect(
    clippy::too_many_lines,
    reason = "top-level orchestration, not worth splitting"
)]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let fps = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_FPS);

    eprintln!("bmc-virt-relay: fps={fps}");

    // 1. Bind the IPC listener (does not block for a connection)
    let listen_addr = format!("0.0.0.0:{DEFAULT_PORT}");
    let endpoint = GuestEndpoint::bind(&listen_addr)
        .unwrap_or_else(|e| panic!("failed to bind IPC listener: {e}"));
    let sender = endpoint.sender();

    // 2. Open touch device for input injection
    if touch::Touch::open(DEFAULT_TOUCH_DEVICE).is_err() {
        eprintln!("WARNING: could not open {DEFAULT_TOUCH_DEVICE}, touch injection disabled");
    }

    // 3. Start LED capture thread (runs forever, drops when no host)
    {
        let led_sender = sender.clone();
        std::thread::Builder::new()
            .name("led-capture".into())
            .spawn(move || {
                let mut decoder = bmc_virt_leds::apa102::Decoder::new();
                let mut led_seq: u64 = 0;

                if let Err(e) = bmc_virt_leds::proc_stream::run(DEFAULT_SPI_CAPTURE, |data| {
                    if let Some(leds) = decoder.feed(data) {
                        let mut led_state = [LedState::default(); LED_COUNT];
                        for (i, led) in leds.iter().enumerate() {
                            led_state[i].brightness = led.brightness;
                            led_state[i].r = led.r;
                            led_state[i].g = led.g;
                            led_state[i].b = led.b;
                        }
                        led_seq += 1;
                        led_sender.send_leds(LedUpdate {
                            seq: led_seq,
                            leds: led_state,
                        });
                    }
                }) {
                    eprintln!("LED capture error: {e}");
                }
            })
            .unwrap_or_else(|e| panic!("failed to spawn LED capture thread: {e}"));
    }

    // 4. Start backlight brightness watcher (inotify, reacts instantly to changes)
    let backlight = start_backlight_watcher();

    // 5. Frame capture parameters
    #[expect(
        clippy::integer_division,
        reason = "FPS to nanosecond conversion is intentionally integer"
    )]
    let frame_interval = Duration::from_nanos(1_000_000_000 / u64::from(fps));
    let mut frame_count: u64 = 0;
    let bpp = Bpp(32);
    // Mute audio when no console is connected
    volume_override_write(0);

    // 6. Accept loop: wait for host connections, serve until disconnect, repeat
    loop {
        eprintln!("relay: waiting for host connection...");
        let mut conn = match endpoint.accept_next() {
            Ok(conn) => conn,
            Err(e) => {
                eprintln!("ipc: accept error: {e}");
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        };

        // Publish initial feature states for this connection.
        sender.send_capture_status(FeatureState::Waiting, None);
        sender.send_controls_status(FeatureState::Waiting, None);

        // Start fresh log tailers for this connection — each reads backlog + follows.
        // Stopped on disconnect so the next connection gets a clean slate.
        let tailers = logs::start_tailers(&sender);

        // Clear volume override — let the app's own setting take effect
        volume_override_clear();

        // Start input reader for this connection
        let input_sender = sender.clone();
        let input_handle = std::thread::Builder::new()
            .name("input-reader".into())
            .spawn(move || {
                let mut touch_dev = touch::Touch::open(DEFAULT_TOUCH_DEVICE).ok();
                let mut grpc = commands::GrpcClient::new();

                // Send current volume state to host (app value, no override).
                // On failure, only send the error — no VolumeLevel, so the
                // console keeps grpc_error set and disables dependent controls.
                let app_vol = match grpc.get_volume() {
                    Ok(vol) => {
                        input_sender.send_volume(vol, None);
                        input_sender.send_controls_status(FeatureState::Ready, None);
                        vol
                    }
                    Err(err) => {
                        eprintln!("volume query error: {err}");
                        input_sender.send_controls_status(
                            FeatureState::Unavailable,
                            Some(format!("BMC web API unavailable: {err}")),
                        );
                        50
                    }
                };

                loop {
                    match conn.recv() {
                        Ok(Some(msg)) => {
                            handle_host_message(
                                msg,
                                &mut touch_dev,
                                &mut grpc,
                                &input_sender,
                                app_vol,
                            );
                        }
                        Ok(None) => {
                            eprintln!("ipc: host disconnected");
                            break;
                        }
                        Err(e) => {
                            eprintln!("ipc: input read error: {e}");
                            break;
                        }
                    }
                }
            })
            .unwrap_or_else(|e| panic!("failed to spawn input reader thread: {e}"));

        // Delay Wayland capture until a real host is connected. This keeps
        // the compositor's readback path dormant during boot and retries
        // cleanly if the compositor is still starting up.
        let mut capture_error_notified = false;
        let mut wayland = loop {
            if input_handle.is_finished() {
                break None;
            }

            eprintln!("relay: connecting to compositor (WAYLAND_DISPLAY)...");
            match capture::WaylandCapture::connect() {
                Ok(wayland) => {
                    eprintln!(
                        "capture: {}x{} stride={}",
                        wayland.width(),
                        wayland.height(),
                        wayland.stride()
                    );
                    break Some(wayland);
                }
                Err(err) => {
                    eprintln!("capture connect error: {err}");
                    if !capture_error_notified {
                        sender.send_capture_status(
                            FeatureState::Waiting,
                            Some(format!("Waiting for compositor capture: {err}")),
                        );
                        capture_error_notified = true;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        };

        let Some(mut wayland) = wayland.take() else {
            tailers.stop();
            volume_override_write(0);
            continue;
        };
        let stride = Stride(wayland.stride());
        sender.send_capture_status(FeatureState::Ready, None);

        // Frame capture loop — runs until host disconnects.
        // Keeps a copy of the last sent frame to filter virgl
        eprintln!("relay: entering frame loop ({fps} FPS, interval={frame_interval:?})");
        let mut capture_blocked = false;
        loop {
            if capture_blocked {
                if input_handle.is_finished() {
                    eprintln!("relay: host disconnected, waiting for next connection");
                    break;
                }
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }

            let frame_start = Instant::now();

            let pixels = match wayland.capture_frame() {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("capture error: {e}");
                    sender.send_capture_status(
                        FeatureState::Unavailable,
                        Some(format!("Display capture unavailable: {e}")),
                    );
                    capture_blocked = true;
                    continue;
                }
            };

            frame_count += 1;
            let brightness = backlight.load(Ordering::Relaxed);
            let header = FrameHeader {
                seq: frame_count,
                width: FB_WIDTH,
                height: FB_HEIGHT,
                stride,
                bpp,
                // The compositor's capture readback uses
                // `glReadPixels(GL_RGBA, GL_UNSIGNED_BYTE)` everywhere — see
                // `update_capture_cache` in
                // bmc-openwrt/src/compositor/scene_renderer.rs. The SHM
                // buffer the compositor copies into is labelled Xrgb8888
                // (BGRA byte order) but the bytes inside are actually
                // R, G, B, A. Tell the host the truth so it can upload the
                // texture with the right source format.
                format: bmc_virt_ipc::PixelFormat::Rgba8888,
                brightness,
            };
            sender.send_frame(header, pixels.to_vec());

            let elapsed = frame_start.elapsed();
            if let Some(remaining) = frame_interval.checked_sub(elapsed) {
                std::thread::sleep(remaining);
            }

            // Check if the input reader thread has exited (host disconnected)
            if input_handle.is_finished() {
                eprintln!("relay: host disconnected, waiting for next connection");
                break;
            }
        }

        // Stop log tailers — next connection will spawn fresh ones with backlog
        tailers.stop();

        // Mute audio when console disconnects
        volume_override_write(0);
    }
}

fn handle_host_message(
    msg: HostMessage,
    touch: &mut Option<touch::Touch>,
    grpc: &mut commands::GrpcClient,
    sender: &bmc_virt_ipc::GuestSender,
    app_vol: u8,
) {
    use bmc_virt_ipc::buttons;

    match msg {
        HostMessage::Input(event) => match event {
            InputEvent::TouchDown { x, y } => {
                if let Some(t) = touch {
                    t.down(x, y);
                }
            }
            InputEvent::TouchMove { x, y } => {
                if let Some(t) = touch {
                    t.move_to(x, y);
                }
            }
            InputEvent::TouchUp => {
                if let Some(t) = touch {
                    t.up();
                }
            }
            InputEvent::ButtonPress { button, data } => match button {
                buttons::LED_EFFECT_SET => {
                    let idx = data as usize;
                    if let Some(preset) = commands::PRESETS.get(idx) {
                        eprintln!("LED effect: {}", preset.name);
                        match grpc.set_effect(preset) {
                            Ok(()) => {
                                sender.send_controls_status(FeatureState::Ready, None);
                                sender.send_active_effect(data);
                            }
                            Err(err) => {
                                eprintln!("LED effect error: {err}");
                                sender.send_controls_status(
                                    FeatureState::Unavailable,
                                    Some(format!("LED effect failed: {err}")),
                                );
                                sender.send_notify(
                                    NotifyLevel::Error,
                                    format!("LED effect failed: {err}"),
                                );
                            }
                        }
                    }
                }
                buttons::LED_EFFECT_CLEAR => {
                    eprintln!("LED effect: off");
                    match grpc.clear_effect() {
                        Ok(()) => {
                            sender.send_controls_status(FeatureState::Ready, None);
                            sender.send_active_effect(0xFF);
                        }
                        Err(err) => {
                            eprintln!("LED clear error: {err}");
                            sender.send_controls_status(
                                FeatureState::Unavailable,
                                Some(format!("LED clear failed: {err}")),
                            );
                            sender.send_notify(
                                NotifyLevel::Error,
                                format!("LED clear failed: {err}"),
                            );
                        }
                    }
                }
                buttons::VOLUME_SET => {
                    eprintln!("volume override: {data}");
                    volume_override_write(data);
                    sender.send_volume(app_vol, Some(data));
                }
                buttons::VOLUME_RESET => {
                    eprintln!("volume override: cleared");
                    volume_override_clear();
                    sender.send_volume(app_vol, None);
                }
                other => eprintln!("unknown button: {other}"),
            },
        },
        HostMessage::RunCommand(cmd) => {
            eprintln!("running command: {cmd}");
            let _ = std::process::Command::new("sh").args(["-c", &cmd]).status();
        }
        HostMessage::GpioButton { pressed } => {
            inject_button_uevent(pressed);
        }
        HostMessage::Ping => {
            sender.send_pong();
        }
    }
}

// ── GPIO button uevent injection ─────────────────────────────────────────

/// Inject a kobject uevent for the reset button.
///
/// Sends unicast to every `NETLINK_KOBJECT_UEVENT` listener found in
/// `/proc/net/netlink` (multicast from userspace doesn't reliably
/// deliver to other userspace sockets on all kernels).
#[expect(
    clippy::cast_possible_truncation,
    reason = "libc sockaddr constants are small fixed values"
)]
fn inject_button_uevent(pressed: bool) {
    const NETLINK_KOBJECT_UEVENT: libc::c_int = 15;
    const SOCKADDR_NL_SIZE: libc::socklen_t =
        std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t;

    let action = if pressed { "pressed" } else { "released" };

    // Build uevent packet: null-delimited key=value pairs, matching the
    // format produced by OpenWrt's gpio-button-hotplug kernel module.
    let packet = format!("{action}@/\0ACTION={action}\0SUBSYSTEM=button\0BUTTON=reset\0");
    let packet = packet.as_bytes();

    // Find all NETLINK_KOBJECT_UEVENT listener PIDs from /proc/net/netlink.
    // Format: sk Eth Pid Groups Rmem Wmem Dump Locks Drops Inode
    // We want proto=15 with groups & 1 != 0 and pid != 0 (skip kernel socket).
    let pids = match std::fs::read_to_string("/proc/net/netlink") {
        Ok(content) => content
            .lines()
            .skip(1)
            .filter_map(|line| {
                let mut cols = line.split_whitespace();
                let _sk = cols.next()?;
                let proto: i32 = cols.next()?.parse().ok()?;
                let pid: u32 = cols.next()?.parse().ok()?;
                let groups: u32 = u32::from_str_radix(cols.next()?, 16).ok()?;
                if proto == NETLINK_KOBJECT_UEVENT && pid != 0 && groups & 1 != 0 {
                    Some(pid)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>(),
        Err(e) => {
            eprintln!("gpio: failed to read /proc/net/netlink: {e}");
            return;
        }
    };

    if pids.is_empty() {
        eprintln!("gpio: no NETLINK_KOBJECT_UEVENT listeners found");
        return;
    }

    unsafe {
        let fd = libc::socket(libc::AF_NETLINK, libc::SOCK_DGRAM, NETLINK_KOBJECT_UEVENT);
        if fd < 0 {
            eprintln!("gpio: failed to create netlink socket");
            return;
        }

        let mut src: libc::sockaddr_nl = std::mem::zeroed();
        src.nl_family = libc::AF_NETLINK as u16;
        src.nl_pid = std::process::id();

        if libc::bind(fd, std::ptr::from_ref(&src).cast(), SOCKADDR_NL_SIZE) < 0 {
            eprintln!("gpio: failed to bind netlink socket");
            libc::close(fd);
            return;
        }

        // Unicast to each listener
        for pid in &pids {
            let mut dst: libc::sockaddr_nl = std::mem::zeroed();
            dst.nl_family = libc::AF_NETLINK as u16;
            dst.nl_pid = *pid;

            let ret = libc::sendto(
                fd,
                packet.as_ptr().cast(),
                packet.len(),
                0,
                std::ptr::from_ref(&dst).cast(),
                SOCKADDR_NL_SIZE,
            );
            if ret < 0 {
                eprintln!(
                    "gpio: failed to send uevent to pid {pid}: {}",
                    std::io::Error::last_os_error()
                );
            }
        }

        eprintln!("gpio: injected {action} reset → {pids:?}");
        libc::close(fd);
    }
}

// ── Backlight watcher (inotify) ──────────────────────────────────────────

/// Spawn a thread that watches the fake backlight brightness file with inotify.
/// Returns a shared atomic holding the normalized brightness (0–255).
fn start_backlight_watcher() -> Arc<AtomicU8> {
    let brightness = Arc::new(AtomicU8::new(u8::MAX));
    let shared = Arc::clone(&brightness);

    std::thread::Builder::new()
        .name("backlight-watch".into())
        .spawn(move || backlight_watch_loop(&shared))
        .unwrap_or_else(|e| panic!("failed to spawn backlight watcher: {e}"));

    brightness
}

fn backlight_watch_loop(brightness: &AtomicU8) {
    // Wait for the fake backlight directory to appear (VM init may lag)
    while !std::path::Path::new(BACKLIGHT_DIR).exists() {
        std::thread::sleep(Duration::from_secs(1));
    }

    // Read max_brightness once (fixed hardware property)
    let max = read_sysfs_u16(BACKLIGHT_MAX_PATH).unwrap_or(255);
    eprintln!("backlight: max_brightness={max}");

    // Helper: read raw brightness and normalize to 0–255
    let read_normalized = || -> u8 {
        let raw = read_sysfs_u16(BACKLIGHT_BRIGHTNESS_PATH).unwrap_or(max);
        if max == 0 {
            return u8::MAX;
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::integer_division,
            reason = "result is clamped to 0..=255"
        )]
        {
            (raw.min(max) * 255 / max) as u8
        }
    };

    // Read initial value
    let initial = read_normalized();
    brightness.store(initial, Ordering::Relaxed);
    eprintln!("backlight: initial brightness={initial}/255");

    // Set up inotify
    let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
    if fd < 0 {
        eprintln!("backlight: inotify_init1 failed, falling back to no dimming");
        return;
    }

    let c_path = std::ffi::CString::new(BACKLIGHT_BRIGHTNESS_PATH)
        .unwrap_or_else(|e| panic!("invalid backlight path: {e}"));
    let wd = unsafe {
        libc::inotify_add_watch(fd, c_path.as_ptr(), libc::IN_MODIFY | libc::IN_CLOSE_WRITE)
    };
    if wd < 0 {
        eprintln!("backlight: inotify_add_watch failed, falling back to no dimming");
        unsafe { libc::close(fd) };
        return;
    }

    eprintln!("backlight: watching {BACKLIGHT_BRIGHTNESS_PATH}");

    // Block on inotify events, re-read and normalize brightness on each change
    let mut buf = [0_u8; 256];
    loop {
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n <= 0 {
            break;
        }
        brightness.store(read_normalized(), Ordering::Relaxed);
    }

    unsafe { libc::close(fd) };
}

/// Check whether two frames differ enough to warrant sending.
///
/// Samples a sparse set of pixels and declares the frame "changed" only
/// if enough of them differ by more than a small threshold, filtering
/// Read a small sysfs-style file containing a single integer value.
fn read_sysfs_u16(path: &str) -> Option<u16> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

// ── Volume override file helpers ─────────────────────────────────────────

/// Write a volume percentage (0–100) to the override file.
fn volume_override_write(pct: u8) {
    if let Err(e) = std::fs::write(VOLUME_OVERRIDE_PATH, format!("{pct}")) {
        eprintln!("failed to write volume override: {e}");
    }
}

/// Clear the override file (empty = use app's own volume).
fn volume_override_clear() {
    if let Err(e) = std::fs::write(VOLUME_OVERRIDE_PATH, "") {
        eprintln!("failed to clear volume override: {e}");
    }
}
