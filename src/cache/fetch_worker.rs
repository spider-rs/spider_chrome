//! Background worker for the Fetch-interceptor response-stage cache
//! dump path.
//!
//! `spawn_fetch_interceptor_cache_listener` produces a stream of
//! `Fetch.requestPaused` events. At the *response* stage, each
//! eligible event drives a multi-step async pipeline:
//!
//! 1. Stream the response body via CDP `IO.read`.
//! 2. Build an `HttpResponse` from headers + streamed body.
//! 3. Hand off to `put_hybrid_cache`, which enqueues the remote
//!    dump to the shared `spider_remote_cache` worker.
//!
//! Previously, every response-stage event was dispatched via its
//! own `tokio::spawn`. Each spawn allocates a task header (~2-4 KB)
//! and costs ~1 µs of setup — manageable per-event, but it adds
//! up under burst (one page with 100 subresources = 100 spawns,
//! ~200-400 KB transient task memory).
//!
//! This module consolidates dispatch behind a singleton
//! `OnceLock<UnboundedSender<FetchResponseJob>>` + batched
//! dispatcher, identical in shape to [`crate::bg_cleanup`] and
//! [`crate::runtime_release`]:
//!
//! * Submit is a single atomic `OnceLock::get()` + lock-free
//!   `UnboundedSender::send` — wait-free, safe from any async
//!   context.
//! * The dispatcher drains up to [`DISPATCH_BATCH`] jobs per wake
//!   via `rx.recv_many(..)` and hands each batch to **one**
//!   `tokio::spawn`'d batch worker.
//! * The batch worker drives a `FuturesUnordered` of every job's
//!   pipeline, polling all of them concurrently. Independent jobs
//!   (different `request_id`s, different URLs) have no cross-job
//!   ordering constraint, so unordered polling is correct and
//!   maximally parallel.
//! * One spawn per batch vs per job → ~64× reduction in spawn
//!   count while preserving full concurrency of the I/O pipeline.
//!
//! Safety properties:
//!
//! * **No deadlock.** Dispatcher never `.await`s on job work;
//!   loop body is `drain → spawn batch-worker`. A slow job cannot
//!   block subsequent batches from being picked up.
//! * **No panic propagation.** Any panic inside a job is contained
//!   by the batch worker's `tokio::spawn` boundary and does not
//!   affect the dispatcher or other batches.
//! * **No cross-job ordering needed.** CDP dispatches events in
//!   arrival order and each job's `request_id` is unique, so the
//!   per-job pipelines are independent.

use crate::cache::manager::{handle_fetch_response_stage, CacheStrategy};
use crate::cdp::browser_protocol::fetch::EventRequestPaused;
use crate::page::Page;
use futures_util::stream::{FuturesUnordered, StreamExt};
use std::sync::{Arc, OnceLock};
use tokio::sync::mpsc;

/// Maximum number of jobs drained per dispatcher wake. Under burst
/// the dispatcher spawns one batch worker per drain; concurrent
/// batches are not limited by this constant.
const DISPATCH_BATCH: usize = 64;

/// A queued response-stage handling job.
///
/// Carries only cheap-clone / `Arc`-wrapped references:
/// `Page` is `Arc<PageInner>`, `ev` is already `Arc<_>` from the
/// event listener, `auth` is an owned `String`, `cache_strategy`
/// is `Copy`. No heavy clone is performed at submit time.
#[derive(Debug)]
pub struct FetchResponseJob {
    pub page: Page,
    pub ev: Arc<EventRequestPaused>,
    pub auth: Option<String>,
    pub cache_strategy: Option<CacheStrategy>,
}

static FETCH_TX: OnceLock<mpsc::UnboundedSender<FetchResponseJob>> = OnceLock::new();

/// Spawn the dispatcher and return its sender. Only ever invoked
/// once, from inside the `OnceLock::get_or_init` closure on the
/// very first `init_worker` call.
fn spawn_worker() -> mpsc::UnboundedSender<FetchResponseJob> {
    let (tx, mut rx) = mpsc::unbounded_channel::<FetchResponseJob>();

    tokio::spawn(async move {
        // Reused across iterations via `mem::replace` — one
        // allocation per batch, sized to match the workload.
        let mut batch: Vec<FetchResponseJob> = Vec::with_capacity(DISPATCH_BATCH);
        loop {
            let n = rx.recv_many(&mut batch, DISPATCH_BATCH).await;
            if n == 0 {
                break; // channel closed — no more producers
            }
            let next_cap = n.min(DISPATCH_BATCH);
            let jobs: Vec<FetchResponseJob> =
                std::mem::replace(&mut batch, Vec::with_capacity(next_cap));
            tokio::spawn(async move {
                let mut in_flight: FuturesUnordered<_> = jobs
                    .into_iter()
                    .map(|job| async move {
                        if let Err(err) = handle_fetch_response_stage(
                            &job.page,
                            &job.ev,
                            job.auth.as_deref(),
                            job.cache_strategy.as_ref(),
                        )
                        .await
                        {
                            tracing::debug!(
                                "cache stream interceptor error: {err:?} - {:?}",
                                job.ev.request.url
                            );
                        }
                    })
                    .collect();
                while in_flight.next().await.is_some() {}
            });
        }
    });

    tx
}

/// Ensure the fetch-response background worker is running.
///
/// Must be called from a tokio runtime context on the first call.
/// Subsequent calls are a single atomic load and return
/// immediately.
#[inline]
pub fn init_worker() {
    if FETCH_TX.get().is_some() {
        return;
    }
    let _ = FETCH_TX.get_or_init(spawn_worker);
}

/// Enqueue a fetch-response job for background processing.
///
/// Lock-free (one atomic load + one wait-free mpsc push). If the
/// worker has not been initialised the job is silently dropped —
/// the caller's page will fall back to the non-streaming
/// Network-listener path.
#[inline]
pub fn submit(job: FetchResponseJob) {
    if let Some(tx) = FETCH_TX.get() {
        let _ = tx.send(job);
    }
}

/// Returns `true` if the worker has been initialised. Intended for
/// diagnostics; not part of the hot path.
#[inline]
pub fn worker_inited() -> bool {
    FETCH_TX.get().is_some()
}
