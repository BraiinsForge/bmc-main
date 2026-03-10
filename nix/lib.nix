# bmc-lib: Shared package builders for the BMC project.
{ pkgs, lib }:
{
  mkCorePackage = import ./pkgs/core/package.nix { inherit pkgs lib; };
  inherit (import ./widget.nix { inherit pkgs lib; }) mkWidgetPackage mkAllWidgets;
}
