//! A thread-safe, `Future`-aware metrics library.
//!
//! Many programs need to information about runtime performance: the number of requests
//! served, a distribution of request latency, the number of failures, the number of loop
//! iterations, etc. `tacho::new` creates a shareable, scopable metrics registry and a
//! `Reporter`. The `Scope` supports the creation of `Counter`, `Gauge`, and `Stat`
//! handles that may be used to report values. Each of these receivers maintains a weak
//! reference back to the central stats registry.
//!
//! ## Performance
//!
//! Labels are stored in a `BTreeMap` because they are used as hash keys and, therefore,
//! need to implement `Hash`.

//#![cfg_attr(test, feature(test))]

extern crate futures;
extern crate hdrhistogram;
#[macro_use]
extern crate log;
extern crate indexmap;
// #[cfg(test)]
// extern crate test;

use futures::future::FutureExt;
use futures::task::Poll;
use hdrhistogram::Histogram;
use indexmap::IndexMap;
use pin_project::pin_project;
use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

pub mod prometheus;
mod report;
mod timing;

pub use report::{Report, Reporter};
pub use timing::Timing;

type Labels = BTreeMap<&'static str, String>;
type CounterMap = IndexMap<Key, Arc<AtomicUsize>>;
type GaugeMap = IndexMap<Key, Arc<AtomicUsize>>;
type StatMap = IndexMap<Key, Arc<Mutex<HistogramWithSum>>>;

#[derive(Debug, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub enum Prefix {
    Root,
    Node {
        prefix: Arc<Prefix>,
        value: &'static str,
    },
}

/// Creates a metrics registry.
///
/// The returned `Scope` may be you used to instantiate metrics. Labels may be attached to
/// the scope so that all metrics created by this `Scope` are annotated.
///
/// The returned `Reporter` supports consumption of metrics values.
pub fn new() -> (Scope, Reporter) {
    let registry = Arc::new(Mutex::new(Registry::default()));

    let scope = Scope {
        labels: Labels::default(),
        prefix: Arc::new(Prefix::Root),
        registry: registry.clone(),
    };

    (scope, report::new(registry))
}

/// Describes a metric.
#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    name: &'static str,
    prefix: Arc<Prefix>,
    labels: Labels,
}
impl Key {
    fn new(name: &'static str, prefix: Arc<Prefix>, labels: Labels) -> Key {
        Key {
            name,
            prefix,
            labels,
        }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }
    pub fn prefix(&self) -> &Arc<Prefix> {
        &self.prefix
    }
    pub fn labels(&self) -> &Labels {
        &self.labels
    }
}

#[derive(Default)]
pub struct Registry {
    counters: CounterMap,
    gauges: GaugeMap,
    stats: StatMap,
}

/// Supports creation of scoped metrics.
///
/// `Scope`s may be cloned without copying the underlying metrics registry.
///
/// Labels may be attached to the scope so that all metrics created by the `Scope` are
/// labeled.
#[derive(Clone)]
pub struct Scope {
    labels: Labels,
    prefix: Arc<Prefix>,
    registry: Arc<Mutex<Registry>>,
}

impl Scope {
    /// Accesses scoping labels.
    pub fn labels(&self) -> &Labels {
        &self.labels
    }

    /// Adds a label into scope (potentially overwriting).
    pub fn labeled<D: fmt::Display>(mut self, k: &'static str, v: D) -> Self {
        self.labels.insert(k, format!("{}", v));
        self
    }

    /// Appends a prefix to the current scope.
    pub fn prefixed(mut self, value: &'static str) -> Self {
        let p = Prefix::Node {
            prefix: self.prefix,
            value,
        };
        self.prefix = Arc::new(p);
        self
    }

    /// Creates a Counter with the given name.
    pub fn counter(&self, name: &'static str) -> Counter {
        let key = Key::new(name, self.prefix.clone(), self.labels.clone());
        let mut reg = self
            .registry
            .lock()
            .expect("failed to obtain lock on registry");

        if let Some(c) = reg.counters.get(&key) {
            return Counter(Arc::downgrade(c));
        }

        let c = Arc::new(AtomicUsize::new(0));
        let counter = Counter(Arc::downgrade(&c));
        reg.counters.insert(key, c);
        counter
    }

    /// Creates a Gauge with the given name.
    pub fn gauge(&self, name: &'static str) -> Gauge {
        let key = Key::new(name, self.prefix.clone(), self.labels.clone());
        let mut reg = self
            .registry
            .lock()
            .expect("failed to obtain lock on registry");

        if let Some(g) = reg.gauges.get(&key) {
            return Gauge(Arc::downgrade(g));
        }

        let g = Arc::new(AtomicUsize::new(0));
        let gauge = Gauge(Arc::downgrade(&g));
        reg.gauges.insert(key, g);
        gauge
    }

