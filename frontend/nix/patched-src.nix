{ pkgs }:

pkgs.stdenv.mkDerivation {
  name = "bmc-fe-patched-src";
  src = ../.;

  dontBuild = true;
  dontUnpack = true;

  installPhase = "cp -r $src $out";

  # dependencies used for automatic shebang patching in fixupPhase
  buildInputs = [ pkgs.yarn ];
}
