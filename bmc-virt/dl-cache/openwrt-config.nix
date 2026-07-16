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

# Shared OpenWrt configuration for bmc-virt.
#
# Imported by:
#   - bmc-virt/flake.nix            (VM image builder)
#   - bmc-virt/dl-cache/flake.nix   (feed cache generator)
#
# After changing packageList or openwrtVersion, rebuild the feed cache:
#   `just update-cache` from `bmc-virt`
{
  openwrtVersion = "24.10.6";

  # Packages to install in the VM image.
  # Prefix with "-" to exclude a default package.
  packageList = [
    "alsa-utils"
    "bash"
    "ca-certificates"
    "coreutils"
    # `od` ships as a sub-package; `hexdump` lives in util-linux and busybox
    # already exposes it (`busybox hexdump -C`), so we don't carry a second
    # copy. `xxd` (below) covers the same use case for tidier output.
    "coreutils-od"
    # HTTP tooling for poking the BMC web stack from inside the VM without
    # having to hand-roll headers via busybox `nc` / `wget`. `grpcurl` is
    # intentionally absent: OpenWrt 24.10's stock feeds don't carry Go-built
    # binaries, so a `curl` + the proto descriptors path stays the recipe.
    "curl"
    "hostapd-openssl"
    "htop"
    "ip-full"
    "iw"
    "iwinfo"
    "jq"
    "kmod-drm"
    "kmod-drm-kms-helper"
    "kmod-input-evdev"
    "kmod-input-uinput"
    "kmod-mac80211-hwsim"
    "kmod-sound-core"
    "mpg123"
    "rpcd-mod-iwinfo"
    "rpcd-mod-ucode"
    "socat"
    "strace"
    "wget-ssl"
    "wpa-supplicant-openssl"
    "xxd"
    "-luci"
    "-uhttpd"
  ];

  # SHA-256 hashes for the OpenWrt ImageBuilder tarballs (per guest arch).
  # These are stable per release — the ImageBuilder artifact itself doesn't change.
  imageBuilderHash = {
    aarch64 = "sha256-1e2DR6eqCTpKBgn+4ROJ4nDlOoH4E4xtU0CwrM1pKf0=";
    x86_64 = "sha256-sxSFdaYvuwelIvVeDb/5oM7OHmk/KSqE8kWvx8EeNUA=";
  };

  # Canonical fingerprint of a package list.
  # Sorted, joined, SHA-256 hashed — a single opaque string that changes
  # when the list changes. Used by:
  #   - dl-cache builder: writes cache/{arch}.sha256
  #   - VM builder: compares against committed .sha256 at eval time
  mkManifest = pkgList:
    builtins.hashString "sha256"
      (builtins.concatStringsSep "\n" (builtins.sort builtins.lessThan pkgList));
}
