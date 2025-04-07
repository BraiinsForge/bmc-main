// Copyright (C) 2025  Braiins Systems s.r.o.

pub trait BmcManager: Sync + Send + 'static {
    fn version(&self) -> String;
}
