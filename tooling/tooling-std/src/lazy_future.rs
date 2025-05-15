// Copyright (C) 2023  Braiins Systems s.r.o.

use async_once_cell::Lazy;
use futures::future::BoxFuture;
use pin_project::pin_project;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use sync_wrapper::SyncWrapper;

pub type LazyFuture<'a, T> = Arc<Lazy<T, SyncFuture<BoxFuture<'a, T>>>>;

pub trait IntoLazyFuture: Future + Send + Sized {
    fn into_lazy_future<'a>(self) -> LazyFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Arc::new(Lazy::new(SyncFuture::new(Box::pin(self))))
    }
}

impl<F: Future + Send> IntoLazyFuture for F {}

#[derive(Debug)]
#[pin_project]
pub struct SyncFuture<F: Future> {
    #[pin]
    fut: SyncWrapper<F>,
}

impl<F: Future> SyncFuture<F> {
    pub fn new(fut: F) -> Self {
        Self {
            fut: SyncWrapper::new(fut),
        }
    }
}

impl<F: Future> Future for SyncFuture<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().fut.get_pin_mut().poll(cx)
    }
}
