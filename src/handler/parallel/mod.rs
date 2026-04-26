//! Parallel handler — per-session sub-loops.
//!
//! Replaces the single `tokio::select!` driver with one `Router` task plus
//! one `SessionTask` per attached page. The Router demuxes the WS reader
//! using a slot index encoded in CDP `id` bits, so response routing has no
//! shared map. Each SessionTask owns its `Target` outright — no locks, no
//! contention.
//!
//! Opt-in via the `parallel-handler` feature. When the feature is off this
//! module is not compiled.
//!
//! # Phase 2 minimum scope
//!
//! Only what the bench / smoke tests need:
//! * `Browser::new_page` end-to-end (target attach + init chain).
//! * `Page::execute` round-trip through the per-page mpsc + per-session
//!   pending map.
//! * Browser-level `Browser::execute` (slot-0 dispatch).
//!
//! Deferred to a follow-up:
//! * Navigation tracking (`Page::goto` and friends).
//! * Target detached / crashed handling.
//! * Eviction of stalled pending commands across slots.
//! * Multi-context / GetTargets / FetchTargets.
//! * Event listener API (`Browser::event_listener`).

mod ids;
mod router;
mod session;
mod types;

pub(crate) use router::Router;
