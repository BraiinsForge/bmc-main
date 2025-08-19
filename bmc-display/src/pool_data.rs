// Copyright (C) 2025  Braiins Systems s.r.o.

use serde::Deserialize;
use slint::SharedString;

pub const POOL_API_URL: &str = "https://pool.braiins.com/api/v1";
pub const USER_HASHRATE_CURRENT: &str = "/user/hashrate/current";

#[derive(Debug, Default, Deserialize)]
pub struct CurrentUserHashrate {
    hashrate_th_per_sec: f32,
}

impl CurrentUserHashrate {
    #[must_use]
    pub fn hashrate_as_shared(self) -> SharedString {
        SharedString::from(format!("{:.1}", self.hashrate_th_per_sec))
    }
}
