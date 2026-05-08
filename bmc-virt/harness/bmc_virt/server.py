"""Guest-side event daemon — TCP server, poller, command dispatcher.

Runs inside the VM. Listens on TCP for a single client, polls system state
for lifecycle events, accepts signals from init scripts via a Unix socket,
and dispatches commands from the host.

Usage: python3 -m bmc_virt.server
"""

import contextlib
import json
import logging
import os
import signal
import socket
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Any

from bmc_virt.commands import Cmd
from bmc_virt.events import Event
from bmc_virt.paths import BMC_BIN, BMC_LOG, BMC_PID_FILE, RR_BUNDLE, RR_TRACE_DIR
from bmc_virt.protocol import Msg, MsgType
from bmc_virt.protocol import ack as mk_ack
from bmc_virt.protocol import event as mk_event
from bmc_virt.protocol import hello as mk_hello
from bmc_virt.protocol import shutdown as mk_shutdown
from bmc_virt.protocol import synced as mk_synced

log = logging.getLogger("bmc-virt-eventd")

# ── Configuration ──────────────────────────────────────────────────────────────

LISTEN_PORT = 5920
LISTEN_HOST = "0.0.0.0"
UNIX_SOCKET_PATH = Path("/var/run/bmc-virt-eventd.sock")
POLL_INTERVAL = 0.5  # seconds

# rr-related aliases kept for local readability. Paths themselves live in
# bmc_virt.paths so they stay synchronised with flake.nix `guestPaths`.
RR_PID_FILE = BMC_PID_FILE
BMC_OPENWRT_BIN = BMC_BIN


# ── Daemon ─────────────────────────────────────────────────────────────────────


