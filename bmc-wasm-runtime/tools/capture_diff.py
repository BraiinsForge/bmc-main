#!/usr/bin/env nix
#!nix shell nixpkgs#python312 nixpkgs#odiff
#!nix --command python3

"""
Visual regression comparison using odiff.

Usage:
    capture_compare.py --baseline=<dir> --current=<dir> --diff=<dir> [--threshold=0.1]
"""

import argparse
import subprocess
import sys
from pathlib import Path
from shutil import which


def main() -> int:
    parser = argparse.ArgumentParser(
        description='Compare screenshots against baselines using odiff'
    )
    parser.add_argument(
        '--baseline', required=True, type=Path, help='Baseline directory'
    )
    parser.add_argument(
        '--current', required=True, type=Path, help='Current capture directory'
    )
    parser.add_argument(
        '--diff', required=True, type=Path, help='Output directory for diff images'
    )
    parser.add_argument(
        '--threshold', default='0.1', help='odiff threshold (default: 0.1)'
    )
    args = parser.parse_args()

    if not which('odiff'):
        print('Error: odiff not found (nix shebang should provide it)', file=sys.stderr)
        return 1

    if not args.baseline.is_dir():
        print(
            f'Error: baseline directory does not exist: {args.baseline}',
            file=sys.stderr,
        )
        return 1

    if not args.current.is_dir():
        print(
            f'Error: current directory does not exist: {args.current}', file=sys.stderr
        )
        return 1

    passed = 0
    failed = 0
    missing = 0
    errors: list[str] = []

    for baseline_file in sorted(args.baseline.rglob('*.png')):
        rel = baseline_file.relative_to(args.baseline)
        current_file = args.current / rel
        diff_file = args.diff / rel

        if not current_file.exists():
            print(f'MISSING: {rel}')
            errors.append(f'MISSING: {rel}')
            missing += 1
            continue

        diff_file.parent.mkdir(parents=True, exist_ok=True)

        result = subprocess.run(
            [
                'odiff',
                str(baseline_file),
                str(current_file),
                str(diff_file),
                '--threshold',
                args.threshold,
            ],
            capture_output=True,
        )

        if result.returncode == 0:
            print(f'PASS: {rel}')
            passed += 1
            diff_file.unlink(missing_ok=True)
        else:
            print(f'DIFF: {rel}')
            errors.append(f'DIFF: {rel} → {diff_file}')
            failed += 1

    print()
    print(f'=== Summary ===')
    print(f'Pass: {passed}  Fail: {failed}  Missing: {missing}')

    if errors:
        print()
        print('Regressions:')
        for err in errors:
            print(f'  {err}')
        return 1

    return 0


if __name__ == '__main__':
    sys.exit(main())
