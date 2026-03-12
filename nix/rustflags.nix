# Runtime dependency lists and RUSTFLAGS helpers for rpath-based linking.
{ lib }:
let
  X11RuntimeDeps = pkgs:
    with pkgs; [
      xorg.libX11
      xorg.libXcursor
      xorg.libXi
      xorg.libXrandr
      xorg.libXinerama
      xorg.libXext
      xorg.libXft
      xorg.libXrender
      xorg.libxcb
      vulkan-loader
      mesa
    ];

  waylandRuntimeDeps = pkgs:
    with pkgs; [
      wayland
      libxkbcommon
      vulkan-loader
      mesa
    ];

  allRuntimeDeps = pkgs: ((X11RuntimeDeps pkgs) ++ (waylandRuntimeDeps pkgs));

  # Add rpath to produced binaries
  makeRpathLinkArgument = { packages }:
    "-C link-args=-Wl,-rpath,${lib.makeLibraryPath packages}";

  # Create RUSTFLAGS for runtime dlopen of libraries in 'runtimePackages'
  makeRustflagsEnv =
    { runtimePackages, rustCrossTarget }:
    let
      target = lib.toUpper (builtins.replaceStrings [ "-" ] [ "_" ] rustCrossTarget);
      value = makeRpathLinkArgument { packages = runtimePackages; };
    in
    {
      "CARGO_TARGET_${target}_RUSTFLAGS" = value;
    };
in
{
  inherit
    X11RuntimeDeps
    waylandRuntimeDeps
    allRuntimeDeps
    makeRpathLinkArgument
    makeRustflagsEnv
    ;
}
