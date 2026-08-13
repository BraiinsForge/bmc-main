# Copyright (C) 2026  Braiins Forge s.r.o.
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.
#
# Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
# to grant any party a license to this program, or any part thereof,
# under any terms, and such a grant shall be considered distinct from
# the grant above.

"""Drive every image-widget source format through a real Deck.

Sources are verified before anything is pushed: a moved or rewritten upstream
would otherwise look exactly like a decoder regression on the device.

Verdicts come from a profiling-gated pair of log lines. Every source is probed;
only a decode that actually ran logs its cost.
A size rejection is therefore a probe without a decode.

Decoding is asynchronous, so a decode can land after the next scene
has already fetched.
Every probe, decode and error carries the body length, which identifies
the case exactly.
"""

import binascii
import json
import re
import struct
import subprocess
import tempfile
import urllib.error
import urllib.request
import uuid
import zlib
from collections.abc import Callable
from dataclasses import dataclass, field
from http import HTTPStatus
from pathlib import Path
from typing import Literal

from bmc_tui import catalog, console, nix
from bmc_tui.device import Device, RemotePath
from bmc_tui.nix import Attr, StorePath
from bmc_tui.server import Request, ServerHandle, ViewConfig, server
from bmc_tui.stage import Abort, entrypoint, require

LOG = RemotePath("/var/log/bmc/run-bmc-wasm-host-sdk-v0.log")
CONFIG = RemotePath("/etc/bmc/config.json")
BACKUP = RemotePath("/etc/bmc/config.json.image-formats-bak")
PROCESS = "bmc-wasm-host"
WIDGET = "image"
# The profiling build is required below, so debug is the only sensible profile.
WIDGET_ATTR = Attr(".#deck-packages-debug.widget-image")
# Only the profiling build contains this literal, so the running binary
# proves what is installed. A log does not: it outlives its own build.
PROFILING_MARKER = "host_image_probe"
WIDGET_TYPE_ID = "f9e4956c-719d-450c-909d-4fc9d4440e15"

# Repo-relative: the fixtures ship with the tree, not the CLI, and the
# `.#deck-packages-debug` attrs above already require the flake root as cwd.
FIXTURES = Path("bmc-tui/fixtures/image-formats")

# Mirrors bmc_wasm_protocol::FetchOutcome: the host puts its wire value in
# the status field, so a refusal is distinguishable from a dead network.
FETCH_NETWORK = 0
FETCH_BODY_TOO_LARGE = 1000

Expect = Literal["decode", "reject-size", "reject-body"]


@dataclass(frozen=True, slots=True)
class Case:
    name: str
    fmt: str
    file: str
    magic: str  # lowercase hex the body must start with
    expect: Expect
    note: str = ""
    # A size-limit case carries information in its length, never its content,
    # so storing flat pixels would spend LFS payload on reproducible bytes.
    make: Callable[[], bytes] | None = None

    def url(self, base: str) -> str:
        return f"{base}/{self.file}"

    def body(self) -> bytes:
        return self.make() if self.make is not None else (FIXTURES / self.file).read_bytes()


def _png_chunk(kind: bytes, payload: bytes) -> bytes:
    body = kind + payload
    return struct.pack(">I", len(payload)) + body + struct.pack(">I", binascii.crc32(body))


def flat_png(width: int, height: int) -> bytes:
    """Truecolour PNG of one colour. Flat scanlines deflate to a few KB,
    so the pixel count can sit past the decode budget with the body under the fetch cap."""
    raw = (b"\x00" + b"\x1e\x78\xc8" * width) * height  # filter byte 0 per row
    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + b"".join(
        (
            _png_chunk(b"IHDR", header),
            _png_chunk(b"IDAT", zlib.compress(raw, 9)),
            _png_chunk(b"IEND", b""),
        )
    )


def flat_bmp(width: int, height: int) -> bytes:
    """24-bit BMP of one colour: uncompressed pixels make the body
    `width * height * 3` plus a 54-byte header, a size placed above the fetch cap."""
    row = b"\x1e\x78\xc8" * width
    pixels = (row + b"\x00" * (-len(row) % 4)) * height
    return b"".join(
        (
            struct.pack("<2sIHHI", b"BM", 54 + len(pixels), 0, 0, 54),
            struct.pack("<IiiHHIIiiII", 40, width, height, 1, 24, 0, len(pixels), 2835, 2835, 0, 0),
            pixels,
        )
    )


