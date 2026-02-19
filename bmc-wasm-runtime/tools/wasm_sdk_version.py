#!/usr/bin/env python3
"""Print the embedded BMC SDK version from a compiled WASM widget.

Usage:
    ./tools/wasm_sdk_version.py path/to/widget.wasm
"""

import argparse
import re
import subprocess
import sys

from _common import require_tools


def main() -> None:
    parser = argparse.ArgumentParser(
        description='Print BMC SDK version from a WASM binary'
    )
    parser.add_argument('wasm', help='Path to .wasm file')
    args = parser.parse_args()

    require_tools(('wasm-objdump', 'wabt'))

    result = subprocess.run(
        ['wasm-objdump', '-d', args.wasm],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        print(f'wasm-objdump failed: {result.stderr.strip()}', file=sys.stderr)
        sys.exit(1)

    in_fn = False
    for line in result.stdout.splitlines():
        if '<__bmc_sdk_version>' in line:
            in_fn = True
            continue
        if in_fn:
            m = re.search(r'i64\.const\s+(-?\d+)', line)
            if m:
                v = int(m.group(1)) & 0xFFFF_FFFF_FFFF_FFFF
                major = v & 0xFFFF
                minor = (v >> 16) & 0xFFFF
                patch = (v >> 32) & 0xFFFF
                print(f'{major}.{minor}.{patch}')
                return
            if 'end' in line:
                break

    print('__bmc_sdk_version export not found', file=sys.stderr)
    sys.exit(1)


if __name__ == '__main__':
    main()
