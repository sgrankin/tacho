use futures::future;
use futures::stream::StreamExt;
use futures::FutureExt;
use std::time::Duration;
use tacho::Timing;
use tokio;

fn do_work(metrics: tacho::Scope) -> impl future::Future<Output = ()> {
    let metrics = metrics.labeled("labelkey", "labelval");
    let iter_time_us = metrics.stat("iter_time_us".into());
    let work = futures::stream::iter(0..100)
        .then(move |n| {
            // Clones are shallow, minimizing allocation.
            let iter_time_us = iter_time_us.clone();

            let start = Timing::start();
            tokio::time::delay_for(Duration::from_millis(20 * (n % 5))).map(move |_| {
                iter_time_us.add(start.elapsed_us());
                n
            })
        })
        .take_while(|n| future::ready(*n != 0))
        .into_future()
        .map(|_| ());
    work
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    drop(pretty_env_logger::init());

    let (metrics, reporter) = tacho::new();
    let reported = do_work(metrics.clone()).then(move |_| {
        tokio::time::delay_for(Duration::from_millis(1000)).map(move |_| {
            let r = reporter.peek();
            println!("# metrics:");
            println!("");
            println!("{}", tacho::prometheus::string(&r).unwrap());
        })
    });

    tokio::runtime::Runtime::new()?.block_on(reported);
    Ok(())
}
