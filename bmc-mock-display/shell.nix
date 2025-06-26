{ pkgs ? import <nixpkgs> { } }:

pkgs.mkShell {
  buildInputs = with pkgs; [
    # Rust dependencies
    rustc
    cargo
    rustfmt
    clippy

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
    wayland-protocols
    libxkbcommon

    # Additional graphics libraries
    libGL
    vulkan-loader

    # EGL libraries (needed for graphics rendering)
    libGL.dev
  ];

  # Set environment variables if needed
  shellHook = ''
    export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath [
      pkgs.xorg.libX11
      pkgs.xorg.libXcursor
      pkgs.xorg.libXrandr
      pkgs.xorg.libXi
      pkgs.xorg.libXinerama
      pkgs.libxkbcommon
      pkgs.libGL
      pkgs.wayland
      pkgs.mesa
      pkgs.vulkan-loader
    ]}:$LD_LIBRARY_PATH

    # Set Wayland/X11 backend preference (optional)
    # export WINIT_UNIX_BACKEND=wayland
    # export WINIT_UNIX_BACKEND=x11
  '';
}
