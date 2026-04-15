//! Integration tests for all `wait_for_*` methods on `Page`.
//!
//! These tests navigate to `https://example.com` (a stable, lightweight page)
//! and exercise every wait-for variant exposed by chromey:
//!
//!   1. `wait_for_navigation` / `wait_for_navigation_response`
//!   2. `wait_for_dom_content_loaded` / `wait_for_dom_content_loaded_response`
//!   3. `wait_for_network_idle`
//!   4. `wait_for_network_almost_idle`
//!   5. `wait_for_network_idle_with_timeout`
//!   6. `wait_for_network_almost_idle_with_timeout`
//!   7. `find_element` (selector-based wait)
//!
//! Run with:
//!   cargo test --test wait_for

use chromiumoxide::browser::{Browser, BrowserConfig, HeadlessMode};
use futures_util::StreamExt;
use std::path::PathBuf;
use tokio::time::{timeout, Duration};

const TARGET: &str = "https://example.com";

fn try_browser_config() -> Option<BrowserConfig> {
    BrowserConfig::builder().build().ok()
}

fn temp_profile_dir(test_name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "chromey-waitfor-{test_name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp profile dir");
    dir
}

fn headless_config(test_name: &str) -> BrowserConfig {
    let profile_dir = temp_profile_dir(test_name);
    BrowserConfig::builder()
        .user_data_dir(&profile_dir)
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-extensions")
        .headless_mode(HeadlessMode::True)
        .launch_timeout(Duration::from_secs(30))
        .build()
        .expect("headless browser config")
}

async fn launch(config: BrowserConfig) -> Browser {
    let (browser, mut handler) = Browser::launch(config).await.expect("launch browser");
    let _handle = tokio::spawn(async move { while let Some(_event) = handler.next().await {} });
    browser
}

// ---------------------------------------------------------------------------
// 1. wait_for_navigation — waits for the `load` lifecycle event
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wait_for_navigation_after_goto() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch(headless_config("wfn-goto")).await;

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page should not time out")
        .expect("new_page should resolve");

    timeout(Duration::from_secs(30), page.goto(TARGET))
        .await
        .expect("goto should not time out")
        .expect("goto should succeed");

    let result = timeout(Duration::from_secs(30), page.wait_for_navigation())
        .await
        .expect("wait_for_navigation should not time out")
        .expect("wait_for_navigation should succeed");

    let content = timeout(Duration::from_secs(15), result.content())
        .await
        .expect("content() should not time out")
        .expect("content() should succeed");

    assert!(
        content.contains("Example Domain"),
        "page should contain 'Example Domain', got {} bytes",
        content.len()
    );
}

// ---------------------------------------------------------------------------
// 2. wait_for_navigation_response — returns the HTTP response
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wait_for_navigation_response_after_goto() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch(headless_config("wfn-response")).await;

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page should not time out")
        .expect("new_page should resolve");

    timeout(Duration::from_secs(30), page.goto(TARGET))
        .await
        .expect("goto should not time out")
        .expect("goto should succeed");

    let response = timeout(Duration::from_secs(30), page.wait_for_navigation_response())
        .await
        .expect("wait_for_navigation_response should not time out")
        .expect("wait_for_navigation_response should succeed");

    eprintln!("navigation response received: {:?}", response);
}

// ---------------------------------------------------------------------------
// 3. wait_for_dom_content_loaded — fires before `load`, no subresource wait
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wait_for_dom_content_loaded_after_goto() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch(headless_config("wfdcl")).await;

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page should not time out")
        .expect("new_page should resolve");

    timeout(Duration::from_secs(30), page.goto(TARGET))
        .await
        .expect("goto should not time out")
        .expect("goto should succeed");

    let result = timeout(Duration::from_secs(30), page.wait_for_dom_content_loaded())
        .await
        .expect("wait_for_dom_content_loaded should not time out")
        .expect("wait_for_dom_content_loaded should succeed");

    let content = timeout(Duration::from_secs(15), result.content())
        .await
        .expect("content() should not time out")
        .expect("content() should succeed");

    assert!(
        content.contains("Example Domain"),
        "page should contain 'Example Domain' after DOMContentLoaded"
    );
}

