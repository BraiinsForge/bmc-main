// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use fs_err as fs;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::{env, io};

fn main() {
    let proto_files = glob::glob("proto/web/**/*.proto")
        .expect("BUG: glob pattern is invalid")
        .collect::<Result<Vec<_>, _>>()
        .expect("BUG: glob failed");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("BUG: OUT_DIR is not defined"));

    let build_server = std::env::var("CARGO_FEATURE_SERVER").is_ok();
    let build_client = std::env::var("CARGO_FEATURE_CLIENT").is_ok();

    tonic_build::configure()
        // this is needed by older protoc compilers like the one in ubuntu 22.04 lts
        .protoc_arg("--experimental_allow_proto3_optional")
        .file_descriptor_set_path(out_dir.join("file-descriptor-set.bin"))
        .out_dir(&out_dir)
        .build_client(build_client)
        .build_server(build_server)
        .emit_rerun_if_changed(true)
        .compile_protos(&proto_files, &["proto/"])
        .expect("BUG: unable to compile proto files");

    let hash = {
        let mut hasher = Sha256::new();
        for proto_file in proto_files {
            let mut file = fs::File::open(proto_file).expect("BUG: failed to open proto file");
            let _ = io::copy(&mut file, &mut hasher)
                .expect("BUG: failed to copy proto file content into hasher");
        }

        hex::encode(hasher.finalize())
    };

    println!("cargo:rustc-env=PROTO_HASH={hash}");
}
