{ pkgs }:

pkgs.stdenv.mkDerivation {
  name = "ii-fe-yarn-files-fixup";

  dontUnpack = true;
  dontConfigure = true;
  dontBuild = true;

  installPhase = ''
    cp -r $offlineCache $out
  '';

  buildInputs = [ pkgs.gcc-unwrapped ];
  nativeBuildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.autoPatchelfHook ];

  # There are some binaries that
  #  - we do not use
  #  - use musl instead of glibc
  #  - we haven't found a way to not install
  # This is a workaround to ignore errors arising from these.
  autoPatchelfIgnoreMissingDeps = [ "libc.musl-x86_64.so.1" ];

  offlineCache = pkgs.stdenv.mkDerivation {
    pname = "bmc-fe-yarn-offline-cache";

    # This is the essential trick that allows FODs to be invalidated automatically.
    version = "0-" + builtins.hashFile "sha1" ../yarn.lock;

    src = ../.;

    nativeBuildInputs = with pkgs; [
      yarn
      cacert
    ];

    buildPhase = ''
      export HOME="$(mktemp -d)"

      # Disabled telemetry and make sure
      # that cache will be where we expect it
      yarn config set enableTelemetry 0
      yarn config set cacheFolder .yarn/cache
      yarn config set globalFolder .yarn/cache

      yarn config set --json supportedArchitectures.os '["linux", "darwin"]'
      yarn config set --json supportedArchitectures.cpu '["arm", "arm64", "ia32", "x64"]'

      yarn install
    '';

    installPhase = ''
      mkdir -p $out/.yarn
      mv -t $out/.yarn .yarn/cache
      mv -t $out .pnp.* node_modules
    '';

    dontConfigure = true;
    dontFixup = true;

    outputHashMode = "recursive";
    outputHash =
      # Platform-specific hashes because yarn/npm binaries differ between platforms
      # To get the hash for a new platform, set it to pkgs.lib.fakeHash
      # and run `nix build .#yarnFiles` - nix will tell you the expected hash
      {
        "x86_64-linux" = "sha256-TRW4C+5dnlOAOarz2jOGLWa/GEVevBuSTQ50RfB4cdg=";
        "aarch64-linux" = "sha256-TRW4C+5dnlOAOarz2jOGLWa/GEVevBuSTQ50RfB4cdg=";
        "x86_64-darwin" = "sha256-+1ECHtN3HSSSL7YJu2nk5bEPlTbNvR2odhAvzPpt+Mw=";
        "aarch64-darwin" = "sha256-+1ECHtN3HSSSL7YJu2nk5bEPlTbNvR2odhAvzPpt+Mw=";
      }.${pkgs.stdenv.hostPlatform.system} or (throw "Unsupported platform: ${pkgs.stdenv.hostPlatform.system}");
  };
}