CASES: tuple[Case, ...] = (
    Case(
        name="png",
        fmt="PNG",
        file="blue-marble.png",
        magic="89504e47",
        expect="decode",
    ),
    Case(
        name="jpeg",
        fmt="JPEG",
        file="blue-marble.jpg",
        magic="ffd8ff",
        expect="decode",
        note="the only format that DCT-scales on load",
    ),
    Case(
        name="webp",
        fmt="WebP",
        file="blue-marble.webp",
        magic="52494646",
        expect="decode",
        note="the format that stopped decoding when the set narrowed to PNG/JPEG",
    ),
    Case(
        name="gif",
        fmt="GIF",
        file="blue-marble.gif",
        magic="47494638",
        expect="decode",
    ),
    Case(name="bmp", fmt="BMP", file="blue-marble.bmp", magic="424d", expect="decode"),
    Case(
        name="tiff",
        fmt="TIFF",
        file="blue-marble.tiff",
        magic="49492a00",
        expect="decode",
    ),
    Case(
        name="qoi",
        fmt="QOI",
        file="blue-marble.qoi",
        magic="716f6966",
        expect="decode",
    ),
    Case(
        name="pnm",
        fmt="PNM",
        file="blue-marble.pnm",
        magic="5037",
        expect="decode",
        note="P7/PAM, which is what the encoder emits for RGB",
    ),
    Case(
        name="farbfeld",
        fmt="farbfeld",
        file="blue-marble.ff",
        magic="6661726266656c64",
        expect="decode",
        note="16-bit RGBA, so a lower pixel ceiling",
    ),
    Case(
        name="hdr",
        fmt="Radiance",
        file="blue-marble.hdr",
        magic="233f5241",
        expect="decode",
        note="Rgb32F, so a lower pixel ceiling",
    ),
    Case(
        name="over-pixel-budget",
        fmt="PNG",
        file="over-pixel-budget.png",
        magic="89504e47",
        expect="reject-size",
        note="3000x3000 = 9 Mpx, past the budget but under the fetch cap",
        make=lambda: flat_png(3000, 3000),
    ),
    Case(
        name="over-fetch-cap",
        fmt="BMP",
        file="over-fetch-cap.bmp",
        magic="424d",
        expect="reject-body",
        note="12 MB, refused before any decoder runs",
        make=lambda: flat_bmp(2000, 2000),
    ),
)

_FAILED_FETCH = re.compile(r"\bfetch failed\b")
_FETCH_STATUS = re.compile(r"\bstatus=(\d+)")
_FETCH_BODY_LEN = re.compile(r"\bbody_len=(\d+)")
_FETCH_URL = re.compile(r"\burl=(\S+)")
_ERROR = re.compile(r"(host_decode_image(?: probe)?): (.+?)\s{2}data_len=(\d+)")
_PROBE = re.compile(r"host_image_probe (\d+)x(\d+) px=\d+ data_len=(\d+)")
_DECODE = re.compile(
    r"host_decode_image (\d+)x(\d+) data_len=(\d+) decode_us=(\d+) vmrss_delta_kb=([+-]?\d+)"
)


@dataclass
class Outcome:
    case: Case
    fetched: int | None = None
    status: int | None = None
    error: str = ""
    probed: str = ""  # WxH the source reported, before any budget check
    decoded: str = ""  # WxH actually decoded; empty when the decode never ran
    decode_us: int = 0
    vmrss_delta_kb: int = 0

    @property
    def failed(self) -> bool:
        """A decode case must reach the decoder; a reject case must not."""
        if self.case.expect == "reject-body":
            return self.status != FETCH_BODY_TOO_LARGE
        if self.status != HTTPStatus.OK or self.error:
            return True
        if self.case.expect == "decode":
            return not self.decoded
        return bool(self.decoded) or not self.probed


