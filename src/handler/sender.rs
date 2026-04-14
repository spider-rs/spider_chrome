//! A thin wrapper around `mpsc::Sender<TargetMessage>` that optionally
//! notifies the handler's `tokio::select!` loop when a message is sent.
//!
//! When `Handler::run()` is used (the tokio-native path), the contained
//! `Notify` wakes the event loop so it can drain target channels
//! immediately.  When the legacy `impl Stream for Handler` is used,
//! the notify is `None` and the wrapper adds zero overhead.

use std::sync::Arc;
use tokio::sync::{mpsc, Notify};

use super::target::TargetMessage;

/// Sender half for page → target communication.
///
/// Wraps a bounded `mpsc::Sender<TargetMessage>` and, when running under
/// `Handler::run()`, an `Arc<Notify>` that wakes the handler's
/// `tokio::select!` loop after every successful send.
#[derive(Debug, Clone)]
pub struct PageSender {
    inner: mpsc::Sender<TargetMessage>,
    wake: Option<Arc<Notify>>,
}

impl PageSender {
    /// Create a new `PageSender`.
    ///
    /// Pass `None` for `wake` when using the `Stream`-based handler
    /// (no notification overhead). Pass `Some(notify)` when using
    /// `Handler::run()`.
    pub fn new(inner: mpsc::Sender<TargetMessage>, wake: Option<Arc<Notify>>) -> Self {
        Self { inner, wake }
    }

    /// Notify the handler (if a `Notify` is configured).
    #[inline]
    fn notify(&self) {
        if let Some(wake) = &self.wake {
            wake.notify_one();
        }
    }

    /// Non-blocking send.  Notifies the handler on success.
    pub fn try_send(
        &self,
        msg: TargetMessage,
    ) -> Result<(), mpsc::error::TrySendError<TargetMessage>> {
        let result = self.inner.try_send(msg);
        if result.is_ok() {
            self.notify();
        }
        result
    }

    /// Async send (waits for capacity).  Notifies the handler on success.
    pub async fn send(
        &self,
        msg: TargetMessage,
    ) -> Result<(), mpsc::error::SendError<TargetMessage>> {
        let result = self.inner.send(msg).await;
        if result.is_ok() {
            self.notify();
        }
        result
    }
}