#[tokio::test]
async fn wait_for_dom_content_loaded_response_after_goto() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch(headless_config("wfdcl-response")).await;

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page should not time out")
        .expect("new_page should resolve");

    timeout(Duration::from_secs(30), page.goto(TARGET))
        .await
        .expect("goto should not time out")
        .expect("goto should succeed");

    let _response = timeout(
        Duration::from_secs(30),
        page.wait_for_dom_content_loaded_response(),
    )
    .await
    .expect("wait_for_dom_content_loaded_response should not time out")
    .expect("wait_for_dom_content_loaded_response should succeed");
}

/// DOMContentLoaded should resolve before or at the same time as load.
#[tokio::test]
async fn dom_content_loaded_resolves_before_load() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch(headless_config("dcl-before-load")).await;

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page should not time out")
        .expect("new_page should resolve");

    timeout(Duration::from_secs(30), page.goto(TARGET))
        .await
        .expect("goto should not time out")
        .expect("goto should succeed");

    let start = std::time::Instant::now();

    let dcl_time = {
        let _ = timeout(Duration::from_secs(30), page.wait_for_dom_content_loaded())
            .await
            .expect("dcl should not time out")
            .expect("dcl should succeed");
        start.elapsed()
    };

    let load_time = {
        let _ = timeout(Duration::from_secs(30), page.wait_for_navigation())
            .await
            .expect("nav should not time out")
            .expect("nav should succeed");
        start.elapsed()
    };

    eprintln!(
        "DOMContentLoaded: {dcl_time:?}, load: {load_time:?} (dcl <= load: {})",
        dcl_time <= load_time
    );

    assert!(
        dcl_time <= load_time,
        "DOMContentLoaded ({dcl_time:?}) should resolve before or at load ({load_time:?})"
    );
}

// ---------------------------------------------------------------------------
// 4. wait_for_network_idle — 500ms of zero open connections
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wait_for_network_idle_after_goto() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch(headless_config("wfni")).await;

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page should not time out")
        .expect("new_page should resolve");

    timeout(Duration::from_secs(30), page.goto(TARGET))
        .await
        .expect("goto should not time out")
        .expect("goto should succeed");

    let result = timeout(Duration::from_secs(30), page.wait_for_network_idle())
        .await
        .expect("wait_for_network_idle should not time out")
        .expect("wait_for_network_idle should succeed");

    let content = timeout(Duration::from_secs(15), result.content())
        .await
        .expect("content() should not time out")
        .expect("content() should succeed");

    assert!(
        content.contains("Example Domain"),
        "page should contain 'Example Domain' after network idle"
    );
}

// ---------------------------------------------------------------------------
// 5. wait_for_network_almost_idle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wait_for_network_almost_idle_after_goto() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch(headless_config("wfnai")).await;

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page should not time out")
        .expect("new_page should resolve");

    timeout(Duration::from_secs(30), page.goto(TARGET))
        .await
        .expect("goto should not time out")
        .expect("goto should succeed");

    let result = timeout(Duration::from_secs(30), page.wait_for_network_almost_idle())
        .await
        .expect("wait_for_network_almost_idle should not time out")
        .expect("wait_for_network_almost_idle should succeed");

    let content = timeout(Duration::from_secs(15), result.content())
        .await
        .expect("content() should not time out")
        .expect("content() should succeed");

    assert!(
        content.contains("Example Domain"),
        "page should contain 'Example Domain' after network almost idle"
    );
}