@dataclass
class ImageFormats:
    device: str  # IP or host of the target Deck
    dwell_seconds: int = 20  # scene cycling duration; also paces the wait
    keep_config: bool = False  # leave the test config on the device
    restore: bool = False  # put the backed-up config back and exit
    only: list[str] = field(default_factory=list)  # case names; empty → all selected

    def run(self) -> None:
        dev = Device(self.device)
        console.header("Image widget formats")
        dev.print()
        catalog.ensure_device_reachable(dev)

        if self.restore:
            _restore(dev)
            return

        require(self.dwell_seconds > 0, "--dwell-seconds must be positive")
        require(FIXTURES.is_dir(), f"{FIXTURES} not found — run from the repository root")
        cases = _select(self.only)
        console.kv("cases", str(len(cases)))

        with console.spinner("building this tree's widget"):
            # Realised, not just evaluated: the host is read from this path's
            # store references, which only exist once it is in the store.
            widget_path = nix.real().build_out(Attr(f"{WIDGET_ATTR}.pkg"))

        catalog.check_deployed_build(nix.real(), dev, WIDGET_ATTR, WIDGET)
        catalog.ensure_profiling_build(dev, PROCESS, PROFILING_MARKER)
        build = catalog.running_binary(dev, PROCESS)
        _check_host_build(dev, widget_path, build)

        requests: list[Request] = []
        with server(_views(cases, requests), reachable_from=dev.host) as assets:
            base_url = _device_facing(assets)
            console.kv("serving", f"{FIXTURES} at {base_url}")
            _preflight(cases, base_url)
            requests.clear()
            _backup(dev)
            _push(dev, cases, base_url, self.dwell_seconds)

            with dev.log_window(LOG) as window:
                catalog.restart_compositor(dev)
                _settle(len(cases), self.dwell_seconds)
            _report_drops(assets)

        outcomes = _collect(window.text, cases, base_url, requests)
        require(
            build is not None and catalog.running_binary(dev, PROCESS) == build,
            "the host binary changed mid-run — the window would mix two builds",
        )
        _report(outcomes)

        if not self.keep_config:
            _restore(dev)

        broken = [o for o in outcomes if o.failed]
        if broken:
            names = ", ".join(o.case.name for o in broken)
            raise Abort(f"{len(broken)} format(s) failed on the device: {names}")
        console.ok("every case matched its expected outcome")


def _device_facing(assets: ServerHandle) -> str:
    """The bind the device can reach; loopback only ever answers ourselves."""
    lan = [bind for bind in assets.binds if "127.0.0.1" not in bind]
    require(bool(lan), "no routable address to serve fixtures from — the device cannot reach us")
    return lan[0]


def _views(cases: list[Case], requests: list[Request]) -> dict[str, ViewConfig]:
    def recording_view(case: Case) -> Callable[[Request], bytes | Path]:
        def serve(request: Request) -> bytes | Path:
            requests.append(request)
            return case.body() if case.make is not None else FIXTURES / case.file

        return serve

    return {f"/{case.file}": recording_view(case) for case in cases}


def _expected_host(widget_path: StorePath) -> tuple[StorePath | None, str]:
    """The host binary a widget was built against, from its store references.

    `nix/wasm-widgets.nix` passes `--host-bin`, which makes the host a direct
    reference. Only meaningful once the widget is known to be this tree's.
    """
    query = subprocess.run(
        ["nix-store", "--query", "--references", str(widget_path)],
        capture_output=True,
        text=True,
        check=False,
    )
    hosts = [line for line in query.stdout.split() if f"-{PROCESS}-" in line]
    if len(hosts) == 1:
        return StorePath(hosts[0]), ""
    return None, query.stderr.strip() or f"{len(hosts)} references matched"


def _check_host_build(dev: Device, widget_path: StorePath, running: StorePath | None) -> None:
    """Report which host produced the measurements, and whether it is this tree's.

    Warns rather than aborts, like the widget check it mirrors: a mismatch
    still exercises the decoders, the numbers just describe another build.
    """
    console.kv("host binary", str(running) if running else "not running")
    if running is None:
        console.warn(f"{PROCESS} is not running on {dev.host}")
        return
    expected, why = _expected_host(widget_path)
    if expected is None:
        console.warn(f"cannot tell which host {WIDGET} expects, so no build pin: {why}")
    elif str(running).startswith(str(expected)):
        console.ok(f"{PROCESS} is the build {WIDGET} expects")
    else:
        console.warn(
            f"{PROCESS} runs {running}, but {WIDGET} expects {expected} — "
            "the measurements will not describe this tree"
        )


def _select(only: list[str]) -> list[Case]:
    """Every case runs; the budget and cap cases are the point, not an extra."""
    picked = [c for c in CASES if not only or c.name in only]
    if not picked:
        raise Abort("no cases selected")
    return picked


