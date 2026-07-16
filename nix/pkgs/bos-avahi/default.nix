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

# BOS Avahi package: OpenWrt avahi-daemon service plus miner-gated
# BOS HTTP mDNS advertisement.
{ bmc, armv7Pkgs }:
let
  inherit (bmc.lib) mkPackage mkOpenWrtDaemon;

  avahiConfig = armv7Pkgs.writeText "avahi-daemon.conf" ''
    [server]
    use-ipv4=yes
    use-ipv6=yes
    check-response-ttl=no
    use-iff-running=no
    enable-dbus=no

    [publish]
    publish-addresses=yes
    publish-hinfo=no
    publish-workstation=no
    publish-domain=yes

    [rlimits]
    rlimit-core=0
    rlimit-data=4194304
    rlimit-fsize=0
    rlimit-nofile=30
    rlimit-stack=4194304
    rlimit-nproc=3
  '';

  avahiDaemon = mkOpenWrtDaemon {
    name = "avahi-daemon";
    start = 95;
    stop = 89;
    command = "${armv7Pkgs.avahi}/bin/avahi-daemon";
    args = [
      "--file=/etc/avahi/avahi-daemon.conf"
    ];
    preStart = ''
      mkdir -p /var/run/avahi-daemon
      chown avahi:avahi /var/run/avahi-daemon 2>/dev/null || true
    '';
    pidFile = "/var/run/avahi-daemon/pid";
  };

  activationTest = armv7Pkgs.runCommand "bos-avahi-activation-test" { } ''
    sh ${./tests/activation.sh} ${./activation/bos-avahi}
    touch $out
  '';

  activationScript = armv7Pkgs.runCommand "bos-avahi-activation" { } ''
    mkdir -p $out/bin
    cp ${./activation/bos-avahi} $out/bin/bos-avahi
    chmod 755 $out/bin/bos-avahi
  '';

  package = mkPackage {
    name = "bos-avahi";
    package = armv7Pkgs.avahi;
    activation = [
      { prefix = "070"; bin = activationScript; }
    ];
    services = [ avahiDaemon ];
    copyFiles = [
      { src = avahiConfig; dest = "/etc/avahi/avahi-daemon.conf"; }
    ];
    conffiles = [
      "/etc/avahi/avahi-daemon.conf"
    ];
  };

  packageWithTests = package.overrideAttrs (old: {
    passthru = (old.passthru or { }) // {
      tests.activation = activationTest;
    };
  });
in
{
  pkg = packageWithTests;
  version =
    # Package indexes validate versions with semver::Version::parse, which
    # rejects upstream avahi's two-component "0.8"; assert nixpkgs still
    # ships exactly 0.8 so a bump fails at eval instead of silently
    # reporting a stale version.
    assert armv7Pkgs.avahi.version == "0.8";
    "0.8.0";
  category = "core";
  description = "Avahi daemon and BOS miner mDNS advertisement";
  upgrade_strategy = null;
  install_strategy = null;
}