class EventDaemon:
    """The guest-side event daemon."""

    def __init__(self) -> None:
        self._backlog: list[Msg] = []
        self._backlog_lock = threading.Lock()
        self._client_sock: socket.socket | None = None
        self._client_lock = threading.Lock()
        self._running = True
        self._state = _PollState()

    def run(self) -> None:
        """Main entry point — blocks until SIGTERM/SIGINT."""
        signal.signal(signal.SIGTERM, self._handle_signal)
        signal.signal(signal.SIGINT, self._handle_signal)

        # Start poller and unix socket threads
        threading.Thread(target=self._poller_loop, daemon=True, name="poller").start()
        threading.Thread(target=self._unix_socket_loop, daemon=True, name="unix-sock").start()

        # TCP server — main thread
        server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        server.settimeout(1.0)
        server.bind((LISTEN_HOST, LISTEN_PORT))
        server.listen(1)

        try:
            while self._running:
                try:
                    client, _addr = server.accept()
                except TimeoutError:
                    continue

                # Reject if already connected
                with self._client_lock:
                    if self._client_sock is not None:
                        with contextlib.suppress(OSError):
                            client.sendall(
                                mk_ack("", ok=False, error="another client is connected").to_line()
                            )
                        client.close()
                        continue
                    self._client_sock = client

                self._handle_client(client)
        finally:
            server.close()
            self._cleanup()

    # ── Client handling ────────────────────────────────────────────────────────

    def _handle_client(self, sock: socket.socket) -> None:
        """Serve a single client: handshake, replay backlog, then stream."""
        try:
            # Handshake
            sock.sendall(mk_hello().to_line())

            # Replay backlog
            with self._backlog_lock:
                for msg in self._backlog:
                    sock.sendall(msg.to_line())
            sock.sendall(mk_synced().to_line())

            # Read commands from client
            buf = b""
            sock.settimeout(None)
            while self._running:
                try:
                    chunk = sock.recv(65_536)
                except OSError:
                    break
                if not chunk:
                    break
                buf += chunk
                while b"\n" in buf:
                    line, buf = buf.split(b"\n", 1)
                    if not line:
                        continue
                    try:
                        msg = Msg.from_line(line)
                    except (ValueError, KeyError):
                        continue
                    if msg.type == MsgType.CMD:
                        # Dispatch in a thread so long-running commands
                        # don't block the recv loop
                        threading.Thread(
                            target=self._dispatch_command,
                            args=(msg,),
                            daemon=True,
                        ).start()
        except OSError:
            pass
        finally:
            with self._client_lock:
                self._client_sock = None
            sock.close()

    # ── Event emission ─────────────────────────────────────────────────────────

    def emit(self, name: Event, data: dict[str, Any] | None = None) -> None:
        """Record an event and send it to the connected client."""
        log.info("EVENT %s %s", name, data or "")
        msg = mk_event(name, data)
        with self._backlog_lock:
            self._backlog.append(msg)
        self._send_to_client(msg)

    # ── System state poller ────────────────────────────────────────────────────

    def _poller_loop(self) -> None:
        """Poll system state and emit events on transitions."""
        while self._running:
            self._poll_once()
            time.sleep(POLL_INTERVAL)

    def _poll_once(self) -> None:
        """Check all monitored state and emit events for changes."""
        s = self._state

        # app.started — bmc-openwrt process exists (re-emits after restart)
        app_up = _process_exists("bmc-openwrt")
        if app_up and not s.app_started:
            s.app_started = True
            pid = _pgrep("bmc-openwrt")
            self.emit(Event.APP_STARTED, {"pid": pid} if pid else None)
        elif not app_up and s.app_started:
            s.app_started = False
            s.app_ready = False
            self.emit(Event.SERVICE_STOPPED, {"name": "bmc-openwrt"})

        # app.ready — HTTP port 80 is listening (gRPC-Web multiplexed on the
        # same port, so a single probe covers both). Re-emits after restart.
        http_up = _port_listening(80)
        if s.app_started and http_up and not s.app_ready:
            s.app_ready = True
            self.emit(Event.APP_READY)
        elif not http_up and s.app_ready:
            s.app_ready = False

        # relay.listening — port 5910 (re-emits after restart)
        relay_up = _port_listening(5910)
        if relay_up and not s.relay_listening:
            s.relay_listening = True
            self.emit(Event.RELAY_LISTENING)
        elif not relay_up and s.relay_listening:
            s.relay_listening = False

        # wifi.associated — wlan0 has a link (re-emits after reconnect)
        info = _iw_link_info()
        if info and not s.wifi_associated:
            s.wifi_associated = True
            self.emit(Event.WIFI_ASSOCIATED, info)
        elif not info and s.wifi_associated:
            s.wifi_associated = False
            s.wifi_got_ip = False

        # wifi.got_ip — wlan0 has an IP (re-emits after reconnect)
        if s.wifi_associated and not s.wifi_got_ip:
            ip = _interface_ip("wlan0")
            if ip:
                s.wifi_got_ip = True
                self.emit(Event.WIFI_GOT_IP, {"iface": "wlan0", "ip": ip})

    # ── Unix socket for init script signals ────────────────────────────────────

    def _unix_socket_loop(self) -> None:
        """Listen on a Unix socket for signals from init scripts."""
        if UNIX_SOCKET_PATH.exists():
            UNIX_SOCKET_PATH.unlink()
        UNIX_SOCKET_PATH.parent.mkdir(parents=True, exist_ok=True)

        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.settimeout(1.0)
        sock.bind(str(UNIX_SOCKET_PATH))
        sock.listen(5)

        try:
            while self._running:
                try:
                    client, _ = sock.accept()
                except TimeoutError:
                    continue
                try:
                    data = client.recv(4_096)
                    if data:
                        self._handle_unix_message(data)
                except OSError:
                    pass
                finally:
                    client.close()
        finally:
            sock.close()
            if UNIX_SOCKET_PATH.exists():
                UNIX_SOCKET_PATH.unlink()

    def _handle_unix_message(self, data: bytes) -> None:
        """Parse and handle a message from the Unix socket."""
        try:
            obj = json.loads(data)
            name = Event(obj["name"])
            self.emit(name, obj.get("data"))
        except (ValueError, KeyError, json.JSONDecodeError):
            pass

    # ── Command dispatcher ─────────────────────────────────────────────────────

    def _dispatch_command(self, msg: Msg) -> None:
        """Handle an incoming command from the host."""
        request_id = msg.id or ""
        name = msg.name or ""
        data = msg.data
        log.info("CMD %s id=%s data=%s", name, request_id, data)

        if name == Cmd.SHELL_EXEC:
            self._cmd_shell_exec(request_id, data)
        elif name == Cmd.SERVICE_RESTART:
            self._cmd_service_restart(request_id, data)
        elif name == Cmd.METRICS_COLLECT:
            self._cmd_metrics_collect(request_id, data)
        elif name == Cmd.RR_START:
            self._cmd_rr_start(request_id)
        elif name == Cmd.RR_STOP:
            self._cmd_rr_stop(request_id)
        else:
            self._send_ack(request_id, ok=False, error=f"unknown command: {name}")

    def _cmd_shell_exec(self, request_id: str, data: dict[str, Any]) -> None:
        """Execute a shell command and return the result."""
        cmd_str = str(data.get("cmd", ""))
        timeout = float(data.get("timeout", 30))
        try:
            result = subprocess.run(
                cmd_str,
                shell=True,
                capture_output=True,
                text=True,
                timeout=timeout,
                check=False,
            )
            self._send_ack(
                request_id,
                ok=result.returncode == 0,
                data={
                    "exit_code": result.returncode,
                    "stdout": result.stdout,
                    "stderr": result.stderr,
                },
            )
        except subprocess.TimeoutExpired:
            self._send_ack(request_id, ok=False, error=f"command timed out after {timeout}s")

    def _cmd_service_restart(self, request_id: str, data: dict[str, Any]) -> None:
        """Restart a procd service."""
        name = str(data.get("name", ""))
        if not name:
            self._send_ack(request_id, ok=False, error="missing service name")
            return
        try:
            # Reset poll state so the poller re-emits events after restart.
            # Without this, a fast restart can complete between poll ticks
            # and the poller never sees the service go down.
            self._state.reset_service(name)

            subprocess.run(
                ["service", name, "restart"],
                capture_output=True,
                text=True,
                timeout=30,
                check=True,
            )
            self._send_ack(request_id, ok=True)
        except subprocess.CalledProcessError as exc:
            self._send_ack(request_id, ok=False, error=f"service restart failed: {exc.stderr}")
        except subprocess.TimeoutExpired:
            self._send_ack(request_id, ok=False, error="service restart timed out")

    # ── Metrics ──────────────────────────────────────────────────────────────

    def _cmd_metrics_collect(self, request_id: str, data: dict[str, Any]) -> None:
        """Read /proc directly and return a structured snapshot.

        Cheaper and more robust than shelling out to ``pidof``/``cat``/``awk``
        — busybox flag availability differs across builds, and shelling out
        per snapshot adds non-trivial overhead at 0.5 s polling cadence.
        """
        raw_processes = data.get("processes") or []
        processes = [str(p) for p in raw_processes]
        try:
            payload = {
                "meminfo": _read_meminfo(),
                "loadavg": list(_read_loadavg()),
                "uptime_s": _read_uptime(),
                "processes": _read_processes(processes),
            }
        except OSError as exc:
            self._send_ack(request_id, ok=False, error=f"metrics read failed: {exc}")
            return
        self._send_ack(request_id, ok=True, data=payload)

    # ── rr time-travel debugger ──────────────────────────────────────────────

    def _cmd_rr_start(self, request_id: str) -> None:
        """Stop bmc-openwrt, start recording under rr with headless compositor."""
        run_rr = RR_BUNDLE / "bin" / "run-rr.sh"
        if not run_rr.exists():
            self._send_ack(
                request_id,
                ok=False,
                error=f"rr bundle not deployed (missing {run_rr}), start VM with --rr",
            )
            return

        # Kill any running bmc-openwrt (procd or direct)
        subprocess.run(["killall", "bmc-openwrt"], capture_output=True, timeout=5, check=False)
        # Also kill a leftover rr process from a previous recording
        subprocess.run(["killall", "rr"], capture_output=True, timeout=5, check=False)
        time.sleep(1)

        # Reset poller so it re-emits APP_STARTED when the rr-wrapped process comes up
        self._state.app_started = False
        self._state.app_ready = False

        # Clear old trace — rr creates --output-trace-dir itself and
        # refuses if it already exists.
        subprocess.run(
            ["rm", "-rf", str(RR_TRACE_DIR)],
            capture_output=True,
            timeout=5,
            check=False,
        )

        # Start bmc-openwrt under rr via start-stop-daemon.
        # --headless-compositor: rr hides /dev/dri from recorded
        # processes, so the compositor runs Wayland protocol only.
        result = subprocess.run(
            [
                "/sbin/start-stop-daemon",
                "-S",
                "-b",
                "-m",
                "-p",
                str(RR_PID_FILE),
                "-x",
                "/bin/sh",
                "--",
                "-c",
                (
                    f"exec env"
                    f" RUST_BACKTRACE=full"
                    f" RUST_LOG=debug"
                    f" XDG_RUNTIME_DIR=/run/user/0"
                    f" BMC_WIFI_SYSPATH=/sys/devices/virtual/mac80211_hwsim/hwsim0/"
                    f" {run_rr} record -n"
                    f" --output-trace-dir={RR_TRACE_DIR}"
                    f" --resource-path={RR_BUNDLE}"
                    f" {BMC_OPENWRT_BIN} --headless-compositor"
                    f" > {BMC_LOG} 2>&1"
                ),
            ],
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )

        if result.returncode != 0:
            self.emit(Event.RR_FAILED, {"error": result.stderr.strip()})
            self._send_ack(
                request_id,
                ok=False,
                error=f"start-stop-daemon failed: {result.stderr.strip()}",
            )
            return

        self.emit(Event.RR_RECORDING)
        self._send_ack(request_id, ok=True)

    def _cmd_rr_stop(self, request_id: str) -> None:
        """Stop rr recording — SIGTERM the rr process so it finalizes the trace."""
        rr_pid = _pgrep("rr record")
        if rr_pid is None:
            self._send_ack(request_id, ok=False, error="rr is not recording")
            return

        # SIGTERM rr (not bmc-openwrt directly) so rr finalizes the trace cleanly.
        # rr forwards the signal to the child and writes a complete recording.
        try:
            os.kill(rr_pid, signal.SIGTERM)
        except OSError as exc:
            self._send_ack(request_id, ok=False, error=f"failed to signal rr: {exc}")
            return

        # Wait for rr to exit and finalize the trace
        for _ in range(20):
            time.sleep(0.5)
            if _pgrep("rr record") is None:
                break
        else:
            self.emit(Event.RR_FAILED, {"error": "rr did not exit within 10s"})
            self._send_ack(request_id, ok=False, error="rr did not exit within 10s")
            return

        # Reset poller — app is gone
        self._state.app_started = False
        self._state.app_ready = False

        # --output-trace-dir makes RR_TRACE_DIR the trace itself (not a parent)
        trace_path = str(RR_TRACE_DIR) if RR_TRACE_DIR.exists() else None

        self.emit(Event.RR_STOPPED, {"trace": trace_path})
        self._send_ack(request_id, ok=True, data={"trace": trace_path})

    def _send_ack(
        self,
        request_id: str,
        *,
        ok: bool = True,
        data: dict[str, Any] | None = None,
        error: str | None = None,
    ) -> None:
        """Send an ack to the connected client."""
        log.info("ACK id=%s ok=%s error=%s", request_id, ok, error)
        msg = mk_ack(request_id, ok=ok, data=data, error=error)
        self._send_to_client(msg)

    def _send_to_client(self, msg: Msg) -> None:
        """Send a message to the connected client, silently ignoring errors."""
        with self._client_lock:
            if self._client_sock is not None:
                with contextlib.suppress(OSError):
                    self._client_sock.sendall(msg.to_line())

    # ── Signal handling ────────────────────────────────────────────────────────

    def _handle_signal(self, _signum: int, _frame: object) -> None:
        """Handle SIGTERM/SIGINT — emit shutdown and stop."""
        self.emit(Event.SHUTDOWN, {"reason": "signal"})
        self._send_to_client(mk_shutdown("daemon stopping"))
        self._running = False

    def _cleanup(self) -> None:
        """Clean up resources."""
        with self._client_lock:
            if self._client_sock is not None:
                with contextlib.suppress(OSError):
                    self._client_sock.close()
                self._client_sock = None