def _require_distinct_sizes(sizes: list[tuple[str, int]]) -> None:
    """Collection keys results on body length, so a repeated size would make
    a probe or decode impossible to attribute to the case that caused it."""
    seen: dict[int, str] = {}
    clashes = []
    for name, size in sizes:
        if (other := seen.get(size)) is not None:
            clashes.append(f"{other} and {name} both {size} bytes")
        seen[size] = name
    if clashes:
        raise Abort("ambiguous corpus — duplicate body length: " + "; ".join(clashes))


def _preflight(cases: list[Case], base_url: str) -> None:
    console.header("Pre-flight: verify fixtures")
    observed: list[tuple[str, int]] = []
    bad: list[str] = []
    for case in cases:
        try:
            head, total = _probe(case.url(base_url))
        except (urllib.error.URLError, TimeoutError, OSError) as exc:
            bad.append(f"{case.name}: unreachable ({exc})")
            continue
        got = head.hex()
        if head.startswith(b"version https://git-lfs"):
            bad.append(f"{case.name}: fixture is an LFS pointer — run `git lfs pull`")
        elif not got.startswith(case.magic):
            bad.append(f"{case.name}: magic {got[: len(case.magic)]} != {case.magic}")
        else:
            observed.append((case.name, total if total is not None else -1))
            console.ok(f"{case.name} ({case.fmt}, {total} B)")
    if bad:
        for line in bad:
            console.error(line)
        raise Abort(f"{len(bad)} fixture(s) did not serve as expected — refusing to push")
    _require_distinct_sizes(observed)


def _probe(url: str) -> tuple[bytes, int | None]:
    """Head of the body plus its full length, without downloading the whole file."""
    req = urllib.request.Request(url, headers={"Range": "bytes=0-15"})
    with urllib.request.urlopen(req, timeout=30) as resp:
        head = resp.read(16)
        if resp.status == HTTPStatus.PARTIAL_CONTENT:
            rng = resp.headers.get("Content-Range", "")
            return head, int(rng.rsplit("/", 1)[1]) if "/" in rng else None
        length = resp.headers.get("Content-Length")
    return head, int(length) if length else None


def _scene(case: Case, base_url: str) -> dict[str, object]:
    return {
        "id": str(uuid.uuid4()),
        "enabled": True,
        "kind": "fullscreen",
        "widgets": [
            {
                "id": str(uuid.uuid4()),
                "row": 0,
                "col": 0,
                "placement": "fullscreen",
                "widget_type_id": WIDGET_TYPE_ID,
                "viewport_shape": "rectangular",
                "params": {"url": case.url(base_url), "refresh_seconds": 3600, "sizing": "contain"},
            }
        ],
    }


def _push(dev: Device, cases: list[Case], base_url: str, dwell: int) -> None:
    """Stream the config rather than inline it.

    A scene per case puts the whole document in the ssh command line.
    Past about a dozen scenes the device refuses it and drops the connection
    rather than truncating, so the run dies either way.
    """
    console.header("Push test config")
    config = {
        "version": 1,
        "scenes": [_scene(c, base_url) for c in cases],
        "scene_cycling": {
            "automatic_cycling_enabled": True,
            "automatic_cycling_default_duration": f"{dwell}s",
            "transition": "slide",
        },
        "accounts": [],
    }
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
        json.dump(config, handle, indent=2)
        staged = Path(handle.name)
    try:
        dev.push(staged, CONFIG)
    finally:
        staged.unlink(missing_ok=True)
    console.ok(f"{len(cases)} scenes written to {CONFIG}")


def _backup(dev: Device) -> None:
    dev.run(f"[ -f {BACKUP} ] || cp {CONFIG} {BACKUP}")
    console.kv("backup", BACKUP)


def _restore(dev: Device) -> None:
    if dev.read(f"[ -f {BACKUP} ] && echo yes || echo no").strip() != "yes":
        console.warn(f"no backup at {BACKUP} — leaving the device config alone")
        return
    dev.run(f"cp {BACKUP} {CONFIG} && rm -f {BACKUP}")
    catalog.restart_compositor(dev)
    console.ok("previous config restored")


def _settle(scenes: int, dwell: int) -> None:
    """One full cycle plus a scene, so the last entry has fetched and rendered."""
    console.countdown(f"cycling {scenes} scenes", (scenes + 1) * dwell)


