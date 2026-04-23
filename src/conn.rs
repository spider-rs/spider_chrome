use std::collections::VecDeque;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::ready;

use futures_util::stream::SplitSink;
use futures_util::{SinkExt, Stream, StreamExt};
use std::task::{Context, Poll};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::{tungstenite::protocol::WebSocketConfig, WebSocketStream};

use chromiumoxide_cdp::cdp::browser_protocol::target::SessionId;
use chromiumoxide_types::{CallId, EventMessage, Message, MethodCall, MethodId};

use crate::error::CdpError;
use crate::error::Result;

type ConnectStream = MaybeTlsStream<tokio::net::TcpStream>;

/// Exchanges the messages with the websocket
#[must_use = "streams do nothing unless polled"]
#[derive(Debug)]
pub struct Connection<T: EventMessage> {
    /// Queue of commands to send.
    pending_commands: VecDeque<MethodCall>,
    /// The websocket of the chromium instance
    ws: WebSocketStream<ConnectStream>,
    /// The identifier for a specific command
    next_id: usize,
    /// Whether the write buffer has unsent data that needs flushing.
    needs_flush: bool,
    /// The phantom marker.
    _marker: PhantomData<T>,
}

lazy_static::lazy_static! {
    /// Nagle's algorithm disabled?
    static ref DISABLE_NAGLE: bool = match std::env::var("DISABLE_NAGLE") {
        Ok(disable_nagle) => disable_nagle == "true",
        _ => true
    };
    /// Websocket config defaults
    static ref WEBSOCKET_DEFAULTS: bool = match std::env::var("WEBSOCKET_DEFAULTS") {
        Ok(d) => d == "true",
        _ => false
    };
}

/// Default number of WebSocket connection retry attempts.
pub const DEFAULT_CONNECTION_RETRIES: u32 = 4;

/// Initial backoff delay between connection retries (in milliseconds).
const INITIAL_BACKOFF_MS: u64 = 50;

/// Maximum backoff delay between connection retries (in milliseconds).
const MAX_BACKOFF_MS: u64 = 2_000;

impl<T: EventMessage + Unpin> Connection<T> {
    pub async fn connect(debug_ws_url: impl AsRef<str>) -> Result<Self> {
        Self::connect_with_retries(debug_ws_url, DEFAULT_CONNECTION_RETRIES).await
    }

    pub async fn connect_with_retries(debug_ws_url: impl AsRef<str>, retries: u32) -> Result<Self> {
        let mut config = WebSocketConfig::default();

        // Cap the internal write buffer so a slow receiver cannot cause
        // unbounded memory growth (default is usize::MAX).
        config.max_write_buffer_size = 4 * 1024 * 1024;

        if !*WEBSOCKET_DEFAULTS {
            config.max_message_size = None;
            config.max_frame_size = None;
        }

        let url = debug_ws_url.as_ref();
        let use_uring = crate::uring_fs::is_enabled();
        let mut last_err = None;

        for attempt in 0..=retries {
            let result = if use_uring {
                Self::connect_uring(url, config).await
            } else {
                Self::connect_default(url, config).await
            };

            match result {
                Ok(ws) => {
                    return Ok(Self {
                        pending_commands: Default::default(),
                        ws,
                        next_id: 0,
                        needs_flush: false,
                        _marker: Default::default(),
                    });
                }
                Err(e) => {
                    // Detect non-retriable errors early to avoid wasting time
                    // on connections that will never succeed.
                    let should_retry = match &e {
                        // Connection refused — nothing is listening on this port.
                        CdpError::Io(io_err)
                            if io_err.kind() == std::io::ErrorKind::ConnectionRefused =>
                        {
                            false
                        }
                        // HTTP response to a WebSocket upgrade (e.g. wrong path
                        // returns 404 / redirect) — retrying the same URL won't help.
                        CdpError::Ws(tungstenite_err) => !matches!(
                            tungstenite_err,
                            tokio_tungstenite::tungstenite::Error::Http(_)
                                | tokio_tungstenite::tungstenite::Error::HttpFormat(_)
                        ),
                        _ => true,
                    };

                    last_err = Some(e);

                    if !should_retry {
                        break;
                    }

                    if attempt < retries {
                        let backoff_ms =
                            (INITIAL_BACKOFF_MS * 3u64.saturating_pow(attempt)).min(MAX_BACKOFF_MS);
                        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                    }
                }
            }
        }

        Err(last_err.unwrap_or_else(|| CdpError::msg("connection failed")))
    }

