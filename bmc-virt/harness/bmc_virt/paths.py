"""Guest-side paths used by the harness.

Loads ``guest-paths.toml`` from alongside this module — the same file the
flake reads via ``builtins.fromTOML``. That TOML is the single source of
truth; nothing is duplicated here. Keys map 1:1 to env var names
(``BMC_LOG``, ``RR_BUNDLE``, …) which are the names the init.d scripts, the
relay process, and this module all use.
"""

from __future__ import annotations

import tomllib
from pathlib import Path

_PATHS_TOML = Path(__file__).parent / "guest-paths.toml"
_raw: dict[str, str] = tomllib.loads(_PATHS_TOML.read_text())

BMC_LOG: str = _raw["BMC_LOG"]
BMC_BIN: str = _raw["BMC_BIN"]
BMC_CONFIG: str = _raw["BMC_CONFIG"]
BMC_PID_FILE: Path = Path(_raw["BMC_PID_FILE"])
BMC_RUNTIME_DIR: str = _raw["BMC_RUNTIME_DIR"]
WASM_DIR: Path = Path(_raw["WASM_DIR"])
RELAY_LOG: str = _raw["RELAY_LOG"]
RR_BUNDLE: Path = Path(_raw["RR_BUNDLE"])
RR_TRACE_DIR: Path = Path(_raw["RR_TRACE_DIR"])

__all__ = [
    "BMC_BIN",
    "BMC_CONFIG",
    "BMC_LOG",
    "BMC_PID_FILE",
    "BMC_RUNTIME_DIR",
    "WASM_DIR",
    "RELAY_LOG",
    "RR_BUNDLE",
    "RR_TRACE_DIR",
]
