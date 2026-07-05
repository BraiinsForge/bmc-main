# upgrade-server: Turn the developer machine into a Deck upgrade source.
#
# Serves the local /nix/store as a signed binary cache (harmonia, as in
# production) and a package index plus servers.json fragment over static
# HTTP, for `bmc-nix-cli register-server` on the device.
{ pkgs }:
pkgs.writeShellApplication {
  name = "upgrade-server";
  runtimeInputs = with pkgs; [ coreutils curl harmonia jq nix python3 ];
  text = ''exec ${./upgrade-server.sh} "$@"'';
}
