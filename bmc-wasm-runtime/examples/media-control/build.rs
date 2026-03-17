// Copyright (C) 2026  Braiins Systems s.r.o.

fn main() {
    let mut config = prost_build::Config::new();
    config.type_attribute(".", "#[allow(dead_code)]");
    config
        .compile_protos(&["proto/cast_channel.proto"], &["proto/"])
        .expect("BUG: proto compilation failed");
}
