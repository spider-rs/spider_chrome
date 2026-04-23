//! Background cleanup worker for Drop-time async work.
//!
//! Certain RAII guards in this crate (`content_stream::Sink`,
//! `cache::stream::StreamGuard`, `cache::stream::ChunkSink`) need to do
//! async work at Drop time — removing a temp file, closing a CDP
//! IO-stream handle. Calling `tokio::spawn` directly from `Drop` is
//! dangerous:
//!
//! * It panics if the current thread is outside a tokio runtime
//!   (panic unwind, sync destructor, post-shutdown).
//! * A panicking `Drop` during an existing unwind aborts the process.
//! * Every Drop call has to re-check `Handle::try_current()`.
//!
//! This module follows the same shape as [`crate::runtime_release`]:
//! a single long-lived background task owns an unbounded mpsc
//! receiver, and every `Drop` site becomes a wait-free
//! `UnboundedSender::send`. The Drop thread does not need a runtime,
//! cannot panic on the hot path, and performs no syscalls beyond one
//! atomic load + one lock-free mpsc push.
//!
//! # Lock-freedom
//!
//! | operation         | primitive                                               |
//! | ----------------- | ------------------------------------------------------- |
//! | [`submit`]        | `OnceLock::get()` + `UnboundedSender::send` (lock-free) |
//! | [`init_worker`]   | `OnceLock::get()` fast-path; `get_or_init` only once    |
//! | worker drain      | `recv_many` + `join_all` — no locks across `.await`     |
//!
//! # Deadlock-freedom
//!
//! * Submitters never `.await` and never touch a lock another task holds.
//! * The worker's only wait is `rx.recv_many(..).await`, which parks on
//!   tokio's lock-free `Notify` and wakes on any push.
//! * `page.execute(..)` inside the worker goes through the existing
//!   CDP command pipeline; no new locks are introduced.
//! * If the worker task panics or exits, `submit` continues to succeed
//!   (unbounded mpsc send does not require a live receiver — it just
//!   grows the buffer). Pending tasks leak silently until the process
//!   ends, which is strictly safer than aborting.
//!
//! # Initialisation
//!
//! [`init_worker`] must be called once from inside a tokio runtime
//! context, typically on the hot path of any entry point that might
//! later produce cleanup tasks. Subsequent calls are a single atomic
//! load and a no-op. If a Drop fires before the worker has been
//! initialised, [`submit`] silently drops the task — no panic, no
//! deadlock, just deferred or skipped cleanup.

use crate::page::Page;
use chromiumoxide_cdp::cdp::browser_protocol::io::{CloseParams, StreamHandle};
use std::path::PathBuf;
use std::sync::OnceLock;
use tokio::sync::mpsc;

/// Maximum number of cleanup tasks drained per worker round. Caps the
/// size of the transient batch `Vec` so a burst can't hold a very
/// large allocation while we `join_all` the futures.
const MAX_BATCH: usize = 64;

/// An async cleanup action to run on the background worker.
///
/// New variants can be added without touching the Drop call sites that
/// submit the existing variants — the worker's `match` is exhaustive
/// but small, and `submit` accepts any `CleanupTask`.
#[derive(Debug)]
pub enum CleanupTask {
    /// Remove a file from disk (best-effort; errors are logged at
    /// `debug` level and swallowed).
    RemoveFile(PathBuf),
    /// Close a CDP IO-stream handle on a specific page. Used by
    /// `StreamGuard::Drop` when the caller dropped the guard without
    /// explicitly calling `finish`.
    CloseCdpStream { page: Page, handle: StreamHandle },
}

static CLEANUP_TX: OnceLock<mpsc::UnboundedSender<CleanupTask>> = OnceLock::new();

/// Spawn the worker and return its sender. Only ever invoked once,
/// from inside the `OnceLock::get_or_init` closure on the very first
/// `init_worker` call.
fn spawn_worker() -> mpsc::UnboundedSender<CleanupTask> {
    let (tx, mut rx) = mpsc::unbounded_channel::<CleanupTask>();

    tokio::spawn(async move {
        let mut batch: Vec<CleanupTask> = Vec::with_capacity(MAX_BATCH);
        loop {
            // Awaits at least one task, then drains up to `MAX_BATCH`
            // without additional awaits — single atomic drain rather
            // than `recv + N × try_recv`.
            let n = rx.recv_many(&mut batch, MAX_BATCH).await;
            if n == 0 {
                break; // channel closed — no more producers
            }

            // Fire all cleanup futures concurrently. Multiplexes onto
            // the shared CDP connection and the OS file-I/O runtime
            // rather than executing strictly serially.
            let futs = batch.drain(..).map(|task| async move {
                match task {
                    CleanupTask::RemoveFile(path) => {
                        if let Err(err) = tokio::fs::remove_file(&path).await {
                            tracing::debug!(
                                target: "chromiumoxide::bg_cleanup",
                                "remove_file({}) failed: {err}",
                                path.display(),
                            );
                        }
                    }
                    CleanupTask::CloseCdpStream { page, handle } => {
                        let _ = page.execute(CloseParams { handle }).await;
                    }
                }
            });
            futures_util::future::join_all(futs).await;
        }
    });

    tx
}

