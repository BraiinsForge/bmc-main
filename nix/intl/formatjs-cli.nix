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

# The FormatJS CLI (`formatjs extract|compile|verify`), the Rust build.
# `formatjs_cli` is `publish = false` past 1.4.2, so it builds from the monorepo's release tag,
# one package of its workspace.
{ lib, ii, makeRustPlatform, fetchFromGitHub }:

let
  # Its `rust-version` is past the base nixpkgs' rustc; the workspace's own toolchain meets it.
  rustPlatform = makeRustPlatform {
    cargo = ii.rust.toolchain;
    rustc = ii.rust.toolchain;
  };
  version = "1.8.2";
in
rustPlatform.buildRustPackage {
  pname = "formatjs-cli";
  inherit version;
  src = fetchFromGitHub {
    owner = "formatjs";
    repo = "formatjs";
    tag = "formatjs_cli_v${version}";
    hash = "sha256-+uRsp11UJrIpiJOXMGVqk8yLhlJRiPRmwlq7RXIiqSo=";
  };
  cargoHash = "sha256-M7jwiqLbuS6EFjVRPj5A0DHiq52NSMy1Y160Q8QuNac=";
  cargoBuildFlags = [ "--package" "formatjs_cli" ];
  doCheck = false;
  meta = {
    description = "FormatJS CLI: extract, compile and verify ICU MessageFormat catalogs";
    homepage = "https://formatjs.github.io/docs/rust";
    license = lib.licenses.mit;
    mainProgram = "formatjs";
  };
}
