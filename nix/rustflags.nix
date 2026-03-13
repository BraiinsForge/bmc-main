# Runtime dependency lists and RUSTFLAGS helpers for rpath-based linking.
{ lib }:
let
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
    makeRpathLinkArgument
    makeRustflagsEnv;
}
