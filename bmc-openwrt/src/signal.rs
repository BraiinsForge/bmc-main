// Copyright (C) 2025  Braiins Systems s.r.o.

use futures::{FutureExt, future::select_all};
use tokio::signal::unix::{Signal, SignalKind, signal};

pub const SHUTDOWN_SIGNALS: &[SignalKind] = &[
    SignalKind::interrupt(),
    SignalKind::terminate(),
    SignalKind::quit(),
];

pub async fn wait_for_first_signal(signals: &[SignalKind]) -> SignalKind {
    let mut signal_streams: Vec<Signal> = signals
        .iter()
        .map(|&sig| {
            signal(sig).unwrap_or_else(|_| panic!("BUG: cannot create signal receiver for {sig:?}"))
        })
        .collect();

    let receivers: Vec<_> = signal_streams
        .iter_mut()
        .map(|sig| sig.recv().boxed())
        .collect();

    let (_, index, _) = select_all(receivers).await;

    signals[index]
}
