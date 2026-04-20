//! Enqueue-throughput benchmark for the `runtime_release` batcher.
//!
//! Measures the wait-free cost of `try_release` on the hot path: one
//! atomic `OnceLock::get()` load + one `UnboundedSender::send` per call.
//!
//! The worker's CDP `page.execute(...)` half is NOT exercised here (that
//! would require a live Chrome); we install our own receiver-drain task
//! so the channel doesn't back up and distort the enqueue measurement.
//!
//! Run with:
//!   cargo bench --bench runtime_release_bench

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use std::sync::OnceLock;
use tokio::sync::mpsc;

/// Bench-local mirror of the release batcher's channel shape.  We keep a
/// separate static here so the bench is self-contained — measuring the
/// *pattern* (OnceLock + UnboundedSender) rather than the real
/// `RELEASE_TX` (which we don't want to feed with fake Pages anyway).
static BENCH_TX: OnceLock<mpsc::UnboundedSender<u64>> = OnceLock::new();

fn sender(rt: &tokio::runtime::Runtime) -> &'static mpsc::UnboundedSender<u64> {
    BENCH_TX.get_or_init(|| {
        let (tx, mut rx) = mpsc::unbounded_channel::<u64>();
        // Spawn a sink task that drains the channel as fast as it fills.
        rt.spawn(async move {
            let mut batch: Vec<u64> = Vec::with_capacity(64);
            loop {
                let first = match rx.recv().await {
                    Some(v) => v,
                    None => return,
                };
                batch.clear();
                batch.push(first);
                // recv_many-style batch drain.
                while batch.len() < 64 {
                    match rx.try_recv() {
                        Ok(v) => batch.push(v),
                        Err(_) => break,
                    }
                }
                // Pretend to process.  `black_box` keeps the compiler from
                // optimising the loop away.
                for v in batch.drain(..) {
                    let _ = black_box(v);
                }
            }
        });
        tx
    })
}

#[inline]
fn try_release(id: u64) {
    if let Some(tx) = BENCH_TX.get() {
        let _ = tx.send(id);
    }
}

fn bench_single_thread_enqueue(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime");
    // Prime the OnceLock from the runtime context.
    rt.block_on(async {
        let _ = sender(&rt);
    });

    let mut g = c.benchmark_group("runtime_release/enqueue");
    g.throughput(Throughput::Elements(1));
    g.bench_function("single_thread", |b| b.iter(|| try_release(black_box(42))));
    g.finish();
}

fn bench_burst_enqueue(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(async {
        let _ = sender(&rt);
    });

    let mut g = c.benchmark_group("runtime_release/burst");
    for &n in &[64_usize, 1024, 8192] {
        g.throughput(Throughput::Elements(n as u64));
        g.bench_function(format!("burst_{n}"), |b| {
            b.iter(|| {
                for i in 0..n {
                    try_release(black_box(i as u64));
                }
            })
        });
    }
    g.finish();
}

criterion_group!(benches, bench_single_thread_enqueue, bench_burst_enqueue);
criterion_main!(benches);
