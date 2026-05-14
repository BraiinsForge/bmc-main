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
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_FPS: u32 = 60;
const DEFAULT_SPI_CAPTURE: &str = "/proc/bmc_virt_spi0";
const LED_CAPTURE_RETRY_DELAY: Duration = Duration::from_secs(1);

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

    // 2. Verify a touchscreen evdev node is reachable at startup. The
    //    actual fd is opened per host connection below; this is a probe.
    match touch::discover_touch_node() {
        Some(path) => eprintln!("touch: discovered {}", path.display()),
        None => eprintln!("WARNING: no touchscreen evdev node found; touch injection disabled"),
    }

    // 3. Start LED capture thread (runs forever, drops when no host)
    let latest_led = Arc::new(Mutex::new(None::<LedUpdate>));
    {
        let led_sender = sender.clone();
        let latest_led_capture = Arc::clone(&latest_led);
        std::thread::Builder::new()
            .name("led-capture".into())
            .spawn(move || run_led_capture_loop(&led_sender, &latest_led_capture))
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
        if let Some(update) = latest_led
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            sender.send_leds(update);
        }

        // Start fresh log tailers for this connection — each reads backlog + follows.
        // Stopped on disconnect so the next connection gets a clean slate.
        let tailers = logs::start_tailers(&sender);

        // Clear volume override — let the app's own setting take effect
        volume_override_clear();

        // Shared app-volume baseline. The BMC web service ships our initial
        // value (and updates after settings changes); we cache it here so
        // command handlers can mix it with the console's override without
        // re-querying gRPC on every InputEvent. Defaults to 50 until the
        // probe thread reads the real value.
        let app_vol_state = Arc::new(AtomicU8::new(50));

        // Lets the parallel probe thread (below) know when this connection
        // is torn down so it can exit instead of polling against a guest
        // that's already past this loop iteration.
        let connection_running = Arc::new(AtomicBool::new(true));

        // Start input reader for this connection
        let input_sender = sender.clone();
        let input_app_vol = Arc::clone(&app_vol_state);
        let input_done_flag = Arc::clone(&connection_running);
        let input_handle = std::thread::Builder::new()
            .name("input-reader".into())
            .spawn(move || {
                let mut touch_dev =
                    touch::discover_touch_node().and_then(|path| touch::Touch::open(&path).ok());
                let mut grpc = commands::GrpcClient::new();

                loop {
                    match conn.recv() {
                        Ok(Some(msg)) => {
                            handle_host_message(
                                msg,
                                &mut touch_dev,
                                &mut grpc,
                                &input_sender,
                                input_app_vol.load(Ordering::Relaxed),
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
                input_done_flag.store(false, Ordering::Relaxed);
            })
            .unwrap_or_else(|e| panic!("failed to spawn input reader thread: {e}"));

        // Polling probe for the BMC web service. Mirrors `establish_capture`
        // (compositor) — sends Waiting once on the first failure, then Ready
        // (with the real app volume) when the service comes up. Runs in its
        // own thread so a slow boot doesn't stall the input reader.
        //
        // Per-connection: a fresh console attach reprobes, and the thread
        // exits on host disconnect via `connection_running`. Mid-session
        // service failures are still surfaced ad-hoc by the existing button
        // handlers in `handle_host_message`.
        let probe_sender = sender.clone();
        let probe_app_vol = Arc::clone(&app_vol_state);
        let probe_running = Arc::clone(&connection_running);
        std::thread::Builder::new()
            .name("bmc-probe".into())
            .spawn(move || {
                let mut grpc = commands::GrpcClient::new();
                let mut waiting_notified = false;
                while probe_running.load(Ordering::Relaxed) {
                    match grpc.get_volume() {
                        Ok(vol) => {
                            probe_app_vol.store(vol, Ordering::Relaxed);
                            probe_sender.send_volume(vol, None);
                            probe_sender.send_controls_status(FeatureState::Ready, None);
                            return;
                        }
                        Err(err) => {
                            if !waiting_notified {
                                eprintln!("relay: BMC web API not ready yet: {err}");
                                probe_sender.send_controls_status(
                                    FeatureState::Waiting,
                                    Some(format!("Waiting for BMC web API ({err})")),
                                );
                                waiting_notified = true;
                            }
                            thread::sleep(Duration::from_millis(500));
                        }
                    }
                }
            })
            .unwrap_or_else(|e| panic!("failed to spawn bmc-probe thread: {e}"));

        // Delay Wayland capture until a real host is connected. This keeps
        // the compositor's readback path dormant during boot and retries
        // cleanly if the compositor is still starting up. The helper is
        // shared with the mid-run reconnect path below, so a compositor
        // restart (#BDK-420) is recovered through the same code.
        let Some(mut wayland) = establish_capture(&input_handle, &sender) else {
            tailers.stop();
            volume_override_write(0);
            continue;
        };
        let mut stride = Stride(wayland.stride());

        // Frame capture loop — runs until host disconnects.
        // Keeps a copy of the last sent frame to filter virgl
        eprintln!("relay: entering frame loop ({fps} FPS, interval={frame_interval:?})");
        loop {
            let frame_start = Instant::now();

            let pixels = match wayland.capture_frame() {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("capture error: {e}");
                    sender.send_capture_status(
                        FeatureState::Unavailable,
                        Some(format!("Display capture unavailable: {e}")),
                    );
                    sender.send_notify(
                        NotifyLevel::Warning,
                        format!("Compositor capture lost ({e}); reconnecting"),
                    );
                    let Some(new_wayland) = establish_capture(&input_handle, &sender) else {
                        eprintln!("relay: host disconnected during reconnect");
                        break;
                    };
                    wayland = new_wayland;
                    stride = Stride(wayland.stride());
                    sender.send_notify(
                        NotifyLevel::Info,
                        format!(
                            "Compositor reconnected ({}x{})",
                            wayland.width(),
                            wayland.height()
                        ),
                    );
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

/// Establish a Wayland capture session, retrying until it succeeds or the
/// host disconnects. Shared between the initial connect on host accept and
/// the mid-run reconnect when the compositor is restarted out from under
/// the relay (#BDK-420).
///
/// The relay owns compositor discovery: it scans `$XDG_RUNTIME_DIR` for a
/// `wayland-*` socket on each attempt and connects directly to that socket.
/// This keeps the init script dumb and lets the same retry loop handle both
/// boot-time startup ordering and compositor reconnects.
fn establish_capture(
    input_handle: &thread::JoinHandle<()>,
    sender: &bmc_virt_ipc::GuestSender,
) -> Option<capture::WaylandCapture> {
    let runtime_dir = env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    let runtime_dir = PathBuf::from(runtime_dir);

    let mut waiting_notified = false;
    loop {
        if input_handle.is_finished() {
            return None;
        }

        let display = match discover_wayland_display(&runtime_dir) {
            Ok(Some(display)) => display,
            Ok(None) => {
                if !waiting_notified {
                    let reason =
                        format!("Waiting for compositor socket in {}", runtime_dir.display());
                    eprintln!("relay: {reason}");
                    sender.send_capture_status(FeatureState::Waiting, Some(reason));
                    waiting_notified = true;
                }
                thread::sleep(Duration::from_millis(500));
                continue;
            }
            Err(err) => {
                if !waiting_notified {
                    eprintln!("relay: {err}");
                    sender.send_capture_status(FeatureState::Waiting, Some(err.clone()));
                    waiting_notified = true;
                }
                thread::sleep(Duration::from_millis(500));
                continue;
            }
        };
        let socket = runtime_dir.join(&display);

        eprintln!(
            "relay: connecting to compositor ({} at {})",
            display,
            socket.display()
        );
        match capture::WaylandCapture::connect(&socket) {
            Ok(wayland) => {
                eprintln!(
                    "capture: {}x{} stride={}",
                    wayland.width(),
                    wayland.height(),
                    wayland.stride()
                );
                sender.send_capture_status(FeatureState::Ready, None);
                return Some(wayland);
            }
            Err(err) => {
                eprintln!("capture connect error: {err}");
                if !waiting_notified {
                    sender.send_capture_status(
                        FeatureState::Waiting,
                        Some(format!(
                            "Waiting for compositor capture on {}: {err}",
                            socket.display()
                        )),
                    );
                    waiting_notified = true;
                }
                thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

fn discover_wayland_display(runtime_dir: &Path) -> Result<Option<String>, String> {
    let entries = fs::read_dir(runtime_dir)
        .map_err(|e| format!("failed to list {}: {e}", runtime_dir.display()))?;
    let mut displays = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let file_name = entry.file_name();
            let file_name = file_name.to_str()?;
            let suffix = file_name.strip_prefix("wayland-")?;
            let n: u32 = suffix.parse().ok()?;
            Some((n, file_name.to_owned()))
        })
        .collect::<Vec<_>>();
    displays.sort_unstable_by_key(|(n, _)| *n);
    Ok(displays.into_iter().next().map(|(_, name)| name))
}

fn run_led_capture_loop(
    led_sender: &bmc_virt_ipc::GuestSender,
    latest_led_capture: &Mutex<Option<LedUpdate>>,
) {
    let mut led_seq: u64 = 0;

    loop {
        let mut decoder = bmc_virt_leds::apa102::Decoder::new();
        let result = bmc_virt_leds::proc_stream::run(DEFAULT_SPI_CAPTURE, |data| {
            if let Some(leds) = decoder.feed(data) {
                publish_led_update(&leds, &mut led_seq, led_sender, latest_led_capture);
                while let Some(leds) = decoder.feed(&[]) {
                    publish_led_update(&leds, &mut led_seq, led_sender, latest_led_capture);
                }
            }
        });

        if let Err(err) = result {
            eprintln!(
                "LED capture error from {DEFAULT_SPI_CAPTURE}: {err}; retrying in {LED_CAPTURE_RETRY_DELAY:?}"
            );
        } else {
            eprintln!(
                "LED capture stream ended unexpectedly; retrying in {LED_CAPTURE_RETRY_DELAY:?}"
            );
        }
        thread::sleep(LED_CAPTURE_RETRY_DELAY);
    }
}

fn publish_led_update(
    leds: &[bmc_virt_leds::apa102::Led; LED_COUNT],
    led_seq: &mut u64,
    led_sender: &bmc_virt_ipc::GuestSender,
    latest_led_capture: &Mutex<Option<LedUpdate>>,
) {
    let mut led_state = [LedState::default(); LED_COUNT];
    for (i, led) in leds.iter().enumerate() {
        led_state[i].brightness = led.brightness;
        led_state[i].r = led.r;
        led_state[i].g = led.g;
        led_state[i].b = led.b;
    }
    *led_seq += 1;
    let update = LedUpdate {
        seq: *led_seq,
        leds: led_state,
    };
    let mut guard = latest_led_capture
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *guard = Some(update.clone());
    drop(guard);
    led_sender.send_leds(update);
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

#[cfg(test)]
mod tests {
    use super::discover_wayland_display;

    #[test]
    fn picks_the_first_wayland_socket_name() {
        let runtime_dir = tempfile::tempdir().expect("BUG: create tempdir for relay test");
        std::fs::write(runtime_dir.path().join("wayland-1"), b"")
            .expect("BUG: create second socket placeholder");
        std::fs::write(runtime_dir.path().join("wayland-0"), b"")
            .expect("BUG: create first socket placeholder");
        std::fs::write(runtime_dir.path().join("not-wayland"), b"")
            .expect("BUG: create unrelated placeholder");

        let display = discover_wayland_display(runtime_dir.path()).expect("BUG: list tempdir");

        assert_eq!(display.as_deref(), Some("wayland-0"));
    }

    #[test]
    fn returns_none_when_no_wayland_socket_exists() {
        let runtime_dir = tempfile::tempdir().expect("BUG: create tempdir for relay test");
        std::fs::write(runtime_dir.path().join("other"), b"")
            .expect("BUG: create unrelated placeholder");

        let display = discover_wayland_display(runtime_dir.path()).expect("BUG: list tempdir");

        assert_eq!(display, None);
    }
}