    /// Creates a Stat with the given name.
    ///
    /// The underlying histogram is automatically resized as values are added.
    pub fn stat(&self, name: &'static str) -> Stat {
        let key = Key::new(name, self.prefix.clone(), self.labels.clone());
        self.mk_stat(key, None)
    }

    pub fn timer_us(&self, name: &'static str) -> Timer {
        Timer {
            stat: self.stat(name),
            unit: TimeUnit::Micros,
        }
    }

    pub fn timer_ms(&self, name: &'static str) -> Timer {
        Timer {
            stat: self.stat(name),
            unit: TimeUnit::Millis,
        }
    }

    /// Creates a Stat with the given name and histogram paramters.
    pub fn stat_with_bounds(&self, name: &'static str, low: u64, high: u64) -> Stat {
        let key = Key::new(name, self.prefix.clone(), self.labels.clone());
        self.mk_stat(key, Some((low, high)))
    }

    fn mk_stat(&self, key: Key, bounds: Option<(u64, u64)>) -> Stat {
        let mut reg = self
            .registry
            .lock()
            .expect("failed to obtain lock on registry");

        if let Some(h) = reg.stats.get(&key) {
            let histo = Arc::downgrade(h);
            return Stat { histo, bounds };
        }

        let h = Arc::new(Mutex::new(HistogramWithSum::new(bounds)));
        let histo = Arc::downgrade(&h);
        reg.stats.insert(key, h);
        Stat { histo, bounds }
    }
}

/// Counts monotically.
#[derive(Clone)]
pub struct Counter(Weak<AtomicUsize>);
impl Counter {
    pub fn incr(&self, v: usize) {
        if let Some(c) = self.0.upgrade() {
            c.fetch_add(v, Ordering::AcqRel);
        }
    }
}

/// Captures an instantaneous value.
#[derive(Clone)]
pub struct Gauge(Weak<AtomicUsize>);
impl Gauge {
    pub fn incr(&self, v: usize) {
        if let Some(g) = self.0.upgrade() {
            g.fetch_add(v, Ordering::AcqRel);
        } else {
            debug!("gauge dropped");
        }
    }
    pub fn decr(&self, v: usize) {
        if let Some(g) = self.0.upgrade() {
            g.fetch_sub(v, Ordering::AcqRel);
        } else {
            debug!("gauge dropped");
        }
    }
    pub fn set(&self, v: usize) {
        if let Some(g) = self.0.upgrade() {
            g.store(v, Ordering::Release);
        } else {
            debug!("gauge dropped");
        }
    }
}

/// Histograms hold up to 4 significant figures.
const HISTOGRAM_PRECISION: u8 = 4;

/// Tracks a distribution of values with their sum.
///
/// `hdrhistogram::Histogram` does not track a sum by default; but prometheus expects a `sum`
/// for histograms.
#[derive(Clone)]
pub struct HistogramWithSum {
    histogram: Histogram<u64>,
    sum: u64,
}

impl HistogramWithSum {
    /// Constructs a new `HistogramWithSum`, possibly with bounds.
    fn new(bounds: Option<(u64, u64)>) -> Self {
        let h = match bounds {
            None => Histogram::<u64>::new(HISTOGRAM_PRECISION),
            Some((l, h)) => Histogram::<u64>::new_with_bounds(l, h, HISTOGRAM_PRECISION),
        };
        let histogram = h.expect("failed to create histogram");
        HistogramWithSum { histogram, sum: 0 }
    }

    /// Record a value to
    fn record(&mut self, v: u64) {
        if let Err(e) = self.histogram.record(v) {
            error!("failed to add value to histogram: {:?}", e);
        }
        if v >= ::std::u64::MAX - self.sum {
            self.sum = ::std::u64::MAX
        } else {
            self.sum += v;
        }
    }

    pub fn histogram(&self) -> &Histogram<u64> {
        &self.histogram
    }
    pub fn count(&self) -> u64 {
        self.histogram.len()
    }
    pub fn max(&self) -> u64 {
        self.histogram.max()
    }
    pub fn min(&self) -> u64 {
        self.histogram.min()
    }
    pub fn sum(&self) -> u64 {
        self.sum
    }

    pub fn clear(&mut self) {
        self.histogram.reset();
        self.sum = 0;
    }
}

/// Caputres a distribution of values.
#[derive(Clone)]
pub struct Stat {
    histo: Weak<Mutex<HistogramWithSum>>,
    bounds: Option<(u64, u64)>,
}