    /// Default path: let tokio-tungstenite handle TCP connect + WS handshake.
    async fn connect_default(
        url: &str,
        config: WebSocketConfig,
    ) -> Result<WebSocketStream<ConnectStream>> {
        let (ws, _) =
            tokio_tungstenite::connect_async_with_config(url, Some(config), *DISABLE_NAGLE).await?;
        Ok(ws)
    }

    /// io_uring path: pre-connect the TCP socket via io_uring, then do WS
    /// handshake over the pre-connected stream.
    async fn connect_uring(
        url: &str,
        config: WebSocketConfig,
    ) -> Result<WebSocketStream<ConnectStream>> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;

        let request = url.into_client_request()?;
        let host = request
            .uri()
            .host()
            .ok_or_else(|| CdpError::msg("no host in CDP WebSocket URL"))?;
        let port = request.uri().port_u16().unwrap_or(9222);

        // Resolve host → SocketAddr (CDP is always localhost, so this is fast).
        let addr_str = format!("{}:{}", host, port);
        let addr: std::net::SocketAddr = match addr_str.parse() {
            Ok(a) => a,
            Err(_) => {
                // Hostname needs DNS — fall back to default path.
                return Self::connect_default(url, config).await;
            }
        };

        // TCP connect via io_uring.
        let std_stream = crate::uring_fs::tcp_connect(addr)
            .await
            .map_err(CdpError::Io)?;

        // Set non-blocking + Nagle.
        std_stream.set_nonblocking(true).map_err(CdpError::Io)?;
        if *DISABLE_NAGLE {
            let _ = std_stream.set_nodelay(true);
        }

        // Wrap in tokio TcpStream.
        let tokio_stream = tokio::net::TcpStream::from_std(std_stream).map_err(CdpError::Io)?;

        // WebSocket handshake over the pre-connected stream.
        let (ws, _) = tokio_tungstenite::client_async_with_config(
            request,
            MaybeTlsStream::Plain(tokio_stream),
            Some(config),
        )
        .await?;

        Ok(ws)
    }
}

impl<T: EventMessage> Connection<T> {
    fn next_call_id(&mut self) -> CallId {
        let id = CallId::new(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        id
    }

    /// Queue in the command to send over the socket and return the id for this
    /// command
    pub fn submit_command(
        &mut self,
        method: MethodId,
        session_id: Option<SessionId>,
        params: serde_json::Value,
    ) -> serde_json::Result<CallId> {
        let id = self.next_call_id();
        let call = MethodCall {
            id,
            method,
            session_id: session_id.map(Into::into),
            params,
        };
        self.pending_commands.push_back(call);
        Ok(id)
    }

    /// Buffer all queued commands into the WebSocket sink, then flush once.
    ///
    /// This batches multiple CDP commands into a single TCP write instead of
    /// flushing after every individual message.
    fn start_send_next(&mut self, cx: &mut Context<'_>) -> Result<()> {
        // Complete any pending flush from a previous poll first.
        if self.needs_flush {
            match self.ws.poll_flush_unpin(cx) {
                Poll::Ready(Ok(())) => self.needs_flush = false,
                Poll::Ready(Err(e)) => return Err(e.into()),
                Poll::Pending => return Ok(()),
            }
        }

        // Buffer as many queued commands as the sink will accept.
        let mut sent_any = false;
        while !self.pending_commands.is_empty() {
            match self.ws.poll_ready_unpin(cx) {
                Poll::Ready(Ok(())) => {
                    let Some(cmd) = self.pending_commands.pop_front() else {
                        break;
                    };
                    tracing::trace!("Sending {:?}", cmd);
                    let msg = serde_json::to_string(&cmd)?;
                    self.ws.start_send_unpin(msg.into())?;
                    sent_any = true;
                }
                _ => break,
            }
        }

        // Flush the entire batch in one write.
        if sent_any {
            match self.ws.poll_flush_unpin(cx) {
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(e)) => return Err(e.into()),
                Poll::Pending => self.needs_flush = true,
            }
        }

        Ok(())
    }
}

/// Capacity of the bounded channel feeding the background WS writer task.
/// Large enough that bursts of CDP commands never block the handler, small
/// enough to apply back-pressure before memory grows without bound.
const WS_CMD_CHANNEL_CAPACITY: usize = 2048;

