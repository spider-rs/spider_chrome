//! Streaming reader for `page.content()`-style HTML extraction.
//!
//! Unlike [`crate::cache::stream`], which drains a CDP IO stream handle from
//! `Fetch.takeResponseBodyAsStream`, there is no CDP handle for
//! `document.documentElement.outerHTML`.  Instead this module serialises the
//! HTML into a **wrapper object that lives only in V8** (no `window` pollution)
//! and pulls fixed-size slices over repeated `Runtime.callFunctionOn` calls
//! using the wrapper's `objectId`.
//!
//! Release of the V8 remote reference is handled by the shared batching
//! worker in [`crate::runtime_release`].  A tiny RAII guard on the Rust
//! side enqueues the `RemoteObjectId` into the worker's channel on drop —
//! wait-free, no per-call `tokio::spawn`, and still covers cancellation
//! and panic paths.  The worker fires releases in concurrent batches.
//!
//! When `_cache_stream_disk` is enabled, the Rust-side accumulator spills to
//! a temp file with transparent memory fallback on any I/O error, matching
//! the behaviour of the network-body sink in `cache::stream`.

use crate::error::{CdpError, Result};
use crate::page::Page;
use chromiumoxide_cdp::cdp::js_protocol::runtime::{
    CallArgument, CallFunctionOnParams, EvaluateParams, RemoteObjectId,
};
use futures_util::stream::{try_unfold, Stream};

/// Default UTF-16 code units requested per chunk (`65_536` = 64 Ki).
/// Chrome returns the slice as a native JSON string (UTF-8 over the wire),
/// which for BMP-heavy HTML lands at ~64–128 KiB per round-trip and up to
/// ~192 KiB worst case.
pub const DEFAULT_CHUNK_UNITS: u32 = 65_536;

/// Minimum UTF-16 code units a caller-supplied chunk size can be clamped
/// to (`1024` = 1 Ki).  Smaller than this and CDP round-trip overhead
/// starts dominating real payload transfer.
pub const MIN_CHUNK_UNITS: u32 = 1024;

/// Maximum UTF-16 code units a caller-supplied chunk size can be clamped
/// to (`4_194_304` = 4 Mi, worst-case ~12 MiB over the wire).  Caps
/// per-message memory on both ends.
pub const MAX_CHUNK_UNITS: u32 = 4_194_304;

/// Hard ceiling on total document size (UTF-16 code units).  Any page whose
/// HTML exceeds this is rejected immediately after the length probe rather
/// than streamed — a malicious or runaway page can't coerce the client
/// into a multi-gigabyte transfer.  Default: 256 Mi units ≈ 512 MiB–1 GiB
/// of UTF-8 on the wire.
pub const MAX_DOCUMENT_UNITS: u32 = 256 * 1024 * 1024;

/// Default hard ceiling on total accumulated bytes for the non-stream
/// accumulating API ([`content_bytes_streaming`]).  Can be overridden by
/// setting the `CHROMEY_CONTENT_STREAM_MAX_BYTES` env var.  Default:
/// 512 MiB.  The pump-style [`content_bytes_stream`] does not enforce
/// this cap since the caller decides how much to consume.
pub const DEFAULT_MAX_ACCUMULATED_BYTES: usize = 512 * 1024 * 1024;

/// Clamp a caller-supplied chunk size into the safe range.
#[inline]
fn clamp_chunk_units(units: u32) -> u32 {
    units.clamp(MIN_CHUNK_UNITS, MAX_CHUNK_UNITS)
}

/// Hard cap on chunk round-trips (guards against a mutating page or
/// pathological slice loop).  At [`DEFAULT_CHUNK_UNITS`] per chunk this
/// caps streaming at ~16 Gi code units — far beyond any realistic document.
const MAX_CHUNKS: usize = 262_144;

/// JS that builds the document HTML (matching
/// [`crate::javascript::extract::OUTER_HTML`]) and wraps it in an object so
/// the returned `RemoteObject` has a stable `objectId` we can hold onto.
/// Strings are *primitive* in V8 and primitive evaluate results do not carry
/// an `objectId`, hence the single-property wrapper.
const INIT_JS: &str = r###"(()=>{
  let rv='';
  if(document.doctype){rv+=new XMLSerializer().serializeToString(document.doctype);}
  if(document.documentElement){rv+=document.documentElement.outerHTML;}
  return {h:rv};
})()"###;

