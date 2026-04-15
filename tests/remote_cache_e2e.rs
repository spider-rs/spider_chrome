//! End-to-end tests for the remote cache dump worker.
//!
//! These tests launch a local `hybrid_cache_server` instance, enqueue
//! DumpJobs, and verify that the entries are stored and retrievable.
//!
//! Requirements:
//!   - The `hybrid_cache_server` binary must be pre-built at
//!     `../index_cache_server/target/release/hybrid_cache_server`
//!     (relative to this repo's root).
//!
//! Run with:
//!   cargo test --test remote_cache_e2e --features cache

use std::collections::HashMap;
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::Duration;

use spider_remote_cache::{DumpJob, HybridCachePayload, HttpVersion};
use spider_remote_cache::{build_payload, dump_batch_to_remote, dump_to_remote};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find a free TCP port by binding to port 0.
fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind to free port");
    listener.local_addr().unwrap().port()
}

/// Path to the pre-built cache server binary.
fn server_binary() -> PathBuf {
    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let base = PathBuf::from(manifest_dir);
    let candidates = [
        base.join("../index_cache_server/target/release/hybrid_cache_server"),
        base.join("../index_cache_server/target/debug/hybrid_cache_server"),
    ];
    for candidate in &candidates {
        if candidate.exists() {
            return candidate.clone();
        }
    }
    panic!(
        "hybrid_cache_server binary not found. Build it first:\n\
         cd ../index_cache_server && cargo build --release"
    );
}

struct TestServer {
    child: std::process::Child,
    port: u16,
    _temp_dir: tempfile::TempDir,
}

impl TestServer {
    async fn start() -> Self {
        let port = free_port();
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let child = std::process::Command::new(server_binary())
            .env("CACHE_PORT", port.to_string())
            .env("MEILI_DISABLE", "true")
            .env("RUST_LOG", "warn")
            .current_dir(temp_dir.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn hybrid_cache_server");

        let url = format!("http://127.0.0.1:{port}/cache/size");
        let client = reqwest::Client::new();
        for _ in 0..100 {
            match client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    return Self {
                        child,
                        port,
                        _temp_dir: temp_dir,
                    }
                }
                _ => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
        panic!("cache server did not start on port {port} within 10 seconds");
    }

    fn endpoint(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Create a test DumpJob.
fn make_job(key: &str, body: &str, endpoint: &str) -> DumpJob {
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
        body: body.as_bytes().to_vec(),
        http_version: HttpVersion::Http11,
        dump_remote: Some(endpoint.to_string()),
    }
}

/// Upload jobs directly (bypassing the OnceCell worker) by building payloads
/// and calling the remote upload functions. This avoids test interference from
/// the global OnceCell.
async fn upload_jobs_directly(jobs: Vec<DumpJob>, endpoint: &str) {
    if jobs.len() == 1 {
        let job = &jobs[0];
        dump_to_remote(
            &job.cache_key,
            &job.cache_site,
            &job.url,
            &job.body,
            &job.method,
            job.status,
            &job.request_headers,
            &job.response_headers,
            &job.http_version,
            Some(endpoint),
        )
        .await;
    } else {
        let payloads: Vec<HybridCachePayload> = jobs
            .iter()
            .map(|job| {
                build_payload(
                    &job.cache_key,
                    &job.url,
                    &job.body,
                    &job.method,
                    job.status,
                    &job.request_headers,
                    &job.response_headers,
                    &job.http_version,
                )
            })
            .collect();
        dump_batch_to_remote(payloads, endpoint).await;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Enqueue a handful of jobs via single-item uploads and verify they arrive.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn single_item_uploads_are_stored_in_remote_cache() {
    let server = TestServer::start().await;
    let endpoint = server.endpoint();

    // Upload 10 jobs one at a time.
    for i in 0..10 {
        let job = make_job(
            &format!("resource-{i}"),
            &format!("<html><body>Page {i}</body></html>"),
            &endpoint,
        );
        upload_jobs_directly(vec![job], &endpoint).await;
    }

    // Verify entries exist.
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{endpoint}/cache/site/example.com"))
        .send()
        .await
        .expect("GET site");

    assert!(
        resp.status().is_success(),
        "expected success from /cache/site, got {}",
        resp.status()
    );

    let body = resp.text().await.expect("read body");

    for i in 0..10 {
        assert!(
            body.contains(&format!("resource-{i}"))
                || body.contains(&format!("example.com/resource-{i}")),
            "resource-{i} not found in cache site response"
        );
    }
}

/// Verify batch upload works correctly.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn batch_upload_stores_all_entries() {
    let server = TestServer::start().await;
    let endpoint = server.endpoint();

    // Upload 20 jobs as a batch.
    let jobs: Vec<DumpJob> = (0..20)
        .map(|i| {
            make_job(
                &format!("batch-{i}"),
                &format!("<html><body>Batch page {i}</body></html>"),
                &endpoint,
            )
        })
        .collect();

    upload_jobs_directly(jobs, &endpoint).await;

    // Verify entries exist.
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{endpoint}/cache/site/example.com"))
        .send()
        .await
        .expect("GET site");

    assert!(resp.status().is_success());
    let body = resp.text().await.expect("read body");

    for i in 0..20 {
        assert!(
            body.contains(&format!("batch-{i}"))
                || body.contains(&format!("example.com/batch-{i}")),
            "batch-{i} not found in cache site response"
        );
    }
}

/// Stress test: concurrent uploads do not deadlock or panic.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_uploads_do_not_deadlock() {
    let server = TestServer::start().await;
    let endpoint = server.endpoint();

    // Spawn 50 concurrent upload tasks, each uploading a batch of 5 items.
    let mut handles = Vec::new();
    for t in 0..50 {
        let endpoint = endpoint.clone();
        handles.push(tokio::spawn(async move {
            let jobs: Vec<DumpJob> = (0..5)
                .map(|j| {
                    make_job(
                        &format!("stress-{t}-{j}"),
                        &format!("<html>Stress {t}-{j}</html>"),
                        &endpoint,
                    )
                })
                .collect();
            upload_jobs_directly(jobs, &endpoint).await;
        }));
    }

    // All tasks must complete within 30 seconds (no deadlock).
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        for h in handles {
            h.await.expect("task join");
        }
    })
    .await;

