//! Per-session task: drives one `Target` independently of all others.
//!
//! Owns the Target's mutable state (frame manager, network manager, init
//! state machine, pending command map). All state mutations happen on this
//! single task — no locks, no contention.
//!
//! Inputs:
//! * `page_wake` — wake signal raised when the Page handle pushed a
//!   `TargetMessage` (the Page's `mpsc::Sender` raises this Notify on send).
//! * `router_rx` — responses + events demuxed by the Router and handed to
//!   this slot.
//!
//! Outputs:
//! * `ws_tx` — shared write half of the WebSocket. All session tasks share
//!   this sender, but the writer task drains it serially so wire ordering
//!   is preserved.
//! * `session_to_router_tx` — lifecycle hints (session_id discovered, task
//!   exited).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chromiumoxide_cdp::cdp::browser_protocol::target::{AttachToTargetParams, SessionId};
use chromiumoxide_cdp::cdp::CdpEventMessage;
use chromiumoxide_types::{CallId, MethodCall, MethodId, Request, Response};
use tokio::sync::mpsc;
use tokio::sync::oneshot::Sender as OneshotSender;
use tokio::sync::Notify;

use crate::cmd::to_command_response;
use crate::error::{CdpError, Result};
use crate::handler::target::{Target, TargetEvent};

use super::ids;
use super::types::{RouterToSession, SessionToRouter};

/// One in-flight CDP request.
enum SessionPending {
    /// Driven by the Target's init chain — response goes back to
    /// `target.on_response()`. Carries the method id so we can route the
    /// payload through `to_command_response::<X>`.
    Internal,
    /// Driven by `Page::execute()` — response unblocks the caller via this
    /// oneshot.
    External(OneshotSender<Result<Response>>),
}

pub(crate) struct SessionTask {
    /// Slot index. Used to encode call_ids that the Router can demux.
    slot: u16,
    target: Target,
    page_wake: Arc<Notify>,
    router_rx: mpsc::Receiver<RouterToSession>,
    ws_tx: mpsc::Sender<MethodCall>,
    session_to_router_tx: mpsc::Sender<SessionToRouter>,
    next_seq: u64,
    pending: HashMap<CallId, (SessionPending, MethodId, Instant)>,
    /// Set once we've reported `SessionAttached` to the router.
    session_id_reported: bool,
    /// Reserved for future per-session eviction logic.
    #[allow(dead_code)]
    request_timeout: Duration,
}

impl SessionTask {
    pub fn new(
        slot: u16,
        target: Target,
        page_wake: Arc<Notify>,
        router_rx: mpsc::Receiver<RouterToSession>,
        ws_tx: mpsc::Sender<MethodCall>,
        session_to_router_tx: mpsc::Sender<SessionToRouter>,
        request_timeout: Duration,
    ) -> Self {
        Self {
            slot,
            target,
            page_wake,
            router_rx,
            ws_tx,
            session_to_router_tx,
            next_seq: 1,
            pending: HashMap::new(),
            session_id_reported: false,
            request_timeout,
        }
    }

    pub async fn run(mut self) {
        // First tick: kick the init state machine immediately so the Target
        // emits `Target.attachToTarget`.
        self.drive(Instant::now()).await;

        loop {
            tokio::select! {
                biased;

                msg = self.router_rx.recv() => {
                    match msg {
                        Some(RouterToSession::Response(call_id, resp, method)) => {
                            self.on_response(call_id, resp, method);
                        }
                        Some(RouterToSession::Event(ev)) => {
                            self.target.on_event(*ev);
                        }
                        Some(RouterToSession::SetInitiator(tx)) => {
                            self.target.set_initiator(tx);
                        }
                        Some(RouterToSession::Shutdown) | None => {
                            break;
                        }
                    }
                }

                _ = self.page_wake.notified() => {
                    // page_rx will be drained in `drive()` below
                }
            }

            self.drive(Instant::now()).await;
        }

        let _ = self
            .session_to_router_tx
            .send(SessionToRouter::Detached { slot: self.slot })
            .await;
    }

