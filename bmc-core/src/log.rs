// Copyright (C) 2025  Braiins Systems s.r.o.

use tracing_subscriber::{
    EnvFilter,
    filter::{self, FilterExt},
    layer::{Layer, SubscriberExt},
    util::SubscriberInitExt,
};

pub fn init() {
    let filter = filter::Targets::new()
        .with_default(filter::LevelFilter::TRACE)
        .and(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::default().add_directive(filter::Directive::from(filter::LevelFilter::INFO))
        }));

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(filter))
        .init();
}