impl Stat {
    pub fn add(&self, v: u64) {
        if let Some(h) = self.histo.upgrade() {
            let mut histo = h.lock().expect("failed to obtain lock for stat");
            histo.record(v);
        }
    }

    pub fn add_values(&mut self, vs: &[u64]) {
        if let Some(h) = self.histo.upgrade() {
            let mut histo = h.lock().expect("failed to obtain lock for stat");
            for v in vs {
                histo.record(*v)
            }
        }
    }
}

#[derive(Clone)]
pub struct Timer {
    stat: Stat,
    unit: TimeUnit,
}
#[derive(Copy, Clone)]
pub enum TimeUnit {
    Millis,
    Micros,
}
impl Timer {
    pub fn record_since(&self, t0: Instant) {
        self.stat.add(to_u64(t0, self.unit));
    }

    pub fn time<F: Future>(&self, fut: F) -> Timed<impl Future<Output = F::Output>> {
        let stat = self.stat.clone();
        let unit = self.unit;
        let f = futures::future::lazy(|_| {
            // Start timing once the future is actually being invoked (and not
            // when the object is created).
            Timing::start()
        })
        .then(move |t0| {
            fut.map(move |v| {
                stat.add(to_u64(t0, unit));
                v
            })
        });
        Timed(f)
    }
}

fn to_u64(t0: Instant, unit: TimeUnit) -> u64 {
    match unit {
        TimeUnit::Millis => t0.elapsed_ms(),
        TimeUnit::Micros => t0.elapsed_us(),
    }
}