/// Function body for reading the length of the wrapped HTML (UTF-16 code
/// units).  Called once via `Runtime.callFunctionOn` against the wrapper.
const LEN_FN: &str = "function(){return this.h.length}";

/// Function body for slicing the wrapped HTML.  Adjusts `end` backwards if it
/// would split a UTF-16 surrogate pair, so every returned chunk is valid
/// UTF-16 (and therefore valid UTF-8 after JSON serialisation by Chrome).
const SLICE_FN: &str = r###"function(start,size){
  const s=this.h;
  const L=s.length;
  if(start>=L)return '';
  let end=start+size;
  if(end>L)end=L;
  if(end<L){
    const c=s.charCodeAt(end-1);
    if(c>=0xD800&&c<=0xDBFF)end-=1;
  }
  return s.slice(start,end);
}"###;

// ---------------------------------------------------------------------------
// Tiny RAII guard: enqueue release into the batching worker on drop
// ---------------------------------------------------------------------------

/// Holds the wrapper's `RemoteObjectId` and enqueues it with
/// [`crate::runtime_release::try_release`] when dropped.  Drop is
/// synchronous, wait-free, and panic-free — no `tokio::spawn`, no CDP call
/// issued here.
///
/// The id is stored directly (not in an `Option`) so `id()` is a simple
/// reference access with no runtime unwrap.  On `Drop` the id is swapped
/// out via `mem::take` (leaving a default empty id in its place); the
/// extracted id is enqueued only if non-empty, so double-drop or zero-init
/// paths are inert.
struct RemoteRefGuard {
    page: Page,
    object_id: RemoteObjectId,
}

impl RemoteRefGuard {
    #[inline]
    fn new(page: Page, object_id: RemoteObjectId) -> Self {
        Self { page, object_id }
    }

    #[inline]
    fn id(&self) -> &RemoteObjectId {
        &self.object_id
    }
}

impl Drop for RemoteRefGuard {
    fn drop(&mut self) {
        let id = std::mem::take(&mut self.object_id);
        if !id.0.is_empty() {
            crate::runtime_release::try_release(self.page.clone(), id);
        }
    }
}

// ---------------------------------------------------------------------------
// Chunk sink: disk with transparent memory fallback
// ---------------------------------------------------------------------------

#[cfg(feature = "_cache_stream_disk")]
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(feature = "_cache_stream_disk")]
static CONTENT_FILE_SEQ: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "_cache_stream_disk")]
enum Sink {
    Disk {
        file: tokio::fs::File,
        path: std::path::PathBuf,
    },
    Memory {
        buf: Vec<u8>,
    },
}

#[cfg(feature = "_cache_stream_disk")]
impl Sink {
    async fn open(cap_hint: usize) -> Self {
        match Self::try_open_disk().await {
            Ok(s) => s,
            Err(err) => {
                tracing::debug!("content stream disk init failed, using memory: {err}");
                Sink::Memory {
                    buf: Vec::with_capacity(cap_hint),
                }
            }
        }
    }

    async fn try_open_disk() -> std::io::Result<Self> {
        let tmp_dir = std::env::temp_dir();
        tokio::fs::create_dir_all(&tmp_dir).await?;
        let seq = CONTENT_FILE_SEQ.fetch_add(1, Ordering::Relaxed);
        let name = format!("chromey_content_{}_{}.tmp", std::process::id(), seq);
        let path = tmp_dir.join(name);
        let file = tokio::fs::File::create(&path).await?;
        Ok(Sink::Disk { file, path })
    }

    async fn write(&mut self, data: &[u8]) {
        match self {
            Sink::Disk { file, path } => {
                use tokio::io::AsyncWriteExt;
                if let Err(err) = file.write_all(data).await {
                    tracing::debug!(
                        "content stream disk write failed, falling back to memory: {err}"
                    );
                    let _ = file.flush().await;
                    let mut recovered = tokio::fs::read(path.as_path()).await.unwrap_or_default();
                    let _ = tokio::fs::remove_file(path.as_path()).await;
                    recovered.extend_from_slice(data);
                    *self = Sink::Memory { buf: recovered };
                }
            }
            Sink::Memory { buf } => buf.extend_from_slice(data),
        }
    }

