//! Minimal in-process CDP mock server used by integration tests and benches.
//!
//! Speaks just enough Chrome DevTools Protocol to let `Browser::connect` attach
//! and to drive `Browser::new_page` to a usable `Page` handle. Every supported
//! method returns a canned shape; unknown methods return `{}` so unrelated CDP
//! calls don't error out.
//!
//! The mock binds to `127.0.0.1:0`, so each test gets its own ephemeral port.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message as WsMessage;

/// Handle to a running mock server. Drop it to shut the server down.
pub struct CdpMock {
    addr: SocketAddr,
    accept_task: Option<JoinHandle<()>>,
    shutdown: Arc<tokio::sync::Notify>,
}

impl CdpMock {
    /// Bind to a free local port and start accepting WebSocket connections.
    pub async fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind 127.0.0.1:0");
        let addr = listener.local_addr().expect("local_addr");
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let shutdown_clone = shutdown.clone();

        let accept_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_clone.notified() => break,
                    accept = listener.accept() => {
                        match accept {
                            Ok((stream, _)) => {
                                tokio::spawn(handle_connection(stream));
                            }
                            Err(_) => break,
                        }
                    }
                }
            }
        });

        Self {
            addr,
            accept_task: Some(accept_task),
            shutdown,
        }
    }

    /// `ws://…` URL that `Browser::connect` understands.
    pub fn ws_url(&self) -> String {
        format!("ws://{}/devtools/browser/mock", self.addr)
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

impl Drop for CdpMock {
    fn drop(&mut self) {
        self.shutdown.notify_waiters();
        if let Some(t) = self.accept_task.take() {
            t.abort();
        }
    }
}

// ---------------------------------------------------------------------------
//  Per-connection state machine
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ConnState {
    next_target: u64,
    next_session: u64,
    next_loader: u64,
    /// session_id → (target_id, frame_id, loader_id)
    sessions: hashbrown::HashMap<String, (String, String, String)>,
}

async fn handle_connection(stream: tokio::net::TcpStream) {
    let _ = stream.set_nodelay(true);
    let ws = match tokio_tungstenite::accept_async(tokio_tungstenite::MaybeTlsStream::Plain(
        stream,
    ))
    .await
    {
        Ok(ws) => ws,
        Err(_) => return,
    };
    let (mut sink, mut stream) = ws.split();
    let mut state = ConnState::default();

    while let Some(msg) = stream.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(_) => break,
        };
        let text = match msg {
            WsMessage::Text(t) => t,
            WsMessage::Binary(b) => match std::str::from_utf8(&b) {
                Ok(s) => s.to_string().into(),
                Err(_) => continue,
            },
            WsMessage::Close(_) => break,
            WsMessage::Ping(_) | WsMessage::Pong(_) | WsMessage::Frame(_) => continue,
        };

        let req: serde_json::Value = match serde_json::from_str(text.as_str()) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let id = req.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
        let method = req
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let session_id = req
            .get("sessionId")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let params = req.get("params").cloned().unwrap_or(serde_json::Value::Null);

        let outbox = handle_method(&mut state, id, &method, session_id.as_deref(), &params);
        for line in outbox {
            if sink.send(WsMessage::Text(line.into())).await.is_err() {
                return;
            }
        }
    }
}

