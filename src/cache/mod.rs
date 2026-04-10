/// Dump remote cache.
pub mod dump_remote;
/// Cache manager.
pub mod manager;
/// Remote cache.
pub mod remote;
/// CDP IO-stream based response body reader.
pub mod stream;

pub use manager::{
    get_cached_url, put_hybrid_cache, rewrite_base_tag, spawn_fetch_cache_interceptor,
    spawn_response_cache_listener, BasicCachePolicy, CacheStrategy,
};
pub use stream::{inflight_stream_count, streaming_threshold_bytes};
