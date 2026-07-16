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

# Kernel config delta applied on top of the stock OpenWrt config at build time.
#
# The stock config is extracted from the ImageBuilder tarball — it's the exact
# config the prebuilt kmod packages were compiled against. Our customizations
# go on top via scripts/config, then `make olddefconfig` resolves dependencies.
#
# To add an option: add a scripts/config line here and rebuild.
# To see the resolved config: check $out/config in the kernel derivation output.
{
  # x86_64-specific patches — the OpenWrt x86/64 target ships a 32-bit kernel
  # config; we need 64-bit + only virtio drivers for QEMU.
  x86_64 = ''
    # Force 64-bit so the rootfs binaries (x86_64) can execute.
    scripts/config --enable CONFIG_64BIT

    # Disable hardware GPU drivers (only virtio-gpu needed in QEMU)
    scripts/config \
      --disable CONFIG_DRM_I915 \
      --disable CONFIG_DRM_AMDGPU \
      --disable CONFIG_DRM_RADEON \
      --disable CONFIG_DRM_NOUVEAU \
      --disable CONFIG_DRM_VMWGFX \
      --disable CONFIG_DRM_QXL \
      --disable CONFIG_DRM_BOCHS \
      --disable CONFIG_DRM_AST \
      --disable CONFIG_DRM_MGAG200

    # USB host controllers (not needed in QEMU)
    scripts/config \
      --disable CONFIG_USB_XHCI_HCD \
      --disable CONFIG_USB_XHCI_PCI \
      --disable CONFIG_USB_EHCI_HCD \
      --disable CONFIG_USB_EHCI_PCI \
      --disable CONFIG_USB_OHCI_HCD \
      --disable CONFIG_USB_UHCI_HCD \
      --disable CONFIG_USB_STORAGE \
      --disable CONFIG_USB_ACM

    # Real NIC drivers (only virtio-net needed)
    scripts/config \
      --disable CONFIG_E1000 \
      --disable CONFIG_E1000E \
      --disable CONFIG_IGB \
      --disable CONFIG_IGBVF \
      --disable CONFIG_IXGBE \
      --disable CONFIG_I40E \
      --disable CONFIG_R8169 \
      --disable CONFIG_BNX2 \
      --disable CONFIG_BE2NET \
      --disable CONFIG_MLX4_EN \
      --disable CONFIG_MLX5_CORE

    # Bluetooth (WiFi stack kept for mac80211_hwsim)
    scripts/config \
      --disable CONFIG_BT \
      --disable CONFIG_BT_HCIBTUSB \
      --disable CONFIG_BT_HCIUART

    # Storage controllers not needed in QEMU (keep SATA_AHCI + ATA_PIIX)
    scripts/config \
      --disable CONFIG_SATA_SIL24 \
      --disable CONFIG_SATA_NV \
      --disable CONFIG_SATA_SIL \
      --disable CONFIG_SATA_VIA \
      --disable CONFIG_PATA_AMD \
      --disable CONFIG_PATA_ARTOP \
      --disable CONFIG_PATA_ATIIXP \
      --disable CONFIG_PATA_OLDPIIX \
      --disable CONFIG_PATA_PDC_OLD \
      --disable CONFIG_PATA_VIA \
      --disable CONFIG_PATA_MPIIX \
      --disable CONFIG_PATA_PLATFORM \
      --disable CONFIG_FUSION_SPI \
      --disable CONFIG_FUSION_SAS

    # Sound — use virtio-sound instead of HDA
    scripts/config \
      --disable CONFIG_SND_HDA \
      --disable CONFIG_SND_HDA_INTEL

    # Miscellaneous hardware
    scripts/config \
      --disable CONFIG_IIO \
      --disable CONFIG_FIREWIRE \
      --disable CONFIG_INPUT_JOYDEV
  '';

  # Options applied on both architectures.
  common = ''
    # rr time-travel debugger
    scripts/config --enable CONFIG_PROC_PAGE_MONITOR

    # /proc/config.gz for config verification
    scripts/config \
      --enable CONFIG_IKCONFIG \
      --enable CONFIG_IKCONFIG_PROC

    # Virtio (QEMU)
    scripts/config \
      --enable CONFIG_VIRTIO \
      --enable CONFIG_VIRTIO_PCI \
      --enable CONFIG_VIRTIO_PCI_LEGACY \
      --enable CONFIG_VIRTIO_BLK \
      --enable CONFIG_VIRTIO_NET \
      --enable CONFIG_VIRTIO_CONSOLE \
      --enable CONFIG_VIRTIO_BALLOON \
      --enable CONFIG_VIRTIO_INPUT

    # 9p filesystem (share host /nix/store with guest)
    scripts/config \
      --enable CONFIG_NET_9P \
      --enable CONFIG_NET_9P_VIRTIO \
      --enable CONFIG_9P_FS \
      --enable CONFIG_9P_FS_POSIX_ACL

    # DRM display
    scripts/config \
      --enable CONFIG_I2C \
      --enable CONFIG_FB \
      --enable CONFIG_BACKLIGHT_CLASS_DEVICE \
      --enable CONFIG_DRM \
      --enable CONFIG_DRM_KMS_HELPER \
      --enable CONFIG_DRM_TTM \
      --enable CONFIG_DRM_TTM_HELPER \
      --enable CONFIG_DRM_VRAM_HELPER \
      --enable CONFIG_DRM_VKMS \
      --enable CONFIG_DRM_VIRTIO_GPU

    # SPI (LED data capture)
    scripts/config \
      --enable CONFIG_SPI \
      --enable CONFIG_SPI_SPIDEV \
      --enable CONFIG_SPI_BMC_VIRT

    # Input
    scripts/config \
      --enable CONFIG_INPUT_EVDEV \
      --enable CONFIG_INPUT_UINPUT

    # Sound (ALSA + virtio-sound for QEMU)
    scripts/config \
      --enable CONFIG_SOUND \
      --enable CONFIG_SND \
      --enable CONFIG_SND_PCM \
      --enable CONFIG_SND_TIMER \
      --enable CONFIG_SND_PCM_OSS \
      --enable CONFIG_SND_MIXER_OSS \
      --enable CONFIG_SND_VIRTIO

    # WiFi (mac80211_hwsim)
    scripts/config \
      --enable CONFIG_MAC80211 \
      --enable CONFIG_CFG80211 \
      --enable CONFIG_MAC80211_HWSIM

    # Networking (firewall NAT for WiFi subnet)
    scripts/config \
      --enable CONFIG_SFP \
      --enable CONFIG_PHYLINK \
      --enable CONFIG_STACKPROTECTOR \
      --enable CONFIG_NETFILTER_NETLINK \
      --enable CONFIG_NETFILTER_NETLINK_QUEUE \
      --enable CONFIG_NETFILTER_NETLINK_LOG \
      --enable CONFIG_NETFILTER_XTABLES \
      --enable CONFIG_NF_CONNTRACK \
      --enable CONFIG_NF_DEFRAG_IPV4 \
      --enable CONFIG_NF_DEFRAG_IPV6 \
      --enable CONFIG_NF_NAT \
      --enable CONFIG_NF_TABLES \
      --enable CONFIG_NF_TABLES_INET \
      --enable CONFIG_NF_TABLES_IPV4 \
      --enable CONFIG_NF_TABLES_IPV6 \
      --enable CONFIG_NFT_CT \
      --enable CONFIG_NFT_NAT \
      --enable CONFIG_NFT_MASQ \
      --enable CONFIG_NFT_REJECT \
      --enable CONFIG_NFT_REJECT_INET \
      --enable CONFIG_NFT_LIMIT \
      --enable CONFIG_NFT_CHAIN_NAT \
      --enable CONFIG_NF_TABLES_NETDEV \
      --enable CONFIG_LIBCRC32C \
      --enable CONFIG_IP_NF_IPTABLES \
      --enable CONFIG_IP_NF_FILTER \
      --enable CONFIG_IP_NF_NAT

    # We boot from rootfs, not initramfs
    scripts/config --set-str CONFIG_INITRAMFS_SOURCE ""

    make olddefconfig
  '';
}
