{
  description = "BDK-377 firmware signer: keygen + init-tarball signing";

  inputs.bmc-main.url = "path:../../..";

  outputs = { self, bmc-main, ... }:
    let
      system = "x86_64-linux";
      pkgs = bmc-main.legacyPackages.${system}.pkgs;

      fw-keygen = pkgs.writeShellApplication {
        name = "fw-keygen";
        runtimeInputs = [ pkgs.signify ];
        text = builtins.readFile ./fw-keygen.sh;
      };

      fw-sign-init-tarball = pkgs.writeShellApplication {
        name = "fw-sign-init-tarball";
        runtimeInputs = [ pkgs.signify pkgs.nix ];
        text = builtins.readFile ./fw-sign-init-tarball.sh;
      };
    in
    {
      packages.${system} = {
        inherit fw-keygen fw-sign-init-tarball;
      };

      devShells.${system}.default = pkgs.mkShell {
        name = "bdk-377-fw-signer";
        packages = [
          pkgs.signify
          fw-keygen
          fw-sign-init-tarball
        ];
      };
    };
}