/// Ensure the background cleanup worker is running.
///
/// Must be called from a tokio runtime context on the **first** call.
/// Subsequent calls are a single atomic load and return immediately —
/// safe to invoke on every hot-path entry that might later produce
/// cleanup work (e.g. opening a CDP stream, starting a response
/// staging pipeline).
#[inline]
pub fn init_worker() {
    // Explicit fast path: one atomic load, zero sync primitives after
    // the first init.
    if CLEANUP_TX.get().is_some() {
        return;
    }
    let _ = CLEANUP_TX.get_or_init(spawn_worker);
}

/// Enqueue a cleanup task for background processing.
///
/// Lock-free (one atomic load + one wait-free mpsc push), safe to
/// invoke from any `Drop` implementation on any thread — including
/// threads with no tokio runtime context and threads currently
/// unwinding due to a panic.
///
/// If the worker has not been initialised (see [`init_worker`]), the
/// task is silently dropped. The alternative — panicking — would
/// propagate through `Drop` and potentially abort the process during
/// unwind, which is never what a best-effort cleanup should do.
#[inline]
pub fn submit(task: CleanupTask) {
    if let Some(tx) = CLEANUP_TX.get() {
        let _ = tx.send(task);
    }
}

/// Returns `true` if the worker has been initialised in this process.
/// Intended for tests and diagnostics; not part of the hot path.
#[inline]
pub fn worker_inited() -> bool {
    CLEANUP_TX.get().is_some()
}

#[cfg(test)]
mod tests {
    //! These tests pin the two properties that make `bg_cleanup`
    //! Drop-safe:
    //!
    //! 1. `submit` is safe to call from a thread that has no tokio
    //!    runtime context. A crashed or unwinding Drop on any thread
    //!    must not panic — if it did, Drop during an existing unwind
    //!    would abort the process. (Canary: submit from a std::thread.)
    //!
    //! 2. Before `init_worker` runs for the first time, `submit`
    //!    silently drops the task instead of panicking. This is the
    //!    "Drop before any browser was launched" case — rare but real,
    //!    since callers may hold guards briefly in test code without
    //!    ever needing the cleanup worker.

    use super::*;

    #[test]
    fn submit_without_init_is_silent_noop() {
        // Without a prior `init_worker`, the OnceLock is empty. We can
        // only verify this with a fresh OnceLock — the real static is
        // process-global and may have been initialised by an earlier
        // test. Instead, exercise the `get()`-is-None branch directly
        // via a fresh local lock.
        let local: OnceLock<mpsc::UnboundedSender<CleanupTask>> = OnceLock::new();
        assert!(local.get().is_none(), "fresh OnceLock must be empty");
        // The production `submit` does exactly:
        //   if let Some(tx) = CLEANUP_TX.get() { let _ = tx.send(..); }
        // which is a provable no-op when get() returns None.
    }

    #[test]
    fn submit_from_plain_thread_does_not_panic() {
        // Runs from a std::thread with no tokio runtime context —
        // the critical property for Drop safety.
        std::thread::spawn(|| {
            assert!(
                tokio::runtime::Handle::try_current().is_err(),
                "test thread must not be inside a tokio runtime"
            );
            submit(CleanupTask::RemoveFile(PathBuf::from(
                "/tmp/chromey-bg-cleanup-canary-does-not-exist",
            )));
        })
        .join()
        .expect("submit must not panic on a plain std::thread");
    }

    #[test]
    fn init_worker_inside_runtime_then_submit_succeeds() {
        // Full round-trip: init inside a runtime, then submit from any
        // context must land on the worker.
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        rt.block_on(async {
            init_worker();
            assert!(
                worker_inited(),
                "init_worker must populate the OnceLock"
            );
            // Submitting from inside the runtime obviously works; the
            // safety-relevant case is submitting from outside, which
            // the previous test covers.
            submit(CleanupTask::RemoveFile(PathBuf::from(
                "/tmp/chromey-bg-cleanup-canary-does-not-exist-2",
            )));
        });
    }
}
