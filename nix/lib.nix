# bmc-lib: Shared package builders for the BMC project.
{ pkgs, lib, armv7Pkgs }:
let
  autopatchelfBinaries = import ./autopatchelf-binaries.nix {
    inherit lib;
    autoPatchelfHook = armv7Pkgs.autoPatchelfHook;
  };
  mkIndex = import ./mkIndex.nix { inherit pkgs lib; };
  mkTarball = import ./mkTarball.nix { inherit pkgs lib mkIndex; };
  mkFactoryIndex = import ./mkFactoryIndex.nix { inherit pkgs; };
in
{
  inherit autopatchelfBinaries mkIndex mkTarball mkFactoryIndex;
  mkCorePackage = import ./pkgs/core/package.nix { inherit pkgs lib; };
  inherit (import ./widget.nix { inherit pkgs lib autopatchelfBinaries; })
    mkWidgetPackage mkAllWidgets;
}
