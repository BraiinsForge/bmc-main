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
BMC_CONFIG_LEGACY: str = _raw["BMC_CONFIG_LEGACY"]
BMC_PID_FILE: Path = Path(_raw["BMC_PID_FILE"])
BMC_RUNTIME_DIR: str = _raw["BMC_RUNTIME_DIR"]
WASM_DIR: Path = Path(_raw["WASM_DIR"])
RELAY_LOG: str = _raw["RELAY_LOG"]
RR_BUNDLE: Path = Path(_raw["RR_BUNDLE"])
RR_TRACE_DIR: Path = Path(_raw["RR_TRACE_DIR"])

__all__ = [
    "BMC_BIN",
    "BMC_CONFIG",
    "BMC_CONFIG_LEGACY",
    "BMC_LOG",
    "BMC_PID_FILE",
    "BMC_RUNTIME_DIR",
    "RELAY_LOG",
    "RR_BUNDLE",
    "RR_TRACE_DIR",
    "WASM_DIR",
]
