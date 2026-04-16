use base64::engine::general_purpose;
use base64::prelude::Engine as _;
use hashbrown::HashMap;
use http_cache_reqwest::CacheManager;
use http_cache_semantics::CachePolicy;
use http_global_cache::CACACHE_MANAGER;
use lazy_static::lazy_static;
use reqwest::Method;
use reqwest::StatusCode;
use url::Url;

use crate::cache::manager::site_key_for_target_url;
use crate::http::{convert_headers, HttpRequestLike, HttpResponseLike};

// Re-export from the shared crate so existing call sites keep working.
pub use spider_remote_cache::{
    build_payload, dump_batch_to_remote as dump_batch_to_remote_cache,
    dump_to_remote as dump_to_remote_cache_parts, get_client, get_endpoint,
    resolve_base_url as resolve_remote_base_url, set_client, set_endpoint, HybridCachePayload,
};

lazy_static! {
    /// The local session cache per run cleared.
    pub static ref LOCAL_SESSION_CACHE: dashmap::DashMap<String, HashMap<String, (http_cache_reqwest::HttpResponse, CachePolicy)>> = dashmap::DashMap::new();
    /// URLs currently being streamed via `Fetch.takeResponseBodyAsStream`.
    /// Checked by the `Network.responseReceived` listener to avoid a
    /// redundant `getResponseBody` call for the same resource.
    pub(crate) static ref PENDING_STREAM_URLS: dashmap::DashSet<String> = dashmap::DashSet::new();
}

/// Convert a `spider_remote_cache::HttpVersion` to an `http_cache::HttpVersion`.
fn remote_version_to_http_cache(v: spider_remote_cache::HttpVersion) -> http_cache::HttpVersion {
    match v {
        spider_remote_cache::HttpVersion::H2 => http_cache::HttpVersion::H2,
        spider_remote_cache::HttpVersion::H3 => http_cache::HttpVersion::H3,
        spider_remote_cache::HttpVersion::Http09 => http_cache::HttpVersion::Http09,
        spider_remote_cache::HttpVersion::Http10 => http_cache::HttpVersion::Http10,
        spider_remote_cache::HttpVersion::Http11 | _ => http_cache::HttpVersion::Http11,
    }
}

/// Best-effort dump of a cached response into the remote hybrid cache server [experimental]
pub async fn dump_to_remote_cache(
    cache_key: &str,
    cache_site: &str,
    http_response: &crate::http::HttpResponse,
    method: &str,
    http_request_headers: &std::collections::HashMap<String, String>,
    dump_remote: Option<&str>,
) {
    // Convert chromey HttpVersion to spider_remote_cache HttpVersion.
    let version = match http_response.version {
        crate::http::HttpVersion::Http09 => spider_remote_cache::HttpVersion::Http09,
        crate::http::HttpVersion::Http10 => spider_remote_cache::HttpVersion::Http10,
        crate::http::HttpVersion::H2 => spider_remote_cache::HttpVersion::H2,
        crate::http::HttpVersion::H3 => spider_remote_cache::HttpVersion::H3,
        _ => spider_remote_cache::HttpVersion::Http11,
    };

    dump_to_remote_cache_parts(
        cache_key,
        cache_site,
        http_response.url.as_str(),
        &http_response.body,
        method,
        http_response.status,
        http_request_headers,
        &http_response.headers,
        &version,
        dump_remote,
    )
    .await
}

