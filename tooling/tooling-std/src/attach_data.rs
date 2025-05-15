// Copyright (C) 2023  Braiins Systems s.r.o.

use futures::Stream;
use pin_project::pin_project;
use std::pin::Pin;
use std::task::{Context, Poll};

#[derive(Debug)]
#[pin_project]
pub struct StreamWithData<S: Stream, T> {
    #[pin]
    inner: S,
    data: T,
}

impl<S: Stream, T> Stream for StreamWithData<S, T> {
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        this.inner.poll_next(cx)
    }
}

pub trait AttachData: Stream + Sized {
    /// Attach arbitrary data to a stream. This is useful for keeping various RAII guards alive, for example.
    fn attach_data<T>(self, data: T) -> StreamWithData<Self, T> {
        StreamWithData { inner: self, data }
    }
}

impl<S: Stream> AttachData for S {}