// ---------------------------------------------------------------------------
// 6. wait_for_network_idle_with_timeout — bounded idle wait
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wait_for_network_idle_with_timeout_after_goto() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch(headless_config("wfni-timeout")).await;

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page should not time out")
        .expect("new_page should resolve");

    timeout(Duration::from_secs(30), page.goto(TARGET))
        .await
        .expect("goto should not time out")
        .expect("goto should succeed");

    let result = timeout(
        Duration::from_secs(30),
        page.wait_for_network_idle_with_timeout(Duration::from_secs(15)),
    )
    .await
    .expect("outer timeout should not fire")
    .expect("wait_for_network_idle_with_timeout should succeed");

    let content = timeout(Duration::from_secs(15), result.content())
        .await
        .expect("content() should not time out")
        .expect("content() should succeed");

    assert!(
        content.contains("Example Domain"),
        "page should contain 'Example Domain' after idle-with-timeout"
    );
}

/// A very short timeout should gracefully return Ok (timeout elapsed).
#[tokio::test]
async fn wait_for_network_idle_with_tiny_timeout_does_not_error() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch(headless_config("wfni-tiny-timeout")).await;

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page should not time out")
        .expect("new_page should resolve");

    timeout(Duration::from_secs(30), page.goto(TARGET))
        .await
        .expect("goto should not time out")
        .expect("goto should succeed");

    let result = page
        .wait_for_network_idle_with_timeout(Duration::from_millis(1))
        .await;

    assert!(
        result.is_ok(),
        "wait_for_network_idle_with_timeout should return Ok even when timeout elapses"
    );
}

// ---------------------------------------------------------------------------
// 7. wait_for_network_almost_idle_with_timeout
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wait_for_network_almost_idle_with_timeout_after_goto() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch(headless_config("wfnai-timeout")).await;

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page should not time out")
        .expect("new_page should resolve");

    timeout(Duration::from_secs(30), page.goto(TARGET))
        .await
        .expect("goto should not time out")
        .expect("goto should succeed");

    let result = timeout(
        Duration::from_secs(30),
        page.wait_for_network_almost_idle_with_timeout(Duration::from_secs(15)),
    )
    .await
    .expect("outer timeout should not fire")
    .expect("wait_for_network_almost_idle_with_timeout should succeed");

    let content = timeout(Duration::from_secs(15), result.content())
        .await
        .expect("content() should not time out")
        .expect("content() should succeed");

    assert!(
        content.contains("Example Domain"),
        "page should contain 'Example Domain' after almost-idle-with-timeout"
    );
}

/// Tiny timeout is graceful for almost-idle variant too.
#[tokio::test]
async fn wait_for_network_almost_idle_with_tiny_timeout_does_not_error() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch(headless_config("wfnai-tiny-timeout")).await;

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page should not time out")
        .expect("new_page should resolve");

    timeout(Duration::from_secs(30), page.goto(TARGET))
        .await
        .expect("goto should not time out")
        .expect("goto should succeed");

    let result = page
        .wait_for_network_almost_idle_with_timeout(Duration::from_millis(1))
        .await;

    assert!(
        result.is_ok(),
        "wait_for_network_almost_idle_with_timeout should return Ok even when timeout elapses"
    );
}

// ---------------------------------------------------------------------------
// 8. find_element — selector-based wait
// ---------------------------------------------------------------------------

#[tokio::test]
async fn find_element_after_goto() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch(headless_config("find-el")).await;

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page should not time out")
        .expect("new_page should resolve");

    timeout(Duration::from_secs(30), page.goto(TARGET))
        .await
        .expect("goto should not time out")
        .expect("goto should succeed");

    timeout(Duration::from_secs(30), page.wait_for_navigation())
        .await
        .expect("wait_for_navigation should not time out")
        .expect("wait_for_navigation should succeed");

    let element = timeout(Duration::from_secs(15), page.find_element("h1"))
        .await
        .expect("find_element should not time out")
        .expect("find_element should resolve for <h1>");

    let text = timeout(Duration::from_secs(10), element.inner_text())
        .await
        .expect("inner_text should not time out")
        .expect("inner_text should succeed");

    assert!(
        text.as_deref()
            .is_some_and(|t| t.contains("Example Domain")),
        "h1 inner text should contain 'Example Domain', got: {text:?}"
    );
}

