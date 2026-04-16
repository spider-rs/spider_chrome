//! Benchmarks for the remote cache dump worker.
//!
//! These benchmarks measure the overhead of the batch-drain, dedup, and
//! concurrent upload machinery without requiring a running cache server.
//!
//! Run with:
//!   cargo bench --bench remote_cache_bench --features cache

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use spider_remote_cache::{DumpJob, HttpVersion};

/// Create a test DumpJob with a given cache_key.
fn make_job(key: &str) -> DumpJob {
    DumpJob {
        cache_key: key.to_string(),
        cache_site: "example.com".to_string(),
        url: format!("https://example.com/{key}"),
        method: "GET".to_string(),
        status: 200,
        request_headers: HashMap::new(),
        response_headers: {
            let mut h = HashMap::new();
            h.insert("content-type".into(), "text/html".into());
            h
        },
        body: vec![b'<'; 1024], // 1 KiB body
        http_version: HttpVersion::Http11,
        dump_remote: None,
    }
}

// ---------------------------------------------------------------------------
//  Batch drain throughput — channel drain + dedup (no HTTP)
// ---------------------------------------------------------------------------

fn bench_batch_drain_throughput(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    for num_jobs in [10, 100, 1000] {
        c.bench_function(
            &format!("batch_drain: {num_jobs} jobs (drain + dedup)"),
            |b| {
                b.iter(|| {
                    rt.block_on(async {
                        let (tx, mut rx) = tokio::sync::mpsc::channel::<DumpJob>(num_jobs + 1);

                        // Enqueue all jobs.
                        for i in 0..num_jobs {
                            let _ = tx.try_send(make_job(&format!("key-{i}")));
                        }

                        // Drain.
                        let mut batch = Vec::with_capacity(num_jobs);
                        if let Some(first) = rx.try_recv().ok() {
                            batch.push(first);
                        }
                        while let Ok(job) = rx.try_recv() {
                            batch.push(job);
                        }

                        // Dedup against a DashSet.
                        let inflight = dashmap::DashSet::new();
                        let mut accepted = 0usize;
                        for job in &batch {
                            if inflight.insert(job.cache_key.clone()) {
                                accepted += 1;
                            }
                        }

                        black_box((batch.len(), accepted));
                    });
                });
            },
        );
    }
}

// ---------------------------------------------------------------------------
//  Dedup effectiveness — measure skip rate with duplicate keys
// ---------------------------------------------------------------------------

fn bench_dedup_with_duplicates(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    // 50% duplicates: 500 unique keys, each sent twice = 1000 jobs.
    c.bench_function("dedup: 1000 jobs, 50% duplicates", |b| {
        b.iter(|| {
            rt.block_on(async {
                let inflight = dashmap::DashSet::new();
                let mut accepted = 0usize;
                let mut skipped = 0usize;

                for i in 0..1000usize {
                    let key = format!("key-{}", i % 500);
                    if inflight.insert(key) {
                        accepted += 1;
                    } else {
                        skipped += 1;
                    }
                }

                black_box((accepted, skipped));
            });
        });
    });
}

// ---------------------------------------------------------------------------
//  Concurrent upload simulation — mock endpoint, measure wall time
// ---------------------------------------------------------------------------

fn bench_concurrent_upload_simulation(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    // Simulate uploads that take 1ms each.
    // Sequential: 100 x 1ms = 100ms.
    // Concurrent (32 slots): ~4ms.
    for (concurrency, num_jobs) in [(1, 100), (8, 100), (32, 100)] {
        c.bench_function(
            &format!("upload_sim: {num_jobs} jobs, concurrency {concurrency}, 1ms each"),
            |b| {
                b.iter(|| {
                    rt.block_on(async {
                        let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
                        let completed = Arc::new(AtomicUsize::new(0));
                        let mut join_set = tokio::task::JoinSet::new();

                        for _ in 0..num_jobs {
                            let permit = sem.clone().acquire_owned().await.unwrap();
                            let completed = completed.clone();
                            join_set.spawn(async move {
                                // Simulate 1ms upload.
                                tokio::time::sleep(Duration::from_millis(1)).await;
                                completed.fetch_add(1, Ordering::Relaxed);
                                drop(permit);
                            });
                        }

                        while join_set.join_next().await.is_some() {}

                        black_box(completed.load(Ordering::Relaxed));
                    });
                });
            },
        );
    }
}

// ---------------------------------------------------------------------------
//  End-to-end worker throughput — enqueue + drain + dedup (no HTTP)
// ---------------------------------------------------------------------------

fn bench_worker_enqueue_throughput(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    for num_jobs in [100, 1000] {
        c.bench_function(
            &format!("enqueue_throughput: {num_jobs} try_send calls"),
            |b| {
                b.iter(|| {
                    rt.block_on(async {
                        let (tx, _rx) = tokio::sync::mpsc::channel::<DumpJob>(num_jobs + 1);

                        let mut sent = 0usize;
                        let mut dropped = 0usize;

                        for i in 0..num_jobs {
                            match tx.try_send(make_job(&format!("key-{i}"))) {
                                Ok(()) => sent += 1,
                                Err(_) => dropped += 1,
                            }
                        }

                        black_box((sent, dropped));
                    });
                });
            },
        );
    }
}

// ---------------------------------------------------------------------------
//  Payload serialization throughput
// ---------------------------------------------------------------------------

fn bench_payload_serialization(c: &mut Criterion) {
    use spider_remote_cache::build_payload;

    let req_headers = HashMap::new();
    let mut resp_headers = HashMap::new();
    resp_headers.insert("content-type".into(), "text/html".into());
    let body = vec![b'X'; 4096]; // 4 KiB
    let version = HttpVersion::Http11;

    c.bench_function("build_payload + serialize (4KB body)", |b| {
        b.iter(|| {
            let payload = build_payload(
                black_box("cache-key-1"),
                black_box("https://example.com/page"),
                black_box(&body),
                black_box("GET"),
                black_box(200),
                black_box(&req_headers),
                black_box(&resp_headers),
                black_box(&version),
            );
            let json = serde_json::to_string(&payload).unwrap();
            black_box(json);
        });
    });

    // Batch serialization: 10 payloads.
    c.bench_function("build_payload x10 + serialize batch (4KB each)", |b| {
        b.iter(|| {
            let payloads: Vec<_> = (0..10)
                .map(|i| {
                    build_payload(
                        &format!("key-{i}"),
                        &format!("https://example.com/page-{i}"),
                        &body,
                        "GET",
                        200,
                        &req_headers,
                        &resp_headers,
                        &version,
                    )
                })
                .collect();
            let json = serde_json::to_string(&payloads).unwrap();
            black_box(json);
        });
    });
}

criterion_group!(
    benches,
    bench_batch_drain_throughput,
    bench_dedup_with_duplicates,
    bench_concurrent_upload_simulation,
    bench_worker_enqueue_throughput,
    bench_payload_serialization,
);
criterion_main!(benches);
