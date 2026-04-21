"""rr time-travel debugger — first-class API with platform checks."""

from __future__ import annotations

import pathlib
import platform
import shutil
import subprocess
from typing import TYPE_CHECKING

from bmc_virt import ui
from bmc_virt.commands import Ack, Cmd
from bmc_virt.paths import RR_TRACE_DIR

if TYPE_CHECKING:
    from bmc_virt.vm import VM


class PlatformError(Exception):
    """Raised when rr cannot run on the current platform."""


_GUEST_TRACE_DIR = str(RR_TRACE_DIR)


class RrHandle:
    """First-class API for rr time-travel debugging — accessed via vm.rr.

    Requires the VM to be started with --rr (deploys the rr bundle to /root/rr).
    """

    def __init__(self, vm: VM) -> None:
        self._vm = vm
        self._last_trace: str | None = None

    def start(self, *, timeout: float = 30, verbose: bool = False) -> Ack:
        """Stop bmc-openwrt and restart it under rr record (headless compositor).

        Raises PlatformError on non-x86_64.
        Prints a warning if AMD MSR is not configured.
        """
        _check_platform()
        _warn_amd_msr()
        return self._vm._send_cmd(Cmd.RR_START, timeout=timeout, verbose=verbose)

    def stop(self, *, timeout: float = 30, verbose: bool = False) -> Ack:
        """Stop rr recording and finalize the trace.

        The ack's data["trace"] contains the guest-side trace path.
        """
        ack = self._vm._send_cmd(Cmd.RR_STOP, timeout=timeout, verbose=verbose)
        if ack.ok and ack.data.get("trace"):
            self._last_trace = ack.data["trace"]
        return ack

    def pull(self, local: str | pathlib.Path) -> None:
        """Pull the latest rr trace from the VM to the host.

        Uses the trace path returned by the last stop(), or falls back
        to pulling the entire trace directory. Removes any stale local
        copy first so scp does not merge into previous pulls.
        """
        local = pathlib.Path(local)
        if local.exists():
            shutil.rmtree(local)
        src = self._last_trace or _GUEST_TRACE_DIR
        self._vm.pull(src=src, dst=local)


def _check_platform() -> None:
    """Hard error if not x86_64 — rr is x86_64-only."""
    arch = platform.machine()
    if arch != "x86_64":
        msg = (
            f"rr requires x86_64, but this host is {arch}.\nrr cannot run on non-x86_64 platforms."
        )
        raise PlatformError(msg)


def _warn_amd_msr() -> None:
    """Warn if AMD CPU and MSR not configured for rr accuracy."""
    try:
        cpuinfo = pathlib.Path("/proc/cpuinfo").read_text()
    except OSError:
        return

    if "AuthenticAMD" not in cpuinfo:
        return

    # Check if the MSR tweak is applied
    try:
        result = subprocess.run(
            ["rdmsr", "-c", "0xc0011020"],
            capture_output=True,
            text=True,
            timeout=5,
            check=False,
        )
        if result.returncode == 0:
            value = int(result.stdout.strip(), 16)
            if value & (1 << 54):
                return  # MSR is set correctly
    except (OSError, ValueError, subprocess.TimeoutExpired):
        pass

    ui.warn(
        "AMD CPU detected but MSR 0xc0011020 bit 54 may not be set.\nrr traces may be inaccurate.\n"
    )
    ui.panel(
        "sudo modprobe msr\n"
        "sudo wrmsr -a 0xc0011020 $(($(sudo rdmsr -c 0xc0011020) | (1 << 54)))\n"
        "sudo sysctl kernel.perf_event_paranoid=1",
        title="To fix",
        style="yellow",
        lexer="bash",
    )
