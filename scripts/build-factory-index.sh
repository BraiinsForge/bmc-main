#!/usr/bin/env bash
# Build factory.json from metadata.json files.
#
# Usage:
#   ./scripts/build-factory-index.sh \
#     --base-url https://cache.braiins.com/v1 \
#     --metadata result/metadata.json \
#     [--metadata other/metadata.json] \
#     --output factory.json
#
# Each metadata.json contains:
#   { "bos_version": "26.02",
#     "profile_path": "/nix/var/nix/gcroots/profiles/bmc",
#     "tarball_name": "nix-26.02.tar.gz" }
#
# The script constructs download_url as:
#   <base_url>/<tarball_name>
set -euo pipefail

base_url=""
metadata_files=()
output=""

while [[ $# -gt 0 ]]; do
    case "$1" in
    --base-url)
        base_url="$2"
        shift 2
        ;;
    --metadata)
        metadata_files+=("$2")
        shift 2
        ;;
    --output)
        output="$2"
        shift 2
        ;;
    *)
        echo "Unknown option: $1" >&2
        exit 1
        ;;
    esac
done

if [[ -z $base_url || ${#metadata_files[@]} -eq 0 || -z $output ]]; then
    echo "Usage: $0 --base-url URL --metadata FILE [--metadata FILE...] --output FILE" >&2
    exit 1
fi

# Build the factory index using a single Python invocation.
# Pass all metadata files and base URL as arguments to avoid
# shell injection via string interpolation.
python3 - "$base_url" "$output" "${metadata_files[@]}" <<'PYEOF'
import json
import sys

base_url = sys.argv[1].rstrip("/")
output_path = sys.argv[2]
metadata_files = sys.argv[3:]

REQUIRED_FIELDS = ("bos_version", "tarball_name", "profile_path")

tarballs = []
for meta_file in metadata_files:
    with open(meta_file) as f:
        meta = json.load(f)
    for field in REQUIRED_FIELDS:
        if field not in meta:
            print(f"ERROR: {meta_file} missing required field '{field}'", file=sys.stderr)
            sys.exit(1)
    tarballs.append({
        "bos_version": meta["bos_version"],
        "download_url": f"{base_url}/{meta['tarball_name']}",
        "profile_path": meta["profile_path"],
    })

factory = {"version": 1, "tarballs": tarballs}
with open(output_path, "w") as f:
    json.dump(factory, f, indent=2)
print(f"Written {len(tarballs)} tarballs to {output_path}", file=sys.stderr)
PYEOF