/// Capacity of the bounded channel from the background WS reader task to
/// the Handler. Keeps decoded CDP messages buffered so the reader task
/// can keep reading the socket while the Handler processes a backlog;
/// applies TCP-level back-pressure on Chrome when the Handler is slow
/// (the reader awaits channel capacity, stops draining the socket).
const WS_READ_CHANNEL_CAPACITY: usize = 1024;

/// Split parts returned by [`Connection::into_async`].
#[derive(Debug)]
pub struct AsyncConnection<T: EventMessage> {
    /// Receive half for decoded CDP messages. Backed by a bounded mpsc
    /// fed by a dedicated background reader task — decode runs on that
    /// task, never on the Handler task, so large CDP responses (multi-MB
    /// screenshots, huge event payloads) cannot stall the Handler's
    /// event loop.
    pub reader: WsReader<T>,
    /// Sender half for submitting outgoing CDP commands.
    pub cmd_tx: mpsc::Sender<MethodCall>,
    /// Handle to the background writer task.
    pub writer_handle: tokio::task::JoinHandle<Result<()>>,
    /// Handle to the background reader task (reads + decodes WS frames).
    pub reader_handle: tokio::task::JoinHandle<()>,
    /// Next command-call-id counter (continue numbering from where Connection left off).
    pub next_id: usize,
}

impl<T: EventMessage + Unpin + Send + 'static> Connection<T> {
    /// Consume the connection and split into a background reader + writer
    /// pair, exposing the Handler-facing ends via `AsyncConnection`.
    ///
    /// Two `tokio::spawn`'d tasks are created:
    ///
    /// * `ws_write_loop` — batches outgoing commands and flushes them in
    ///   one write per wakeup.
    /// * `ws_read_loop`  — reads WS frames, decodes them to typed
    ///   `Message<T>`, and forwards them via a bounded mpsc to the
    ///   Handler. Ping/pong/malformed frames are skipped on this task
    ///   and never reach the Handler. Large-message decode (SerDe CPU
    ///   work) runs here, **not** on the Handler task, so the Handler's
    ///   poll loop never stalls for tens of milliseconds on a 10 MB
    ///   screenshot response.
    ///
    /// The design uses only `tokio::spawn` (cooperative async) — no
    /// `spawn_blocking` or blocking thread-pool — so it scales with the
    /// tokio runtime's worker threads on multi-threaded runtimes, and
    /// interleaves cleanly with the Handler task on single-threaded
    /// runtimes.
    pub fn into_async(self) -> AsyncConnection<T> {
        let (ws_sink, ws_stream) = self.ws.split();
        let (cmd_tx, cmd_rx) = mpsc::channel(WS_CMD_CHANNEL_CAPACITY);
        let (msg_tx, msg_rx) = mpsc::channel::<Result<Box<Message<T>>>>(WS_READ_CHANNEL_CAPACITY);

        let writer_handle = tokio::spawn(ws_write_loop(ws_sink, cmd_rx));
        let reader_handle = tokio::spawn(ws_read_loop::<T, _>(ws_stream, msg_tx));

        let reader = WsReader {
            rx: msg_rx,
            _marker: PhantomData,
        };

        AsyncConnection {
            reader,
            cmd_tx,
            writer_handle,
            reader_handle,
            next_id: self.next_id,
        }
    }
}

