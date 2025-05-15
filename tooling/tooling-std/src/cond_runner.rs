// Copyright (C) 2024  Braiins Systems s.r.o.

use crate::cancel::Cancel;
use futures::FutureExt;
use parking_lot::Mutex;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::watch;

#[derive(Debug, Eq, PartialEq)]
enum Status {
    Running,
    Stopped,
}

/// Runs a future when at least one guard exists.
#[derive(Debug)]
pub struct ConditionalRunner {
    tx: watch::Sender<Status>,
    shared: Arc<Mutex<Shared>>,
}

impl ConditionalRunner {
    #[must_use]
    pub fn new() -> Self {
        let (tx, _) = watch::channel(Status::Stopped);

        Self {
            tx: tx.clone(),
            shared: Arc::new(Mutex::new(Shared { guards: 0, tx })),
        }
    }

    /// When this [`ConditionalRunner`] becomes *active*, the future is started. When it becomes *inactive*, the future
    /// is canceled (if it's still running). When the future itself resolves, it is restarted. The future returned by
    /// this function resolves when the [`ConditionalRunner`] is dropped.
    pub fn run_when_active(
        &self,
        f: impl AsyncFn() + 'static,
    ) -> impl Future<Output = ()> + 'static {
        let mut rx = self.tx.subscribe();

        async move {
            loop {
                if rx.wait_for(|val| *val == Status::Running).await.is_err() {
                    // `ConditionalRunner` is dropped => exit
                    break;
                }

                let _: Result<(), ()> = f()
                    .cancel(rx.wait_for(|val| *val == Status::Stopped).map(|_| ()))
                    .await;
            }
        }
    }

    /// Activate the runner by acquiring a guard. When at least one guard exists, this [`ConditionalRunner`] is
    /// considered *active*.
    #[must_use]
    pub fn activate(&self) -> ActivationGuard {
        self.shared.lock().increment();
        ActivationGuard {
            shared: self.shared.clone(),
        }
    }
}

#[derive(Debug)]
pub struct ActivationGuard {
    shared: Arc<Mutex<Shared>>,
}

impl Drop for ActivationGuard {
    fn drop(&mut self) {
        self.shared.lock().decrement();
    }
}

#[derive(Debug)]
struct Shared {
    tx: watch::Sender<Status>,
    guards: usize,
}

impl Shared {
    fn increment(&mut self) {
        self.guards += 1;
        self.tx.send_replace(Status::Running);
    }

    fn decrement(&mut self) {
        self.guards = self.guards.saturating_sub(1);
        if self.guards == 0 {
            self.tx.send_replace(Status::Stopped);
        }
    }
}