/// Returns the wire-format JSON strings the mock should emit for this request,
/// in send order. Events come before the response when the handler relies on
/// event-driven state (e.g. `Target.targetCreated` must arrive before the
/// `Target.createTarget` response).
fn handle_method(
    state: &mut ConnState,
    id: u64,
    method: &str,
    session_id: Option<&str>,
    params: &serde_json::Value,
) -> Vec<String> {
    let mut out = Vec::with_capacity(2);

    match method {
        "Target.setDiscoverTargets" => {
            out.push(empty_response(id));
        }

        "Target.createTarget" => {
            state.next_target += 1;
            let target_id = format!("target-{:08x}", state.next_target);
            let url = params
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("about:blank")
                .to_string();
            // `Target.targetCreated` event before the response so the handler
            // has the target in its map by the time `on_response` runs.
            out.push(target_created_event(&target_id, &url));
            out.push(json_response(
                id,
                serde_json::json!({ "targetId": target_id }),
            ));
        }

        "Target.attachToTarget" => {
            let target_id = params
                .get("targetId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            state.next_session += 1;
            state.next_loader += 1;
            let sid = format!("session-{:08x}", state.next_session);
            let frame_id = format!("frame-{}", &sid);
            let loader_id = format!("loader-{:08x}", state.next_loader);
            state
                .sessions
                .insert(sid.clone(), (target_id.clone(), frame_id.clone(), loader_id));
            out.push(attached_to_target_event(&sid, &target_id));
            out.push(json_response(id, serde_json::json!({ "sessionId": sid })));
        }

        "Page.getFrameTree" => {
            let (frame_id, loader_id) = session_id
                .and_then(|s| state.sessions.get(s))
                .map(|(_t, f, l)| (f.clone(), l.clone()))
                .unwrap_or_else(|| ("frame-unknown".into(), "loader-unknown".into()));
            out.push(json_response(
                id,
                serde_json::json!({
                    "frameTree": {
                        "frame": frame_tree_frame(&frame_id, &loader_id),
                        "childFrames": []
                    }
                }),
            ));
            // Mark the main frame as loaded so the target's `Initialized`
            // state can deliver the Page back to the caller of `new_page`.
            if let Some(sid) = session_id {
                out.push(lifecycle_event(sid, &frame_id, &loader_id, "init"));
                out.push(lifecycle_event(sid, &frame_id, &loader_id, "DOMContentLoaded"));
                out.push(lifecycle_event(sid, &frame_id, &loader_id, "load"));
            }
        }

        // All other CDP commands the chromey init chain issues — return `{}`.
        // This is enough because the handler only inspects responses for the
        // specific methods above; everything else just clears its pending entry.
        _ => {
            out.push(empty_response(id));
        }
    }

    out
}

// ---------------------------------------------------------------------------
//  Wire helpers
// ---------------------------------------------------------------------------

fn empty_response(id: u64) -> String {
    format!(r#"{{"id":{},"result":{{}}}}"#, id)
}

fn json_response(id: u64, result: serde_json::Value) -> String {
    serde_json::json!({ "id": id, "result": result }).to_string()
}

fn target_created_event(target_id: &str, url: &str) -> String {
    serde_json::json!({
        "method": "Target.targetCreated",
        "params": {
            "targetInfo": target_info(target_id, url, false)
        }
    })
    .to_string()
}

fn attached_to_target_event(session_id: &str, target_id: &str) -> String {
    serde_json::json!({
        "method": "Target.attachedToTarget",
        "params": {
            "sessionId": session_id,
            "targetInfo": target_info(target_id, "about:blank", true),
            "waitingForDebugger": false
        }
    })
    .to_string()
}

fn target_info(target_id: &str, url: &str, attached: bool) -> serde_json::Value {
    serde_json::json!({
        "targetId": target_id,
        "type": "page",
        "title": "",
        "url": url,
        "attached": attached,
        "canAccessOpener": false
    })
}

fn frame_tree_frame(frame_id: &str, loader_id: &str) -> serde_json::Value {
    serde_json::json!({
        "id": frame_id,
        "loaderId": loader_id,
        "url": "about:blank",
        "domainAndRegistry": "",
        "securityOrigin": "://",
        "mimeType": "text/html",
        "secureContextType": "Secure",
        "crossOriginIsolatedContextType": "NotIsolated",
        "gatedAPIFeatures": []
    })
}

fn lifecycle_event(session_id: &str, frame_id: &str, loader_id: &str, name: &str) -> String {
    serde_json::json!({
        "method": "Page.lifecycleEvent",
        "sessionId": session_id,
        "params": {
            "frameId": frame_id,
            "loaderId": loader_id,
            "name": name,
            "timestamp": 0.0
        }
    })
    .to_string()
}

#[allow(unused)]
pub static SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);
