{ pkgs ? import <nixpkgs> { } }:

pkgs.mkShell {
  buildInputs = with pkgs; [
    # JavaScript/Node.js tools
    yarn
    volta
    # Build tools
    cmake
  ];

  # Shell hook for environment variables
  shellHook = ''
    echo "Development environment loaded with yarn, volta, and cmake"
    yarn install
    echo "yarn install done"
    echo "Build frontend with 'make build'"
  '';
}
