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

# Its minor must match the locked ICU4X crates, or a blob carries markers they cannot read.
{ lib, rustPlatform, fetchCrate }:

let
  lockedPackages = (fromTOML (builtins.readFile ../../Cargo.lock)).package;
  icu_providers = map (package: package.version)
    (builtins.filter (package: package.name == "icu_provider") lockedPackages);

  icu4x-datagen = rustPlatform.buildRustPackage rec {
    pname = "icu4x-datagen";
    version = "2.4.0";
    src = fetchCrate {
      inherit pname version;
      hash = "sha256-FI8UcgzL+EZa62X+JFhcxnHhaYN6wznCHijE1d1PzNA=";
    };
    cargoHash = "sha256-bhi7x2k1qELcc7NE+QfaJXJsfj0ZMF6PunwngN1HgOY=";
    # The defaults without `networking`: a source the pinned cldr-json lacks is an error, never a download.
    buildNoDefaultFeatures = true;
    buildFeatures = [ "use_wasm" "fs_exporter" "blob_exporter" "baked_exporter" "provider" "rayon" ];
    doCheck = false;
    meta = {
      description = "Generate data for ICU4X DataProvider";
      homepage = "https://icu4x.unicode.org";
      license = lib.licenses.unicode-30;
      mainProgram = "icu4x-datagen";
    };
  };
in
assert lib.assertMsg (icu_providers != [ ]) "Cargo.lock has no icu_provider";
assert lib.assertMsg
  (lib.all (version: lib.versions.majorMinor version == lib.versions.majorMinor icu4x-datagen.version) icu_providers)
  "icu4x-datagen ${icu4x-datagen.version} must share the minor version of every icu_provider in Cargo.lock: ${lib.concatStringsSep ", " icu_providers}";
icu4x-datagen
