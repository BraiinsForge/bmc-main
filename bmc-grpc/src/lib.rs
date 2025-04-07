// Copyright (C) 2025  Braiins Systems s.r.o.

pub mod web {
    #![allow(warnings)]
    include!(concat!(env!("OUT_DIR"), "/braiins.bmc.web.rs"));

    pub const FILE_DESCRIPTOR_SET: &[u8] =
        include_bytes!(concat!(env!("OUT_DIR"), "/file-descriptor-set.bin"));

    pub const PROTO_HASH: &str = env!("PROTO_HASH");
}
