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

# The protoc plugins `buf generate` runs for the Rust bindings (buf.gen.yaml):
# `protoc-gen-buffa` for the message types, `protoc-gen-connect-rust` for the services.
# Each is held to the runtime crate in Cargo.lock that its output is written against:
# exactly for buffa, which releases its crates together,
# and to the minor version for connectrpc, which patches its crates one by one.
{ lib, rustPlatform, fetchCrate }:

let
  lockedPackages = (builtins.fromTOML (builtins.readFile ../Cargo.lock)).package;
  lockedVersion = name:
    (lib.findSingle (package: package.name == name)
      (throw "Cargo.lock has no ${name}")
      (throw "Cargo.lock has more than one ${name}")
      lockedPackages).version;

  mkPlugin =
    { crate
    , version
    , hash
    , cargoHash
    , mainProgram
    , description
    , homepage
    ,
    }:
    rustPlatform.buildRustPackage {
      pname = mainProgram;
      inherit version cargoHash;
      src = fetchCrate {
        pname = crate;
        inherit version hash;
      };
      meta = {
        inherit description homepage mainProgram;
        license = lib.licenses.asl20;
      };
    };

  protoc-gen-buffa = mkPlugin {
    crate = "protoc-gen-buffa";
    version = "0.9.2";
    hash = "sha256-BEK+Nfn3/uBO78vXCm0zEQYgAkGFRkiPwRvTOZwFc00=";
    cargoHash = "sha256-tYBuO9RjeeqsiXvIrLVmd2HHC4B9o8dGqpl3GyWDjFQ=";
    mainProgram = "protoc-gen-buffa";
    description = "protoc plugin generating buffa protobuf message types";
    homepage = "https://github.com/anthropics/buffa";
  };

  protoc-gen-connect-rust = mkPlugin {
    crate = "connectrpc-codegen";
    version = "0.9.0";
    hash = "sha256-f9MW/JbPW5bbHcJ3l5J3YFiFQdnkg3N8NTpiWH8d7LU=";
    cargoHash = "sha256-eyWjHSUF0ZPtU/nrRXGQSmkhLOA+yHDDKfmfeu51ZMQ=";
    mainProgram = "protoc-gen-connect-rust";
    description = "protoc plugin generating connectrpc service traits and clients";
    homepage = "https://github.com/connectrpc/connect-rust";
  };

  buffa = lockedVersion "buffa";
  buffa-types = lockedVersion "buffa-types";
  connectrpc = lockedVersion "connectrpc";
in
assert lib.assertMsg (protoc-gen-buffa.version == buffa && protoc-gen-buffa.version == buffa-types)
  "protoc-gen-buffa ${protoc-gen-buffa.version} must equal buffa ${buffa} and buffa-types ${buffa-types} in Cargo.lock";
assert lib.assertMsg
  (lib.versions.majorMinor protoc-gen-connect-rust.version == lib.versions.majorMinor connectrpc)
  "connectrpc-codegen ${protoc-gen-connect-rust.version} must share the minor version of connectrpc ${connectrpc} in Cargo.lock";
{
  inherit protoc-gen-buffa protoc-gen-connect-rust;
}
