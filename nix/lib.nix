# bmc-lib: Shared package builders for the BMC project.
{ pkgs, lib, armv7Pkgs }:
let
  autopatchelfBinaries = import ./autopatchelf-binaries.nix {
    inherit lib;
    autoPatchelfHook = armv7Pkgs.autoPatchelfHook;
  };
in
{
  inherit autopatchelfBinaries;
  mkCorePackage = import ./pkgs/core/package.nix { inherit pkgs lib; };
  inherit (import ./widget.nix { inherit pkgs lib autopatchelfBinaries; })
    mkWidgetPackage mkAllWidgets;
}
