// Copyright (C) 2025  Braiins Systems s.r.o.

use serde::Deserialize;
use slint::SharedString;

pub const POOL_API_URL: &str = "https://pool.braiins.com/api/v1";
pub const USER_HASHRATE_CURRENT: &str = "/user/hashrate/current";
pub const USER_REWARD_LATEST: &str = "/user/rewards/latest";

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

#[derive(Debug, Default, Deserialize)]
pub struct LatestUserRewards {
    todays_reward_estimate_btc: f32,
    todays_reward_estimate_usd: f32,
}

impl LatestUserRewards {
    #[must_use]
    pub fn today_reward_btc(&self) -> SharedString {
        SharedString::from(format!("{:.6} BTC", self.todays_reward_estimate_btc))
    }

    #[must_use]
    pub fn today_reward_usd(&self) -> SharedString {
        SharedString::from(format!("~ {:.1} USD", self.todays_reward_estimate_usd))
    }
}
