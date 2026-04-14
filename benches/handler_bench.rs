//! Benchmarks for the internal handler hot paths.
//!
//! These benchmarks measure the overhead of the chromey machinery itself
//! (channel dispatch, serialization, event fan-out) without requiring a
//! running Chrome instance.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
use std::time::{Duration, Instant};

use chromiumoxide::cmd::CommandMessage;
use chromiumoxide::handler::commandfuture::CommandFuture;
use chromiumoxide::handler::target::TargetMessage;
use chromiumoxide::listeners::{EventListenerRequest, EventListeners};

use chromiumoxide_cdp::cdp::browser_protocol::page::NavigateParams;
use chromiumoxide_cdp::cdp::browser_protocol::target::SessionId;

/// Create a no-op waker for synchronous polling in benchmarks.
fn noop_waker() -> Waker {
    fn noop(_: *const ()) {}
    fn clone(p: *const ()) -> RawWaker {
        RawWaker::new(p, &VTABLE)
    }
    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
    unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
}

/// Benchmark: CommandMessage creation (serialisation overhead).
fn bench_command_message_creation(c: &mut Criterion) {
    c.bench_function("CommandMessage::new (NavigateParams)", |b| {
        b.iter(|| {
            let cmd = NavigateParams::new("https://example.com");
            let (tx, _rx) =
                tokio::sync::oneshot::channel::<chromiumoxide::error::Result<chromiumoxide_types::Response>>();
            let msg = CommandMessage::new(cmd, tx).unwrap();
            black_box(msg);
        });
    });
}

/// Benchmark: CommandMessage::with_session (includes session id).
fn bench_command_message_with_session(c: &mut Criterion) {
    c.bench_function("CommandMessage::with_session (NavigateParams)", |b| {
        b.iter(|| {
            let cmd = NavigateParams::new("https://example.com");
            let (tx, _rx) =
                tokio::sync::oneshot::channel::<chromiumoxide::error::Result<chromiumoxide_types::Response>>();
            let session = Some(SessionId::from("session-1".to_string()));
            let msg = CommandMessage::with_session(cmd, tx, session).unwrap();
            black_box(msg);
        });
    });
}

/// Benchmark: try_send fast path on a page channel.
fn bench_try_send_fast_path(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    c.bench_function("try_send fast path (channel has capacity)", |b| {
        b.iter(|| {
            let (tx, _rx) = tokio::sync::mpsc::channel::<TargetMessage>(2048);
            let cmd = NavigateParams::new("https://example.com");
            let (otx, _orx) = tokio::sync::oneshot::channel();
            let msg =
                CommandMessage::with_session(cmd, otx, Some(SessionId::from("s1".to_string())))
                    .unwrap();
            let target_msg = TargetMessage::Command(msg);
            let result = tx.try_send(target_msg);
            let _ = black_box(result);
        });
    });

    c.bench_function("async send path (channel has capacity)", |b| {
        b.iter(|| {
            rt.block_on(async {
                let (tx, _rx) = tokio::sync::mpsc::channel::<TargetMessage>(2048);
                let cmd = NavigateParams::new("https://example.com");
                let (otx, _orx) = tokio::sync::oneshot::channel();
                let msg = CommandMessage::with_session(
                    cmd,
                    otx,
                    Some(SessionId::from("s1".to_string())),
                )
                .unwrap();
                let result = tx.send(TargetMessage::Command(msg)).await;
                let _ = black_box(result);
            });
        });
    });
}

/// Benchmark: CommandFuture creation (measures allocation overhead).
fn bench_command_future_creation(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    c.bench_function("CommandFuture::new (NavigateParams)", |b| {
        b.iter(|| {
            let _guard = rt.enter();
            let (tx, _rx) = tokio::sync::mpsc::channel::<TargetMessage>(2048);
            let cmd = NavigateParams::new("https://example.com");
            let session = Some(SessionId::from("session-1".to_string()));
            let fut = CommandFuture::<NavigateParams>::new(
                cmd,
                tx,
                session,
                Duration::from_secs(30),
            )
            .unwrap();
            black_box(fut);
        });
    });
}

/// Benchmark: EventListeners dispatch throughput.
fn bench_event_listeners_dispatch(c: &mut Criterion) {
    use chromiumoxide_cdp::cdp::browser_protocol::animation::EventAnimationCanceled;

    c.bench_function("EventListeners: dispatch to 10 listeners", |b| {
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);

        b.iter(|| {
            let mut listeners = EventListeners::default();

            // Register 10 listeners
            let mut receivers = Vec::new();
            for _ in 0..10 {
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                listeners
                    .add_listener(EventListenerRequest::new::<EventAnimationCanceled>(tx));
                receivers.push(rx);
            }

            // Dispatch 100 events
            for i in 0..100 {
                listeners.start_send(EventAnimationCanceled {
                    id: format!("anim-{i}"),
                });
            }

            // Flush
            listeners.poll(&mut cx);
            black_box(&listeners);
        });
    });

    c.bench_function("EventListeners: poll with disconnected listeners", |b| {
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);

        b.iter(|| {
            let mut listeners = EventListeners::default();

            // Register 50 listeners then drop all receivers
            for _ in 0..50 {
                let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
                listeners
                    .add_listener(EventListenerRequest::new::<EventAnimationCanceled>(tx));
                // _rx dropped here — listener is disconnected
            }

            // Dispatch events to disconnected listeners
            for i in 0..10 {
                listeners.start_send(EventAnimationCanceled {
                    id: format!("anim-{i}"),
                });
            }

            // Poll should clean up all disconnected listeners
            listeners.poll(&mut cx);
            black_box(&listeners);
        });
    });
}

/// Benchmark: CommandChain state machine polling.
fn bench_command_chain_polling(c: &mut Criterion) {
    use chromiumoxide::cmd::CommandChain;
    use chromiumoxide_types::MethodId;

    c.bench_function("CommandChain: poll 10 commands to completion", |b| {
        b.iter(|| {
            let cmds: Vec<(MethodId, serde_json::Value)> = (0..10)
                .map(|i| {
                    (
                        MethodId::from(format!("Method.{i}")),
                        serde_json::json!({"param": i}),
                    )
                })
                .collect();

            let mut chain = CommandChain::new(cmds, Duration::from_secs(30));
            let now = Instant::now();

            // Simulate polling each command and receiving a response
            for _i in 0..10 {
                match chain.poll(now) {
                    Poll::Ready(Some(Ok((method, _params)))) => {
                        chain.received_response(method.as_ref());
                    }
                    _ => panic!("expected command"),
                }
            }

            // Should be done
            assert!(matches!(chain.poll(now), Poll::Ready(None)));
            black_box(&chain);
        });
    });
}

/// Benchmark: Oneshot channel creation + response round-trip.
fn bench_oneshot_roundtrip(c: &mut Criterion) {
    c.bench_function("oneshot create + send + recv", |b| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        b.iter(|| {
            rt.block_on(async {
                let (tx, rx) = tokio::sync::oneshot::channel::<u64>();
                tx.send(42).unwrap();
                let val = rx.await.unwrap();
                black_box(val);
            });
        });
    });
}

criterion_group!(
    benches,
    bench_command_message_creation,
    bench_command_message_with_session,
    bench_try_send_fast_path,
    bench_command_future_creation,
    bench_event_listeners_dispatch,
    bench_command_chain_polling,
    bench_oneshot_roundtrip,
);
criterion_main!(benches);
