//! Per-poll latency benchmark for the Target waiter-queue drain.
//!
//! Background: `Target::poll` in `src/handler/target.rs` used to drain its
//! five `wait_for_*` queues with unbounded `while let Some(tx) = pop()`
//! loops — one call could fire thousands of oneshot sends synchronously
//! inside the handler's event loop. That produced tail-latency spikes for
//! every other future on the runtime under fan-out (e.g. many tasks all
//! awaiting the same lifecycle state).
//!
//! The drain is now capped at `WAITER_DRAIN_BUDGET = 64` sends per queue
//! per poll, with a self-wake to re-enter on the next tick for any
//! remainder. Total work is unchanged — it's just spread across polls so
//! no single poll burns more than a bounded amount of time.
//!
//! This bench compares the two shapes head-to-head:
//!
//! - `unbounded/N`: the old pop-all loop; one "poll" fires all N senders.
//! - `bounded/N`: the new helper; one "poll" fires at most 64 senders,
//!   then returns control. We measure the cost of a single poll — i.e.
//!   the per-poll latency that other futures on the runtime see.
//!
//! Expected result: `bounded/N` flattens for N >= 64 (each poll caps out
//! at ~64 sends), while `unbounded/N` grows linearly with N. For small
//! N (<= 64) the two are identical — the budget never fires.
//!
//! Run with:
//!   cargo bench --bench waiter_drain_bench

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use tokio::sync::oneshot::{self, Sender};

/// Mirror of the production constant in `src/handler/target.rs`. Kept in
/// sync by hand; a drift check lives in the unit tests (any change must
/// update the bench too).
const WAITER_DRAIN_BUDGET: usize = 64;

type Payload = Option<u64>;

/// Baseline: the previous unbounded `while let Some(tx) = pop()` shape.
/// Fires every queued sender in one call; returns when the queue empties.
#[inline]
fn drain_unbounded(queue: &mut Vec<Sender<Payload>>, value: Payload) {
    while let Some(tx) = queue.pop() {
        let _ = tx.send(value);
    }
}

/// Mirror of `drain_waiters_bounded` in `src/handler/target.rs`. Pops up
/// to `budget` senders and reports whether the queue is non-empty.
#[inline]
fn drain_bounded(queue: &mut Vec<Sender<Payload>>, value: Payload, budget: usize) -> bool {
    let to_fire = queue.len().min(budget);
    for _ in 0..to_fire {
        if let Some(tx) = queue.pop() {
            let _ = tx.send(value);
        }
    }
    !queue.is_empty()
}

/// Build N live oneshot senders. Receivers are kept alive so `send`
/// isn't a no-op (dropped receivers short-circuit the send path and
/// would skew the measurement).
fn make_waiters(n: usize) -> (Vec<Sender<Payload>>, Vec<oneshot::Receiver<Payload>>) {
    let mut txs = Vec::with_capacity(n);
    let mut rxs = Vec::with_capacity(n);
    for _ in 0..n {
        let (tx, rx) = oneshot::channel();
        txs.push(tx);
        rxs.push(rx);
    }
    (txs, rxs)
}

/// Measure the cost of a single "poll" — for unbounded, this is a full
/// drain; for bounded, it's one budget-capped pass.
fn bench_single_poll(c: &mut Criterion) {
    let mut g = c.benchmark_group("waiter_drain/single_poll");

    for &n in &[8_usize, 64, 256, 1024, 4096, 16_384] {
        g.throughput(Throughput::Elements(n.min(WAITER_DRAIN_BUDGET) as u64));

        g.bench_with_input(BenchmarkId::new("unbounded", n), &n, |b, &n| {
            b.iter_batched_ref(
                || make_waiters(n),
                |(txs, _rxs)| {
                    drain_unbounded(txs, black_box(Some(42)));
                },
                criterion::BatchSize::LargeInput,
            );
        });

        g.bench_with_input(BenchmarkId::new("bounded", n), &n, |b, &n| {
            b.iter_batched_ref(
                || make_waiters(n),
                |(txs, _rxs)| {
                    // One "poll" = one bounded drain pass. This is the
                    // number we care about: how long the handler blocks
                    // other futures per-poll under fan-out.
                    let _ = drain_bounded(txs, black_box(Some(42)), WAITER_DRAIN_BUDGET);
                },
                criterion::BatchSize::LargeInput,
            );
        });
    }
    g.finish();
}

/// Measure total drain cost across multiple polls. Proves the bounded
/// shape doesn't leak work — aggregate cost is unchanged, just amortised.
fn bench_full_drain(c: &mut Criterion) {
    let mut g = c.benchmark_group("waiter_drain/full_drain");

    for &n in &[64_usize, 256, 1024, 4096, 16_384] {
        g.throughput(Throughput::Elements(n as u64));

        g.bench_with_input(BenchmarkId::new("unbounded", n), &n, |b, &n| {
            b.iter_batched_ref(
                || make_waiters(n),
                |(txs, _rxs)| drain_unbounded(txs, black_box(Some(42))),
                criterion::BatchSize::LargeInput,
            );
        });

        g.bench_with_input(BenchmarkId::new("bounded", n), &n, |b, &n| {
            b.iter_batched_ref(
                || make_waiters(n),
                |(txs, _rxs)| {
                    // Repeatedly call bounded drain until empty — this is
                    // the equivalent of N handler polls in production.
                    while drain_bounded(txs, black_box(Some(42)), WAITER_DRAIN_BUDGET) {}
                },
                criterion::BatchSize::LargeInput,
            );
        });
    }
    g.finish();
}

criterion_group!(benches, bench_single_poll, bench_full_drain);
criterion_main!(benches);