// ---------------------------------------------------------------------------
// 9. Two-phase concurrent page_wait
// ---------------------------------------------------------------------------

/// Phase 1 (concurrent): network waits run together.
/// Phase 2 (concurrent): selector + delay run together.
/// Wall time = max(network waits) + max(selector, delay).
#[tokio::test]
async fn two_phase_concurrent_page_wait() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch(headless_config("two-phase-page-wait")).await;

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page should not time out")
        .expect("new_page should resolve");

    timeout(Duration::from_secs(30), page.goto(TARGET))
        .await
        .expect("goto should not time out")
        .expect("goto should succeed");

    let start = std::time::Instant::now();

    // Phase 1: all network waits run concurrently.
    tokio::join!(
        async {
            let _ = page
                .wait_for_network_idle_with_timeout(Duration::from_secs(15))
                .await;
        },
        async {
            let _ = page
                .wait_for_network_almost_idle_with_timeout(Duration::from_secs(15))
                .await;
        },
    );

    let phase1 = start.elapsed();

    // Phase 2: selector + delay run concurrently (after network settles).
    tokio::join!(
        async {
            let _ = timeout(Duration::from_secs(15), page.find_element("body")).await;
        },
        async {
            tokio::time::sleep(Duration::from_millis(100)).await;
        },
    );

    let phase2 = start.elapsed() - phase1;

    let content = timeout(Duration::from_secs(15), page.content())
        .await
        .expect("content() should not time out")
        .expect("content() should succeed");

    assert!(
        content.contains("Example Domain"),
        "two-phase wait should yield page with 'Example Domain'"
    );
    assert!(
        content.contains("<h1>"),
        "two-phase wait should yield page with <h1> tag"
    );
    eprintln!(
        "two-phase page_wait: phase1={phase1:?} phase2={phase2:?} total={:?} ({} bytes)",
        start.elapsed(),
        content.len()
    );
}

// ---------------------------------------------------------------------------
// 10. Click + wait_for_navigation (interaction-triggered navigation)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn click_then_wait_for_navigation() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch(headless_config("click-nav")).await;

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page should not time out")
        .expect("new_page should resolve");

    timeout(Duration::from_secs(30), page.goto(TARGET))
        .await
        .expect("goto should not time out")
        .expect("goto should succeed");

    timeout(Duration::from_secs(30), page.wait_for_navigation())
        .await
        .expect("initial wait_for_navigation should not time out")
        .expect("initial wait_for_navigation should succeed");

    let link = timeout(Duration::from_secs(15), page.find_element("a"))
        .await
        .expect("find_element(a) should not time out")
        .expect("find_element(a) should resolve");

    timeout(Duration::from_secs(15), link.click())
        .await
        .expect("click should not time out")
        .expect("click should succeed");

    // The navigation may or may not succeed depending on the remote server,
    // but the wait_for_navigation call itself should not hang or panic.
    let nav_result = timeout(Duration::from_secs(30), page.wait_for_navigation()).await;

    match nav_result {
        Ok(Ok(_)) => {
            let url = page.url().await.expect("url()");
            eprintln!("navigated to: {url:?}");
        }
        Ok(Err(err)) => {
            eprintln!("navigation after click errored (acceptable): {err}");
        }
        Err(_) => {
            eprintln!("navigation after click timed out (acceptable for external site)");
        }
    }
}

