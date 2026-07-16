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

{ lib
, ffmpeg
, alsa-lib
, pkg-config
, xxd
, stdenv
, ...
}:

let
  # ALSA with config paths pointing to /usr/share/alsa and without installing config files
  alsa-libCustom = alsa-lib.overrideAttrs (old: {
    configureFlags = (old.configureFlags or [ ]) ++ [
      "--with-configdir=/usr/share/alsa"
      "--with-plugindir=/usr/lib/alsa-lib"
    ];

    # Disable install-alsaDATA target to prevent installing config files to absolute paths
    preInstall = ''
      find . -name Makefile -exec sed -i "s|install-alsaDATA||g" {} +
    '';

    # Disable symlinking of ucm and topology to the output
    postInstall = "";
  });

  # Minimal FFmpeg build with only required libraries for bmc-video
  # Required: libavformat libavcodec libswscale libswresample libavutil
  ffmpegCustom = (ffmpeg.override {
    # Alsa lib with patched config path
    alsa-lib = alsa-libCustom;

    # Disable all dependency groups
    withHeadlessDeps = false;
    withSmallDeps = false;
    withFullDeps = false;

    # Keep only essential features
    withBzlib = true;
    withIconv = true;
    withZlib = true;
    withAlsa = true;

    # Disable runtime CPU detection - compile for target CPU directly
    withRuntimeCPUDetection = false;

    # Disable programs - we only need ffmpeg
    buildFfmpeg = true;
    buildFfplay = false;
    buildFfprobe = false;

    # Enable only required libraries
    buildAvcodec = true;
    buildAvformat = true;
    buildAvutil = true;
    buildSwresample = true;
    buildSwscale = true;
    buildAvdevice = true;
    buildAvfilter = true;
    buildAvresample = true;

    # Disable documentation
    withDocumentation = false;
    withManPages = false;
  }).overrideAttrs (old: {
    # Use preConfigure for flags with spaces (configureFlags splits on spaces)
    preConfigure = (old.preConfigure or "") + ''
      configureFlagsArray+=(
        "--extra-cflags=-mcpu=cortex-a7 -mfpu=neon-vfpv4 -mfloat-abi=hard -Os -ffast-math"
        "--extra-ldflags=-static"
      )
    '';

    configureFlags = (old.configureFlags or [ ]) ++ [
      # Target CPU
      "--cpu=cortex-a7"
      "--enable-neon"
      "--enable-asm"
      "--disable-runtime-cpudetect"
      "--enable-small"

      "--disable-bzlib"
      "--disable-iconv"
      "--disable-zlib"

      # Minimal build - disable everything, then enable only what we need
      "--disable-everything"
      "--disable-doc"
      "--disable-network"

      "--enable-static"
      "--disable-shared"

      # Demuxers (container formats)
      "--enable-demuxer=mov"
      "--enable-demuxer=mp4"
      "--enable-demuxer=m4v"

      # Fbdev output
      "--enable-avdevice"
      # "--enable-outdevs"
      # "--enable-indevs"
      "--enable-muxer=fbdev"
      "--enable-outdev=fbdev"
      "--enable-outdev=alsa"
      "--enable-encoder=rawvideo"

      # Conversion (video and audio)
      "--enable-filter=scale"
      "--enable-filter=format"
      "--enable-filter=volume"
      "--enable-filter=aresample"

      # Alsa output
      "--enable-encoder=pcm_s16le"

      # Video decoders
      "--enable-decoder=h264"

      # Audio decoders
      "--enable-decoder=aac"
      "--enable-decoder=mp3"
      "--enable-decoder=pcm_s16le"

      # Parsers
      "--enable-parser=h264"
      "--enable-parser=aac"

      # Protocols
      "--enable-protocol=file"

      # Swscale for pixel format conversion
      "--enable-swscale"

      # Swresample for audio resampling
      "--enable-swresample"
    ];
  });

in
stdenv.mkDerivation {
  pname = "bmc-video";
  version = "1.0.0";

  src = ./.;

  makeFlags = [
    "CROSS_PREFIX=${stdenv.cc.targetPrefix}"
    "FFMPEG_BIN=${ffmpegCustom.bin}/bin/ffmpeg"
  ];

  nativeBuildInputs = [
    pkg-config
    xxd
  ];

  buildInputs = [
    ffmpegCustom
    alsa-libCustom
  ];

  installPhase = ''
    runHook preInstall
    mkdir -p $out/bin
    cp bmc-video-play $out/bin/
    runHook postInstall
  '';
}
