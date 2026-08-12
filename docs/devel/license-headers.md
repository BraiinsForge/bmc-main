# License Headers

Every first-party source file (Rust, TypeScript/JavaScript, SCSS, Python, shell, Nix, C, protobuf) carries a copyright
header followed by the GNU GPLv3 boilerplate notice and the license-reservation paragraph. The full license text lives
in [`COPYING`](../../COPYING); `.license.tpl` in the repository root holds the header template for editor tooling.

## Header format

The header consists of one copyright line per company, a blank comment line, and the boilerplate:

```
Copyright (C) <years>  Braiins Systems s.r.o.
Copyright (C) <years>  Braiins Forge s.r.o.

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

You should have received a copy of the GNU General Public License
along with this program.  If not, see <https://www.gnu.org/licenses/>.

Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
to grant any party a license to this program, or any part thereof,
under any terms, and such a grant shall be considered distinct from
the grant above.
```

Note the two spaces between the year list and the company name, and the years as a comma-separated ascending list
(`2025, 2026`).

## Comment style per language

- `//` line comments: Rust, TypeScript/TSX, JavaScript, SCSS, C, Protocol Buffers.
- `#` line comments: Python, shell, Nix, YAML, TOML.
- `<!-- -->` block: XML (Wayland protocol definitions).

The header goes at the very top of the file; a shebang stays first, with the header directly below it. A multi-line nix
shebang (`#!/usr/bin/env nix` followed by `#!nix ...` continuation lines) must stay contiguous — the header goes below
the whole `#!` block. One blank line separates the header from the file content.

## Attribution rules

Copyright lines are derived from the calendar years of copyrightable edits touching the file, split by date regardless
of who authored them:

- edits before 2026-01-01 → Braiins Systems s.r.o.
- edits from 2026-01-01 on → Braiins Forge s.r.o.

Each company's line lists the calendar years of its edits, so years up to 2025 go on the Braiins Systems line and years
from 2026 on the Braiins Forge line. When both companies appear, Braiins Systems comes first. Years that predate the
repository history (imported code) stay attributed to Braiins Systems. New files get a single Braiins Forge line with
the current year — this is what `.license.tpl` templates.

Exception: `tooling/` is imported from the Braiins Systems tooling repository and keeps its upstream copyright lines
unchanged — Braiins Systems only, including 2026 years — with just the boilerplate added below them. Do not rewrite
those lines to the date-based split.

## Third-party and generated files

Do not add Braiins headers to:

- Files carrying an upstream BOSI header: all of `bmc-net/bmc-net-types/` and the BOSI-headed files in
  `bmc-net/{bmc-net-drv,bmc-net-dns}` and `bmc-shared/stopwatch`. The BOSI notice is GPLv3 boilerplate too and stays
  verbatim.
- Generated code: `frontend/src/proto/gen/` (protobuf) and `src/manifest_params.rs` in widget crates (written by
  `bmc-widget-codegen` via `just wasm::gen`) — regeneration would drop any hand-added header.
- Anything covered by a colocated `LICENSE-*` or license file, which keeps its upstream terms — e.g. the
  AnySoftKeyboard-derived layouts in `bmc-render/keyboard/assets/layouts/` (`LICENSE-ANYSOFTKEYBOARD`, Apache-2.0),
  fonts under `assets/fonts/` and `frontend/src/styles/fonts/`, `LICENSE-CARBON`, `LICENSE-AOSP`,
  `LICENSE-CIRCLE-FLAGS`, `LICENSE-PHOSPHOR`.
- `frontend/src/lib/react/props.tsx` — carries the IBM Corp. Apache-2.0 notice.
- `widgets-wasm-examples/media-control/proto/cast_channel.proto` — Chromium, BSD-style license.
- `bmc-virt/kernel-patches/ref/spi-bmc-virt.c` — Linux kernel module reference, GPL-2.0-only.

Files that embed fragments of third-party code under a Braiins header must keep an attribution note next to the fragment
stating the origin and license — see the Carbon Design System notes in
`frontend/src/components/DataTable/DataTable.scss` and `frontend/src/styles/carbon/colors.scss`. Files derived from
other projects keep an origin note below the header, e.g. the nixpkgs-derived `nix/pkgs/mesa/`.