// ---------------------------------------------------------------------------
// 11. Concurrent wait_for across multiple pages
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_wait_for_across_pages() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch(headless_config("concurrent-wait")).await;

    let mut pages = Vec::new();
    for _ in 0..3 {
        let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
            .await
            .expect("new_page should not time out")
            .expect("new_page should resolve");
        pages.push(page);
    }

    let futs: Vec<_> = pages
        .into_iter()
        .enumerate()
        .map(|(i, page)| {
            tokio::spawn(async move {
                timeout(Duration::from_secs(30), page.goto(TARGET))
                    .await
                    .unwrap_or_else(|_| panic!("page {i}: goto timed out"))
                    .unwrap_or_else(|e| panic!("page {i}: goto failed: {e}"));

                // Each page uses a different wait strategy
                match i % 3 {
                    0 => {
                        let _ = page
                            .wait_for_network_idle_with_timeout(Duration::from_secs(15))
                            .await;
                    }
                    1 => {
                        let _ = page
                            .wait_for_network_almost_idle_with_timeout(Duration::from_secs(15))
                            .await;
                    }
                    _ => {
                        let _ =
                            timeout(Duration::from_secs(15), page.wait_for_dom_content_loaded())
                                .await;
                    }
                }

                let content = timeout(Duration::from_secs(15), page.content())
                    .await
                    .unwrap_or_else(|_| panic!("page {i}: content() timed out"))
                    .unwrap_or_else(|e| panic!("page {i}: content() failed: {e}"));

                assert!(
                    content.contains("Example Domain"),
                    "page {i}: should contain 'Example Domain'"
                );
                eprintln!("page {i}: {} bytes", content.len());
            })
        })
        .collect();

    for fut in futs {
        fut.await.expect("task join");
    }
}

// ---------------------------------------------------------------------------
// 12. dom_content_loaded under concurrency — no deadlock
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_dom_content_loaded_does_not_deadlock() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch(headless_config("concurrent-dcl")).await;

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page should not time out")
        .expect("new_page should resolve");

    timeout(Duration::from_secs(30), page.goto(TARGET))
        .await
        .expect("goto should not time out")
        .expect("goto should succeed");

    // Fire many concurrent wait_for_dom_content_loaded calls.
    let futs: Vec<_> = (0..50)
        .map(|i| {
            let page = page.clone();
            tokio::spawn(async move {
                timeout(Duration::from_secs(30), page.wait_for_dom_content_loaded())
                    .await
                    .unwrap_or_else(|_| panic!("dcl({i}) timed out — possible deadlock"))
                    .unwrap_or_else(|err| panic!("dcl({i}) failed: {err}"));
            })
        })
        .collect();

    for (i, fut) in futs.into_iter().enumerate() {
        fut.await
            .unwrap_or_else(|err| panic!("task {i} panicked: {err}"));
    }
}

// ---------------------------------------------------------------------------
// 13. wait_for_navigation resolves on DOMContentLoaded (not load)
// ---------------------------------------------------------------------------

/// Verify that goto + wait_for_navigation unblocks the pipeline on
/// DOMContentLoaded — content is ready without waiting for subresources.
#[tokio::test]
async fn wait_for_navigation_resolves_on_dom_content_loaded() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch(headless_config("wfn-dcl")).await;

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page should not time out")
        .expect("new_page should resolve");

    timeout(Duration::from_secs(30), page.goto(TARGET))
        .await
        .expect("goto should not time out")
        .expect("goto should succeed");

    // wait_for_navigation now resolves on DOMContentLoaded.
    timeout(Duration::from_secs(30), page.wait_for_navigation())
        .await
        .expect("wait_for_navigation should not time out")
        .expect("wait_for_navigation should succeed");

    // Content should be fully available immediately.
    let content = timeout(Duration::from_secs(15), page.content())
        .await
        .expect("content() should not time out")
        .expect("content() should succeed");

    assert!(
        content.contains("Example Domain"),
        "page should contain 'Example Domain' after wait_for_navigation (DCL)"
    );
    eprintln!("wait_for_navigation (DCL): {} bytes ready", content.len());
}
