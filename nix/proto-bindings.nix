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

# Fails when the checked-in protobuf bindings differ from a fresh `buf generate`.
# Runs offline on inputs CI already caches: the protoc plugins, the frontend's node_modules,
# and the pinned googleapis commit, fetched once as a fixed-output derivation.
{ lib
, stdenv
, runCommand
, buf
, delta
, git
, nodejs
, cacert
, protoPlugins
, frontendNodeModules
,
}:

let
  module = "buf.build/googleapis/googleapis";
  pins = lib.filter (match: match != null)
    (map (line: builtins.match " *- module: ${lib.escapeRegex module}:([0-9a-f]+)" line)
      (lib.splitString "\n" (builtins.readFile ../buf.gen.yaml)));
  pin =
    assert lib.assertMsg (lib.length pins == 1) "buf.gen.yaml must pin ${module} to one commit";
    lib.head (lib.head pins);

  # The pin is part of the name, so moving it forces a refetch rather than reusing the old image.
  # That refetch fails on this hash and prints the new one to paste here.
  #
  # Limited to google/rpc like the template's input: `file_per_package` output follows file order,
  # and a whole-module build orders google/rpc after its importers elsewhere in googleapis.
  googleapis = runCommand "googleapis-rpc-${pin}.binpb"
    {
      nativeBuildInputs = [ buf ];
      SSL_CERT_FILE = "${cacert}/etc/ssl/certs/ca-bundle.crt";
      outputHashMode = "flat";
      outputHash = "sha256-TpVZIX5VZvsAMMxej3zVJXolqe2CiggW/m6VdX0bVYk=";
    }
    ''
      export HOME=$TMPDIR
      buf build ${module}:${pin} --path google/rpc --output "$out#format=binpb"
    '';
in
stdenv.mkDerivation {
  name = "proto-bindings";
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../buf.gen.yaml
      ../buf.yaml
      ../bmc-grpc/proto
      ../bmc-grpc/src/gen
      ../frontend/src/proto/gen
    ];
  };

  # nodejs runs the `node_modules/.bin` shims, which pnpm writes to call `node` from PATH.
  nativeBuildInputs = [
    buf
    delta
    git
    nodejs
    protoPlugins.protoc-gen-buffa
    protoPlugins.protoc-gen-connect-rust
  ];

  buildPhase = ''
    export HOME=$TMPDIR
    trees="bmc-grpc/src/gen frontend/src/proto/gen"

    for tree in $trees; do mkdir -p $TMPDIR/checked-in/$tree && cp -r $tree/. $TMPDIR/checked-in/$tree; done
    substituteInPlace buf.gen.yaml --replace-fail "module: ${module}:${pin}" "binary_image: ${googleapis}"
    PATH=${frontendNodeModules}/node_modules/.bin:$PATH buf generate
    for tree in $trees; do mkdir -p $TMPDIR/generated/$tree && cp -r $tree/. $TMPDIR/generated/$tree; done

    # delta exits 1 on a difference, like diff, and shows it syntax-highlighted.
    # Without `--detect-dark-light=never` it writes terminal queries into the log,
    # which a terminal later printing that log answers with junk input.
    cd $TMPDIR
    if ! delta --no-gitconfig --detect-dark-light=never --paging=never --width=variable --line-numbers checked-in generated; then
      echo 'The checked-in bindings are stale; run `just gen`.'
      exit 1
    fi
  '';

  installPhase = "touch $out";
}
