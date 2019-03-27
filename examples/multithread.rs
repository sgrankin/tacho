//! A performance test for stats being accessed across threads.

#[macro_use]
extern crate log;

use futures::channel::oneshot;
use futures::future;
use futures::stream::StreamExt;
use futures::Future;
use futures::FutureExt;
use std::thread;
use std::time::Duration;
use tacho::Timing;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    drop(pretty_env_logger::init());

    let (metrics, report) = tacho::new();

    let (work_done_tx0, work_done_rx0) = oneshot::channel();
    let (work_done_tx1, work_done_rx1) = oneshot::channel();
    let reporter = {
        let work_done_rx = future::join(work_done_rx0, work_done_rx1).map(|_| {});
        let interval = Duration::from_secs(2);
        reporter(interval, work_done_rx, report)
    };

    let metrics = metrics.clone().labeled("test", "multithread");
    let loop_iter_us = metrics.stat("loop_iter_us");
    for (i, work_done_tx) in vec![(0, work_done_tx0), (1, work_done_tx1)] {
        let metrics = metrics.clone().labeled("thread".into(), format!("{}", i));
        let loop_counter = metrics.counter("loop_counter".into());
        let current_iter = metrics.gauge("current_iter".into());
        let loop_iter_us = loop_iter_us.clone();
        thread::spawn(move || {
            let mut prior = None;
            for i in 0..10_000_000 {
                let t0 = Timing::start();
                current_iter.set(i);
                loop_counter.incr(1);
                if let Some(p) = prior {
                    loop_iter_us.add(p);
                }
                prior = Some(t0.elapsed_us());
            }
            if let Some(p) = prior {
                loop_iter_us.add(p);
            }

            work_done_tx.send(()).expect("could not send");
        });
    }
    tokio::runtime::Runtime::new()?.block_on(reporter);
    Ok(())
}

/// Prints a report every `interval` and when the `done` is satisfied.
fn reporter<D>(
    interval: Duration,
    done: D,
    reporter: tacho::Reporter,
) -> impl Future<Output = ()> + Send
where
    D: Future<Output = ()> + Send + 'static,
{
    let periodic = {
        let mut reporter = reporter.clone();
        tokio::time::interval(interval).map(move |_| {
            print_report(&reporter.take());
            future::ready(())
        })
    };
    let done = done.map(move |_| {
        print_report(&reporter.peek());
    });
    periodic
        .take_until(done)
        .map(|_| ())
        .fold((), |_, _| future::ready(()))
}

fn print_report(report: &tacho::Report) {
    let out = tacho::prometheus::string(report).unwrap();
    info!("\n{}", out);
}
