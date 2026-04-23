#!/usr/bin/env bash
# Build, copy, and sign the init tarball for the Braiins Deck.
#
# Steps:
#   1. nix build ".#init-tarball-armv7"
#   2. copy the resulting tarball from the nix store into CWD
#   3. produce a detached signify signature (compatible with OpenWrt usign)
#   4. append the signature trailer to the tarball via fwtool -S
#
# Usage: fw-sign-init-tarball [SECRET_KEY]
#   SECRET_KEY  default: bos-nix-init.sec
#
# Environment:
#   FLAKE       flake ref for the init tarball (default: the bmc-main
#               checkout three levels up from this script, resolved to
#               `path:<script_dir>/../../..`)
#
# Self-contained: if signify or fwtool are not in PATH, the script
# re-executes itself inside `nix develop <script_dir>`, which provides
# both tools via the sibling flake.nix.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [ -z "${_BDK377_FW_SIGN_IN_SHELL:-}" ] \
    && { ! command -v signify >/dev/null 2>&1 \
        || ! command -v fwtool >/dev/null 2>&1; }; then
    export _BDK377_FW_SIGN_IN_SHELL=1
    exec nix develop "$script_dir" --command bash "$0" "$@"
fi

sec="${1:-bos-nix-init.sec}"
flake="${FLAKE:-git+file:${script_dir}/../../..}"

if [ ! -f "$sec" ]; then
    echo "error: secret key $sec not found — run fw-keygen first" >&2
    exit 1
fi

echo "==> building ${flake}#init-tarball-armv7"
out=$(nix build --no-link --print-out-paths "${flake}#init-tarball-armv7")

# mkTarball writes a single nix-<bos_version>.tar.gz into $out.
shopt -s nullglob
tarballs=("$out"/*.tar.gz)
shopt -u nullglob
if [ ${#tarballs[@]} -eq 0 ]; then
    echo "error: no tarball found under $out" >&2
    exit 1
fi
src="${tarballs[0]}"

tarball="$(basename "$src")"
sig="${tarball%.tar.gz}.sig"

echo "==> copying $src -> ./$tarball"
install -m 0644 "$src" "./$tarball"

echo "==> signing ./$tarball with $sec"
signify -S -m "./$tarball" -s "$sec" -x "./$sig"

echo "==> appending signature trailer via fwtool"
fwtool -S "./$sig" "./$tarball"

echo "done: ./$tarball (signed in place; detached signature at ./$sig)"
