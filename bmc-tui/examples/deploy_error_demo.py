"""Demo: the prettied deploy-error rendering (BDK-582), driven end-to-end.

No device and no hand-built exception. Each stage runs a real local subprocess
through the same capture shape as the ssh transport (`check=True`,
`capture_output=True`, `text=True`), so what you see is the genuine flow: the
`@stage` headers, the `✓` lines of the stages that pass, then a stage whose
command *intentionally* fails — rendered by the real `@entrypoint` handler,
exactly as a failed `deck deploy` stage would look.

Run in a terminal (piping strips rich's colour):

    uv run python bmc-tui/examples/deploy_error_demo.py [--scenario NAME]

`conflict` (default) exits 1 with captured stderr; `signal` dies by SIGTERM;
`uncaptured` streams its output live, so only the status header is rendered.
"""

import subprocess
from dataclasses import dataclass
from typing import Literal

from bmc_tui import console
from bmc_tui.stage import entrypoint, stage

Scenario = Literal["conflict", "signal", "uncaptured"]

# A real command emitting BDK-582-shaped output then exiting non-zero. The
# `[bmc-nix-cli]`/`[core]` brackets are the escaping proof: pre-fix rich parsed
# them as markup, post-fix `console.cmd_output` prints them verbatim.
# Only double quotes inside, so `shlex.join` renders the command as one clean
# single-quoted blob instead of the POSIX `'"'"'` escape dance.
_CONFLICT_SCRIPT = (
    'echo "error: packages failed to register";'
    'echo "symlink conflict at cargo-timings/cargo-timing-workspace-deps-check.html:" >&2;'
    'echo "  provided by [bmc-nix-cli] and [core]" >&2;'
    "exit 1"
)


@dataclass
class Args:
    scenario: Scenario = "conflict"  # how the final stage's real command fails


def _run(cmd: list[str]) -> None:
    """Mirror `device.run`: capture output as text and raise on non-zero — the
    exact call shape whose failure the entrypoint now surfaces."""
    subprocess.run(cmd, check=True, capture_output=True, text=True)


@stage("Check local toolchain")
def check_toolchain() -> str:
    _run(["sh", "-c", "command -v sh > /dev/null"])
    return "sh present"


@stage("Register packages")
def register_packages(scenario: Scenario) -> str:
    match scenario:
        case "conflict":
            _run(["sh", "-c", _CONFLICT_SCRIPT])
        case "signal":
            # sh kills itself → returncode -15; a genuine signal death.
            _run(["sh", "-c", "kill -TERM $$"])
        case "uncaptured":
            # No capture: the command's output streams straight to the terminal,
            # so the handler has nothing to render but the status header.
            subprocess.run(["sh", "-c", 'echo "live output to your terminal"; exit 2'], check=True)
    return "packages registered"


@entrypoint
def main(args: Args) -> None:
    console.header("Demo deploy")
    console.kv("target", "local subprocess (no device)")
    check_toolchain()
    register_packages(args.scenario)


if __name__ == "__main__":
    main()
