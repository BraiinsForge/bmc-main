// Copyright (C) 2026  Braiins Systems s.r.o.

//! The device profiles. Each is a slim module with a typed `Params` struct
//! and a `resource` builder; the exhaustive blueprint schema is derived from those params.
//! Adding a device is a new module here plus one `Instance` variant in [`crate::blueprint`].

pub mod axeos;
pub mod bos;
pub mod ubos;
