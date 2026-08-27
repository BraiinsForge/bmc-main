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

{
  description = "BMC Virtual Environment — OpenWrt QEMU system VM (x86_64 or aarch64 guest, matched to host)";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    harness.url = "path:./harness";

    # macOS-only: QEMU patched for virgl + ANGLE (OpenGL ES on Metal). Required
    # for the Wayland capture path inside the guest to work — vanilla QEMU on
    # macOS uses software rendering and `glReadPixels` returns OOM at panel
    # size. Outputs `aarch64-darwin` only; the conditional in `qemu` below
    # gates access on darwin.
    darwin-qemu-virgl.url = "github:kubijo/darwin-qemu-virgl-flake";
  };

  outputs = { nixpkgs, flake-utils, harness, darwin-qemu-virgl, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        inherit (builtins) readFile pathExists match elem concatStringsSep;
        pkgs = import nixpkgs { inherit system; config.allowUnfreePredicate = pkg: elem (nixpkgs.lib.getName pkg) [ "corefonts" ]; };

        owrtCfg = import ./dl-cache/openwrt-config.nix;
        openwrtVersion = owrtCfg.openwrtVersion;
        linuxVersion = "6.6.127";

        overlayDir = ./rootfs/overlay;

        # ── Architecture detection ────────────────────────────────────────────
        # Guest arch matches host: aarch64 hosts get aarch64 guest (HVF/KVM),
        # x86_64 hosts keep x86_64 guest (KVM + rr support).
        isAarch64 = match "aarch64-.*" system != null;
        isDarwin = match ".*-darwin" system != null;
        guestArch = if isAarch64 then "aarch64" else "x86_64";
        guestLinuxSystem = if isAarch64 then "aarch64-linux" else "x86_64-linux";

        # OpenWrt target
        openwrtTarget = if isAarch64 then "armsr/armv8" else "x86/64";
        openwrtTargetDash = if isAarch64 then "armsr-armv8" else "x86-64";

        # Host-side QEMU.
        # Linux: nixpkgs QEMU with openGL + virgl flags (pulls in libdrm etc).
        # Darwin: a separately maintained flake that builds QEMU 10.0.0 with
        # Akihiko Odaki's macOS texture-borrowing patch + ANGLE-backed
        # virglrenderer. Required for the Wayland capture path; vanilla QEMU
        # on macOS only does software rendering inside the guest.
        qemu =
          if isDarwin
          then darwin-qemu-virgl.packages.${system}.default
          else pkgs.qemu.override { openGLSupport = true; virglSupport = true; };
        qemuBin = if isAarch64 then "qemu-system-aarch64" else "qemu-system-x86_64";
        qemuMachine = if isAarch64 then "virt" else "q35,i8042=off";
        consoleDevice = if isAarch64 then "ttyAMA0" else "ttyS0";

        # Kernel
        kernelTarget = if isAarch64 then "Image" else "bzImage";
        kernelOutputPath = if isAarch64 then "arch/arm64/boot/Image" else "arch/x86/boot/bzImage";
        # Stock kernel config extracted from the ImageBuilder at build time.
        # Path inside the imagebuilder tarball:
        stockKernelConfigPath =
          if isAarch64
          then "build_dir/target-aarch64_generic_musl/linux-armsr_armv8/linux-${linuxVersion}/.config"
          else "build_dir/target-x86_64_musl/linux-x86_64/linux-${linuxVersion}/.config";
        kernelConfigDelta = import ./kernel-config-delta.nix;

        # Fake upstream WiFi AP — radio1 runs this permanently so radio0 can connect as STA.
        uplinkSsid = "BMC-VIRT-UPLINK";
        uplinkKey = "braiins-virt";

        # Host ports (forwarded to guest via QEMU user-mode NAT).
        # gRPC-Web is multiplexed onto the same listener as HTTP (port 80
        # in the guest), so there is no separate gRPC entry here.
        ports = {
          ssh = 2222; # → guest 22
          http = 50080; # → guest 80 (high range to avoid the common 8080 clash)
          ipc = 5910; # → guest 5910 (bmc-virt-console TCP IPC)
          event = 5920; # → guest 5920 (bmc-virt-eventd)
        };

        # Event daemon — built from harness/uv.lock with all deps.
        # Runs from the 9p-mounted nix store — no opkg python3 needed.
        # MUST be built for the guest's Linux system (not the host's), or the
        # Mach-O binary lands in the Linux guest and `/bin/sh` chokes on its
        # header. On macOS hosts this requires a Linux builder (handled by
        # darwin-ensure-builder.sh).
        eventdEnv = harness.packages.${guestLinuxSystem}.default;

        # Guest-targeted static packages (for binaries deployed into the VM)
        guestPkgsStatic =
          if isAarch64
          then pkgs.pkgsCross.aarch64-multiplatform.pkgsStatic
          else pkgs.pkgsStatic;

        # OpenWrt upstream only produces the ImageBuilder as an x86_64-linux binary.
        # On non-x86_64 hosts, Nix builds this derivation via binfmt emulation
        # (requires qemu-user-static + extra-platforms = x86_64-linux).
        x86Pkgs = import nixpkgs { system = "x86_64-linux"; config.allowUnfreePredicate = pkg: elem (nixpkgs.lib.getName pkg) [ "corefonts" ]; };

        # On macOS, kernel and vmImage builds delegate to the aarch64-linux builder.
        linuxPkgs =
          if isDarwin
          then import nixpkgs { system = if isAarch64 then "aarch64-linux" else "x86_64-linux"; config.allowUnfreePredicate = pkg: elem (nixpkgs.lib.getName pkg) [ "corefonts" ]; }
          else pkgs;

        # ── Step 1: Prebuilt OpenWrt image via ImageBuilder ─────────────────
        imageBuilder = pkgs.fetchurl {
          url = "https://downloads.openwrt.org/releases/${openwrtVersion}/targets/${openwrtTarget}/openwrt-imagebuilder-${openwrtVersion}-${openwrtTargetDash}.Linux-x86_64.tar.zst";
          hash = owrtCfg.imageBuilderHash.${guestArch};
        };

        packageList = concatStringsSep " " owrtCfg.packageList;

        # ── Step 1a: Vendored download cache (Git LFS) ──────────────────────
        # Contains .ipk packages and feed indexes for the ImageBuilder.
        # Committed as 7z archives in Git LFS — OpenWrt release feeds are NOT
        # immutable (packages get security updates and rebuilds upstream).
        #
        # To refresh after changing packageList or openwrtVersion:
        #   `just update-cache` from `bmc-virt`
        openwrtFeedCache =
          let
            archiveRel = "dl-cache/data/${guestArch}.tar";
            manifestRel = "dl-cache/data/${guestArch}.sha256";
            archivePath = ./. + "/${archiveRel}";
            manifestPath = ./. + "/${manifestRel}";
            expectedManifest = owrtCfg.mkManifest owrtCfg.packageList;
            # Fail fast at eval time with a clear message.
            archive =
              if !pathExists manifestPath || !pathExists archivePath then
                throw ''
                  OpenWrt feed cache not found (${archiveRel}).
                  Run: `just update-cache` from `bmc-virt`
                ''
              else if readFile manifestPath != expectedManifest then
                throw ''
                  OpenWrt feed cache is stale — packageList or openwrtVersion
                  in dl-cache/openwrt-config.nix changed but the cache was not rebuilt.
                  Run: `just update-cache` from `bmc-virt`
                ''
              else archivePath;
          in
          x86Pkgs.runCommand "openwrt-feed-cache-${guestArch}-${openwrtVersion}" { } ''
            mkdir -p $out
            tar xf ${archive} -C $out
          '';

        # ── Step 1b: Prebuilt OpenWrt image (pure — no network) ──────────────
        # Uses the vendored feed cache to build fully offline.
        # Overlay or flake.nix changes only rebuild this derivation — no downloads.
        vmImageBase = x86Pkgs.runCommand "openwrt-${guestArch}-base-${openwrtVersion}"
          {
            nativeBuildInputs = with x86Pkgs; [
              # Build system
              gnumake
              bash
              perl
              python311
              # Shell utilities (ImageBuilder prereq checks)
              gawk
              getopt
              coreutils
              ncurses
              findutils
              wget
              which
              file
              # Archive / compression
              unzip
              bzip2
              xz
              zstd
              zlib
              # File sync
              rsync
            ];
          } ''
          tar xf ${imageBuilder}
          cd openwrt-imagebuilder-*
          patchShebangs .
          find . \( -name "Makefile" -o -name "GNUmakefile" -o -name "*.mk" \) \
            -exec sed -i 's|/usr/bin/env|${x86Pkgs.coreutils}/bin/env|g' {} +

          # Pre-populate opkg download cache from vendored archive → offline build.
          # The cache contains feed indexes and .ipk files.  `opkg update` fails
          # (no network) but `|| true` in the ImageBuilder Makefile swallows it;
          # opkg install then finds everything it needs in the cache.
          cp -a ${openwrtFeedCache}/. dl/
          chmod -R u+w dl

          # Disable signature verification — the vendored cache doesn't carry
          # .sig files (upstream re-signs feeds, so signatures drift from the
          # ImageBuilder's keys).
          sed -i '/^option check_signature/d' repositories.conf

          MERGED_OVERLAY=$(mktemp -d)
          mkdir -p "$MERGED_OVERLAY/usr/share/fonts/truetype"
          cp -a ${overlayDir}/. "$MERGED_OVERLAY/"
          cp ${x86Pkgs.corefonts}/share/fonts/truetype/Arial.ttf \
            "$MERGED_OVERLAY/usr/share/fonts/truetype/"

          # Make the entire overlay writable — files inherited Nix store read-only
          # permissions and the imagebuilder's finalizeRootfs runs sed -i on /etc/*.
          # Must happen before the env-file installs below: their target dir
          # (/etc/bmc-virt) came from the read-only nix store copy.
          chmod -R u+w "$MERGED_OVERLAY"

          # Bake the env files into the image so first boot (before any deploy)
          # can source them cleanly. Deploy overwrites them anyway, so the
          # baked version only matters when guest-paths.toml / ports match.
          ${x86Pkgs.coreutils}/bin/install -m644 ${pathsEnvFile} \
            "$MERGED_OVERLAY/etc/bmc-virt/paths.env"
          ${x86Pkgs.coreutils}/bin/install -m644 ${portsEnvFile} \
            "$MERGED_OVERLAY/etc/bmc-virt/ports.env"

          # Template WiFi uplink credentials in all overlay files
          ${templateUplink x86Pkgs "$MERGED_OVERLAY"}
          if grep -rq '@@UPLINK_' "$MERGED_OVERLAY/etc/"; then
            echo "ERROR: overlay still contains unsubstituted @@UPLINK_*@@ placeholders" >&2
            grep -r '@@UPLINK_' "$MERGED_OVERLAY/etc/"
            exit 1
          fi

          make image \
            SHELL="${x86Pkgs.bash}/bin/bash" \
            PROFILE="generic" \
            PACKAGES="${packageList}" \
            FILES="$MERGED_OVERLAY" \
            BIN_DIR="$out" \
            ROOTFS_PARTSIZE=1024 \
            2>&1
        '';

        # ── Step 2: Custom kernel with CONFIG_PROC_PAGE_MONITOR (cached) ─────
        # Compiles the OpenWrt-patched kernel (871 patches on vanilla 5.15.167)
        # with one extra config flag. ~10 min native x86_64, cached by nix.
        # Vermagic unaffected — prebuilt kmod packages still load.
        openwrtSrc = pkgs.fetchFromGitHub {
          owner = "openwrt";
          repo = "openwrt";
          rev = "v${openwrtVersion}";
          hash = "sha256-uamRaFWoRdxxaTEacvKV+fs+B57nXxyoE5xLgszqjtA=";
        };

        customKernel = linuxPkgs.gcc13Stdenv.mkDerivation {
          pname = "openwrt-kernel-custom";
          version = "${openwrtVersion}-${linuxVersion}";
          src = pkgs.fetchurl {
            url = "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-${linuxVersion}.tar.xz";
            hash = "sha256-p82cl7TwsxzAMLzcYKvlQ0//slVuKT90OM55Cd/4yf4=";
          };
          nativeBuildInputs = with linuxPkgs; [
            bc
            flex
            bison
            elfutils.dev
            openssl.dev
            perl
            coreutils
            gnumake
            bash
            findutils
            zstd
          ];
          hardeningDisable = [ "all" ];
          buildPhase = ''
            runHook preBuild

            chmod -R u+w .
            patchShebangs scripts/

            # Copy OpenWrt extra kernel source files (drivers, configs)
            for filedir in \
              ${openwrtSrc}/target/linux/generic/files \
              ${if isAarch64 then "${openwrtSrc}/target/linux/armsr/files" else "${openwrtSrc}/target/linux/x86/files"}; do
              if [ -d "$filedir" ]; then
                echo "=== Copying files from $(basename $filedir) ==="
                cp -a "$filedir"/. .
              fi
            done
            chmod -R u+w .

            # Apply all OpenWrt kernel patches (backports, pending, hacks, arch-specific)
            for patchdir in \
              ${openwrtSrc}/target/linux/generic/backport-6.6 \
              ${openwrtSrc}/target/linux/generic/pending-6.6 \
              ${openwrtSrc}/target/linux/generic/hack-6.6 \
              ${if isAarch64 then "${openwrtSrc}/target/linux/armsr/patches-6.6" else "${openwrtSrc}/target/linux/x86/patches-6.6"}; do
              if [ -d "$patchdir" ]; then
                echo "=== Applying patches from $(basename $patchdir) ==="
                for p in "$patchdir"/*.patch; do
                  patch -p1 -s < "$p" || echo "WARNING: $(basename $p) failed (non-fatal)"
                done
              fi
            done

            # Our patches must apply cleanly — fail the build if they don't.
            echo "=== Applying bmc-virt kernel patches ==="
            for p in ${./kernel-patches}/*.patch; do
              echo "  $(basename $p)"
              patch -p1 < "$p"
            done

            # Extract stock kernel config from the ImageBuilder — this is the exact
            # config the prebuilt kmod packages were compiled against.
            tar --zstd -xf ${imageBuilder} --wildcards '*/build_dir/*/linux-*/.config'
            cp --no-preserve=mode openwrt-imagebuilder-*/${stockKernelConfigPath} .config

            # Apply our delta on top of the stock config.
            ${if !isAarch64 then kernelConfigDelta.x86_64 else ""}
            ${kernelConfigDelta.common}

            make olddefconfig

            grep "CONFIG_PROC_PAGE_MONITOR=y" .config || { echo "FATAL: PROC_PAGE_MONITOR not set"; exit 1; }
            grep "CONFIG_DRM=y" .config || { echo "FATAL: DRM not set"; exit 1; }
            make -j$NIX_BUILD_CORES ${kernelTarget}
            runHook postBuild
          '';
          installPhase = ''
            mkdir -p $out
            cp ${kernelOutputPath} $out/vmlinuz
            cp .config $out/config
          '';
        };

        # ── Step 3: Swap kernel in base image (seconds, no compilation) ───────
        # On x86_64: swap custom kernel into boot partition via debugfs.
        # On aarch64: pass through as-is (we always direct-boot with -kernel).
        vmImage =
          if isAarch64
          then
            pkgs.runCommand "openwrt-aarch64-bmc-virt-${openwrtVersion}" { } ''
              mkdir -p $out
              cp -a ${vmImageBase}/. $out/
            ''
          else
            linuxPkgs.runCommand "openwrt-x86_64-bmc-virt-${openwrtVersion}"
              {
                nativeBuildInputs = with linuxPkgs; [ gzip util-linux jq e2fsprogs coreutils ];
              } ''
              mkdir -p $out
              cp -a ${vmImageBase}/. $out/
              chmod -R u+w $out

              IMG=$(find $out -name '*ext4-combined.img.gz' ! -name '*efi*' -print -quit)
              test -n "$IMG" || { echo "FATAL: no ext4-combined.img.gz"; ls $out; exit 1; }

              # Decompress (ignore trailing garbage from OpenWrt's image builder)
              gunzip -f "$IMG" || true
              IMG_RAW="''${IMG%.gz}"
              test -f "$IMG_RAW" || { echo "FATAL: gunzip failed"; exit 1; }

              # Find boot partition via sfdisk JSON
              read -r START SIZE <<< "$(sfdisk --json "$IMG_RAW" \
                | jq -r '.partitiontable.partitions[] | select(.bootable) | "\(.start) \(.size)"')"
              test -n "$START" || { echo "FATAL: no bootable partition"; sfdisk --dump "$IMG_RAW"; exit 1; }
              echo "Boot partition: start=$START sectors=$SIZE"

              # Extract, swap kernel, write back
              dd if="$IMG_RAW" of=/tmp/boot.img bs=512 skip="$START" count="$SIZE"
              echo "Before: $(debugfs -R 'stat /boot/vmlinuz' /tmp/boot.img 2>&1 | grep Size:)"

              debugfs -w -R "rm /boot/vmlinuz" /tmp/boot.img || true
              debugfs -w -R "write ${customKernel}/vmlinuz /boot/vmlinuz" /tmp/boot.img
              echo "After:  $(debugfs -R 'stat /boot/vmlinuz' /tmp/boot.img 2>&1 | grep Size:)"

              ACTUAL=$(debugfs -R 'stat /boot/vmlinuz' /tmp/boot.img 2>&1 | grep -oP 'Size: \K\d+' | head -1 | tr -d '[:space:]')
              test -n "$ACTUAL" || { echo "FATAL: vmlinuz not in boot partition"; exit 1; }
              echo "Kernel written: $ACTUAL bytes"

              dd if=/tmp/boot.img of="$IMG_RAW" bs=512 seek="$START" conv=notrunc
              rm /tmp/boot.img

              gzip -f "$IMG_RAW"
              echo "Kernel swapped: $ACTUAL bytes"
            '';

        # Guest-side paths shared across init scripts, the relay binary, and
        # the host-side harness. The checked-in `guest-paths.toml` is the ONE
        # source of truth: Nix reads it here, the harness reads it from
        # bmc_virt/paths.py (which sits next to it in the package), and it
        # gets rendered into /etc/bmc-virt/paths.env on the VM at deploy time
        # for the shell consumers (init.d scripts, the relay, justfile,
        # get-logs.sh). Add a new path by editing the TOML and referencing
        # the new key in the consumer.
        guestPaths = fromTOML (readFile ./harness/bmc_virt/guest-paths.toml);
        rrGuestBundle = guestPaths.RR_BUNDLE;
        rrGuestTraceDir = guestPaths.RR_TRACE_DIR;

        # Env files rendered at eval time into nix-store paths, then `install`-ed
        # into the overlay. Avoids the heredoc indent trap — the exact bytes are
        # visible via `nix eval` on these attributes (via `.#internal`) and land
        # on the VM unchanged. `toKeyValue` emits `KEY=VALUE\n` lines; values
        # here are plain paths/ports so no shell escaping is needed.
        pathsEnvFile = pkgs.writeText "paths.env" (
          pkgs.lib.generators.toKeyValue { } guestPaths
        );
        portsEnvFile = pkgs.writeText "ports.env" (
          pkgs.lib.generators.toKeyValue { } {
            PORT_SSH = toString ports.ssh;
            PORT_HTTP = toString ports.http;
            PORT_IPC = toString ports.ipc;
            PORT_EVENT = toString ports.event;
          }
        );

        # ── Step 4: rr bundle (glibc + rr for musl VM, x86_64 only) ──────────
        rrBundle = if isAarch64 then null else
        pkgs.runCommand "rr-bundle" { } ''
          mkdir -p $out/{bin,lib/rr,share}

          # rr binary + stubs
          cp ${pkgs.rr}/bin/.rr-wrapped $out/bin/rr
          cp ${pkgs.rr}/bin/rr_exec_stub $out/bin/
          cp -a ${pkgs.rr}/lib/rr/*.so $out/lib/rr/
          cp -a ${pkgs.rr}/share/rr $out/share/ 2>/dev/null || true

          # glibc (rr is dynamically linked against it)
          for lib in libc.so.6 libpthread.so.0 librt.so.1 libdl.so.2 libm.so.6 ld-linux-x86-64.so.2; do
            cp ${pkgs.glibc}/lib/$lib $out/lib/ 2>/dev/null || true
          done

          # C++ runtime + rr deps
          cp ${pkgs.stdenv.cc.cc.lib}/lib/libstdc++.so.6 $out/lib/
          cp ${pkgs.stdenv.cc.cc.lib}/lib/libgcc_s.so.1 $out/lib/
          cp ${pkgs.lib.getLib pkgs.zstd}/lib/libzstd.so.1 $out/lib/
          cp ${pkgs.lib.getLib pkgs.capnproto}/lib/libcapnp*.so* $out/lib/
          cp ${pkgs.lib.getLib pkgs.capnproto}/lib/libkj*.so* $out/lib/

          # Wrapper that uses bundled ld-linux
          cat > $out/bin/run-rr.sh << 'WRAPPER'
          #!/bin/bash
          DIR="$(cd "$(dirname "$0")/.." && pwd)"
          exec "$DIR/lib/ld-linux-x86-64.so.2" --library-path "$DIR/lib" "$DIR/bin/rr" "$@"
          WRAPPER
          chmod +x $out/bin/run-rr.sh
        '';

        # Shell snippet: template @@UPLINK_*@@ placeholders in all files under a directory.
        # Usage: ${templateUplink p} "$DIR"  (where p is the package set)
        templateUplink = p: dir: ''
          ${p.findutils}/bin/find "${dir}" -type f \
            -exec ${p.gnugrep}/bin/grep -q '@@UPLINK_' {} \; \
            -exec ${p.gnused}/bin/sed -i \
              -e 's|@@UPLINK_SSID@@|${uplinkSsid}|g' \
              -e 's|@@UPLINK_KEY@@|${uplinkKey}|g' {} +
        '';

        # ── Scripts ────────────────────────────────────────────────────────────

        # Host-native `bmc-virt-console` links against winit, which dlopens
        # Wayland/X11/GL at runtime.  On NixOS/Guix those libs aren't in
        # /usr/lib, so we bake their store paths into the binary's rpath via
        # RUSTFLAGS.  Shared between the `run` script (build path) and the
        # `devShells.default` (interactive `cargo run` path).
        consoleHostRustflagsEnv = pkgs.lib.optionalAttrs (!isDarwin) (
          let
            libPath = pkgs.lib.makeLibraryPath (with pkgs; [
              libx11
              libxcursor
              libxrandr
              libxi
              libxext
              libxcb
              wayland
              libxkbcommon
              fontconfig
              libGL
              vulkan-loader
            ]);
            hostTriple = if isAarch64 then "AARCH64_UNKNOWN_LINUX_GNU" else "X86_64_UNKNOWN_LINUX_GNU";
          in
          {
            "CARGO_TARGET_${hostTriple}_RUSTFLAGS" = "-C link-args=-Wl,-rpath,${libPath}";
          }
        );
        consoleHostRustflagsExports = pkgs.lib.concatStringsSep "\n" (
          pkgs.lib.mapAttrsToList (k: v: "export ${k}=${pkgs.lib.escapeShellArg v}") consoleHostRustflagsEnv
        );

        sshOpts = "-F /dev/null -o StrictHostKeyChecking=no -o UserKnownHostsFile=vm-data/known_hosts -o WarnWeakCrypto=no -p ${toString ports.ssh}";
        scpOpts = "-F /dev/null -o StrictHostKeyChecking=no -o UserKnownHostsFile=vm-data/known_hosts -o WarnWeakCrypto=no -P ${toString ports.ssh} -O";
        sshProbeOpts = "${sshOpts} -o PreferredAuthentications=password -o PubkeyAuthentication=no -o ConnectTimeout=2 -o ConnectionAttempts=1";

        run = pkgs.writeShellApplication {
          name = "bmc-virt-run";
          runtimeInputs = with pkgs; [
            coreutils
            findutils
            gnugrep
            gnused
            gnutar
            gzip
            procps
          ];
          text = ''
            set -euo pipefail
            trap 'echo "ERROR: bmc-virt-run failed at line $LINENO" >&2' ERR
            _section_start=0
            header() {
              if (( _section_start > 0 )); then
                local elapsed=$(( SECONDS - _section_start ))
                echo -e "\033[2;33m    done in ''${elapsed}s\033[0m"
              fi
              _section_start=$SECONDS
              echo -e "\n\033[1;36m=== $1 ===\033[0m"
            }
            WORKSPACE=$(${pkgs.git}/bin/git rev-parse --show-toplevel)
            DATADIR="''${BMC_VIRT_DATA:-$(pwd)/vm-data}"
            LOGDIR="$DATADIR/logs"
            PROFILE="''${BMC_PROFILE:-${guestArch}-debug}"
            CONFIG="''${CONFIG:-}"
            # VM overlay is always recreated from the base image on every run.
            RR="''${RR:-}"
            HOST_PATH_DIRS="''${BMC_VIRT_HOST_PATH:-}"
            LED_BINARY="''${BMC_VIRT_LED_BINARY:-}"
            # Guest artifacts always come from Linux flake outputs keyed by guest
            # architecture. macOS only matters for the launcher/runtime (HVF,
            # local QEMU selection), not for selecting VM package sets.
            GUEST_PKG_PREFIX="$WORKSPACE#packages.${guestLinuxSystem}"
            mkdir -p "$DATADIR"
            rm -rf "$LOGDIR" && mkdir -p "$LOGDIR"

            ssh_vm() {
              ${pkgs.sshpass}/bin/sshpass -p root ${pkgs.openssh}/bin/ssh ${sshOpts} "$@"
            }
            scp_vm() {
              ${pkgs.sshpass}/bin/sshpass -p root ${pkgs.openssh}/bin/scp ${scpOpts} "$@"
            }
            ssh_vm_probe() {
              ${pkgs.sshpass}/bin/sshpass -p root ${pkgs.openssh}/bin/ssh ${sshProbeOpts} "$@"
            }

            header "Building bmc-openwrt ($PROFILE)"
            BINARY=$(${pkgs.nix}/bin/nix build -L "$GUEST_PKG_PREFIX.bmc-openwrt-$PROFILE" \
              --no-link --print-out-paths)/bin/bmc-openwrt

            header "Building frontend"
            FRONTEND=$(${pkgs.nix}/bin/nix build -L "$WORKSPACE#frontend" \
              --no-link --print-out-paths)

            header "Building sounds"
            SOUNDS=$(${pkgs.nix}/bin/nix build -L "$WORKSPACE#sounds" \
              --no-link --print-out-paths)

            if [[ -z "$LED_BINARY" ]]; then
              header "Building LED visualizer"
              LED_BINARY=$(${pkgs.nix}/bin/nix build -L "$GUEST_PKG_PREFIX.bmc-virt-leds-${guestArch}-debug" \
                --no-link --print-out-paths)/bin/bmc-virt-leds
            fi

            header "Building relay daemon"
            RELAY_BINARY=$(${pkgs.nix}/bin/nix build -L "$GUEST_PKG_PREFIX.bmc-virt-relay-${guestArch}-debug" \
              --no-link --print-out-paths)/bin/bmc-virt-relay

            header "Building widgets"
            WIDGETS=$(${pkgs.nix}/bin/nix build -L "$GUEST_PKG_PREFIX.widgets-${guestArch}" \
              --no-link --print-out-paths)
            WASM_HOST=$(${pkgs.nix}/bin/nix build -L "$GUEST_PKG_PREFIX.widgets-${guestArch}.host" \
              --no-link --print-out-paths)

            header "Building WASM widgets"
            WASM_EXAMPLES=$(${pkgs.nix}/bin/nix build -L "$GUEST_PKG_PREFIX.wasm-examples" \
              --no-link --print-out-paths)
            WASM_WIDGETS=$(${pkgs.nix}/bin/nix build -L "$GUEST_PKG_PREFIX.wasm-widgets" \
              --no-link --print-out-paths)

            header "Building console app (host-native)"
            ${consoleHostRustflagsExports}
            cargo build -p bmc-virt-console --manifest-path "$WORKSPACE/Cargo.toml"

            header "Building OpenWrt image (rootfs + kernel)"
            if ! IMAGE=$(${pkgs.nix}/bin/nix build -L \
              "path:$WORKSPACE/bmc-virt#vmImage" \
              --option sandbox false \
              --no-link --print-out-paths); then
              exit 1
            fi

            BASE_IMAGE=$(find "$IMAGE" -maxdepth 1 -name '*ext4-combined.img*' ! -name '*efi*' -print -quit)
            if [[ -z "$BASE_IMAGE" ]]; then
              BASE_IMAGE=$(find "$IMAGE" -maxdepth 1 -name '*ext4-combined*.img*' -print -quit)
            fi
            if [[ -z "$BASE_IMAGE" ]]; then
              echo "ERROR: Could not find ext4 combined image in $IMAGE"
              ls "$IMAGE/"
              exit 1
            fi

            OVERLAY="$DATADIR/overlay.qcow2"
            if [[ -f "$OVERLAY" ]]; then
              rm -f "$OVERLAY" "$DATADIR/known_hosts" "$DATADIR/binary.sha256"
            fi
            if [[ ! -f "$OVERLAY" ]]; then
              header "Creating qcow2 overlay"
              if [[ "$BASE_IMAGE" == *.gz ]]; then
                DECOMPRESSED="$DATADIR/base.img"
                ${pkgs.gzip}/bin/gunzip -c "$BASE_IMAGE" > "$DECOMPRESSED" || true
                BASE_IMAGE="$DECOMPRESSED"
              fi
              ${qemu}/bin/qemu-img create -f qcow2 -b "$BASE_IMAGE" -F raw "$OVERLAY"
            fi

            PIDFILE="$DATADIR/qemu.pid"
            VM_RUNNING=false
            if [[ -f "$PIDFILE" ]]; then
              PID=$(cat "$PIDFILE")
              if kill -0 "$PID" 2>/dev/null; then
                if ssh_vm_probe root@localhost true 2>/dev/null; then
                  echo "VM already running (PID $PID)"
                  VM_RUNNING=true
                else
                  echo "QEMU running but SSH not responding, restarting..."
                  kill "$PID" 2>/dev/null || true
                  for _ in $(seq 1 10); do kill -0 "$PID" 2>/dev/null || break; sleep 1; done
                  kill -9 "$PID" 2>/dev/null || true
                  rm -f "$PIDFILE"
                fi
              else
                rm -f "$PIDFILE"
              fi
            fi

            if [[ "$VM_RUNNING" == false ]]; then
              header "Starting VM"
              ACCEL_ARGS=""
              ${if isDarwin then ''
              ACCEL_ARGS="-accel hvf -cpu host"
              '' else ''
              if [[ -e /dev/kvm ]]; then
                ACCEL_ARGS="-enable-kvm -cpu host"
              else
                echo "WARNING: KVM not available, using TCG (slow)"
                ACCEL_ARGS="-cpu max"
              fi
              ''}

              # Host QEMU binary.
              GPU_ARGS=""
              QEMU_BIN="${qemu}/bin/${qemuBin}"
              # Optional pin of virgl's host GL render node (e.g. a discrete GPU)
              # via BMC_VIRT_RENDERNODE; unset = QEMU auto-selects. Applies to the
              # egl-headless paths only.
              RENDERNODE_ARG="''${BMC_VIRT_RENDERNODE:+,rendernode=$BMC_VIRT_RENDERNODE}"
              if [[ "$(uname)" == "Darwin" ]]; then
                # The QEMU here comes from the darwin-qemu-virgl flake input —
                # patched for ANGLE-backed virgl on macOS. The working display
                # backend is `cocoa,gl=es`, which opens a small QEMU-owned
                # native window where virgl gets its Metal-backed GL context.
                # `egl-headless` is listed by `-display help` but fails at
                # runtime ("egl: not available on this platform") — macOS has
                # no native EGL.
                GPU_ARGS="-device virtio-gpu-gl-pci,xres=480,yres=1280 -display cocoa,gl=es"
                echo "QEMU: using nix binary at $QEMU_BIN"
                echo "GPU: virgl via ANGLE→Metal (hardware-accelerated)"
              elif [ -c /dev/dri/renderD128 ]; then
                # GPU: try virgl (hardware-accelerated) first, fall back to software.
                # virgl needs a host render node + QEMU with egl-headless.
                # Nix-built QEMU/libgbm hardcodes /run/opengl-driver (NixOS convention),
                # so on non-NixOS we prefer the system QEMU which links against system mesa.
                # Prefer system QEMU for virgl on non-NixOS (avoids mesa path mismatch)
                SYS_QEMU="$(command -v ${qemuBin} 2>/dev/null || true)"
                if [ -n "$SYS_QEMU" ] && "$SYS_QEMU" -display help 2>&1 | grep -q egl-headless; then
                  QEMU_BIN="$SYS_QEMU"
                  GPU_ARGS="-device virtio-gpu-gl-pci,xres=480,yres=1280 -display egl-headless$RENDERNODE_ARG"
                  echo "GPU: virgl via system QEMU (hardware-accelerated)"
                elif ${qemu}/bin/${qemuBin} -display help 2>&1 | grep -q egl-headless && \
                     [ -d /run/opengl-driver ]; then
                  GPU_ARGS="-device virtio-gpu-gl-pci,xres=480,yres=1280 -display egl-headless$RENDERNODE_ARG"
                  echo "GPU: virgl via nix QEMU (hardware-accelerated)"
                else
                  GPU_ARGS="-device virtio-gpu-pci,xres=480,yres=1280 -display none"
                  echo "GPU: software rendering (no virgl-capable QEMU found)"
                fi
              else
                GPU_ARGS="-device virtio-gpu-pci,xres=480,yres=1280 -display none"
                echo "GPU: software rendering (no render node)"
              fi

              # Always boot the custom kernel so we control boot params.
              # The video= parameter forces virtio-gpu to expose a 480x1280 mode
              # matching the real hardware panel.
              CUSTOM_KERNEL=$(${pkgs.nix}/bin/nix build -L \
                "path:$WORKSPACE/bmc-virt#customKernel" \
                --no-link --print-out-paths)/vmlinuz

              # Audio passthrough — detect host audio backend and add virtual sound card.
              # Note: a pipewire socket on the host is not enough — QEMU must also be
              # built with the pipewire backend (distros like Guix ship without it).
              AUDIO_ARGS=""
              XDG="''${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
              SOUND_DEV="-device virtio-sound-pci,audiodev=snd0"
              QEMU_AUDIODEVS="$("$QEMU_BIN" -audiodev help 2>&1 || true)"
              if [[ "$(uname)" == "Darwin" ]]; then
                AUDIO_ARGS="-audiodev coreaudio,id=snd0 $SOUND_DEV"
                echo "Audio: coreaudio"
              elif [[ -S "$XDG/pipewire-0" ]] && grep -qw pipewire <<<"$QEMU_AUDIODEVS"; then
                AUDIO_ARGS="-audiodev pipewire,id=snd0 $SOUND_DEV"
                echo "Audio: pipewire"
              elif [[ -S "$XDG/pulse/native" ]] || [[ -n "''${PULSE_SERVER:-}" ]]; then
                AUDIO_ARGS="-audiodev pa,id=snd0 $SOUND_DEV"
                echo "Audio: pulseaudio"
              else
                echo "WARNING: No audio backend detected, sound disabled"
              fi

              # hwsim params:
              #   radios=2  — radio0 for app, radio1 as fake upstream AP (BMC-VIRT-UPLINK)
              #   channels=2 — enables hw_scan ops so AP-mode interfaces can scan
              #                 (works with kernel-patches/002-mac80211-ap-scan.patch)
              #
              # ACCEL/GPU/AUDIO args are intentionally preassembled shell words.
              # Quoting them would collapse each option bundle into a single argv
              # entry and break the QEMU invocation.
              #
              # -daemonize forks qemu off the controlling terminal so a host-side
              # SIGHUP (terminal close, ssh disconnect, IDE restart) doesn't kill
              # the VM. -pidfile lets qemu write the post-fork pid itself.
              # shellcheck disable=SC2086
              $QEMU_BIN \
                -machine ${qemuMachine} \
                $ACCEL_ARGS \
                -m 512M \
                -drive file="$OVERLAY",format=qcow2,if=virtio \
                -kernel "$CUSTOM_KERNEL" \
                -append "root=/dev/vda2 rootfstype=ext4 rootwait console=${consoleDevice} drm_kms_helper.fbdev_emulation=0 mac80211_hwsim.radios=2 mac80211_hwsim.channels=2" \
                -vga none \
                $GPU_ARGS \
                -device virtio-net-pci,netdev=net0 \
                -netdev user,id=net0,net=192.168.1.0/24,hostfwd=tcp::${toString ports.ssh}-192.168.1.1:22,hostfwd=tcp::${toString ports.http}-192.168.1.1:80,hostfwd=tcp::${toString ports.ipc}-192.168.1.1:${toString ports.ipc},hostfwd=tcp::${toString ports.event}-192.168.1.1:${toString ports.event} \
                -device virtio-tablet-pci \
                -object rng-random,filename=/dev/urandom,id=rng0 \
                -device virtio-rng-pci,rng=rng0 \
                -virtfs local,path=/nix/store,mount_tag=nixstore,security_model=none,readonly=on \
                $AUDIO_ARGS \
                -serial file:"$LOGDIR/serial.log" \
                -daemonize \
                -pidfile "$PIDFILE" \
                > "$LOGDIR/qemu.log" 2>&1

              echo "Waiting for SSH..."
              for i in $(seq 1 90); do
                if ssh_vm_probe root@localhost true 2>/dev/null; then echo "SSH ready."; break; fi
                if ! kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
                  echo "ERROR: QEMU exited unexpectedly"
                  cat "$LOGDIR/qemu.log" 2>/dev/null || true
                  exit 1
                fi
                if (( i % 5 == 0 )); then echo "Still waiting... ($i/90)"; fi
                if [[ $i -eq 90 ]]; then
                  echo "ERROR: VM did not become ready"
                  tail -50 "$LOGDIR/serial.log" 2>/dev/null || true
                  exit 1
                fi
                sleep 1
              done
              # Run the first-boot scripts through the guest shell. The single-
              # quoted heredoc is intentional: host-side expansion must not touch
              # the guest loop variables or redirections.
              # shellcheck disable=SC2016
              ssh_vm root@localhost 'for f in /etc/uci-defaults/*; do
                [ -x "$f" ] || continue; "$f" && rm -f "$f" || echo "WARNING: $f failed" >&2
              done' || true
            fi

            if [[ -n "$CONFIG" ]]; then
              header "Deploying config"
              ssh_vm root@localhost 'mkdir -p /etc/bmc'
              scp_vm "$CONFIG" "root@localhost:${guestPaths.BMC_CONFIG}"

              header "Applying provisioned device state"
              ssh_vm root@localhost '
                # Preloaded configs should bypass first-boot onboarding.
                . /lib/functions/bos-defaults.sh
                unset_factory_default
                unset_setup_pending
                unset_wifi_reconfig

                # Connect radio0 to the fake upstream AP so the app sees WiFi as connected.
                # This mimics a device that has already completed WiFi setup.
                uci set wireless.default_radio0.mode="sta"
                uci set wireless.default_radio0.network="wifi_sta"
                uci set wireless.default_radio0.ssid="${uplinkSsid}"
                uci set wireless.default_radio0.encryption="psk2"
                uci set wireless.default_radio0.key="${uplinkKey}"
                uci set wireless.default_radio0.disabled="0"
                uci commit wireless
                wifi reload
              '
            fi

            if [[ -n "$RR" ]]; then
              ${if isAarch64 then ''
              echo "ERROR: rr is x86_64-only, not available on aarch64 hosts" >&2
              exit 1
              '' else ''
              header "Deploying rr bundle"
              RR_BUNDLE=$(${pkgs.nix}/bin/nix build -L \
                "path:$WORKSPACE/bmc-virt#rrBundle" \
                --no-link --print-out-paths)
              # Always deploy — overlay is recreated fresh on every run
              ssh_vm root@localhost 'rm -rf ${rrGuestBundle}'
              tar -C "$RR_BUNDLE" -cf - . | ssh_vm root@localhost 'mkdir -p ${rrGuestBundle} && tar -C ${rrGuestBundle} -xf -'
              ssh_vm root@localhost 'chmod +x ${rrGuestBundle}/bin/*'
              ''}
            fi

            if [[ -n "$HOST_PATH_DIRS" ]]; then
              header "Installing host tools"
              ssh_vm root@localhost 'mkdir -p /root/host-tools'
              IFS=':' read -ra DIRS <<< "$HOST_PATH_DIRS"
              for dir in "''${DIRS[@]}"; do
                tar -C "$dir" -cf - . | ssh_vm root@localhost 'tar -C /root/host-tools -xf -'
              done
              # Append a guest-side PATH export literally. The escaped `$PATH`
              # must survive the host shell unchanged so the guest shell expands
              # it when `/etc/profile` is sourced later.
              # shellcheck disable=SC2016
              ssh_vm root@localhost \
                'grep -qF /root/host-tools /etc/profile || echo "export PATH=\"\$PATH:/root/host-tools\"" >> /etc/profile'
            fi

            header "Deploying guest overlay"
            TMP_GUEST_OVERLAY=$(mktemp -d)
            cleanup_guest_overlay() {
              rm -rf "$TMP_GUEST_OVERLAY"
            }
            trap cleanup_guest_overlay EXIT
            cp -a ${overlayDir}/. "$TMP_GUEST_OVERLAY/"
            chmod -R u+w "$TMP_GUEST_OVERLAY"

            # Template WiFi uplink credentials in all overlay files
            ${templateUplink pkgs "$TMP_GUEST_OVERLAY"}

            # Binaries
            mkdir -p "$TMP_GUEST_OVERLAY/usr/bin"
            install -m755 "${guestPkgsStatic.just}/bin/just" \
              "$TMP_GUEST_OVERLAY/usr/bin/just"
            install -m755 "$RELAY_BINARY" \
              "$TMP_GUEST_OVERLAY/usr/bin/bmc-virt-relay"
            install -m755 "$BINARY" \
              "$TMP_GUEST_OVERLAY${guestPaths.BMC_BIN}"
            if [[ -n "$LED_BINARY" ]]; then
              install -m755 "$LED_BINARY" \
                "$TMP_GUEST_OVERLAY/root/bmc-virt-leds"
            fi

            # Widgets path (nix store path, accessible via 9p mount)
            echo "$WIDGETS/lib/bmc-widgets" > "$TMP_GUEST_OVERLAY/etc/bmc-virt/widgets-path"
            printf '{"compositor":{"commands":[["%s/bin/bmc-wasm-host"]]}}\n' "$WASM_HOST" \
              > "$TMP_GUEST_OVERLAY/etc/bmc_system.json"

            # Event daemon — deployed from nix-built harness venv (uv.lock deps)
            echo "${eventdEnv}/bin/python3" > "$TMP_GUEST_OVERLAY/etc/bmc-virt/eventd-python"

            # Env files rendered via `pkgs.writeText` in the let block above.
            # Host-side forwarded ports are sourced by the login banner; guest
            # paths are sourced by init scripts, the relay, the justfile, and
            # get-logs.sh.
            install -m644 ${portsEnvFile} \
              "$TMP_GUEST_OVERLAY/etc/bmc-virt/ports.env"
            install -m644 ${pathsEnvFile} \
              "$TMP_GUEST_OVERLAY/etc/bmc-virt/paths.env"

            # Frontend assets
            mkdir -p "$TMP_GUEST_OVERLAY/www/bmc"
            cp -a "$FRONTEND"/. "$TMP_GUEST_OVERLAY/www/bmc/"

            # Sounds
            mkdir -p "$TMP_GUEST_OVERLAY/usr/share/bmc/sounds"
            cp -a "$SOUNDS"/. "$TMP_GUEST_OVERLAY/usr/share/bmc/sounds/"

            # Prebuilt WASM bytes used by VM configs and harnesses
            # — SDK examples plus production widgets, both flat in WASM_DIR.
            mkdir -p "$TMP_GUEST_OVERLAY${guestPaths.WASM_DIR}"
            cp -a "$WASM_EXAMPLES"/. "$TMP_GUEST_OVERLAY${guestPaths.WASM_DIR}/"
            # `cp -a` stamps the dir mode to the read-only store source.
            # Restore write before the second copy can add files into it.
            chmod -R u+w "$TMP_GUEST_OVERLAY${guestPaths.WASM_DIR}"
            cp -a "$WASM_WIDGETS"/. "$TMP_GUEST_OVERLAY${guestPaths.WASM_DIR}/"

            # Fonts
            mkdir -p "$TMP_GUEST_OVERLAY/usr/share/fonts/truetype"
            cp "${pkgs.corefonts}/share/fonts/truetype/Arial.ttf" \
              "$TMP_GUEST_OVERLAY/usr/share/fonts/truetype/Arial.ttf"

            # Ensure all files are writable so cleanup_guest_overlay can rm them
            # (Nix store sources are read-only).
            chmod -R u+w "$TMP_GUEST_OVERLAY"

            # Push to VM
            ssh_vm root@localhost 'killall bmc-openwrt bmc-virt-relay 2>/dev/null; sleep 1
              # Clean up all bmc-virt rc.d/init.d entries before deploying fresh overlay
              rm -f /etc/rc.d/S*-bmc-virt-* /etc/rc.d/S*-bmc-openwrt
              rm -f /etc/init.d/*-bmc-virt-* /etc/init.d/*-bmc-openwrt
            ' || true
            tar -C "$TMP_GUEST_OVERLAY" --exclude='./etc/config' --exclude='./etc/uci-defaults' -cf - . | ssh_vm root@localhost 'tar -C / -xf -'
            cleanup_guest_overlay
            trap - EXIT

            # Device setup + services are handled by init.d scripts in the
            # overlay (80-bmc-virt-setup, 85-bmc-openwrt, 90-bmc-virt-relay).
            # On first deploy we run them explicitly; on reboot they run automatically.
            header "Starting services"
            # Select bmc-openwrt's launch mode via an env file that the init.d
            # script sources. Writing or clearing the file keeps a single
            # startup path (procd) for both modes.
            if [[ -n "$RR" ]]; then
              ssh_vm root@localhost '
                mkdir -p /etc/bmc-virt
                cat > /etc/bmc-virt/bmc-openwrt.env <<EOF
            RR_ENABLED=1
            RR_BUNDLE=${rrGuestBundle}
            RR_TRACE_DIR=${rrGuestTraceDir}
            XDG_RUNTIME_DIR_OVERRIDE=/run/user/0
            EOF
              '
            else
              ssh_vm root@localhost 'rm -f /etc/bmc-virt/bmc-openwrt.env'
            fi
            # Start all bmc-virt services in rc.d order.
            # Uses "boot" action — works for both boot()-only scripts
            # (a-bmc-virt-setup, c-bmc-virt-wifi) and procd start_service() scripts.
            #
            # The service loop executes on the guest. Keep the block single-
            # quoted so `$(basename ...)` and `$s` are evaluated by the guest
            # shell rather than interpolated by the host shellcheck wrapper.
            # shellcheck disable=SC2016
            ssh_vm root@localhost '
              for s in /etc/rc.d/S*-bmc-*; do
                echo "Starting $(basename $s)..."
                "$s" boot
              done
            '
            if [[ -n "$RR" ]]; then
              # Give rr a moment to start (or crash) and verify it is alive
              # `${guestPaths.BMC_LOG}` is substituted by Nix here, but the rest
              # of the block must stay literal until it reaches the guest shell.
              # shellcheck disable=SC2016
              ssh_vm root@localhost '
                sleep 2
                if ! pgrep -f "rr record" >/dev/null 2>&1; then
                  echo "ERROR: rr failed to start. Log:"
                  cat ${guestPaths.BMC_LOG}
                  exit 1
                fi
                echo "bmc-openwrt started under rr"
              '
            fi

            if [[ -z "$RR" ]]; then
              header "Opening display"
              WORKSPACE="$WORKSPACE" ${display}/bin/bmc-virt-display
            else
              echo "Skipping display console (headless compositor under rr)"
            fi

            header "Connecting (Ctrl+D or 'exit' to disconnect, VM keeps running)"
            echo "Ports: HTTP/gRPC=${toString ports.http}  SSH=${toString ports.ssh}"
            if [[ -n "$RR" ]]; then
              echo "rr recording active — exit SSH to stop and pull the recording"
            fi
            ssh_vm -t root@localhost bash -l

            # After SSH disconnect: pull rr recording if active
            if [[ -n "$RR" ]]; then
              header "Pulling rr recording from VM"
              # SIGTERM rr (not bmc-openwrt directly) so rr finalizes the trace cleanly.
              # rr forwards the signal to the child and writes a complete recording.
              #
              # `RR_PID` is a guest-side shell variable, so this command must stay
              # single-quoted until it runs on the VM.
              # shellcheck disable=SC2016
              ssh_vm root@localhost 'RR_PID=$(pgrep -f "rr record"); kill $RR_PID 2>/dev/null; sleep 5' || true

              RR_TRACE=$(ssh_vm root@localhost 'ls -td ${rrGuestTraceDir}/bmc-openwrt-* 2>/dev/null | head -1' 2>/dev/null)
              if [[ -n "$RR_TRACE" ]]; then
                LOCAL_TRACE="$DATADIR/rr-traces/$(basename "$RR_TRACE")"
                mkdir -p "$LOCAL_TRACE"
                ssh_vm root@localhost "tar -C '$RR_TRACE' -cf - ." | tar -C "$LOCAL_TRACE" -xf -

                echo ""
                echo -e "\033[1;32mRecording saved: $LOCAL_TRACE\033[0m"
                echo ""
                echo "Replay:"
                echo "  nix shell nixpkgs#rr -c rr replay $LOCAL_TRACE"
                echo ""
                echo "Useful commands inside rr replay (GDB):"
                echo "  reverse-continue    — run backwards to previous breakpoint"
                echo "  reverse-step        — step one source line backwards"
                echo "  reverse-next        — step backwards over function calls"
                echo "  watch -l <expr>     — break when memory changes (works in reverse too)"
                echo ""
                echo "AMD Zen CPUs: disable SpecLockMap before replay or rr will diverge:"
                echo "  sudo modprobe msr"
                echo "  sudo wrmsr -a 0xc0011020 \$(\$(sudo rdmsr -c 0xc0011020) | (1 << 54))"
                echo "  sudo sysctl kernel.perf_event_paranoid=1"
                echo ""
              else
                echo "WARNING: No rr recording found on VM"
              fi
            fi
          '';
        };

        # Launch the host console app (connects to relay via TCP IPC).
        display = pkgs.writeShellScriptBin "bmc-virt-display" ''
          set -euo pipefail
          DATADIR="''${BMC_VIRT_DATA:-$(pwd)/vm-data}"
          : "''${WORKSPACE:?WORKSPACE must be set by caller}"

          echo "Starting console app..."
          # nohup so the console survives parent shell SIGHUP (terminal close,
          # ssh disconnect, IDE restart). </dev/null detaches stdin.
          BMC_VIRT_RELAY_ADDR="127.0.0.1:${toString ports.ipc}" \
            nohup cargo run --manifest-path "$WORKSPACE/Cargo.toml" -p bmc-virt-console \
            </dev/null >> "$DATADIR/console.log" 2>&1 &
          disown
        '';

        stop = pkgs.writeShellScriptBin "bmc-virt-stop" ''
          set -euo pipefail

          DATADIR="''${BMC_VIRT_DATA:-$(pwd)/vm-data}"
          PIDFILE="$DATADIR/qemu.pid"
          if [[ -f "$PIDFILE" ]]; then
            PID=$(cat "$PIDFILE")
            if kill -0 "$PID" 2>/dev/null; then kill "$PID"; echo "Stopped VM (PID $PID)"; fi
            rm -f "$PIDFILE"
          fi

          # Kill any lingering qemu using our overlay
          pkill -f "qemu-system.*$DATADIR/overlay.qcow2" 2>/dev/null || true
        '';

        # ── Custom linux-builder VM with x86_64 binfmt (macOS only) ─────────
        # The OpenWrt ImageBuilder is an x86_64 binary.  On aarch64 hosts we
        # need binfmt emulation so nix can build x86_64-linux derivations.
        # This builds a linux-builder VM identical to the Determinate Nix one
        # but with boot.binfmt.emulatedSystems = ["x86_64-linux"].
        linuxBuilder = if !isDarwin then null else
        (nixpkgs.lib.nixosSystem {
          system = "aarch64-linux";
          modules = [
            "${nixpkgs}/nixos/modules/profiles/nix-builder-vm.nix"
            {
              boot.binfmt.emulatedSystems = [ "x86_64-linux" ];
              networking.nameservers = [ "1.1.1.1" "8.8.8.8" "9.9.9.9" ];
              virtualisation.host.pkgs = pkgs;
              virtualisation.cores = 4;
              virtualisation.darwin-builder.diskSize = 80 * 1024;
              virtualisation.darwin-builder.memorySize = 12 * 1024;
            }
          ];
        }).config.system.build.macos-builder-installer;

      in
      {
        packages = {
          inherit vmImageBase customKernel vmImage run stop display;
        } // pkgs.lib.optionalAttrs (!isAarch64) {
          inherit rrBundle;
        } // pkgs.lib.optionalAttrs isDarwin {
          inherit linuxBuilder;
        };

        apps = {
          run = flake-utils.lib.mkApp { drv = run; };
          stop = flake-utils.lib.mkApp { drv = stop; };
          display = flake-utils.lib.mkApp { drv = display; };
          default = flake-utils.lib.mkApp { drv = run; };
        };

        devShells.default = pkgs.mkShell {
          name = "bmc-virt-env";
          packages = [ run stop qemu pkgs.libxkbcommon.dev pkgs.libinput.dev ];
          env = consoleHostRustflagsEnv;
        };
      });
}
