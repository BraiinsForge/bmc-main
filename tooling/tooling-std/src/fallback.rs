// Copyright (C) 2023  Braiins Systems s.r.o.

use std::future::Future;

pub async fn with_fallback<O>(
    future: impl Future<Output = O>,
    fallback_future: impl Future<Output = O>,
    predicate: impl Fn(&O) -> bool,
) -> O {
    let mut result = future.await;
    if predicate(&result) {
        result = fallback_future.await;
    }
    result
}