#[pin_project]
pub struct Timed<F>(#[pin] F);
impl<F: Future> Future for Timed<F> {
    type Output = F::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut futures::task::Context) -> Poll<Self::Output> {
        self.project().0.poll(cx)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_report_peek() {
        let (metrics, reporter) = super::new();
        let metrics = metrics.labeled("joy", "painting");

        let happy_accidents = metrics.counter("happy_accidents");
        let paint_level = metrics.gauge("paint_level");
        let mut stroke_len = metrics.stat("stroke_len");

        happy_accidents.incr(1);
        paint_level.set(2);
        stroke_len.add_values(&[1, 2, 3]);

        {
            let report = reporter.peek();
            {
                let k = report
                    .counters()
                    .keys()
                    .find(|k| k.name() == "happy_accidents")
                    .expect("expected counter: happy_accidents");
                assert_eq!(k.labels.get("joy"), Some(&"painting".to_string()));
                assert_eq!(report.counters().get(k), Some(&1));
            }
            {
                let k = report
                    .gauges()
                    .keys()
                    .find(|k| k.name() == "paint_level")
                    .expect("expected gauge: paint_level");
                assert_eq!(k.labels.get("joy"), Some(&"painting".to_string()));
                assert_eq!(report.gauges().get(k), Some(&2));
            }
            assert_eq!(
                report.gauges().keys().find(|k| k.name() == "brush_width"),
                None
            );
            {
                let k = report
                    .stats()
                    .keys()
                    .find(|k| k.name() == "stroke_len")
                    .expect("expected stat: stroke_len");
                assert_eq!(k.labels.get("joy"), Some(&"painting".to_string()));
                assert!(report.stats().contains_key(k));
            }
            assert_eq!(report.stats().keys().find(|k| k.name() == "tree_len"), None);
        }

        drop(paint_level);
        let brush_width = metrics.gauge("brush_width");
        let mut tree_len = metrics.stat("tree_len");

        happy_accidents.incr(2);
        brush_width.set(5);
        stroke_len.add_values(&[1, 2, 3]);
        tree_len.add_values(&[3, 4, 5]);

        {
            let report = reporter.peek();
            {
                let k = report
                    .counters()
                    .keys()
                    .find(|k| k.name() == "happy_accidents")
                    .expect("expected counter: happy_accidents");
                assert_eq!(k.labels.get("joy"), Some(&"painting".to_string()));
                assert_eq!(report.counters().get(k), Some(&3));
            }
            {
                let k = report
                    .gauges()
                    .keys()
                    .find(|k| k.name() == "paint_level")
                    .expect("expected gauge: paint_level");
                assert_eq!(k.labels.get("joy"), Some(&"painting".to_string()));
                assert_eq!(report.gauges().get(k), Some(&2));
            }
            {
                let k = report
                    .gauges()
                    .keys()
                    .find(|k| k.name() == "brush_width")
                    .expect("expected gauge: brush_width");
                assert_eq!(k.labels.get("joy"), Some(&"painting".to_string()));
                assert_eq!(report.gauges().get(k), Some(&5));
            }
            {
                let k = report
                    .stats()
                    .keys()
                    .find(|k| k.name() == "stroke_len")
                    .expect("expected stat: stroke_len");
                assert_eq!(k.labels.get("joy"), Some(&"painting".to_string()));
                assert!(report.stats().contains_key(k));
            }
            {
                let k = report
                    .stats()
                    .keys()
                    .find(|k| k.name() == "tree_len")
                    .expect("expected stat: tree_len");
                assert_eq!(k.labels.get("joy"), Some(&"painting".to_string()));
                assert!(report.stats().contains_key(k));
            }
        }
    }

    #[test]
    fn test_report_take() {
        let (metrics, mut reporter) = super::new();
        let metrics = metrics.labeled("joy", "painting");

        let happy_accidents = metrics.counter("happy_accidents");
        let paint_level = metrics.gauge("paint_level");
        let mut stroke_len = metrics.stat("stroke_len");
        happy_accidents.incr(1);
        paint_level.set(2);
        stroke_len.add_values(&[1, 2, 3]);
        {
            let report = reporter.take();
            {
                let k = report
                    .counters()
                    .keys()
                    .find(|k| k.name() == "happy_accidents")
                    .expect("expected counter: happy_accidents");
                assert_eq!(k.labels.get("joy"), Some(&"painting".to_string()));
                assert_eq!(report.counters().get(k), Some(&1));
            }
            {
                let k = report
                    .gauges()
                    .keys()
                    .find(|k| k.name() == "paint_level")
                    .expect("expected gauge");
                assert_eq!(k.labels.get("joy"), Some(&"painting".to_string()));
                assert_eq!(report.gauges().get(k), Some(&2));
            }
            assert_eq!(
                report.gauges().keys().find(|k| k.name() == "brush_width"),
                None
            );
            {
                let k = report
                    .stats()
                    .keys()
                    .find(|k| k.name() == "stroke_len")
                    .expect("expected stat");
                assert_eq!(k.labels.get("joy"), Some(&"painting".to_string()));
                assert!(report.stats().contains_key(k));
            }
            assert_eq!(report.stats().keys().find(|k| k.name() == "tree_len"), None);
            {
                let k = report
                    .stats()
                    .keys()
                    .find(|k| k.name() == "stroke_len")
                    .expect("expected stat");
                assert_eq!(k.labels.get("joy"), Some(&"painting".to_string()));
                assert!(report.stats().contains_key(k));
            }
        }

        drop(paint_level);
        drop(stroke_len);
        {
            let report = reporter.take();
            {
                let counters = report.counters();
                let k = counters
                    .keys()
                    .find(|k| k.name() == "happy_accidents")
                    .expect("expected counter: happy_accidents");
                assert_eq!(k.labels.get("joy"), Some(&"painting".to_string()));
                assert_eq!(counters.get(k), Some(&1));
            }
            {
                let k = report
                    .gauges()
                    .keys()
                    .find(|k| k.name() == "paint_level")
                    .expect("expected gauge");
                assert_eq!(k.labels.get("joy"), Some(&"painting".to_string()));
                assert_eq!(report.gauges().get(k), Some(&2));
            }
            {
                let k = report
                    .stats()
                    .keys()
                    .find(|k| k.name() == "stroke_len")
                    .expect("expected stat");
                assert_eq!(k.labels.get("joy"), Some(&"painting".to_string()));
                assert!(report.stats().contains_key(k));
            }
        }

        let brush_width = metrics.gauge("brush_width");
        let mut tree_len = metrics.stat("tree_len");
        happy_accidents.incr(2);
        brush_width.set(5);
        tree_len.add_values(&[3, 4, 5]);
        {
            let report = reporter.take();
            {
                let k = report
                    .counters()
                    .keys()
                    .find(|k| k.name() == "happy_accidents")
                    .expect("expected counter: happy_accidents");
                assert_eq!(k.labels.get("joy"), Some(&"painting".to_string()));
                assert_eq!(report.counters().get(k), Some(&3));
            }
            assert_eq!(
                report.gauges().keys().find(|k| k.name() == "paint_level"),
                None
            );
            {
                let k = report
                    .gauges()
                    .keys()
                    .find(|k| k.name() == "brush_width")
                    .expect("expected gauge");
                assert_eq!(k.labels.get("joy"), Some(&"painting".to_string()));
                assert_eq!(report.gauges().get(k), Some(&5));
            }
            assert_eq!(
                report.stats().keys().find(|k| k.name() == "stroke_len"),
                None
            );
            {
                let k = report
                    .stats()
                    .keys()
                    .find(|k| k.name() == "tree_len")
                    .expect("expeced stat");
                assert_eq!(k.labels.get("joy"), Some(&"painting".to_string()));
                assert!(report.stats().contains_key(k));
            }
        }
    }
}
