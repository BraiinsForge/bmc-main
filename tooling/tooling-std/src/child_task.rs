// Copyright (C) 2024  Braiins Systems s.r.o.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, ready};
use tokio::task;
use tokio::task::JoinHandle;
use tracing::Instrument;

/// Convenience function for spawning a new child task.
pub fn spawn<F>(future: F) -> ChildTask<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    task::spawn(future).into_child_task()
}

/// Convenience function for spawning a new child task (in the current tracing span).
pub fn spawn_in_current_span<F>(future: F) -> ChildTask<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    task::spawn(future.in_current_span()).into_child_task()
}

#[derive(Debug)]
pub struct ChildTask<T = ()>(Option<JoinHandle<T>>);

const UNINIT_MSG: &str = "BUG: ChildTask is uninitialized";

// public API
impl<T> ChildTask<T> {
    // `impl<T> JoinHandle<T>` contains only methods `is_finished()` and `abort()`, we want to prevent calling `.abort()` on ChildTask, that's why there is this instead of `impl<T> Deref for ChildTask<T>`.
    /// Checks if the task associated with this `ChildTask` has finished.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.as_ref().is_finished()
    }

    /// Convert this [`ChildTask`] to [`JoinHandle`].
    #[must_use]
    pub fn detach(mut self) -> JoinHandle<T> {
        self.0.take().expect(UNINIT_MSG)
    }

    /// Abort this task. Equivalent to `drop()`, but might be more readable in some contexts.
    pub fn abort(self) {
        drop(self);
    }
}

// private API
impl<T> ChildTask<T> {
    fn as_ref(&self) -> &JoinHandle<T> {
        self.0.as_ref().expect(UNINIT_MSG)
    }

    fn as_mut(&mut self) -> &mut JoinHandle<T> {
        self.0.as_mut().expect(UNINIT_MSG)
    }
}

impl<T> Future for ChildTask<T> {
    type Output = T;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match ready!(Pin::new(self.get_mut().as_mut()).poll(cx)) {
            Ok(res) => Poll::Ready(res),
            Err(err) if err.is_panic() => std::panic::resume_unwind(err.into_panic()),
            Err(_) => Poll::Pending,
        }
    }
}

impl<T> Drop for ChildTask<T> {
    fn drop(&mut self) {
        self.as_ref().abort();
    }
}

pub trait IntoChildTask<T> {
    /// # What does it do
    /// - propagates panics from the inner tokio task
    /// - aborts the inner tokio task when dropped
    fn into_child_task(self) -> ChildTask<T>;
}

impl<T> IntoChildTask<T> for JoinHandle<T> {
    fn into_child_task(self) -> ChildTask<T> {
        ChildTask(Some(self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeout::Timeout;
    use std::future;
    use std::time::Duration;
    use tokio::sync::oneshot;
    use tokio::task;

    #[tokio::test]
    #[should_panic(expected = "explicit panic")]
    async fn panic_propagation() {
        task::spawn(async { panic!() }).into_child_task().await;
    }

    #[tokio::test]
    async fn abort_on_drop() {
        let (tx, rx) = oneshot::channel::<()>();

        let child_task = task::spawn(async move {
            let _hold = tx;
            future::pending::<()>().await;
        })
        .into_child_task();

        drop(child_task);

        // dropping `Sender` without sending should return an error:
        match rx.timeout(Duration::from_millis(100), ()).await {
            Err(()) => panic!("rx timed out - task wasn't aborted"),
            Ok(Ok(())) => panic!("value was sent"),
            Ok(Err(_)) => { /* expected */ }
        }
    }
}
