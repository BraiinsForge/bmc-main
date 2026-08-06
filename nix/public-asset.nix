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

# public-asset: Resolve a manifest-relative icon
# to a flat content-addressed store file (asset.<ext>) fit for publishing.
# The output is served publicly, so paths and extensions are validated strictly.
{ lib }:
let
  canonicalExtension = relativePath:
    let
      components = lib.splitString "/" relativePath;
      filename = lib.last components;
      matched = builtins.match ".*\\.([A-Za-z]+)" filename;
      raw = if matched == null then null else lib.toLower (builtins.head matched);
    in
    if relativePath == ""
      || lib.hasPrefix "/" relativePath
      || lib.hasInfix "\\" relativePath
      || builtins.any (component: component == "" || component == "." || component == "..") components
    then throw "public icon path must be a safe relative path: ${relativePath}"
    # Fold jpeg → jpg so identical bytes can't end up under two store names.
    else if raw == "jpeg" then "jpg"
    else if builtins.elem raw [ "svg" "png" "webp" "jpg" ] then raw
    else throw "public icon extension must be svg, png, webp, jpg, or jpeg: ${relativePath}";
in
{
  mkPublicIcon = manifest: relativePath:
    assert builtins.typeOf manifest == "path";
    let
      extension = canonicalExtension relativePath;
      source = builtins.dirOf manifest + "/${relativePath}";
    in
    builtins.seq extension (
      if !(builtins.pathExists source)
      then throw "public icon source does not exist: ${toString source}"
      # readFileType does not follow symlinks — rejecting them here
      # stops a widget from publishing a file outside its own directory.
      else if builtins.readFileType source != "regular"
      then throw "public icon source must be a regular file: ${toString source}"
      else
        builtins.path {
          path = source;
          name = "asset.${extension}";
          recursive = false;
        }
    );
}
