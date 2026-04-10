//! CDP IO-stream based response body reader.
//!
//! Uses `Fetch.takeResponseBodyAsStream` + `IO.read` to pull response bodies
//! in fixed-size chunks instead of buffering the entire body in a single CDP
//! JSON message (`Network.getResponseBody`).  This lowers peak memory per
//! concurrent request and interleaves better with other CDP commands.
//!
//! When the `_cache_stream_disk` feature is enabled (default with `_cache`),
//! chunks are spilled to a temporary file so that only one chunk lives in
//! memory at a time.  The completed file is read back as a single `Vec<u8>`
//! for cache insertion.  When disabled, chunks accumulate in a `Vec<u8>`
//! directly.

use crate::page::Page;
use base64::{engine::general_purpose, Engine as _};
use chromiumoxide_cdp::cdp::browser_protocol::{
    fetch::TakeResponseBodyAsStreamParams,
    io::{CloseParams, ReadParams, StreamHandle},
};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Semaphore;

// ---------------------------------------------------------------------------
// Tunables
// ---------------------------------------------------------------------------

/// Default chunk size for `IO.read` (bytes requested per round-trip).
const DEFAULT_IO_READ_CHUNK: i64 = 65_536; // 64 KiB

/// Hard upper bound on a single streamed body.  Responses larger than this
/// are abandoned (stream closed) to avoid unbounded allocation.
const MAX_BODY_BYTES: usize = 50 * 1024 * 1024; // 50 MiB

/// Timeout for the *entire* stream read (all chunks), not per-chunk.
const STREAM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Base threshold (bytes) below which we skip streaming and let the
/// `Network.getResponseBody` path handle the response.  This value is
/// *lowered* dynamically as active page count rises.
const BASE_STREAMING_THRESHOLD: usize = 262_144; // 256 KiB

/// The floor the threshold can be reduced to under pressure.
const MIN_STREAMING_THRESHOLD: usize = 32_768; // 32 KiB

/// Page count at which the threshold reaches `MIN_STREAMING_THRESHOLD`.
const HIGH_PRESSURE_PAGES: usize = 64;

// ---------------------------------------------------------------------------
// Global concurrency controls
// ---------------------------------------------------------------------------

/// Per-process cap on concurrent in-flight `IO.read` streams.
/// Prevents CDP command queue saturation when many pages cache simultaneously.
static GLOBAL_STREAM_PERMITS: Semaphore = Semaphore::const_new(12);

/// Number of streams currently in-flight (for diagnostics / metrics).
static INFLIGHT_STREAMS: AtomicUsize = AtomicUsize::new(0);

/// Returns the number of body streams currently being read.
#[inline]
pub fn inflight_stream_count() -> usize {
    INFLIGHT_STREAMS.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Adaptive threshold
// ---------------------------------------------------------------------------

/// Returns the `Content-Length` threshold (in bytes) above which a response
/// should be streamed rather than fetched in one shot.
///
/// The value decreases linearly from [`BASE_STREAMING_THRESHOLD`] to
/// [`MIN_STREAMING_THRESHOLD`] as the active page count grows toward
/// [`HIGH_PRESSURE_PAGES`].  This means under heavy concurrency we stream
/// *more* responses (smaller bodies qualify) to keep peak memory per request
/// low.
#[inline]
pub fn streaming_threshold_bytes() -> usize {
    let pages = crate::handler::page::active_page_count();

    if pages >= HIGH_PRESSURE_PAGES {
        return MIN_STREAMING_THRESHOLD;
    }

    // Linear interpolation: threshold shrinks as pages increase.
    let range = BASE_STREAMING_THRESHOLD - MIN_STREAMING_THRESHOLD;
    let reduction = range * pages / HIGH_PRESSURE_PAGES;
    BASE_STREAMING_THRESHOLD - reduction
}

// ---------------------------------------------------------------------------
// Stream guard (RAII close)
// ---------------------------------------------------------------------------

/// RAII guard that ensures `IO.close` is sent even on early return / error.
struct StreamGuard<'a> {
    page: &'a Page,
    handle: Option<StreamHandle>,
}

impl<'a> StreamGuard<'a> {
    fn new(page: &'a Page, handle: StreamHandle) -> Self {
        Self {
            page,
            handle: Some(handle),
        }
    }

    /// Take ownership of the handle (called on success to close explicitly).
    fn take(&mut self) -> Option<StreamHandle> {
        self.handle.take()
    }
}