/// Background task that reads frames from the WebSocket, decodes them to
/// typed CDP `Message<T>`, and forwards them to the Handler over a
/// bounded mpsc.
///
/// Runs on a `tokio::spawn`'d task — **not** `spawn_blocking` — so CPU
/// time for JSON decode is charged to a regular tokio worker and not the
/// blocking thread pool. On a multi-threaded runtime, the decode can run
/// on a different worker than the Handler, giving true parallelism for
/// large messages. On a single-threaded runtime, it cooperates with the
/// Handler via `.await` points on the send channel.
///
/// Flow per frame:
///
/// * `Text` / `Binary` → `decode_message::<T>`; decoded `Ok(msg)` is
///   sent to the Handler. Decode errors are logged and the frame is
///   dropped (same behavior as the legacy inline decode path).
/// * `Close` → loop exits cleanly, dropping `tx`. The Handler's
///   `next_message().await` returns `None` on the next call.
/// * `Ping` / `Pong` / unexpected frame types → skipped silently; they
///   never cross the channel to the Handler.
/// * Transport error → forwarded as `Err(CdpError::Ws(..))`, then the
///   loop exits (the WS half is considered dead after an error).
///
/// Back-pressure: the outbound `tx` is bounded. If the Handler is busy
/// and the channel fills, `tx.send(..).await` parks this task, which
/// stops draining the WS socket. TCP flow control then applies
/// back-pressure to Chrome instead of letting memory grow without bound.
async fn ws_read_loop<T, S>(mut stream: S, tx: mpsc::Sender<Result<Box<Message<T>>>>)
where
    T: EventMessage,
    S: Stream<Item = std::result::Result<WsMessage, tokio_tungstenite::tungstenite::Error>>
        + Unpin,
{
    while let Some(frame) = stream.next().await {
        match frame {
            Ok(WsMessage::Text(text)) => {
                match decode_message::<T>(text.as_bytes(), Some(&text)) {
                    Ok(msg) => {
                        if tx.send(Ok(msg)).await.is_err() {
                            return;
                        }
                    }
                    Err(err) => {
                        tracing::debug!(
                            target: "chromiumoxide::conn::raw_ws::parse_errors",
                            "Dropping malformed text WS frame: {err}",
                        );
                    }
                }
            }
            Ok(WsMessage::Binary(buf)) => match decode_message::<T>(&buf, None) {
                Ok(msg) => {
                    if tx.send(Ok(msg)).await.is_err() {
                        return;
                    }
                }
                Err(err) => {
                    tracing::debug!(
                        target: "chromiumoxide::conn::raw_ws::parse_errors",
                        "Dropping malformed binary WS frame: {err}",
                    );
                }
            },
            Ok(WsMessage::Close(_)) => return,
            Ok(WsMessage::Ping(_)) | Ok(WsMessage::Pong(_)) => {}
            Ok(msg) => {
                tracing::debug!(
                    target: "chromiumoxide::conn::raw_ws::parse_errors",
                    "Unexpected WS message type: {:?}",
                    msg
                );
            }
            Err(err) => {
                // Forward the error once, then exit. The Handler will
                // observe it on the next `next_message()` call.
                let _ = tx.send(Err(CdpError::Ws(err))).await;
                return;
            }
        }
    }
}

/// Background task that batches and flushes outgoing CDP commands.
async fn ws_write_loop(
    mut sink: SplitSink<WebSocketStream<ConnectStream>, WsMessage>,
    mut rx: mpsc::Receiver<MethodCall>,
) -> Result<()> {
    while let Some(call) = rx.recv().await {
        let msg = crate::serde_json::to_string(&call)?;
        sink.feed(WsMessage::Text(msg.into()))
            .await
            .map_err(CdpError::Ws)?;

        // Batch: drain all buffered commands without waiting.
        while let Ok(call) = rx.try_recv() {
            let msg = crate::serde_json::to_string(&call)?;
            sink.feed(WsMessage::Text(msg.into()))
                .await
                .map_err(CdpError::Ws)?;
        }

        // Flush the entire batch in one write.
        sink.flush().await.map_err(CdpError::Ws)?;
    }
    Ok(())
}

/// Handler-facing read half of the split WebSocket connection.
///
/// Decoded CDP messages are produced by a dedicated background task
/// (see [`ws_read_loop`]) and forwarded over a bounded mpsc. `WsReader`
/// itself is a thin `Receiver` wrapper — calling `next_message()` does
/// a single `rx.recv().await` with no per-message decoding work on the
/// caller's task. This keeps the Handler's poll loop free of CPU-bound
/// deserialize time, which matters for large (multi-MB) CDP responses
/// such as screenshots and wide-header network events.
#[derive(Debug)]
pub struct WsReader<T: EventMessage> {
    rx: mpsc::Receiver<Result<Box<Message<T>>>>,
    _marker: PhantomData<T>,
}

impl<T: EventMessage + Unpin> WsReader<T> {
    /// Read the next CDP message from the WebSocket.
    ///
    /// Returns `None` when the background reader task has exited
    /// (connection closed or sender dropped). This call does only a
    /// channel `recv` — the actual WS read + JSON decode happens on
    /// the background `ws_read_loop` task.
    pub async fn next_message(&mut self) -> Option<Result<Box<Message<T>>>> {
        self.rx.recv().await
    }
}

impl<T: EventMessage + Unpin> Stream for Connection<T> {
    type Item = Result<Box<Message<T>>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let pin = self.get_mut();

