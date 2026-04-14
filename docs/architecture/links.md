# External References

## Confluence

- [Nix Experiments on Braiins Deck](https://braiins.atlassian.net/wiki/spaces/Nix/pages/1458667597) — Nix/Home Manager
  on STM32MP157C
- [BCB 100 Hardware](https://braiins.atlassian.net/wiki/spaces/BF/pages/1659142158/BCB+100) — control board design
  revisions

## Jira

- [BDK project board](https://braiins.atlassian.net/browse/BDK) — Braiins Deck backlog
- [BDK-221 Tech Debt epic](https://braiins.atlassian.net/browse/BDK-221) — parent epic for infrastructure work
- [BDK-378 x86 QEMU emulation](https://braiins.atlassian.net/browse/BDK-378) — bmc-virt VM environment
- [BDK-383 PC debugging with rr](https://braiins.atlassian.net/browse/BDK-383) — time-travel debugging in VM
- [BDK-392 GPIO USR_BTN emulation](https://braiins.atlassian.net/browse/BDK-392) — console hold button → uevent
  injection
- [BDK-212 Extensibility Framework](https://braiins.atlassian.net/browse/BDK-212) — WASM widget runtime
- [BDK-214 GPU acceleration PoC](https://braiins.atlassian.net/browse/BDK-214) — Slint GPU renderer on BMM 101
- [BDK-210 Nix modular updates](https://braiins.atlassian.net/browse/BDK-210) — replacing full-image OTA with Nix store

## GitLab

- [bos-main](https://gitlab.ii.zone/bos/bos-main) — BraiinsOS monorepo (bosminer, boser, firmware)
- [OpenWrt fork](https://gitlab.ii.zone/bos/openwrt) — Braiins OpenWrt with board support
- [bos-packages](https://gitlab.ii.zone/bos/bos-packages) — OpenWrt package feed
- [braiins-bin](https://gitlab.ii.zone/bos/braiins-bin) — CVITEK/Amlogic/Beaglebone buildroot artifacts

## Slint

- [Slint 1.13.1 docs](https://docs.rs/slint/1.13.1)
- [MinimalSoftwareWindow](https://docs.slint.dev/latest/docs/rust/slint/platform/software_renderer/struct.MinimalSoftwareWindow)
- [linuxkms calloop backend](https://github.com/slint-ui/slint/blob/master/internal/backends/linuxkms/calloop_backend.rs)
  — reference for custom event loop implementation

## Hardware

- STM32MP157C — dual Cortex-A7 SoC, runs OpenWrt
- Display: 1280x480 portrait (rendered landscape via 270deg rotation)
- Touch: evdev /dev/input/event1
- LEDs: 10x APA102 addressable via SPI (/dev/spidev0.0)
- WiFi: Realtek USB (vendor 0bda), both hubbed and hubless board variants