    async fn finish(&mut self) -> Vec<u8> {
        match self {
            Sink::Disk { file, path } => {
                use tokio::io::AsyncWriteExt;
                let _ = file.flush().await;
                let p = path.clone();
                let body = tokio::fs::read(&p).await.unwrap_or_default();
                let _ = tokio::fs::remove_file(&p).await;
                *self = Sink::Memory { buf: Vec::new() };
                body
            }
            Sink::Memory { buf } => std::mem::take(buf),
        }
    }
}

#[cfg(feature = "_cache_stream_disk")]
impl Drop for Sink {
    fn drop(&mut self) {
        if let Sink::Disk { path, .. } = self {
            let p = path.clone();
            tokio::spawn(async move {
                let _ = tokio::fs::remove_file(&p).await;
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Threshold
// ---------------------------------------------------------------------------

/// UTF-16 code-unit count at or above which chunked streaming is worthwhile.
/// Below this size a single `callFunctionOn` slice round-trip already fits in
/// one CDP message comfortably.  Decays from 1 Mi → 256 Ki units as active
/// page count rises (mirroring the adaptive curve in `cache::stream`).
#[inline]
fn stream_threshold_units() -> u32 {
    const BASE: u32 = 1_048_576; // ~1 MiB of UTF-16 code units
    const MIN: u32 = 262_144; // ~256 Ki units
    const HIGH_PRESSURE_PAGES: u32 = 128;

    let pages = crate::handler::page::active_page_count() as u32;
    if pages >= HIGH_PRESSURE_PAGES {
        return MIN;
    }
    let range = BASE - MIN;
    let reduction = range * pages / HIGH_PRESSURE_PAGES;
    BASE - reduction
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Streams `document.documentElement.outerHTML` and returns it as UTF-8
/// bytes.  No global state is created on the page — the HTML is held only
/// via a transient V8 `RemoteObjectId` that the batching release worker
/// (see [`crate::runtime_release`]) reclaims after `content_streaming`
/// returns, via a tiny RAII guard.
///
/// # DoS guardrails
///
/// - A page whose HTML exceeds [`MAX_DOCUMENT_UNITS`] UTF-16 code units is
///   rejected immediately after the length probe (no streaming attempted).
/// - Accumulated bytes are capped at [`DEFAULT_MAX_ACCUMULATED_BYTES`]
///   (override with the `CHROMEY_CONTENT_STREAM_MAX_BYTES` env var); hitting
///   the cap returns an error rather than continuing to allocate.
/// - For unbounded consumption, use the pump-style [`content_bytes_stream`]
///   and apply your own per-consumer backpressure.
pub async fn content_bytes_streaming(page: &Page) -> Result<Vec<u8>> {
    let Some((guard, total_units, chunk_units)) = init_state(page).await? else {
        return Ok(Vec::new());
    };
    read_chunks(page, guard.id(), total_units, chunk_units).await
}

/// Pump-style async `Stream` over the page HTML — yields each chunk of
/// UTF-8 bytes as Chrome returns it, without accumulating into a `Vec`.
///
/// `chunk_units` is an optional cap on how many UTF-16 code units the page
/// slices per call:
///
/// - `None` uses [`DEFAULT_CHUNK_UNITS`] (`65_536` = 64 Ki, ~64–128 KiB
///   over the wire).
/// - `Some(n)` pins every slice call to at most `n` units, clamped into
///   `[`[`MIN_CHUNK_UNITS`]` (1024), `[`MAX_CHUNK_UNITS`]` (4_194_304)]`.
///
/// Smaller chunks yield more frequently (lower per-chunk latency); larger
/// chunks amortise CDP round-trip cost.
///
/// # DoS guardrails
///
/// Before streaming begins, a page whose HTML exceeds
/// [`MAX_DOCUMENT_UNITS`] UTF-16 code units is rejected — the length probe
/// returns an error and no slices are read.  The pump itself does **not**
/// enforce a total-bytes cap (that's the caller's responsibility when
/// consuming the stream); if you want one, use [`content_bytes_streaming`]
/// or stop polling once you've read enough.
///
/// Callers drive the stream with `StreamExt::next().await` (or feed it into
/// a writer, compressor, hash, etc.).  The returned stream owns a guard
/// over the V8 `RemoteObjectId`; dropping the stream early — cancellation,
/// break, `take()` — enqueues the release via
/// [`crate::runtime_release::try_release`] on the guard's `Drop`.
///
/// The first poll performs the init + length round-trips and then yields
/// the first chunk.  Subsequent polls each issue one `Runtime.callFunctionOn`
/// slice and yield its bytes.  An empty-byte yield, a length overrun, or
/// a CDP error terminates the stream.
pub fn content_bytes_stream(
    page: &Page,
    chunk_units: Option<u32>,
) -> impl Stream<Item = Result<Vec<u8>>> + Send + 'static {
    let page = page.clone();
    let override_units = chunk_units.map(clamp_chunk_units);
    try_unfold(
        PumpState::Init {
            page,
            override_units,
        },
        |state| async move {
            match state {
                PumpState::Init {
                    page,
                    override_units,
                } => match init_state(&page).await? {
                    None => Ok(None),
                    Some((guard, total, default_units)) => {
                        let chunk_units = override_units.unwrap_or(default_units);
                        pump_next(page, guard, total, 0, chunk_units, 0).await
                    }
                },
                PumpState::Pumping {
                    page,
                    guard,
                    total,
                    offset,
                    chunk_units,
                    rounds,
                } => pump_next(page, guard, total, offset, chunk_units, rounds).await,
            }
        },
    )
}

/// Convenience wrapper over [`content_bytes_stream`]: yield each chunk as
/// `String` rather than raw bytes.  Each chunk is individually validated as
/// UTF-8 (which it always is, since the page-side slice function avoids
/// splitting UTF-16 surrogate pairs).
pub fn content_stream(
    page: &Page,
    chunk_units: Option<u32>,
) -> impl Stream<Item = Result<String>> + Send + 'static {
    use futures_util::StreamExt;
    content_bytes_stream(page, chunk_units).map(|r| {
        r.and_then(|bytes| {
            String::from_utf8(bytes)
                .map_err(|e| CdpError::msg(format!("invalid UTF-8 in page content chunk: {e}")))
        })
    })
}

/// Internal state for the pump-stream unfold.  `Send + 'static` so the
/// stream is usable across `.await` points without borrowing the caller.
enum PumpState {
    /// Haven't run init yet.
    Init {
        page: Page,
        /// Caller-supplied chunk size override (already clamped).  `None`
        /// defers to the adaptive default from [`init_state`].
        override_units: Option<u32>,
    },
    /// Init done; reading chunks.
    Pumping {
        page: Page,
        guard: RemoteRefGuard,
        total: u32,
        offset: u32,
        chunk_units: u32,
        rounds: usize,
    },
}

/// Fetch one chunk and return the next stream state (or terminate).
async fn pump_next(
    page: Page,
    guard: RemoteRefGuard,
    total: u32,
    offset: u32,
    chunk_units: u32,
    rounds: usize,
) -> Result<Option<(Vec<u8>, PumpState)>> {
    if offset >= total || rounds >= MAX_CHUNKS {
        return Ok(None);
    }
    let bytes = read_slice(&page, guard.id(), offset, chunk_units).await?;
    if bytes.is_empty() {
        return Ok(None);
    }
    let advanced = utf16_len_of_utf8(&bytes);
    if advanced == 0 {
        return Ok(None);
    }
    let new_offset = offset.saturating_add(advanced);
    Ok(Some((
        bytes,
        PumpState::Pumping {
            page,
            guard,
            total,
            offset: new_offset,
            chunk_units,
            rounds: rounds + 1,
        },
    )))
}

/// Init the page-side wrapper and measure length.  Returns `None` if the
/// document has zero UTF-16 code units (empty page).
async fn init_state(page: &Page) -> Result<Option<(RemoteRefGuard, u32, u32)>> {
    // Ensure the batched release worker is running in this runtime.
    // Single `OnceLock` load on the hot path after first init — no await.
    crate::runtime_release::init_worker();

    let ctx = page.execution_context().await?;

    // Evaluate, returning a *reference* (returnByValue=false) so we get a
    // stable objectId for the wrapper.
    let mut init = EvaluateParams::new(INIT_JS);
    init.context_id = ctx;
    init.await_promise = Some(true);
    init.return_by_value = Some(false);

    let init_resp = page.execute(init).await?.result;
    if let Some(ex) = init_resp.exception_details {
        return Err(CdpError::JavascriptException(Box::new(ex)));
    }
    let object_id = init_resp
        .result
        .object_id
        .ok_or_else(|| CdpError::msg("content stream: init returned no objectId"))?;

    // Guard covers success, error, cancellation and panic paths — all
    // enqueue the release into the batching worker via a wait-free send.
    let guard = RemoteRefGuard::new(page.clone(), object_id);

    let len_params = CallFunctionOnParams::builder()
        .function_declaration(LEN_FN)
        .object_id(guard.id().clone())
        .return_by_value(true)
        .await_promise(false)
        .build()
        .map_err(CdpError::msg)?;
    let len_resp = page.execute(len_params).await?.result;
    if let Some(ex) = len_resp.exception_details {
        return Err(CdpError::JavascriptException(Box::new(ex)));
    }
    let total_units_u64 = len_resp
        .result
        .value
        .and_then(|v| v.as_u64())
        .ok_or_else(|| CdpError::msg("content stream: length was not a number"))?;

    // DoS guardrail: reject documents larger than MAX_DOCUMENT_UNITS
    // before we start streaming.  Also fails the u32 conversion for
    // anything > u32::MAX.
    if total_units_u64 > MAX_DOCUMENT_UNITS as u64 {
        return Err(CdpError::msg(format!(
            "content stream: document exceeds MAX_DOCUMENT_UNITS ({} > {})",
            total_units_u64, MAX_DOCUMENT_UNITS
        )));
    }
    let total_units: u32 = total_units_u64 as u32;

    if total_units == 0 {
        return Ok(None);
    }

    // Small docs: single-shot slice to avoid N-round-trip overhead.
    let chunk = if total_units < stream_threshold_units() {
        DEFAULT_CHUNK_UNITS.max(total_units)
    } else {
        DEFAULT_CHUNK_UNITS
    };

    Ok(Some((guard, total_units, chunk)))
}

/// Issue one `Runtime.callFunctionOn` slice call and return the bytes.
async fn read_slice(
    page: &Page,
    object_id: &RemoteObjectId,
    offset: u32,
    chunk_units: u32,
) -> Result<Vec<u8>> {
    let params = CallFunctionOnParams::builder()
        .function_declaration(SLICE_FN)
        .object_id(object_id.clone())
        .argument(
            CallArgument::builder()
                .value(serde_json::json!(offset))
                .build(),
        )
        .argument(
            CallArgument::builder()
                .value(serde_json::json!(chunk_units))
                .build(),
        )
        .return_by_value(true)
        .await_promise(false)
        .build()
        .map_err(CdpError::msg)?;

    let resp = page.execute(params).await?.result;
    if let Some(ex) = resp.exception_details {
        return Err(CdpError::JavascriptException(Box::new(ex)));
    }

    match resp.result.value {
        Some(serde_json::Value::String(s)) => Ok(s.into_bytes()),
        Some(serde_json::Value::Null) | None => Ok(Vec::new()),
        other => Err(CdpError::msg(format!(
            "content stream: unexpected slice value: {other:?}"
        ))),
    }
}

/// Same as [`content_bytes_streaming`] but validates/returns a `String`.
pub async fn content_streaming(page: &Page) -> Result<String> {
    let bytes = content_bytes_streaming(page).await?;
    String::from_utf8(bytes)
        .map_err(|e| CdpError::msg(format!("invalid UTF-8 in page content: {e}")))
}

async fn read_chunks(
    page: &Page,
    object_id: &RemoteObjectId,
    total_units: u32,
    chunk_units: u32,
) -> Result<Vec<u8>> {
    // Resolve the per-process byte cap once.  `CHROMEY_CONTENT_STREAM_MAX_BYTES`
    // lets operators override the default without rebuilding.
    let byte_cap = max_accumulated_bytes();

    // Best-effort capacity hint: assume 1.5× the UTF-16 unit count as bytes
    // (pessimistic for ASCII-heavy HTML, realistic for mixed content).
    // Clamp to min(8 MiB prealloc ceiling, byte_cap) so a hostile
    // `total_units` can't coerce a giant `Vec::with_capacity`.
    let cap_hint = (total_units as usize).saturating_mul(3) / 2;
    let cap_hint = cap_hint.min(8 * 1024 * 1024).min(byte_cap);

    #[cfg(feature = "_cache_stream_disk")]
    let mut sink = Sink::open(cap_hint).await;

    #[cfg(not(feature = "_cache_stream_disk"))]
    let mut buf: Vec<u8> = Vec::with_capacity(cap_hint);

    let mut offset: u32 = 0;
    let mut rounds: usize = 0;
    let mut total_bytes: usize = 0;

    while offset < total_units {
        if rounds >= MAX_CHUNKS {
            return Err(CdpError::msg("content stream exceeded MAX_CHUNKS"));
        }
        rounds += 1;

        let chunk_bytes = read_slice(page, object_id, offset, chunk_units).await?;
        if chunk_bytes.is_empty() {
            break;
        }

        let units_advanced = utf16_len_of_utf8(&chunk_bytes);
        if units_advanced == 0 {
            break;
        }

        // DoS guardrail: cap total accumulated bytes.
        total_bytes = total_bytes.saturating_add(chunk_bytes.len());
        if total_bytes > byte_cap {
            return Err(CdpError::msg(format!(
                "content stream: accumulated bytes exceeded cap ({} > {})",
                total_bytes, byte_cap
            )));
        }

        #[cfg(feature = "_cache_stream_disk")]
        sink.write(&chunk_bytes).await;

        #[cfg(not(feature = "_cache_stream_disk"))]
        buf.extend_from_slice(&chunk_bytes);

        offset = offset.saturating_add(units_advanced);
    }

    #[cfg(feature = "_cache_stream_disk")]
    {
        Ok(sink.finish().await)
    }

    #[cfg(not(feature = "_cache_stream_disk"))]
    {
        Ok(buf)
    }
}

/// Resolve the accumulated-bytes cap for this process.  Honours
/// `CHROMEY_CONTENT_STREAM_MAX_BYTES` (integer bytes) when set; otherwise
/// returns [`DEFAULT_MAX_ACCUMULATED_BYTES`].
#[inline]
fn max_accumulated_bytes() -> usize {
    std::env::var("CHROMEY_CONTENT_STREAM_MAX_BYTES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(DEFAULT_MAX_ACCUMULATED_BYTES)
}

/// Count the UTF-16 code units a valid UTF-8 slice decodes to.
///
/// - 1-byte sequence → 1 code unit (ASCII)
/// - 2-byte sequence (U+0080..U+07FF) → 1 code unit
/// - 3-byte sequence (U+0800..U+FFFF) → 1 code unit
/// - 4-byte sequence (U+10000..U+10FFFF) → 2 code units (surrogate pair)
///
/// We walk only the *lead* bytes of each UTF-8 sequence, which is branch-
/// prediction friendly and roughly memory-bandwidth bound.
#[inline]
fn utf16_len_of_utf8(bytes: &[u8]) -> u32 {
    let mut units: u32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        let (adv, u) = if b < 0x80 {
            (1, 1)
        } else if b < 0xC0 {
            // Continuation byte encountered out of sequence — input is not
            // valid UTF-8.  Skip one byte defensively.
            (1, 0)
        } else if b < 0xE0 {
            (2, 1)
        } else if b < 0xF0 {
            (3, 1)
        } else {
            (4, 2)
        };
        i += adv;
        units = units.saturating_add(u);
    }
    units
}