        // Send and flush outgoing messages
        if let Err(err) = pin.start_send_next(cx) {
            return Poll::Ready(Some(Err(err)));
        }

        // Read from the websocket, skipping non-data frames (pings,
        // pongs, malformed messages) without yielding back to the
        // executor.  This avoids a full round-trip per skipped frame.
        //
        // Cap consecutive skips so a flood of non-data frames (many
        // pings, malformed/unexpected types) cannot starve the
        // runtime — yield Pending after `MAX_SKIPS_PER_POLL` and
        // self-wake so we resume on the next tick.
        const MAX_SKIPS_PER_POLL: u32 = 16;
        let mut skips: u32 = 0;
        loop {
            match ready!(pin.ws.poll_next_unpin(cx)) {
                Some(Ok(WsMessage::Text(text))) => {
                    match decode_message::<T>(text.as_bytes(), Some(&text)) {
                        Ok(msg) => return Poll::Ready(Some(Ok(msg))),
                        Err(err) => {
                            tracing::debug!(
                                target: "chromiumoxide::conn::raw_ws::parse_errors",
                                "Dropping malformed text WS frame: {err}",
                            );
                            skips += 1;
                        }
                    }
                }
                Some(Ok(WsMessage::Binary(buf))) => match decode_message::<T>(&buf, None) {
                    Ok(msg) => return Poll::Ready(Some(Ok(msg))),
                    Err(err) => {
                        tracing::debug!(
                            target: "chromiumoxide::conn::raw_ws::parse_errors",
                            "Dropping malformed binary WS frame: {err}",
                        );
                        skips += 1;
                    }
                },
                Some(Ok(WsMessage::Close(_))) => return Poll::Ready(None),
                Some(Ok(WsMessage::Ping(_))) | Some(Ok(WsMessage::Pong(_))) => {
                    skips += 1;
                }
                Some(Ok(msg)) => {
                    tracing::debug!(
                        target: "chromiumoxide::conn::raw_ws::parse_errors",
                        "Unexpected WS message type: {:?}",
                        msg
                    );
                    skips += 1;
                }
                Some(Err(err)) => return Poll::Ready(Some(Err(CdpError::Ws(err)))),
                None => return Poll::Ready(None),
            }

            if skips >= MAX_SKIPS_PER_POLL {
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
        }
    }
}

/// Shared decode path for both text and binary WS frames.
/// `raw_text_for_logging` is only provided for textual frames so we can log the original
/// payload on parse failure if desired.
#[cfg(not(feature = "serde_stacker"))]
fn decode_message<T: EventMessage>(
    bytes: &[u8],
    raw_text_for_logging: Option<&str>,
) -> Result<Box<Message<T>>> {
    match serde_json::from_slice::<Box<Message<T>>>(bytes) {
        Ok(msg) => {
            tracing::trace!("Received {:?}", msg);
            Ok(msg)
        }
        Err(err) => {
            if let Some(txt) = raw_text_for_logging {
                let preview = &txt[..txt.len().min(512)];
                tracing::debug!(
                    target: "chromiumoxide::conn::raw_ws::parse_errors",
                    msg_len = txt.len(),
                    "Skipping unrecognized WS message {err} preview={preview}",
                );
            } else {
                tracing::debug!(
                    target: "chromiumoxide::conn::raw_ws::parse_errors",
                    "Skipping unrecognized binary WS message {err}",
                );
            }
            Err(err.into())
        }
    }
}

