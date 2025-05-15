// Copyright (C) 2023  Braiins Systems s.r.o.

use async_trait::async_trait;
use std::future::Future;
use std::time::Duration;

#[async_trait]
pub trait Timeout: Future + Sized {
    async fn timeout<E: Send>(self, duration: Duration, error: E) -> Result<Self::Output, E> {
        tokio::time::timeout(duration, self)
            .await
            .map_err(|_| error)
    }
}

#[async_trait]
impl<F: Future> Timeout for F {}
