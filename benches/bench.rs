use criterion::{criterion_group, criterion_main, Criterion};
use tacho::*;

static DEFAULT_METRIC_NAME: &'static str = "a_sufficiently_long_name";
fn bench_scope_clone(c: &mut Criterion) {
    c.bench_function("clone", |b| {
        let (metrics, _) = new();
        b.iter(move || {
            let _ = metrics.clone();
        });
    });
}

fn bench_scope_label(c: &mut Criterion) {
    c.bench_function("labeled", |b| {
        let (metrics, _) = new();
        b.iter(move || {
            let _ = metrics.clone().labeled("foo", "bar");
        });
    });
}

fn bench_scope_clone_x1000(c: &mut Criterion) {
    c.bench_function("clone 1000", |b| {
        let scopes = mk_scopes(1000, "bench_scope_clone_x1000");
        b.iter(move || {
            for scope in &scopes {
                let _ = scope.clone();
            }
        });
    });
}

fn bench_scope_label_x1000(c: &mut Criterion) {
    c.bench_function("label 1000", |b| {
        let scopes = mk_scopes(1000, "bench_scope_label_x1000");
        b.iter(move || {
            for scope in &scopes {
                let _ = scope.clone().labeled("foo", "bar");
            }
        });
    });
}

fn bench_counter_create(c: &mut Criterion) {
    c.bench_function("counter", |b| {
        let (metrics, _) = new();
        b.iter(move || {
            let _ = metrics.counter(DEFAULT_METRIC_NAME);
        });
    });
}

fn bench_gauge_create(c: &mut Criterion) {
    c.bench_function("gauge", |b| {
        let (metrics, _) = new();
        b.iter(move || {
            let _ = metrics.gauge(DEFAULT_METRIC_NAME);
        });
    });
}

fn bench_stat_create(c: &mut Criterion) {
    c.bench_function("stat", |b| {
        let (metrics, _) = new();
        b.iter(move || {
            let _ = metrics.stat(DEFAULT_METRIC_NAME);
        });
    });
}

fn bench_counter_create_x1000(c: &mut Criterion) {
    c.bench_function("counter 10000", |b| {
        let scopes = mk_scopes(1000, "bench_counter_create_x1000");
        b.iter(move || {
            for scope in &scopes {
                scope.counter(DEFAULT_METRIC_NAME);
            }
        });
    });
}

fn bench_gauge_create_x1000(c: &mut Criterion) {
    c.bench_function("gauge 1000", |b| {
        let scopes = mk_scopes(1000, "bench_gauge_create_x1000");
        b.iter(move || {
            for scope in &scopes {
                scope.gauge(DEFAULT_METRIC_NAME);
            }
        });
    });
}

fn bench_stat_create_x1000(c: &mut Criterion) {
    c.bench_function("stat 1000", |b| {
        let scopes = mk_scopes(1000, "bench_stat_create_x1000");
        b.iter(move || {
            for scope in &scopes {
                scope.stat(DEFAULT_METRIC_NAME);
            }
        });
    });
}

fn bench_counter_update(c: &mut Criterion) {
    c.bench_function("counter update", |b| {
        let (metrics, _) = new();
        let cnt = metrics.counter(DEFAULT_METRIC_NAME);
        b.iter(move || cnt.incr(1));
    });
}

fn bench_gauge_update(c: &mut Criterion) {
    c.bench_function("gauge update", |b| {
        let (metrics, _) = new();
        let g = metrics.gauge(DEFAULT_METRIC_NAME);
        b.iter(move || g.set(1));
    });
}

fn bench_stat_update(c: &mut Criterion) {
    c.bench_function("stat update", |b| {
        let (metrics, _) = new();
        let s = metrics.stat(DEFAULT_METRIC_NAME);
        b.iter(move || s.add(1));
    });
}

fn bench_counter_update_x1000(c: &mut Criterion) {
    c.bench_function("counter update 1000", |b| {
        let counters: Vec<Counter> = mk_scopes(1000, "bench_counter_update_x1000")
            .iter()
            .map(|s| s.counter(DEFAULT_METRIC_NAME))
            .collect();
        b.iter(move || {
            for c in &counters {
                c.incr(1)
            }
        });
    });
}

fn bench_gauge_update_x1000(c: &mut Criterion) {
    c.bench_function("gauge update 1000", |b| {
        let gauges: Vec<Gauge> = mk_scopes(1000, "bench_gauge_update_x1000")
            .iter()
            .map(|s| s.gauge(DEFAULT_METRIC_NAME))
            .collect();
        b.iter(move || {
            for g in &gauges {
                g.set(1)
            }
        });
    });
}

fn bench_stat_update_x1000(c: &mut Criterion) {
    c.bench_function("stat update 1000", |b| {
        let stats: Vec<Stat> = mk_scopes(1000, "bench_stat_update_x1000")
            .iter()
            .map(|s| s.stat(DEFAULT_METRIC_NAME))
            .collect();
        b.iter(move || {
            for s in &stats {
                s.add(1)
            }
        });
    });
}

fn bench_stat_add_x1000(c: &mut Criterion) {
    c.bench_function("stat add 1000", |b| {
        let s = {
            let (metrics, _) = new();
            metrics.stat(DEFAULT_METRIC_NAME)
        };
        b.iter(move || {
            for i in 0..1000 {
                s.add(i)
            }
        });
    });
}

fn mk_scopes(n: usize, name: &str) -> Vec<Scope> {
    let (metrics, _) = new();
    let metrics = metrics
        .prefixed("t")
        .labeled("test_name", name)
        .labeled("total_iterations", n);
    (0..n)
        .map(|i| metrics.clone().labeled("iteration", format!("{}", i)))
        .collect()
}

criterion_group!(
    benches,
    bench_counter_create,
    bench_counter_create_x1000,
    bench_counter_update,
    bench_counter_update_x1000,
    bench_gauge_create,
    bench_gauge_create_x1000,
    bench_gauge_update,
    bench_gauge_update_x1000,
    bench_scope_clone,
    bench_scope_clone_x1000,
    bench_scope_label,
    bench_scope_label_x1000,
    bench_stat_add_x1000,
    bench_stat_create,
    bench_stat_create_x1000,
    bench_stat_update,
    bench_stat_update_x1000
);
criterion_main!(benches);