    assert!(
        result.is_ok(),
        "concurrent uploads did not complete within 30 seconds — possible deadlock"
    );
}

/// Verify that the worker's batch-drain + dedup logic works via a channel.
/// Uses a local channel (not the global OnceCell) to avoid test interference.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn worker_batch_drain_and_dedup() {
    let server = TestServer::start().await;
    let endpoint = server.endpoint();

    // Simulate the worker's batch-drain logic manually.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DumpJob>(1000);
    let inflight = dashmap::DashSet::new();

    // Enqueue 20 jobs with 10 unique keys (each key appears twice).
    for i in 0..20 {
        let key_idx = i % 10;
        let job = make_job(
            &format!("drain-key-{key_idx}"),
            &format!("<html>Drain {i}</html>"),
            &endpoint,
        );
        tx.send(job).await.expect("send");
    }

    // Batch-drain.
    let mut batch = Vec::new();
    while let Ok(job) = rx.try_recv() {
        batch.push(job);
    }
    assert_eq!(batch.len(), 20, "should have drained all 20 jobs");

    // Dedup.
    let mut deduped = Vec::new();
    for job in batch {
        if inflight.insert(job.cache_key.clone()) {
            deduped.push(job);
        }
    }
    assert_eq!(deduped.len(), 10, "should have 10 unique keys after dedup");

    // Upload the deduped batch.
    upload_jobs_directly(deduped, &endpoint).await;

    // Verify 10 entries stored.
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{endpoint}/cache/site/example.com"))
        .send()
        .await
        .expect("GET site");

    assert!(resp.status().is_success());
    let body = resp.text().await.expect("read body");

    for i in 0..10 {
        assert!(
            body.contains(&format!("drain-key-{i}"))
                || body.contains(&format!("example.com/drain-key-{i}")),
            "drain-key-{i} not found after dedup upload"
        );
    }
}

/// Verify that try_send drops jobs when the queue is full.
#[tokio::test]
async fn try_send_drops_on_full_queue() {
    let (tx, _rx) = tokio::sync::mpsc::channel::<DumpJob>(5);

    let mut sent = 0;
    let mut dropped = 0;

    for i in 0..20 {
        let job = DumpJob {
            cache_key: format!("overflow-{i}"),
            cache_site: "test.com".to_string(),
            url: format!("https://test.com/{i}"),
            method: "GET".to_string(),
            status: 200,
            request_headers: HashMap::new(),
            response_headers: HashMap::new(),
            body: vec![],
            http_version: HttpVersion::Http11,
            dump_remote: Some("http://127.0.0.1:1".to_string()),
        };
        match tx.try_send(job) {
            Ok(()) => sent += 1,
            Err(_) => dropped += 1,
        }
    }

    assert!(sent > 0, "should have sent at least some jobs");
    assert!(dropped > 0, "should have dropped some jobs on full queue");
    eprintln!("try_send: sent={sent}, dropped={dropped}");
}

/// Individual resource retrieval after caching.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cached_resource_is_retrievable() {
    let server = TestServer::start().await;
    let endpoint = server.endpoint();

    let body_content = "<html><body>Hello from e2e test</body></html>";
    let job = make_job("e2e-resource-1", body_content, &endpoint);
    let cache_key = job.cache_key.clone();
    upload_jobs_directly(vec![job], &endpoint).await;

    // Retrieve by resource key.
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{endpoint}/cache/resource/{cache_key}"))
        .send()
        .await
        .expect("GET resource");

    assert!(
        resp.status().is_success(),
        "expected success for resource lookup, got {}",
        resp.status()
    );

    let resp_body = resp.text().await.expect("read body");
    // The response contains base64-encoded body — just verify it's non-empty.
    assert!(
        !resp_body.is_empty(),
        "cached resource response should not be empty"
    );
}
