// Copyright (C) 2024  Braiins Systems s.r.o.

use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use futures::{StreamExt, TryStreamExt, future, stream};
use governor::state::StreamRateLimitExt;
use governor::{Quota, RateLimiter};
use nonzero_ext::nonzero;
use std::future::Future;
use std::num::NonZero;
use tokio::task;
use tokio::task::JoinHandle;
use tracing::Instrument;

/// Spawn tokio task for each item in `items` at a given rate of spawns per second, and wait for all tasks to finish.
/// Futures executed by the tasks are generated using `task_fut_fn`.
pub async fn spawn_all_ratelimit<I, O, Fut>(
    per_second: NonZero<u32>,
    items: impl IntoIterator<Item = I>,
    task_fut_fn: impl Fn(I) -> Fut,
) -> Vec<O>
where
    I: Unpin,
    O: Send + 'static,
    Fut: Future<Output = O> + Send + 'static,
{
    let quota = Quota::per_second(per_second).allow_burst(nonzero!(1_u32));
    let limiter = RateLimiter::direct(quota);

    stream::iter(items.into_iter())
        .ratelimit_stream(&limiter)
        .map(|item| {
            let fut = task_fut_fn(item);
            task::spawn(fut.in_current_span())
        })
        // wait until all tasks are started
        .collect::<FuturesUnordered<JoinHandle<O>>>()
        .await
        // wait for the remaining tasks to finish
        .try_collect::<Vec<O>>()
        .await
        // propagate task panic
        .expect("inner task panicked")
}

// TODO: more efficient implementation with less allocations
/// This is an opposite of `future::try_join_all()`. It returns first successful future, or waits
/// for all futures to complete and returns a list of all errors.
pub async fn select_ok_or_join_err<T, E>(
    futures: impl IntoIterator<Item = BoxFuture<'_, Result<T, E>>>,
) -> Result<T, Vec<E>> {
    async fn inner<T, E>(
        futures: Vec<BoxFuture<'_, Result<T, E>>>,
        mut errors: Vec<E>,
    ) -> Result<T, Vec<E>> {
        // no remaining futures and Ok still not found - return the list of errors
        if futures.is_empty() {
            return Err(errors);
        }

        // run all futures concurrently until one of them finishes
        let (result, _, remaining) = future::select_all(futures).await;

        match result {
            Ok(val) => return Ok(val),
            Err(err) => errors.push(err),
        }

        // continue waiting for the remaining tests
        Box::pin(inner(remaining, errors)).await
    }

    inner(futures.into_iter().collect(), Vec::new()).await
}

#[cfg(test)]
mod tests {
    use futures::FutureExt;
    use futures::future::BoxFuture;
    use std::time::Duration;
    use tokio::time;

    fn wait_and_return(
        duration: Duration,
        res: impl Fn(()) -> Result<(), ()> + Sync + Send + 'static,
    ) -> BoxFuture<'static, Result<(), ()>> {
        async move {
            time::sleep(duration).await;
            res(())
        }
        .boxed()
    }

    #[tokio::test]
    async fn select_ok_or_join_err_err() {
        let futures = [
            wait_and_return(Duration::from_millis(10), Err),
            wait_and_return(Duration::from_millis(20), Err),
            wait_and_return(Duration::from_millis(30), Err),
        ];
        let result = super::select_ok_or_join_err(futures).await;
        assert_eq!(result, Err(vec![(), (), ()]));
    }

    #[tokio::test]
    async fn select_ok_or_join_err_ok() {
        let futures = [
            wait_and_return(Duration::from_millis(10), Err),
            wait_and_return(Duration::from_millis(20), Err),
            wait_and_return(Duration::from_millis(30), Ok),
        ];
        let result = super::select_ok_or_join_err(futures).await;
        assert_eq!(result, Ok(()));
    }
}
