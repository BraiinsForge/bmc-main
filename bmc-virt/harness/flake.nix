{
  description = "BMC-virt harness — test framework and guest-side event daemon";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    pyproject-nix = {
      url = "github:pyproject-nix/pyproject.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    uv2nix = {
      url = "github:pyproject-nix/uv2nix";
      inputs.pyproject-nix.follows = "pyproject-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    pyproject-build-systems = {
      url = "github:pyproject-nix/build-system-pkgs";
      inputs.pyproject-nix.follows = "pyproject-nix";
      inputs.uv2nix.follows = "uv2nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, pyproject-nix, uv2nix, pyproject-build-systems, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        lib = pkgs.lib;
        python = pkgs.python311;

        # Load workspace from uv.lock
        workspace = uv2nix.lib.workspace.loadWorkspace { workspaceRoot = ./.; };

        # Create overlay from lock file
        overlay = workspace.mkPyprojectOverlay { sourcePreference = "wheel"; };

        # Build the full Python package set with all deps resolved from uv.lock
        baseSet = pkgs.callPackage pyproject-nix.build.packages { inherit python; };
        pythonSet = baseSet.overrideScope (
          lib.composeManyExtensions [
            pyproject-build-systems.overlays.default
            overlay
          ]
        );
      in
      {
        # Deployable virtualenv with all deps installed from uv.lock.
        # Used by the VM flake to deploy the event daemon into the guest.
        packages.default = pythonSet.mkVirtualEnv "bmc-virt-harness" workspace.deps.default;

        devShells.default = pkgs.mkShell {
          name = "bmc-virt-harness";
          packages = [
            python
            pkgs.uv
            pkgs.ruff
            pkgs.ty
            pkgs.just
            pkgs.sshpass
            pkgs.openssh
          ];
          env.LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [
            pkgs.stdenv.cc.cc.lib # libstdc++.so.6 for numpy/matplotlib wheels
          ];
          shellHook = ''
            unset PYTHONPATH
            uv sync
            . .venv/bin/activate
          '';
        };
      });
}
