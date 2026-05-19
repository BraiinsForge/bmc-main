#!/usr/bin/env python3
"""Resolve a wasm widget name to its workspace root.

Two modes:

  widget_root.py NAME    Print the absolute workspace root containing the widget crate `NAME`
                         (the directory that holds the workspace `Cargo.toml`, not the crate dir itself).
                         Exits 1 if NAME is unknown or ambiguous.

  widget_root.py         No args: print all known workspace roots, space-separated, in deterministic order.
                         Convenient for shell loops in the justfile.

Source of truth for which roots host wasm widgets. Adding a new workspace
means adding it here and (separately) wiring the Nix-side equivalent in
`workspace.nix` / `nix/wasm-widgets.nix`.
"""

import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent

WIDGET_ROOTS = [
    REPO_ROOT / 'bmc-wasm-runtime' / 'examples',
    REPO_ROOT / 'widgets-wasm',
]


def resolve(name: str) -> Path:
    matches = [r for r in WIDGET_ROOTS if (r / name / 'Cargo.toml').is_file()]
    if not matches:
        roots = ', '.join(str(r) for r in WIDGET_ROOTS)
        print(
            f"widget '{name}' not found in any root: {roots}",
            file=sys.stderr,
        )
        sys.exit(1)
    if len(matches) > 1:
        roots = ', '.join(str(r) for r in matches)
        print(
            f"widget '{name}' is ambiguous; found in: {roots}",
            file=sys.stderr,
        )
        sys.exit(1)
    return matches[0]


def main() -> None:
    if len(sys.argv) == 1:
        print(' '.join(str(r) for r in WIDGET_ROOTS))
        return
    if len(sys.argv) != 2:
        sys.exit(f'Usage: {sys.argv[0]} [WIDGET_NAME]')
    print(resolve(sys.argv[1]))


if __name__ == '__main__':
    main()
