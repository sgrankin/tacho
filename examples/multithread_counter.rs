//! A performance test for counters being accessed across threads.

#[macro_use]
extern crate log;
extern crate pretty_env_logger;
extern crate futures;
extern crate tacho;
extern crate tokio_core;
extern crate tokio_timer;

use futures::{BoxFuture, Future, Stream};
use futures::sync::oneshot;
use std::thread;
use std::time::Duration;
use tacho::Timing;
use tokio_core::reactor::Core;
use tokio_timer::Timer;

fn main() {
    drop(pretty_env_logger::init());

    let (metrics, report) = tacho::new();

    let (work_done_tx0, work_done_rx0) = oneshot::channel();
    let (work_done_tx1, work_done_rx1) = oneshot::channel();
    let reporter = {
        let work_done_rx = work_done_rx0
            .join(work_done_rx1)
            .map(|_| {})
            .map_err(|_| ());
        let interval = Duration::from_secs(2);
        reporter(interval, work_done_rx, report)
    };

    let metrics = metrics
        .clone()
        .labeled("test".into(), "multithread_counter".into());
    let loop_counter = metrics.counter("loop_counter".into());
    let loop_time_us = metrics.counter("loop_time_total_us".into());
    for work_done_tx in vec![work_done_tx0, work_done_tx1] {
        let loop_counter = loop_counter.clone();
        let loop_time_us = loop_time_us.clone();
        thread::spawn(move || {
            let mut prior: usize = 0;
            for _ in 0..2_000_000 {
                loop_counter.incr(1);

                let t0 = Timing::start();
                loop_time_us.incr(prior);
                prior = t0.elapsed_us() as usize;
            }
            loop_time_us.incr(prior);
            work_done_tx.send(()).expect("could not send");
        });
    }

    let mut core = Core::new().expect("Failed to create core");
    core.run(reporter).expect("failed to run reporter");
}

/// Prints a report every `interval` and when the `done` is satisfied.
fn reporter<D>(interval: Duration, done: D, reporter: tacho::Reporter) -> BoxFuture<(), ()>
    where D: Future<Item = (), Error = ()> + Send + 'static
{
    let periodic = {
        let mut reporter = reporter.clone();
        Timer::default()
            .interval(interval)
            .map_err(|_| {})
            .for_each(move |_| {
                          print_report(&reporter.take());
                          Ok(())
                      })
    };
    let done = done.map(move |_| { print_report(&reporter.peek()); });
    periodic.select(done).map(|_| {}).map_err(|_| {}).boxed()
}

fn print_report(report: &tacho::Report) {
    info!("\n{}", tacho::prometheus::format(report));
}