impl Drop for StreamGuard<'_> {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let page = self.page.clone();
            // Fire-and-forget: close on a background task so Drop is
            // non-blocking and cannot deadlock the current task.
            tokio::spawn(async move {
                let _ = page.execute(CloseParams { handle }).await;
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Disk-backed temp file guard
// ---------------------------------------------------------------------------

/// RAII guard that removes the temp file on drop (success or error).
#[cfg(feature = "_cache_stream_disk")]
struct TempFileGuard {
    path: std::path::PathBuf,
}

#[cfg(feature = "_cache_stream_disk")]
impl TempFileGuard {
    fn new(path: std::path::PathBuf) -> Self {
        Self { path }
    }

    /// Consume the guard, returning the path without deleting the file.
    /// The caller takes responsibility for cleanup.
    fn into_path(mut self) -> std::path::PathBuf {
        let p = std::mem::take(&mut self.path);
        // Prevent Drop from running removal on the now-empty path.
        std::mem::forget(self);
        p
    }
}

#[cfg(feature = "_cache_stream_disk")]
impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if !self.path.as_os_str().is_empty() {
            let path = self.path.clone();
            // Best-effort removal — don't block the current task.
            tokio::spawn(async move {
                let _ = tokio::fs::remove_file(&path).await;
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Stream reader — disk-backed path
// ---------------------------------------------------------------------------

/// Disk-backed: stream chunks to a temp file, then read back.
/// Peak memory = one chunk (~64 KiB) rather than the full body.
#[cfg(feature = "_cache_stream_disk")]
async fn read_chunks_to_disk(
    page: &Page,
    stream_handle: &StreamHandle,
    guard: &mut StreamGuard<'_>,
) -> Result<Vec<u8>, StreamError> {
    use tokio::io::AsyncWriteExt;

    // Create a unique temp file.
    let tmp_dir = std::env::temp_dir();
    let file_name = format!(
        "chromey_stream_{:x}_{:x}.tmp",
        std::process::id(),
        INFLIGHT_STREAMS.load(Ordering::Relaxed) as u64
            ^ std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64,
    );
    let tmp_path = tmp_dir.join(file_name);
    let file_guard = TempFileGuard::new(tmp_path.clone());

    let mut file = tokio::fs::File::create(&tmp_path)
        .await
        .map_err(StreamError::Io)?;

    let mut total_bytes: usize = 0;

    loop {
        let read_result = tokio::time::timeout(STREAM_TIMEOUT, async {
            page.execute(
                ReadParams::builder()
                    .handle(stream_handle.clone())
                    .size(DEFAULT_IO_READ_CHUNK)
                    .build()
                    .map_err(StreamError::Build)?,
            )
            .await
            .map_err(StreamError::Cdp)
        })
        .await
        .map_err(|_| StreamError::Timeout)??;

        let chunk = &read_result.result;

        let decoded = if chunk.base64_encoded.unwrap_or(false) {
            general_purpose::STANDARD
                .decode(&chunk.data)
                .map_err(StreamError::Base64)?
        } else {
            chunk.data.as_bytes().to_vec()
        };

        total_bytes += decoded.len();

        if total_bytes > MAX_BODY_BYTES {
            if let Some(h) = guard.take() {
                let _ = page.execute(CloseParams { handle: h }).await;
            }
            // file_guard drops here → removes the temp file.
            return Err(StreamError::BodyTooLarge(total_bytes));
        }

        file.write_all(&decoded).await.map_err(StreamError::Io)?;

        if chunk.eof {
            break;
        }
    }

    file.flush().await.map_err(StreamError::Io)?;
    drop(file);

    // Read the completed file back into memory.
    let body = tokio::fs::read(file_guard.into_path())
        .await
        .map_err(StreamError::Io)?;

    // Clean up the temp file now that we have the bytes.
    let _ = tokio::fs::remove_file(&tmp_path).await;

    Ok(body)
}

// ---------------------------------------------------------------------------
// Stream reader — memory-only path
// ---------------------------------------------------------------------------

/// Memory-only: accumulate chunks into a Vec<u8>.
#[cfg(not(feature = "_cache_stream_disk"))]
async fn read_chunks_to_memory(
    page: &Page,
    stream_handle: &StreamHandle,
    guard: &mut StreamGuard<'_>,
    content_length_hint: Option<usize>,
) -> Result<Vec<u8>, StreamError> {
    let alloc_hint = content_length_hint.unwrap_or(0).min(MAX_BODY_BYTES);
    let mut body = Vec::with_capacity(alloc_hint);

    loop {
        let read_result = tokio::time::timeout(STREAM_TIMEOUT, async {
            page.execute(
                ReadParams::builder()
                    .handle(stream_handle.clone())
                    .size(DEFAULT_IO_READ_CHUNK)
                    .build()
                    .map_err(StreamError::Build)?,
            )
            .await
            .map_err(StreamError::Cdp)
        })
        .await
        .map_err(|_| StreamError::Timeout)??;

        let chunk = &read_result.result;

        let decoded = if chunk.base64_encoded.unwrap_or(false) {
            general_purpose::STANDARD
                .decode(&chunk.data)
                .map_err(StreamError::Base64)?
        } else {
            chunk.data.as_bytes().to_vec()
        };

        if body.len() + decoded.len() > MAX_BODY_BYTES {
            if let Some(h) = guard.take() {
                let _ = page.execute(CloseParams { handle: h }).await;
            }
            return Err(StreamError::BodyTooLarge(body.len() + decoded.len()));
        }

        body.extend_from_slice(&decoded);

        if chunk.eof {
            break;
        }
    }

    Ok(body)
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Opens a CDP IO stream for the given paused Fetch request and reads the
/// body to completion in fixed-size chunks.
///
/// When the `_cache_stream_disk` feature is enabled (default), chunks are
/// spilled to a temporary file so peak memory per stream stays at ~64 KiB.
/// When disabled, chunks accumulate in a `Vec<u8>`.
///
/// # Concurrency
///
/// Acquires a permit from `GLOBAL_STREAM_PERMITS` before starting.  The
/// permit is released when the function returns (success or failure).
///
/// # Errors
///
/// Returns `Err` if:
/// - the semaphore is closed (should never happen),
/// - `takeResponseBodyAsStream` fails (request not paused at HeadersReceived),
/// - a timeout is exceeded,
/// - the body exceeds `MAX_BODY_BYTES`.
///
/// On *any* error the stream is closed via the `StreamGuard`.
pub async fn read_response_body_as_stream(
    page: &Page,
    request_id: impl Into<chromiumoxide_cdp::cdp::browser_protocol::fetch::RequestId>,
    content_length_hint: Option<usize>,
) -> Result<Vec<u8>, StreamError> {
    // Acquire global permit — blocks if we're at capacity.
    let _permit = GLOBAL_STREAM_PERMITS
        .acquire()
        .await
        .map_err(|_| StreamError::SemaphoreClosed)?;

    INFLIGHT_STREAMS.fetch_add(1, Ordering::Relaxed);
    let _dec = DecrementOnDrop(&INFLIGHT_STREAMS);

    // Open the stream.
    let returns = tokio::time::timeout(
        STREAM_TIMEOUT,
        page.execute(TakeResponseBodyAsStreamParams::new(request_id)),
    )
    .await
    .map_err(|_| StreamError::Timeout)?
    .map_err(StreamError::Cdp)?;

    let stream_handle = returns.result.stream;
    let mut guard = StreamGuard::new(page, stream_handle.clone());

    // Read chunks via the appropriate backend.
    #[cfg(feature = "_cache_stream_disk")]
    let body = {
        let _ = content_length_hint; // used only by the memory path
        read_chunks_to_disk(page, &stream_handle, &mut guard).await?
    };

    #[cfg(not(feature = "_cache_stream_disk"))]
    let body = read_chunks_to_memory(page, &stream_handle, &mut guard, content_length_hint).await?;

    // Explicitly close the stream (guard becomes a no-op).
    if let Some(h) = guard.take() {
        let _ = page.execute(CloseParams { handle: h }).await;
    }

    Ok(body)
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors specific to the streaming body reader.
#[derive(Debug)]
pub enum StreamError {
    /// The global semaphore was closed (internal bug).
    SemaphoreClosed,
    /// A CDP command failed.
    Cdp(crate::error::CdpError),
    /// The overall stream read exceeded the time limit.
    Timeout,
    /// The response body exceeded `MAX_BODY_BYTES`.
    BodyTooLarge(usize),
    /// Base64 decoding failed on a chunk.
    Base64(base64::DecodeError),
    /// Builder error from CDP param construction.
    Build(String),
    /// File I/O error (disk-backed streaming).
    Io(std::io::Error),
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SemaphoreClosed => write!(f, "stream semaphore closed"),
            Self::Cdp(e) => write!(f, "CDP error during stream read: {e}"),
            Self::Timeout => write!(f, "stream read timed out"),
            Self::BodyTooLarge(n) => write!(f, "response body too large: {n} bytes"),
            Self::Base64(e) => write!(f, "base64 decode error: {e}"),
            Self::Build(e) => write!(f, "CDP param build error: {e}"),
            Self::Io(e) => write!(f, "stream I/O error: {e}"),
        }
    }
}

impl std::error::Error for StreamError {}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract `Content-Length` from CDP response headers.  Returns `None`
/// when missing or unparseable — the caller should treat that as
/// "unknown length".
pub fn content_length_from_headers(
    headers: &std::collections::HashMap<String, String>,
) -> Option<usize> {
    for (k, v) in headers {
        if k.eq_ignore_ascii_case("content-length") {
            return v.parse().ok();
        }
    }
    None
}

/// Tiny RAII helper: decrements an `AtomicUsize` on drop.
struct DecrementOnDrop<'a>(&'a AtomicUsize);

impl Drop for DecrementOnDrop<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}