/// Get the cache for a website from the remote cache server and seed
/// our local hybrid cache (CACACHE_MANAGER) with **all** entries [experimental].
pub async fn get_cache_site(
    target_url: &str,
    auth: Option<&str>,
    remote: Option<&str>,
    namespace: Option<&str>,
) {
    let base_url = spider_remote_cache::resolve_base_url(remote);

    let cache_key = site_key_for_target_url(target_url, auth, namespace);

    let endpoint = format!("{}/cache/site/{}", base_url, cache_key);

    let result = get_client().get(&endpoint).send().await;

    let resp = match result {
        Ok(resp) => resp,
        Err(err) => {
            tracing::warn!(
                "remote cache get: failed to GET {} from {}: {}",
                cache_key,
                endpoint,
                err
            );
            return;
        }
    };

    if !resp.status().is_success() {
        tracing::warn!(
            "remote cache get: non-success status for {}: {}",
            cache_key,
            resp.status()
        );
        return;
    }

    let payloads: Vec<Box<HybridCachePayload>> = match resp.json().await {
        Ok(p) => p,
        Err(err) => {
            tracing::warn!(
                "remote cache get: failed to parse JSON for {} from {}: {}",
                cache_key,
                endpoint,
                err
            );
            return;
        }
    };

    tracing::debug!(
        "remote cache get: seeding {} entries locally for website {}",
        payloads.len(),
        cache_key
    );

    for payload in payloads {
        if let Err(err) = seed_payload_into_local_cache(&cache_key, &payload, target_url).await {
            tracing::warn!(
                "remote cache get: failed to seed resource {} for website {}: {}",
                payload.resource_key,
                cache_key,
                err
            );
        }
    }
}

/// Get the cache for a resource from the remote cache server and seed
/// our local hybrid cache (CACACHE_MANAGER) with **all** entries [experimental].
pub async fn get_cache_resource(
    target_url: &str,
    auth: Option<&str>,
    remote: Option<&str>,
    namespace: Option<&str>,
) {
    let base_url = spider_remote_cache::resolve_base_url(remote);

    let cache_key = site_key_for_target_url(target_url, auth, namespace);

    let endpoint = format!("{}/cache/resource/{}", base_url, cache_key);

    let result = get_client().get(&endpoint).send().await;

    let resp = match result {
        Ok(resp) => resp,
        Err(err) => {
            tracing::warn!(
                "remote cache get: failed to GET {} from {}: {}",
                cache_key,
                endpoint,
                err
            );
            return;
        }
    };

    if !resp.status().is_success() {
        tracing::warn!(
            "remote cache get: non-success status for {}: {}",
            cache_key,
            resp.status()
        );
        return;
    }

    let payload: Box<HybridCachePayload> = match resp.json().await {
        Ok(p) => p,
        Err(err) => {
            tracing::warn!(
                "remote cache get: failed to parse JSON for {} from {}: {}",
                cache_key,
                endpoint,
                err
            );
            return;
        }
    };

    tracing::debug!(
        "remote cache get: seeding 1 entrie locally for website {}",
        cache_key
    );

    if let Err(err) = seed_payload_into_local_cache(&cache_key, &payload, target_url).await {
        tracing::warn!(
            "remote cache get: failed to seed resource {} for website {}: {}",
            payload.resource_key,
            cache_key,
            err
        );
    }
}

/// Remove item from local session cache.
pub async fn clear_local_session_cache(cache_key: &str) {
    LOCAL_SESSION_CACHE.remove(cache_key);
}

/// Maximum number of top-level site keys in the local session cache.
const SESSION_CACHE_MAX_SITES: usize = 2_000;

/// Maximum number of resource entries per site key.
const SESSION_CACHE_MAX_PER_SITE: usize = 10_000;

/// Insert the item into the dashmap
pub fn session_cache_insert(
    cache_key: &str,
    http_res: http_cache_reqwest::HttpResponse,
    cache_policy: CachePolicy,
    entry_key: &str,
) {
    // Fast path: key already exists — no eviction needed.
    if let Some(mut inner) = LOCAL_SESSION_CACHE.get_mut(cache_key) {
        if inner.len() < SESSION_CACHE_MAX_PER_SITE {
            inner.insert(entry_key.into(), (http_res, cache_policy));
        }
        return;
    }

    // Slow path: new site key — evict oldest entries *before* acquiring
    // the entry lock so we never call `.len()` / `.iter()` / `.remove()`
    // while a shard write-lock is held (which would deadlock DashMap).
    if LOCAL_SESSION_CACHE.len() >= SESSION_CACHE_MAX_SITES {
        let to_remove: Vec<String> = LOCAL_SESSION_CACHE
            .iter()
            .take(SESSION_CACHE_MAX_SITES / 4)
            .map(|r| r.key().clone())
            .collect();
        for key in to_remove {
            LOCAL_SESSION_CACHE.remove(&key);
        }
    }

    // Insert the new site key.  Another thread may have raced us, so use
    // `entry()` to handle the occupied case gracefully.
    use dashmap::mapref::entry::Entry;
    match LOCAL_SESSION_CACHE.entry(cache_key.to_string()) {
        Entry::Occupied(mut occ) => {
            let inner = occ.get_mut();
            if inner.len() < SESSION_CACHE_MAX_PER_SITE {
                inner.insert(entry_key.into(), (http_res, cache_policy));
            }
        }
        Entry::Vacant(vac) => {
            let mut m: HashMap<String, (http_cache_reqwest::HttpResponse, CachePolicy)> =
                HashMap::new();
            m.insert(entry_key.into(), (http_res, cache_policy));
            vac.insert(m);
        }
    }
}

