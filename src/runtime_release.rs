//! Background batcher for `Runtime.releaseObject` calls.
//!
//! # Lock-freedom
//!
//! Every hot-path operation reduces to one or two relaxed-ordering atomic
//! ops, with no mutex, no `Once`-style sync after the first init:
//!
//! | operation       | primitive                                                |
//! | --------------- | -------------------------------------------------------- |
//! | `try_release`   | `OnceLock::get()` (atomic load) + `UnboundedSender::send` (tokio's lock-free MPSC list) |
//! | `init_worker`   | `OnceLock::get()` fast-path check; `get_or_init` only on the *first* call ever |
//! | `Drop` guards   | `Arc::clone` (atomic fetch-add) + `try_release`          |
//!
//! The **first** `init_worker` call runs `OnceLock::get_or_init`, which
//! briefly uses the `Once` sync primitive to serialise racing initialisers.
//! This is a one-time per-process cost; subsequent calls skip it entirely
//! via an explicit `get()` fast-path check.
//!
//! # Deadlock-freedom
//!
//! - `Drop` handlers never await, never acquire a lock another async task
//!   holds, and never allocate beyond an atomic pointer push into the MPSC.
//! - The worker holds no locks across its awaits; `rx.recv().await` parks
//!   on a lock-free `Notify`, and `page.execute` goes through the existing
//!   CDP command pipeline (no new locks introduced).
//! - If the worker task panics or the runtime shuts down, `try_release`
//!   continues to succeed (channel send does not require a live receiver);
//!   releases are silently dropped and V8 reclaims on context teardown.
//!
//! # Batching
//!
//! The worker drains up to [`MAX_BATCH`] pending releases per round and
//! fires them **concurrently** through `futures::join_all`, multiplexing on
//! the shared CDP connection rather than stalling per call.

use crate::page::Page;
use chromiumoxide_cdp::cdp::js_protocol::runtime::{ReleaseObjectParams, RemoteObjectId};
use std::sync::OnceLock;
use tokio::sync::mpsc;

/// Max items drained per worker round.  Cap keeps a burst from holding a
/// giant `Vec` while we issue concurrent CDP commands.
const MAX_BATCH: usize = 64;

static RELEASE_TX: OnceLock<mpsc::UnboundedSender<(Page, RemoteObjectId)>> = OnceLock::new();

/// Spawn the worker and return its sender.  Only ever invoked once, from
/// inside the `OnceLock::get_or_init` closure on the very first init.
fn spawn_worker() -> mpsc::UnboundedSender<(Page, RemoteObjectId)> {
    let (tx, mut rx) = mpsc::unbounded_channel::<(Page, RemoteObjectId)>();

    tokio::spawn(async move {
        let mut batch: Vec<(Page, RemoteObjectId)> = Vec::with_capacity(MAX_BATCH);
        loop {
            // `recv_many` awaits at least one item and then drains up to
            // `limit` without additional awaits — single atomic drain
            // rather than the `recv + N×try_recv` CAS loop.
            let n = rx.recv_many(&mut batch, MAX_BATCH).await;
            if n == 0 {
                break; // channel closed — no more producers
            }

            // Fire all release commands concurrently.  Each `page.execute`
            // multiplexes onto the shared CDP connection; `join_all` lets
            // Chrome process them in parallel rather than strict serial.
            let futs = batch.drain(..).map(|(page, id)| async move {
                let _ = page.execute(ReleaseObjectParams::new(id)).await;
            });
            futures_util::future::join_all(futs).await;
        }
    });

    tx
}

/// Ensure the background release worker is running.
///
/// Hot path is a single atomic `OnceLock::get()` load; only the very first
/// call ever touches `get_or_init` (and thus the `Once` sync primitive).
/// Must be invoked from a tokio runtime context on the first call.
#[inline]
pub fn init_worker() {
    // Explicit fast path: one atomic load, zero sync primitives after the
    // first init.
    if RELEASE_TX.get().is_some() {
        return;
    }
    let _ = RELEASE_TX.get_or_init(spawn_worker);
}

/// Returns `true` if the worker has been initialised in this process.
#[inline]
pub fn worker_inited() -> bool {
    RELEASE_TX.get().is_some()
}

/// Enqueue a remote-object release for background processing.
///
/// Lock-free (one atomic load + one wait-free MPSC push), safe to invoke
/// from a `Drop` implementation on any thread.  If the worker has not yet
/// been initialised the release is silently dropped; V8 reclaims the
/// object on the next execution-context teardown.
#[inline]
pub fn try_release(page: Page, object_id: RemoteObjectId) {
    if let Some(tx) = RELEASE_TX.get() {
        let _ = tx.send((page, object_id));
    }
}
