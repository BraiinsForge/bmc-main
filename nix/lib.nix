# bmc-lib: Shared package builders for the BMC project.
{ pkgs, lib, armv7Pkgs }:
let
  autopatchelfBinaries = import ./autopatchelf-binaries.nix {
    inherit lib;
    autoPatchelfHook = armv7Pkgs.autoPatchelfHook;
  };
  packageLib = import ./package.nix { inherit pkgs lib; };
  serviceLib = import ./service.nix { inherit pkgs lib; };
  mkIndex = import ./mkIndex.nix { inherit pkgs lib; };
  mkTarball = import ./mkTarball.nix { inherit pkgs lib mkIndex; };
  mkFactoryIndex = import ./mkFactoryIndex.nix { inherit pkgs; };
  # Symlink a single named binary out of a pre-built `bmc-nix` derivation so
  # every bin compiles once. Asserts the binary exists first, otherwise a wrong
  # or removed name would yield a dangling link that only fails when
  # dereferenced (packaging / device boot).
  selectBmcNixBin = { pkgs, bmcNix }: name: pkgs.runCommand name { } ''
    test -e ${bmcNix}/bin/${name}
    mkdir -p $out/bin
    ln -s ${bmcNix}/bin/${name} $out/bin/${name}
  '';
in
{
  inherit autopatchelfBinaries selectBmcNixBin;
  inherit (packageLib) mkPackage mkPrioritizedEntries;
  inherit (serviceLib) mkOpenWrtService mkOpenWrtDaemon;
  inherit mkIndex mkTarball mkFactoryIndex;
  mkCorePackage = import ./pkgs/core/package.nix { inherit pkgs lib; };
  inherit (import ./widget.nix { inherit pkgs lib autopatchelfBinaries; })
    mkWidgetPackage mkAllWidgets;
}
