{ pkgs ? import <nixpkgs> {
    config.allowUnfree = true;
  }
}:

pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    # Rust dependencies
    rustc
    cargo
    rustfmt
    clippy
    protobuf

    # Rust dependency resolution
    pkg-config

    # gRPC dep
    protobuf
  ];

  buildInputs = with pkgs; [
    fontconfig
    freetype
  ];

  # Set Wayland/X11 backend preference (optional)
  # export WINIT_UNIX_BACKEND=wayland
  # export WINIT_UNIX_BACKEND=x11

  env = {
    # WINIT_UNIX_BACKEND = "wayland";
    # WINIT_UNIX_BACKEND = "x11";
    FONTCONFIG_FILE = pkgs.makeFontsConf {
      fontDirectories = [ pkgs.corefonts pkgs.font-awesome_6 ];
    };

    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS =
      let
        rpathLibs = with pkgs; lib.makeLibraryPath [
          # X11 dependencies for Winit
          xorg.libX11
          xorg.libXcursor
          xorg.libXrandr
          xorg.libXi
          xorg.libXinerama
          xorg.libXext
          xorg.libXft
          xorg.libXrender
          xorg.libxcb

          # Wayland dependencies
          wayland
          libxkbcommon

          fontconfig

          # Additional graphics libraries
          libGL
          vulkan-loader
          libGL.dev
        ];
      in
      "-C link-args=-Wl,-rpath,${rpathLibs}";
  };
}