/// Seed a single `HybridCachePayload` into the local HTTP cache (CACACHE_MANAGER).
async fn seed_payload_into_local_cache(
    cache_key: &str,
    payload: &HybridCachePayload,
    target_url: &str,
) -> Result<(), String> {
    if payload.body_base64.is_empty() {
        return Ok(());
    }

    let same_document = payload.url == target_url;

    let uri = payload
        .url
        .parse()
        .map_err(|e| format!("invalid URI for {}: {e}", payload.url))?;

    let body = general_purpose::STANDARD
        .decode(&payload.body_base64)
        .map_err(|e| format!("invalid base64 body for {}: {e}", payload.resource_key))?;

    let req = HttpRequestLike {
        uri,
        method: Method::from_bytes(payload.method.as_bytes()).unwrap_or(Method::GET),
        headers: convert_headers(&payload.request_headers),
    };

    let res = HttpResponseLike {
        status: StatusCode::from_u16(payload.status).unwrap_or(StatusCode::EXPECTATION_FAILED),
        headers: convert_headers(&payload.response_headers),
    };

    let policy = CachePolicy::new(&req, &res);

    let url =
        Url::parse(&payload.url).map_err(|e| format!("invalid Url for {}: {e}", payload.url))?;

    let http_res = http_cache_reqwest::HttpResponse {
        url,
        headers: http_cache::HttpHeaders::Modern(crate::http::headers_to_multi(
            &payload.response_headers,
        )),
        version: remote_version_to_http_cache(payload.http_version),
        status: payload.status,
        body,
        metadata: None,
    };

    let key = payload.resource_key.clone();
    let session_key = format!("{}:{}", payload.method, http_res.url);

    if same_document {
        let put_result = CACACHE_MANAGER
            .put(key.clone(), http_res.clone(), policy.clone())
            .await;
        if let Err(e) = put_result {
            return Err(format!("CACACHE_MANAGER.put failed for {}: {e}", key));
        }
    }

    session_cache_insert(cache_key, http_res, policy, &session_key);

    Ok(())
}

/// Get the resource from the cache.
pub fn get_session_cache_item(
    cache_key: &str,
    target_url: &str,
) -> Option<(http_cache_reqwest::HttpResponse, CachePolicy)> {
    LOCAL_SESSION_CACHE
        .get(cache_key)
        .and_then(|local_cache| local_cache.get(target_url).cloned())
}

/// Check the resource from the cache.
pub fn check_session_cache_item(cache_key: &str, target_url: &str) -> bool {
    LOCAL_SESSION_CACHE
        .get(cache_key)
        .map_or(false, |local_cache| local_cache.contains_key(target_url))
}

/// Mark a URL as "stream in-flight" so the Network listener skips it.
pub fn mark_stream_pending(key: &str) {
    PENDING_STREAM_URLS.insert(key.to_string());
}

/// Remove the in-flight marker (called on success *and* failure).
pub fn clear_stream_pending(key: &str) {
    PENDING_STREAM_URLS.remove(key);
}

/// Returns `true` when the URL is currently being body-streamed.
pub fn is_stream_pending(key: &str) -> bool {
    PENDING_STREAM_URLS.contains(key)
}
