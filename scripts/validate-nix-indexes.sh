#!/usr/bin/env bash
# Validate Nix index JSON files against the concept doc schema.
#
# Usage:
#   ./scripts/validate-nix-indexes.sh [--index FILE] [--factory FILE]
set -euo pipefail

index_file=""
factory_file=""

while [[ $# -gt 0 ]]; do
    case "$1" in
    --index)
        index_file="$2"
        shift 2
        ;;
    --factory)
        factory_file="$2"
        shift 2
        ;;
    *)
        echo "Unknown option: $1" >&2
        exit 1
        ;;
    esac
done

if [[ -z $index_file && -z $factory_file ]]; then
    echo "ERROR: No files to validate. Use --index or --factory." >&2
    exit 1
fi

errors=0

if [[ -n $index_file ]]; then
    echo "Validating index: $index_file"
    python3 - "$index_file" <<'PYEOF' || errors=$((errors + 1))
import json, sys

with open(sys.argv[1]) as f:
    idx = json.load(f)

errors = []

# Top-level fields
if idx.get('version') != 1:
    errors.append(f'version must be 1, got {idx.get("version")}')
if 'provenance' not in idx:
    errors.append('missing provenance field')
elif idx['provenance'] is not None:
    commit = idx['provenance'].get('commit', '')
    if not commit or commit == 'dirty':
        errors.append(f'provenance.commit must be a valid hash, got "{commit}"')
if not isinstance(idx.get('indexes', []), list):
    errors.append('indexes must be a list')
if not isinstance(idx.get('caches', []), list):
    errors.append('caches must be a list')
if not isinstance(idx.get('packages', []), list):
    errors.append('packages must be a list')

# Cache fields
for i, c in enumerate(idx.get('caches', [])):
    for field in ('name', 'cache_url', 'cache_key'):
        if field not in c:
            errors.append(f'cache[{i}] missing {field}')

# Package fields
VALID_STRATEGIES = ('reboot',)  # extend when new strategies are added
for i, p in enumerate(idx.get('packages', [])):
    for field in ('name', 'version', 'store_path'):
        if field not in p:
            errors.append(f'package[{i}] missing {field}')
    sp = p.get('store_path', '')
    if sp and not sp.startswith('/nix/store/'):
        errors.append(f'package[{i}] store_path must start with /nix/store/')
    us = p.get('upgrade_strategy')
    if us is not None and us not in VALID_STRATEGIES:
        errors.append(f'package[{i}] invalid upgrade_strategy: {us}')
    ist = p.get('install_strategy')
    if ist is not None and ist not in VALID_STRATEGIES:
        errors.append(f'package[{i}] invalid install_strategy: {ist}')

if errors:
    for e in errors:
        print(f'  ERROR: {e}', file=sys.stderr)
    sys.exit(1)
else:
    print(f'  OK: {len(idx.get("packages", []))} packages validated')
PYEOF
fi

if [[ -n $factory_file ]]; then
    echo "Validating factory index: $factory_file"
    python3 - "$factory_file" <<'PYEOF' || errors=$((errors + 1))
import json, sys

with open(sys.argv[1]) as f:
    idx = json.load(f)

errors = []

if idx.get('version') != 1:
    errors.append(f'version must be 1, got {idx.get("version")}')
if not isinstance(idx.get('tarballs', []), list):
    errors.append('tarballs must be a list')

for i, t in enumerate(idx.get('tarballs', [])):
    for field in ('bos_version', 'download_url', 'profile_path'):
        if field not in t:
            errors.append(f'tarball[{i}] missing {field}')

if errors:
    for e in errors:
        print(f'  ERROR: {e}', file=sys.stderr)
    sys.exit(1)
else:
    print(f'  OK: {len(idx.get("tarballs", []))} tarballs validated')
PYEOF
fi

if [[ $errors -gt 0 ]]; then
    echo "FAILED: $errors validation errors"
    exit 1
fi
echo "All validations passed."
