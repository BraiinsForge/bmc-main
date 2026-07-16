# Copyright (C) 2026  Braiins Forge s.r.o.
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.
#
# Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
# to grant any party a license to this program, or any part thereof,
# under any terms, and such a grant shall be considered distinct from
# the grant above.

# mkIndex: Generate nix-package-index.v1.json from a package list.
#
# Takes a list of package entries (each with a `pkg` derivation and metadata)
# and produces a JSON index file that bmc-nix-cli can consume.
{ pkgs, lib }:
{ packages # [ { pkg; name; version; category; description;
  #     upgrade_strategy; install_strategy; cache ? null;
  #     metadata ? { } } ]
  #   metadata is a free-form JSON map; the core entry carries
  #   bmc_version and optionally changelog, and widget entries carry
  #   nested `widget` picker fields and an `assets` map.
, caches ? [ ] # [ { name; cache_url; cache_key; } ]
, indexes ? [ ] # [ "https://..." ] — federated index URLs
, commit ? "" # git commit hash for provenance field
}:
let
  mkPackageEntry = p: {
    inherit (p) name version;
    store_path = "${p.pkg}";
    category = p.category or null;
    description = p.description or null;
    upgrade_strategy = p.upgrade_strategy or null;
    install_strategy = p.install_strategy or null;
  } // lib.optionalAttrs (p ? metadata && p.metadata != null) {
    inherit (p) metadata;
  } // lib.optionalAttrs (p ? cache && p.cache != null) {
    inherit (p) cache;
  };

  indexData = {
    version = 1;
    provenance = if commit != "" then { inherit commit; } else null;
    inherit indexes caches;
    packages = map mkPackageEntry packages;
  };
in
pkgs.writeTextDir "nix-package-index.v1.json" (builtins.toJSON indexData)
