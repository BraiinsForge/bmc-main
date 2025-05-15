// Copyright (C) 2023  Braiins Systems s.r.o.

use pin_project::pin_project;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

#[derive(Debug)]
#[pin_project]
pub struct Cancelable<F, C> {
    #[pin]
    future: F,
    #[pin]
    cancel_fut: C,
}

impl<F: Future, C: Future> Future for Cancelable<F, C> {
    type Output = Result<F::Output, C::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        // first, try polling the cancel future
        if let Poll::Ready(res) = this.cancel_fut.poll(cx) {
            return Poll::Ready(Err(res));
        }

        // not cancelled, poll the original future
        this.future.poll(cx).map(Ok)
    }
}

pub trait Cancel: Future + Sized {
    /// Make this future cancelable: The future is cancelled
    /// when the `cancel_fut` resolves before the wrapped
    /// future itself resolves.
    ///
    /// If the original future resolves, its value is yielded as `Ok(value)`.
    /// If cancelled, `Err(e)` is yielded,
    /// where `e` is the value yielded by `cancel_fut`.
    ///
    /// This is basically the same operation as `select()`
    /// but yielding a `Result` and with a clearer intent of cancellation.
    fn cancel<C: Future>(self, cancel_fut: C) -> Cancelable<Self, C> {
        Cancelable {
            future: self,
            cancel_fut,
        }
    }
}

impl<F: Future> Cancel for F {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future;

    #[tokio::test]
    async fn cancelling() {
        let cancel = future::ready(1);
        let fut = future::pending::<()>().cancel(cancel);
        assert_eq!(fut.await, Err(1));
    }

    #[tokio::test]
    async fn not_cancelling() {
        let cancel = future::pending::<()>();
        let fut = future::ready(2).cancel(cancel);
        assert_eq!(fut.await, Ok(2));
    }
}
