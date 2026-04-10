# chromey

[![Crates.io](https://img.shields.io/crates/v/chromey.svg)](https://crates.io/crates/chromey)
[![Documentation](https://docs.rs/chromey/badge.svg)](https://docs.rs/chromey)

Chrome DevTools Protocol library for Rust.

## Quick Start

```toml
chromey = "2"
```

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

Optional features enabled via Cargo feature flags:

| Flag | Description |
|---|---|
| `simd` | Fast CDP message parsing (default) |
| `adblock` | Network and cosmetic adblocking |
| `adblock_easylist` | Bundled EasyList filter lists |
| `firewall-default` / `firewall-rustls` | Request blocking by domain or pattern |
| `cache` / `cache_mem` | Disk or in-memory HTTP response caching |
| `_fetcher-native-tokio` / `_fetcher-rusttls-tokio` | Auto-download Chrome for Testing |
| `io_uring` | Linux io_uring I/O |
| `serde_stacker` | Deeply nested CDP payload parsing |

## Auto-Download Chrome

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

Requires `_fetcher-native-tokio` or `_fetcher-rusttls-tokio`.

## CDP Commands

Every CDP command is available through `Page::execute`. Most built-in methods are thin wrappers:

```rust
pub async fn pdf(&self, params: PrintToPdfParams) -> Result<Vec<u8>> {
    let res = self.execute(params).await?;
    Ok(base64::decode(&res.data)?)
}
```

Browse CDP types at [vanilla.aslushnikov.com](https://vanilla.aslushnikov.com/).

## License

Licensed under either of:

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT License](LICENSE-MIT)

## Acknowledgments

Originally forked from [chromiumoxide](https://github.com/mattsse/chromiumoxide).
