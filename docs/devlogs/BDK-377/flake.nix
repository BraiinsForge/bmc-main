{
  description = "BDK-377 firmware signer: keygen + init-tarball signing";

  inputs.bmc-main.url = "path:../../..";

  outputs = { self, bmc-main, ... }:
    let
      system = "x86_64-linux";
      pkgs = bmc-main.legacyPackages.${system}.pkgs;

      # OpenWrt fwtool — appends a signify/usign signature trailer to a
      # firmware image. Hash pins upstream content so `rev = "HEAD"` is
      # effectively frozen at the first successful fetch.
      fwtool = pkgs.stdenv.mkDerivation {
        pname = "fwtool";
        version = "git";
        src = pkgs.fetchgit {
          url = "https://git.openwrt.org/project/fwtool.git";
          rev = "HEAD";
          hash = "sha256-pKMOpgeVDqJY0sMmcMzeG4zgV0JZkm3A5PYUXRY0cX4=";
        };
        nativeBuildInputs = [ pkgs.cmake ];
      };

      fw-keygen = pkgs.writeShellApplication {
        name = "fw-keygen";
        runtimeInputs = [ pkgs.signify ];
        text = builtins.readFile ./fw-keygen.sh;
      };

      fw-sign-init-tarball = pkgs.writeShellApplication {
        name = "fw-sign-init-tarball";
        runtimeInputs = [ pkgs.signify pkgs.nix fwtool ];
        text = builtins.readFile ./fw-sign-init-tarball.sh;
      };
    in
    {
      packages.${system} = {
        inherit fwtool fw-keygen fw-sign-init-tarball;
      };

      devShells.${system}.default = pkgs.mkShell {
        name = "bdk-377-fw-signer";
        packages = [
          pkgs.signify
          fwtool
          fw-keygen
          fw-sign-init-tarball
        ];
      };
    };
}
