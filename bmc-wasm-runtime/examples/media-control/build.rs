// Copyright (C) 2026  Braiins Systems s.r.o.

fn main() {
    prost_build::compile_protos(&["proto/cast_channel.proto"], &["proto/"])
        .expect("BUG: proto compilation failed");
}