# ── Poll state ─────────────────────────────────────────────────────────────────


class _PollState:
    """Tracks what events have already been emitted to avoid duplicates."""

    def __init__(self) -> None:
        self.app_started = False
        self.app_ready = False
        self.relay_listening = False
        self.wifi_associated = False
        self.wifi_got_ip = False

    def reset_service(self, name: str) -> None:
        """Clear poll flags for a service so the poller re-emits on restart."""
        if "relay" in name:
            self.relay_listening = False
        if "bmc-openwrt" in name or "b-bmc-openwrt" in name:
            self.app_started = False
            self.app_ready = False


# ── System probes ──────────────────────────────────────────────────────────────


def _process_exists(name: str) -> bool:
    """Check if a process with the given name exists (BusyBox-compatible)."""
    try:
        # BusyBox pgrep -x matches against full path, not basename.
        # Use pgrep with plain pattern match instead.
        result = subprocess.run(
            ["pgrep", "-f", name],
            capture_output=True,
            timeout=5,
            check=False,
        )
        return result.returncode == 0
    except (OSError, subprocess.TimeoutExpired):
        return False


def _pgrep(name: str) -> int | None:
    """Get the PID of a process by name (BusyBox-compatible)."""
    try:
        result = subprocess.run(
            ["pgrep", "-f", name],
            capture_output=True,
            text=True,
            timeout=5,
            check=False,
        )
        if result.returncode == 0 and result.stdout.strip():
            return int(result.stdout.strip().split()[0])
    except (OSError, subprocess.TimeoutExpired, ValueError):
        pass
    return None


