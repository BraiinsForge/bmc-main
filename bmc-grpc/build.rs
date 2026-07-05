// Copyright (C) 2025  Braiins Systems s.r.o.

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
