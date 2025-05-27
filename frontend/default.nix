 { self, pkgs }:

 let
   src = import ./nix/patched-src.nix { inherit pkgs; };
   yarnFiles = import ./nix/yarn-files.nix { inherit pkgs; };
in {
    build = pkgs.stdenv.mkDerivation {
      name = "bmc-frontend";
      inherit src;

      buildInputs = [ pkgs.yarn ];

      buildPhase = ''
        cp -r ${yarnFiles}/. -t .
        export HOME=$(pwd)
        make build
      '';

      installPhase = ''
        mkdir -p $out
        cp -r ./dist/. -t $out
      '';
    };

    checks = import ./nix/checks.nix { inherit pkgs src yarnFiles; };
}