def _port_listening(port: int) -> bool:
    """Check if a TCP port is in LISTEN state (BusyBox netstat)."""
    try:
        result = subprocess.run(
            ["netstat", "-tln"],
            capture_output=True,
            text=True,
            timeout=5,
            check=False,
        )
        return any(
            f":{port} " in line or f":{port}\t" in line for line in result.stdout.splitlines()
        )
    except (OSError, subprocess.TimeoutExpired):
        return False


def _iw_link_info() -> dict[str, str] | None:
    """Check if wlan0 is associated and return link info."""
    try:
        result = subprocess.run(
            ["iw", "dev", "wlan0", "link"],
            capture_output=True,
            text=True,
            timeout=5,
            check=False,
        )
        if result.returncode != 0 or "Not connected" in result.stdout:
            return None
        info: dict[str, str] = {}
        for raw_line in result.stdout.splitlines():
            stripped = raw_line.strip()
            if stripped.startswith("SSID:"):
                info["ssid"] = stripped.split(":", 1)[1].strip()
            elif stripped.startswith("Connected to"):
                info["bssid"] = stripped.split()[2]
        return info or None
    except (OSError, subprocess.TimeoutExpired):
        return None


def _interface_ip(iface: str) -> str | None:
    """Get the first IPv4 address on an interface."""
    try:
        result = subprocess.run(
            ["ip", "-4", "-o", "addr", "show", iface],
            capture_output=True,
            text=True,
            timeout=5,
            check=False,
        )
        if result.returncode != 0:
            return None
        for line in result.stdout.splitlines():
            parts = line.split()
            for i, part in enumerate(parts):
                if part == "inet" and i + 1 < len(parts):
                    return parts[i + 1].split("/")[0]
    except (OSError, subprocess.TimeoutExpired):
        pass
    return None


