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
    "hostapd-openssl"
    "htop"
    "ip-full"
    "iw"
    "iwinfo"
    "kmod-drm"
    "kmod-drm-kms-helper"
    "kmod-input-evdev"
    "kmod-input-uinput"
    "kmod-mac80211-hwsim"
    "kmod-sound-core"
    "mpg123"
    "rpcd-mod-iwinfo"
    "rpcd-mod-ucode"
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
