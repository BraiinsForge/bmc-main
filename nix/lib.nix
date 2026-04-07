# bmc-lib: Shared package builders for the BMC project.
{ pkgs, lib, armv7Pkgs }:
let
  autopatchelfBinaries = import ./autopatchelf-binaries.nix {
    inherit lib;
    autoPatchelfHook = armv7Pkgs.autoPatchelfHook;
  };
  packageLib = import ./package.nix { inherit pkgs lib; };
  serviceLib = import ./service.nix { inherit pkgs lib; };
in
{
  inherit autopatchelfBinaries;
  inherit (packageLib) mkPackage mkPrioritizedEntries;
  inherit (serviceLib) mkOpenWrtService mkOpenWrtDaemon;
  inherit (import ./widget.nix { inherit pkgs lib autopatchelfBinaries; })
    mkWidgetPackage mkAllWidgets;
}
