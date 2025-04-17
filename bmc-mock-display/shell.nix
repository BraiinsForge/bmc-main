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

    # Additional graphics libraries
    libxkbcommon
    libGL
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
    ]}:$LD_LIBRARY_PATH
  '';
}