/// Shared decode path for both text and binary WS frames.
/// `raw_text_for_logging` is only provided for textual frames so we can log the original
/// payload on parse failure if desired.
#[cfg(feature = "serde_stacker")]
fn decode_message<T: EventMessage>(
    bytes: &[u8],
    raw_text_for_logging: Option<&str>,
) -> Result<Box<Message<T>>> {
    use serde::Deserialize;
    let mut de = serde_json::Deserializer::from_slice(bytes);

    de.disable_recursion_limit();

    let de = serde_stacker::Deserializer::new(&mut de);

    match Box::<Message<T>>::deserialize(de) {
        Ok(msg) => {
            tracing::trace!("Received {:?}", msg);
            Ok(msg)
        }
        Err(err) => {
            if let Some(txt) = raw_text_for_logging {
                let preview = &txt[..txt.len().min(512)];
                tracing::debug!(
                    target: "chromiumoxide::conn::raw_ws::parse_errors",
                    msg_len = txt.len(),
                    "Skipping unrecognized WS message {err} preview={preview}",
                );
            } else {
                tracing::debug!(
                    target: "chromiumoxide::conn::raw_ws::parse_errors",
                    "Skipping unrecognized binary WS message {err}",
                );
            }
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod ws_read_loop_tests {
    //! Unit tests for the `ws_read_loop` background reader task.
    //!
    //! These tests feed a synthetic `Stream<Item = Result<WsMessage, _>>`
    //! into `ws_read_loop` — no real WebSocket, no Chrome — and observe
    //! what comes out the other side of the mpsc channel.
    //!
    //! The properties under test are the ones that make the reader-task
    //! decoupling safe: FIFO ordering, no-deadlock on a bounded channel
    //! under back-pressure, silent drop of non-data frames, graceful
    //! transport-error propagation, and clean exit on `Close`.
    //!
    //! The typed events are `chromiumoxide_cdp::cdp::CdpEventMessage` —
    //! the same instantiation the real Handler uses — so these tests
    //! exercise the actual decode path (`serde_json::from_slice`), not
    //! a simplified fake.
    use super::*;
    use chromiumoxide_cdp::cdp::CdpEventMessage;
    use chromiumoxide_types::CallId;
    use futures_util::stream;
    use tokio::sync::mpsc;
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    /// Build a CDP `Response` WS frame as text — the smallest valid CDP
    /// message. `id` tags the frame for ordering assertions.
    fn response_frame(id: u64) -> WsMessage {
        WsMessage::Text(
            format!(r#"{{"id":{id},"result":{{"ok":true}}}}"#)
                .to_string()
                .into(),
        )
    }

    /// Build a frame far larger than a typical socket chunk, to exercise
    /// the "large message" path that motivated this refactor. The blob
    /// field pushes serde_json through a big allocation even though the
    /// envelope is tiny.
    fn large_response_frame(id: u64, blob_bytes: usize) -> WsMessage {
        let blob = "x".repeat(blob_bytes);
        WsMessage::Text(
            format!(r#"{{"id":{id},"result":{{"blob":"{blob}"}}}}"#)
                .to_string()
                .into(),
        )
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn forwards_messages_in_stream_order() {
        let frames = vec![
            Ok(response_frame(1)),
            Ok(response_frame(2)),
            Ok(response_frame(3)),
        ];
        let stream = stream::iter(frames);
        let (tx, mut rx) = mpsc::channel::<Result<Box<Message<CdpEventMessage>>>>(8);
        let task = tokio::spawn(ws_read_loop::<CdpEventMessage, _>(stream, tx));

        for expected in [1u64, 2, 3] {
            let msg = rx.recv().await.expect("msg").expect("decode ok");
            if let Message::Response(resp) = *msg {
                assert_eq!(resp.id, CallId::new(expected as usize));
            } else {
                panic!("expected Response");
            }
        }
        assert!(rx.recv().await.is_none(), "channel must close on EOF");
        task.await.expect("reader task join");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pings_and_pongs_never_reach_the_handler() {
        let frames = vec![
            Ok(WsMessage::Ping(vec![1, 2, 3].into())),
            Ok(response_frame(7)),
            Ok(WsMessage::Pong(vec![].into())),
            Ok(response_frame(8)),
        ];
        let stream = stream::iter(frames);
        let (tx, mut rx) = mpsc::channel::<Result<Box<Message<CdpEventMessage>>>>(8);
        let task = tokio::spawn(ws_read_loop::<CdpEventMessage, _>(stream, tx));

        for expected in [7u64, 8] {
            let msg = rx.recv().await.expect("msg").expect("decode ok");
            if let Message::Response(resp) = *msg {
                assert_eq!(resp.id, CallId::new(expected as usize));
            }
        }
        assert!(rx.recv().await.is_none());
        task.await.expect("reader task join");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn malformed_frames_do_not_block_subsequent_valid_frames() {
        let frames = vec![
            Ok(WsMessage::Text("{not valid json".to_string().into())),
            Ok(response_frame(42)),
        ];
        let stream = stream::iter(frames);
        let (tx, mut rx) = mpsc::channel::<Result<Box<Message<CdpEventMessage>>>>(8);
        let task = tokio::spawn(ws_read_loop::<CdpEventMessage, _>(stream, tx));

        let msg = rx.recv().await.expect("msg").expect("decode ok");
        if let Message::Response(resp) = *msg {
            assert_eq!(resp.id, CallId::new(42));
        }
        assert!(rx.recv().await.is_none());
        task.await.expect("reader task join");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn close_frame_terminates_the_reader() {
        let frames = vec![
            Ok(response_frame(1)),
            Ok(WsMessage::Close(None)),
            Ok(response_frame(2)), // unreachable after Close
        ];
        let stream = stream::iter(frames);
        let (tx, mut rx) = mpsc::channel::<Result<Box<Message<CdpEventMessage>>>>(8);
        let task = tokio::spawn(ws_read_loop::<CdpEventMessage, _>(stream, tx));

        let msg = rx.recv().await.expect("msg").expect("decode ok");
        if let Message::Response(resp) = *msg {
            assert_eq!(resp.id, CallId::new(1));
        }
        assert!(
            rx.recv().await.is_none(),
            "reader must exit on Close; frames after Close must not appear"
        );
        task.await.expect("reader task join");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn transport_error_is_forwarded_once_then_reader_exits() {
        let frames = vec![
            Ok(response_frame(1)),
            Err(tokio_tungstenite::tungstenite::Error::ConnectionClosed),
            Ok(response_frame(2)),
        ];
        let stream = stream::iter(frames);
        let (tx, mut rx) = mpsc::channel::<Result<Box<Message<CdpEventMessage>>>>(8);
        let task = tokio::spawn(ws_read_loop::<CdpEventMessage, _>(stream, tx));

        let msg = rx.recv().await.expect("msg").expect("ok");
        assert!(matches!(*msg, Message::Response(_)));
        match rx.recv().await {
            Some(Err(CdpError::Ws(_))) => {}
            other => panic!("expected forwarded Ws error, got {other:?}"),
        }
        assert!(rx.recv().await.is_none());
        task.await.expect("reader task join");
    }

    /// Back-pressure property: with the smallest possible channel and
    /// many frames, the reader task awaits capacity after each send and
    /// never deadlocks. This is the core "no deadlock" proof for the
    /// new design — if the reader held anything across its `.await` that
    /// the consumer needed, the consumer's `recv().await` would block
    /// forever. Completion under a 5s watchdog proves it doesn't.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bounded_channel_does_not_deadlock_under_backpressure() {
        const N: u64 = 512;
        let frames: Vec<_> = (1..=N).map(|id| Ok(response_frame(id))).collect();
        let stream = stream::iter(frames);

        let (tx, mut rx) = mpsc::channel::<Result<Box<Message<CdpEventMessage>>>>(1);
        let task = tokio::spawn(ws_read_loop::<CdpEventMessage, _>(stream, tx));

        let deadline = std::time::Duration::from_secs(5);
        let collected = tokio::time::timeout(deadline, async {
            let mut seen = 0u64;
            while let Some(frame) = rx.recv().await {
                let msg = frame.expect("decode ok");
                if let Message::Response(resp) = *msg {
                    seen += 1;
                    assert_eq!(
                        resp.id,
                        CallId::new(seen as usize),
                        "back-pressure must preserve FIFO order"
                    );
                }
            }
            seen
        })
        .await
        .expect("reader must make forward progress despite cap-1 back-pressure");

        assert_eq!(collected, N, "all frames must arrive");
        task.await.expect("reader task join");
    }

    /// Large message (>1 MB) is decoded correctly on the background
    /// task. This is the specific scenario the reader-task refactor
    /// was built for — we don't measure time here (benches cover that),
    /// we just prove the end-to-end path works without corruption or
    /// deadlock.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn large_message_decodes_without_corruption() {
        let big = 2 * 1024 * 1024; // 2 MB payload
        let frames = vec![
            Ok(large_response_frame(100, big)),
            Ok(response_frame(101)),
        ];
        let stream = stream::iter(frames);
        let (tx, mut rx) = mpsc::channel::<Result<Box<Message<CdpEventMessage>>>>(4);
        let task = tokio::spawn(ws_read_loop::<CdpEventMessage, _>(stream, tx));

        let first = rx.recv().await.expect("msg").expect("ok");
        if let Message::Response(resp) = *first {
            assert_eq!(resp.id, CallId::new(100));
        }
        let second = rx.recv().await.expect("msg").expect("ok");
        if let Message::Response(resp) = *second {
            assert_eq!(resp.id, CallId::new(101));
        }
        assert!(rx.recv().await.is_none());
        task.await.expect("reader task join");
    }
}