def _report_drops(assets: ServerHandle) -> None:
    """Report disconnects once the spinner has released the display.

    Routine on their own — a burst of them is how an unreachable server looks.
    """
    if not assets.drops:
        return
    last = assets.drops[-1]
    console.kv(
        "client disconnects",
        f"{len(assets.drops)} (last: {last.method} {last.path} from {last.peer})",
    )


def _collect(
    window: str,
    cases: list[Case],
    base_url: str,
    requests: list[Request],
) -> list[Outcome]:
    console.header("Collect results")
    wanted = re.compile("fetch failed|host_decode_image|host_image_probe")
    lines = [line for line in window.splitlines() if wanted.search(line)]

    by_url = {c.url(base_url): Outcome(case=c) for c in cases}
    by_path = {f"/{c.file}": by_url[c.url(base_url)] for c in cases}
    by_len: dict[int, Outcome | None] = {}

    def claim_length(outcome: Outcome, length: int) -> None:
        claimed = by_len.get(length, outcome)
        by_len[length] = outcome if claimed is outcome else None

    for request in requests:
        outcome = by_path.get(request.path)
        if outcome is None or "Range" in request.headers:
            continue
        length = len(outcome.case.body())
        outcome.status = HTTPStatus.OK
        outcome.fetched = length
        claim_length(outcome, length)

    for line in lines:
        if _FAILED_FETCH.search(line):
            status = _FETCH_STATUS.search(line)
            body_len = _FETCH_BODY_LEN.search(line)
            url = _FETCH_URL.search(line)
            if status and body_len and url and (outcome := by_url.get(url.group(1))) is not None:
                outcome.status = int(status.group(1))
                outcome.fetched = int(body_len.group(1))
        elif probe := _PROBE.search(line):
            if (out := by_len.get(int(probe.group(3)))) is not None:
                out.probed = f"{probe.group(1)}x{probe.group(2)}"
        elif dec := _DECODE.search(line):
            if (out := by_len.get(int(dec.group(3)))) is not None:
                out.decoded = f"{dec.group(1)}x{dec.group(2)}"
                out.decode_us = int(dec.group(4))
                out.vmrss_delta_kb = int(dec.group(5))
        elif err := _ERROR.search(line):
            if (out := by_len.get(int(err.group(3)))) is not None:
                out.error = f"{err.group(1)}: {err.group(2)}".strip()
    return [by_url[c.url(base_url)] for c in cases]


def _fetch_verdict(status: int | None) -> str | None:
    """How the fetch itself ended, or None once a body actually arrived."""
    if status == FETCH_BODY_TOO_LARGE:
        return "refused: body too large"
    if status == FETCH_NETWORK:
        return "NO FETCH (network error)"
    if status != HTTPStatus.OK:
        return f"NO FETCH (http {status})"
    return None


def _verdict(out: Outcome) -> str:
    if (fetch := _fetch_verdict(out.status)) is not None:
        return fetch
    if out.error:
        return f"FAILED — {out.error}"
    if out.case.expect == "decode":
        return f"decoded {out.decoded}" if out.decoded else "NOT DECODED"
    if out.decoded:
        return f"DECODED {out.decoded} — expected rejection"
    return f"rejected at {out.probed}" if out.probed else "NEVER PROBED"


def _report(outcomes: list[Outcome]) -> None:
    name_width = max(len(out.case.name) for out in outcomes) + 2
    rows = []
    for out in outcomes:
        # A refused body carries the host's reason string, so its length
        # describes the message rather than the fixture.
        refused = out.status == FETCH_BODY_TOO_LARGE
        size = "-" if refused else f"{out.fetched or '-'!s}B"
        rss = f"{out.vmrss_delta_kb:+d} kB" if out.decoded else "-"
        took = f"{out.decode_us / 1000:.1f} ms" if out.decode_us else "-"
        rows.append(
            f"{out.case.name:<{name_width}}{out.case.fmt:<10}"
            f"{size:>11}{rss:>11}{took:>10}  {_verdict(out)}"
        )
    console.panel(
        "\n".join(rows), title="Formats — RSS delta is process-wide before/after, not a decode peak"
    )


@entrypoint
def main(args: ImageFormats) -> None:
    args.run()


if __name__ == "__main__":
    main()