    /// Drain the page channel, advance the Target, dispatch every event.
    async fn drive(&mut self, now: Instant) {
        // Drain page channel non-blocking. The page Sender raises
        // `page_wake` after each send so the select! arm above re-fires.
        // Collect into a Vec first to release the &mut borrow on `target`
        // before calling `on_page_message`.
        let mut pending_msgs = Vec::new();
        if let Some(h) = self.target.page_mut() {
            while let Ok(msg) = h.rx.try_recv() {
                pending_msgs.push(msg);
            }
        }
        for msg in pending_msgs {
            self.target.on_page_message(msg);
        }

        // Push the Target's state machine forward.
        loop {
            let event = self.target.advance(now);
            match event {
                None => break,
                Some(TargetEvent::Request(req)) => {
                    self.submit_internal(req, now).await;
                }
                Some(TargetEvent::Command(msg)) => {
                    if msg.is_navigation() {
                        // Phase 2 minimum scope: the bench/smoke do not
                        // navigate. Surface a clear error so any future
                        // caller is alerted.
                        let _ = msg.sender.send(Err(CdpError::msg(
                            "navigation not yet supported by parallel handler",
                        )));
                    } else {
                        self.submit_external(msg, now).await;
                    }
                }
                // Stub: navigation tracking deferred to a follow-up.
                Some(TargetEvent::NavigationRequest(_, _))
                | Some(TargetEvent::NavigationResult(_))
                | Some(TargetEvent::BytesConsumed(_)) => {}
            }
        }

        self.target.event_listeners_mut().flush();
    }

    fn alloc_call_id(&mut self) -> CallId {
        let id = ids::encode(self.slot, self.next_seq);
        self.next_seq = self.next_seq.wrapping_add(1);
        id
    }

    async fn submit_internal(&mut self, req: Request, now: Instant) {
        let call_id = self.alloc_call_id();
        let method = req.method.clone();
        let call = MethodCall {
            id: call_id,
            method: req.method,
            session_id: req.session_id,
            params: req.params,
        };
        match self.ws_tx.send(call).await {
            Ok(()) => {
                self.pending
                    .insert(call_id, (SessionPending::Internal, method, now));
            }
            Err(_) => {
                // Writer is closed — nothing more we can do.
            }
        }
    }

    async fn submit_external(&mut self, msg: crate::cmd::CommandMessage, now: Instant) {
        let call_id = self.alloc_call_id();
        let method = msg.method.clone();
        let (req, sender) = msg.split();
        let call = MethodCall {
            id: call_id,
            method: req.method,
            session_id: req.session_id,
            params: req.params,
        };
        match self.ws_tx.send(call).await {
            Ok(()) => {
                self.pending
                    .insert(call_id, (SessionPending::External(sender), method, now));
            }
            Err(_) => {
                let _ = sender.send(Err(CdpError::msg("WS writer closed")));
            }
        }
    }

    fn on_response(&mut self, call_id: CallId, resp: Response, _method_hint: MethodId) {
        let Some((pending, method, _ts)) = self.pending.remove(&call_id) else {
            return;
        };
        match pending {
            SessionPending::Internal => {
                // Pick up session_id from `Target.attachToTarget` response.
                if method.as_ref() == AttachToTargetParams::IDENTIFIER {
                    if let Ok(parsed) =
                        to_command_response::<AttachToTargetParams>(resp.clone(), method.clone())
                    {
                        let sid: SessionId = parsed.result.session_id;
                        self.target.set_session_id(sid.clone());
                        if !self.session_id_reported {
                            self.session_id_reported = true;
                            let slot = self.slot;
                            let sid_str: String = sid.into();
                            let tx = self.session_to_router_tx.clone();
                            tokio::spawn(async move {
                                let _ = tx
                                    .send(SessionToRouter::SessionAttached {
                                        slot,
                                        session_id: sid_str,
                                    })
                                    .await;
                            });
                        }
                    }
                }
                self.target.on_response(resp, method.as_ref());
            }
            SessionPending::External(tx) => {
                let _ = tx.send(Ok(resp));
            }
        }
    }

    /// Forward an event the Router routed to this session into the Target.
    #[allow(dead_code)]
    pub(crate) fn dispatch_event(&mut self, event: CdpEventMessage) {
        self.target.on_event(event);
    }

    #[allow(dead_code)]
    pub fn slot(&self) -> u16 {
        self.slot
    }

    #[allow(dead_code)]
    pub fn request_timeout(&self) -> Duration {
        self.request_timeout
    }
}