# ── /proc metrics readers ──────────────────────────────────────────────────────

_LOADAVG_FIELDS = 3
_MEMINFO_FIELDS = (
    "MemTotal",
    "MemFree",
    "MemAvailable",
    "Buffers",
    "Cached",
    "Shmem",
    "CmaTotal",
    "CmaFree",
)
_PROC_STATUS_FIELDS = {
    "VmSize": "vm_size_kb",
    "VmRSS": "vm_rss_kb",
    "RssAnon": "rss_anon_kb",
    "RssFile": "rss_file_kb",
    "RssShmem": "rss_shmem_kb",
}


def _read_meminfo() -> dict[str, int]:
    """Parse /proc/meminfo into a sparse dict keyed by field name."""
    out: dict[str, int] = {}
    try:
        with Path("/proc/meminfo").open() as f:
            text = f.read()
    except OSError:
        return out
    for raw in text.splitlines():
        key, _, val = raw.partition(":")
        key = key.strip()
        if key not in _MEMINFO_FIELDS:
            continue
        parts = val.strip().split()
        if not parts:
            continue
        try:
            out[key] = int(parts[0])
        except ValueError:
            continue
    return out


def _read_loadavg() -> tuple[float, float, float]:
    """Parse /proc/loadavg's first three fields (1m/5m/15m)."""
    try:
        with Path("/proc/loadavg").open() as f:
            parts = f.read().split()
    except OSError:
        return (0.0, 0.0, 0.0)
    if len(parts) < _LOADAVG_FIELDS:
        return (0.0, 0.0, 0.0)
    try:
        return (float(parts[0]), float(parts[1]), float(parts[2]))
    except ValueError:
        return (0.0, 0.0, 0.0)


