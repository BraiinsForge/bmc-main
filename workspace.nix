{ self, pkgs }:
let lib = pkgs.lib; in
let
  crates = with pkgs.ii.rust; {
    app = defineCrate {
      path = ./crates/app;
      packageName = "app";
    };
    bmc-core = defineCrate {
      path = ./crates/bmc-core;
      packageName = "bmc-core";
    };
    mock = defineCrate {
      path = ./crates/mock;
      packageName = "mock";
    };
    openwrt = defineCrate {
      path = ./crates/openwrt;
      packageName = "openwrt";
    };
  };

  workspace = pkgs.ii.rust.mkWorkspaceConfig {
    src = ./.;
    # packages that can be executed during compilation
    nativeDeps = pkgs: with pkgs; [
      protobuf
    ];
    # packages that will be cross-compiled for target arch
    targetDeps = build_pkgs: with build_pkgs; [
      openssl.dev
    ];
  };

  gitEnv = {
    # https://nixos.org/manual/nix/stable/command-ref/new-cli/nix3-flake.html#flake-reference-attributes
    GIT_HASH = self.rev or self.dirtyRev or "dirty";
    GIT_TIMESTAMP = self.lastModified;
  };

  build-profiles = with workspace; {
    # fast profile (no cross compilation, non-portable binaries)
    fast = mkBuildProfile {
      minimal_deps = false;
      rustProfile = "fast";
    };
    native = mkBuildProfile {
      minimal_deps = false;
      rustProfile = "release";
    };
    x86_64-linux = mkBuildProfile {
      suffix = "x86_64-linux";
      minimal_deps = true;
      rustProfile = "release";
      rustCrossTarget = "x86_64-unknown-linux-musl";
      build_pkgs = pkgs.pkgsCross.musl64.pkgsStatic;
      env = gitEnv;
    };
  };


  ######################################################################################################################


  nativePackages = with build-profiles.native; {
    app = buildCrate crates.app { };
    bmc-core = buildCrate crates.bmc-core { };
    mock = buildCrate crates.mock { };
    openwrt = buildCrate crates.openwrt { };
  };


  crossOutputs = rec {
    x86_64-linux = with build-profiles.x86_64-linux; {
      app = buildCrate crates.app { };
      bmc-core = buildCrate crates.bmc-core { };
      mock = buildCrate crates.mock { };
      openwrt = buildCrate crates.openwrt { };
    };
  };

  # Convert `crossOutputs` from `${arch}.${name}` to `"${name}-${arch}"`.
  crossPackages = lib.concatMapAttrs
    (arch: packages: pkgs.ii.lib.mapAttrNames (name: "${name}-${arch}") packages)
    crossOutputs;


  specialPackages = {
    workspace-deps = build-profiles.fast.deps;
    inherit (build-profiles.fast) build clippy test nextest;
  };

in
{
  packages = nativePackages // crossPackages // specialPackages;
  devShells = pkgs.ii.lib.mapAttrValues (profile: profile.shell) build-profiles;
}
