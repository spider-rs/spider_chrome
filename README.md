# chromey

[![Crates.io](https://img.shields.io/crates/v/chromey.svg)](https://crates.io/crates/chromey)
[![Documentation](https://docs.rs/chromey/badge.svg)](https://docs.rs/chromey)

A fast, concurrent Chrome DevTools Protocol (CDP) library for Rust.

Control headless or headed Chrome/Chromium with high concurrency, built-in adblocking, network firewalls, HTTP caching, browser fingerprinting, and always up-to-date CDP bindings.

## Quick Start

Add chromey to your `Cargo.toml`:

```toml
chromey = "2"
```

Navigate to a page and interact with it:

```rust
use chromiumoxide::browser::{Browser, BrowserConfig};
use futures_util::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (mut browser, mut handler) =
        Browser::launch(BrowserConfig::builder().with_head().build()?).await?;

    let handle = tokio::task::spawn(async move {
        while let Some(h) = handler.next().await {
            if h.is_err() {
                break;
            }
        }
    });

    let page = browser.new_page("https://en.wikipedia.org").await?;

    page.find_element("input#searchInput")
        .await?
        .click()
        .await?
        .type_str("Rust programming language")
        .await?
        .press_key("Enter")
        .await?;

    let html = page.wait_for_navigation().await?.content().await?;

    browser.close().await?;
    let _ = handle.await;
    Ok(())
}
```

## Features

All features are opt-in via Cargo feature flags (except `simd` and `default-tls` which are on by default).

| Feature | Flag | What it does |
|---|---|---|
| SIMD JSON | `simd` | Fast CDP message parsing via `sonic-rs` (default) |
| Adblocking | `adblock` | Built-in cosmetic + network adblocking engine |
| Adblock EasyList | `adblock_easylist` | Ships with bundled EasyList filter lists |
| Network firewall | `firewall-default` / `firewall-rustls` | Block requests by domain, pattern, or resource type |
| HTTP caching | `cache` / `cache_mem` | Disk or in-memory HTTP response caching |
| Browser fetcher | `_fetcher-native-tokio` / `_fetcher-rusttls-tokio` | Auto-download Chrome for Testing |
| Browser fingerprinting | (always on) | Realistic fingerprint emulation via `spider_fingerprint` |
| io_uring | `io_uring` | Linux io_uring support for I/O-heavy workloads |
| Deep JSON | `serde_stacker` | Parse deeply nested CDP payloads without stack overflow |

## Auto-Download Chrome

If you don't have Chrome installed, chromey can fetch it for you:

```rust
use std::path::Path;
use chromiumoxide::browser::BrowserConfig;
use chromiumoxide::fetcher::{BrowserFetcher, BrowserFetcherOptions};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let download_path = Path::new("./chrome");
    tokio::fs::create_dir_all(&download_path).await?;

    let fetcher = BrowserFetcher::new(
        BrowserFetcherOptions::builder()
            .with_path(&download_path)
            .build()?,
    );
    let info = fetcher.fetch().await?;

    let _config = BrowserConfig::builder()
        .chrome_executable(info.executable_path)
        .build()?;

    Ok(())
}
```

Enable with `_fetcher-native-tokio` or `_fetcher-rusttls-tokio`.

## Remote Caching

Enable remote HTTP caching with [hybrid_cache_server](https://github.com/spider-rs/hybrid_cache_server) by setting `HYBRID_CACHE_ENDPOINT`:

```sh
HYBRID_CACHE_ENDPOINT=http://remote-cache:8080
```

## Extending with CDP Commands

Every CDP command is available through `Page::execute`. Most built-in methods are thin wrappers around it:

```rust
pub async fn pdf(&self, params: PrintToPdfParams) -> Result<Vec<u8>> {
    let res = self.execute(params).await?;
    Ok(base64::decode(&res.data)?)
}
```

Browse all available CDP types at [vanilla.aslushnikov.com](https://vanilla.aslushnikov.com/).

## License

Licensed under either of:

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT License](LICENSE-MIT)