def _read_uptime() -> float:
    """Parse the first column of /proc/uptime (seconds since boot)."""
    try:
        with Path("/proc/uptime").open() as f:
            parts = f.read().split()
    except OSError:
        return 0.0
    if not parts:
        return 0.0
    try:
        return float(parts[0])
    except ValueError:
        return 0.0


def _read_processes(names: list[str]) -> dict[str, dict[str, Any]]:
    """For each name, find the first matching pid and return its memory fields.

    Match is by /proc/<pid>/comm (kernel comm name, 15-char limit). For
    ``bmc-widget-wasm`` (15 chars) and ``bmc-openwrt`` (11 chars) this is
    exact; longer names are truncated by the kernel and the caller would
    need to pass the truncated form. Missing processes record ``pid=None``.
    """
    out: dict[str, dict[str, Any]] = {name: {"pid": None} for name in names}
    if not names:
        return out
    pending = set(names)

    try:
        proc_entries = list(os.scandir("/proc"))
    except OSError:
        return out

    for entry in proc_entries:
        if not pending:
            break
        if not entry.name.isdigit():
            continue
        try:
            with Path(f"/proc/{entry.name}/comm").open() as f:
                comm = f.read().strip()
        except OSError:
            continue
        if comm not in pending:
            continue
        try:
            pid = int(entry.name)
        except ValueError:
            continue
        out[comm] = _read_proc_status(pid)
        pending.discard(comm)

    return out


def _read_proc_status(pid: int) -> dict[str, Any]:
    """Read /proc/<pid>/status and extract VmSize/VmRSS/Rss* fields (KB)."""
    record: dict[str, Any] = {"pid": pid}
    for dst in _PROC_STATUS_FIELDS.values():
        record[dst] = 0

    try:
        with Path(f"/proc/{pid}/status").open() as f:
            text = f.read()
    except OSError:
        # Process exited between scandir and open; treat as missing.
        return {"pid": None}

    for raw in text.splitlines():
        key, _, val = raw.partition(":")
        dst = _PROC_STATUS_FIELDS.get(key.strip())
        if dst is None or "kB" not in val:
            continue
        parts = val.strip().split()
        if not parts:
            continue
        try:
            record[dst] = int(parts[0])
        except ValueError:
            continue
    return record


# ── Entry point ────────────────────────────────────────────────────────────────


def main() -> None:
    """Run the event daemon."""
    logging.basicConfig(
        level=logging.DEBUG,
        format="%(asctime)s %(levelname)s %(message)s",
        stream=sys.stderr,
    )
    log.info("Starting event daemon on %s:%d", LISTEN_HOST, LISTEN_PORT)
    daemon = EventDaemon()
    daemon.run()


if __name__ == "__main__":
    main()
