// Copyright (C) 2024  Braiins Systems s.r.o.

#![allow(unused_crate_dependencies)]

use std::time::Duration;
use tokio::{task, time};
use tooling_std::cond_runner::ConditionalRunner;

#[tokio::main]
async fn main() {
    let runner = ConditionalRunner::new();

    task::spawn({
        let fut = runner.run_when_active(async || {
            loop {
                println!("runner1 running");
                time::sleep(Duration::from_millis(100)).await;
            }
        });

        async {
            fut.await;
            println!("runner1 done");
        }
    });
    task::spawn({
        let fut = runner.run_when_active(async || {
            time::sleep(Duration::from_millis(50)).await;
            loop {
                println!("runner2 running");
                time::sleep(Duration::from_millis(100)).await;
            }
        });

        async {
            fut.await;
            println!("runner2 done");
        }
    });

    time::sleep(Duration::from_secs(1)).await;

    let guard1 = runner.activate();
    time::sleep(Duration::from_secs(1)).await;
    drop(guard1);

    println!();
    time::sleep(Duration::from_secs(1)).await;

    let guard1 = runner.activate();
    let guard2 = runner.activate();
    time::sleep(Duration::from_secs(1)).await;
    drop(guard1);
    drop(guard2);

    println!();
    time::sleep(Duration::from_secs(1)).await;

    drop(runner);

    time::sleep(Duration::from_secs(1)).await;
}
