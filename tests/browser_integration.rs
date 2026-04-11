//! Integration tests that require a real Chrome/Chromium installation.
//!
//! These tests are skipped automatically when no browser executable is found.
//!
//! Run with:
//!   cargo test --test browser_integration

use chromiumoxide::browser::{Browser, BrowserConfig, HeadlessMode};
use futures_util::StreamExt;
use std::path::PathBuf;
use tokio::time::{timeout, Duration};

/// Returns `None` when no browser executable can be found on this machine.
fn try_browser_config() -> Option<BrowserConfig> {
    BrowserConfig::builder().build().ok()
}

fn temp_profile_dir(test_name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "chromey-{test_name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp profile dir");
    dir
}

fn browser_like_config(test_name: &str) -> BrowserConfig {
    let profile_dir = temp_profile_dir(test_name);
    BrowserConfig::builder()
        .user_data_dir(&profile_dir)
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-extensions")
        .headless_mode(HeadlessMode::True)
        .launch_timeout(Duration::from_secs(30))
        .build()
        .expect("browser-like browser config")
}

fn browser_like_headed_config(test_name: &str) -> BrowserConfig {
    let profile_dir = temp_profile_dir(test_name);
    BrowserConfig::builder()
        .user_data_dir(&profile_dir)
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-extensions")
        .with_head()
        .launch_timeout(Duration::from_secs(30))
        .build()
        .expect("browser-like headed browser config")
}

async fn launch_with_handler(config: BrowserConfig) -> Browser {
    let (browser, mut handler) = Browser::launch(config).await.expect("launch browser");
    let _handle = tokio::spawn(async move { while let Some(_event) = handler.next().await {} });
    browser
}

async fn open_about_blank_with_timeout(
    config: BrowserConfig,
    timeout_secs: u64,
) -> Result<(), String> {
    let browser = launch_with_handler(config).await;
    let page = timeout(
        Duration::from_secs(timeout_secs),
        browser.new_page("about:blank"),
    )
    .await
    .map_err(|_| "new_page(about:blank) timed out".to_string())?
    .map_err(|err| format!("new_page(about:blank) failed: {err}"))?;

    let url = page
        .url()
        .await
        .map_err(|err| format!("url() failed: {err}"))?;
    if url.as_deref() != Some("about:blank") {
        return Err(format!("unexpected URL: {url:?}"));
    }

    Ok(())
}

async fn retried_open_start_page(browser: &mut Browser) -> Result<chromiumoxide::Page, String> {
    let create_timeout = Duration::from_secs(30);

    for attempt in 1..=2 {
        eprintln!("[chromey test] Creating initial page (attempt {attempt}/2)");

        match timeout(create_timeout, browser.new_page("about:blank")).await {
            Ok(Ok(page)) => {
                eprintln!("[chromey test] Created initial page on attempt {attempt}");
                return Ok(page);
            }
            Ok(Err(err)) => {
                eprintln!(
                    "[chromey test] Failed to create initial page on attempt {attempt}: {err}"
                );
                if attempt == 2 {
                    return Err(format!("failed to create initial page: {err}"));
                }
            }
            Err(_) => {
                eprintln!(
                    "[chromey test] Timed out creating initial page after {}s on attempt {attempt}",
                    create_timeout.as_secs()
                );
                if attempt == 2 {
                    return Err(format!(
                        "timed out after {}s creating initial page (about:blank)",
                        create_timeout.as_secs()
                    ));
                }
            }
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    Err("unreachable: initial page retry loop exhausted".to_string())
}

/// Launch Chrome and open a new `about:blank` page.
///
/// This is the real-Chrome counterpart of the unit test
/// `handler::target::tests::about_blank_page_creation_should_resolve_after_get_frame_tree`.
/// It verifies that `new_page("about:blank")` resolves (i.e. the initiator
/// channel is completed) and that the page reports the correct URL.
#[tokio::test]
async fn about_blank_page_creation_resolves() {
    let Some(config) = try_browser_config() else {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    };

    let (browser, mut handler) = Browser::launch(config).await.expect("launch browser");

    let _handle = tokio::spawn(async move { while let Some(_event) = handler.next().await {} });

    let page = browser
        .new_page("about:blank")
        .await
        .expect("new_page(about:blank) should resolve");

    let url = page.url().await.expect("url()");
    assert_eq!(
        url.as_deref(),
        Some("about:blank"),
        "page URL should be about:blank"
    );
}

/// Launch Chrome with an explicit profile and browser flags similar to an
/// embedding application and ensure the initial `about:blank` page resolves.
#[tokio::test]
async fn browser_like_about_blank_page_creation_resolves() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch_with_handler(browser_like_config("browser-like-about-blank")).await;

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page(about:blank) should not time out")
        .expect("new_page(about:blank) should resolve");

    let url = page.url().await.expect("url()");
    assert_eq!(
        url.as_deref(),
        Some("about:blank"),
        "page URL should be about:blank"
    );
}

/// Exercise the startup-tab discovery path before creating a new page.
///
/// Touch discovery APIs before creating a new page to cover the startup path
/// where targets exist before the first page handle is requested.
#[tokio::test]
async fn browser_like_startup_discovery_then_new_page_resolves() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let mut browser = launch_with_handler(browser_like_config("browser-like-discovery")).await;

    let targets = timeout(Duration::from_secs(10), browser.fetch_targets())
        .await
        .expect("fetch_targets should not time out")
        .expect("fetch_targets should succeed");
    eprintln!("startup targets: {}", targets.len());

    let pages_before = timeout(Duration::from_secs(10), browser.pages())
        .await
        .expect("pages() should not time out")
        .expect("pages() should succeed");
    eprintln!("startup pages before create: {}", pages_before.len());

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page(about:blank) should not time out after startup discovery")
        .expect("new_page(about:blank) should resolve after startup discovery");

    let url = page.url().await.expect("url()");
    assert_eq!(url.as_deref(), Some("about:blank"));
}

/// Cover the same startup flow in headed mode.
#[tokio::test]
async fn browser_like_headed_about_blank_page_creation_resolves() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch_with_handler(browser_like_headed_config("browser-like-headed")).await;

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page(about:blank) should not time out in headed mode")
        .expect("new_page(about:blank) should resolve in headed mode");

    let url = page.url().await.expect("url()");
    assert_eq!(url.as_deref(), Some("about:blank"));
}

/// Try to surface scheduler-sensitive issues by running multiple headed
/// launches concurrently on a multi-thread runtime.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn browser_like_headed_about_blank_parallel_multithread_resolves() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let tasks = (0..6)
        .map(|iter| {
            tokio::spawn(async move {
                open_about_blank_with_timeout(
                    browser_like_headed_config(&format!("parallel-headed-{iter}")),
                    30,
                )
                .await
                .map_err(|err| format!("iteration {iter}: {err}"))
            })
        })
        .collect::<Vec<_>>();

    for task in tasks {
        let result = task.await.expect("task join");
        assert!(result.is_ok(), "parallel headed launch failed: {result:?}");
    }
}

/// Exercise a browser startup helper that launches Chrome, starts the handler,
/// and retries initial page creation using only chromey's public API.
#[tokio::test]
async fn browser_startup_example_equivalent_resolves() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    for iter in 0..1 {
        let config = browser_like_headed_config(&format!("browser-example-{iter}"));
        let (mut browser, mut handler) = Browser::launch(config).await.expect("launch browser");
        eprintln!("[chromey test] Browser launched for iter {iter}");

        let _handle = tokio::spawn(async move {
            eprintln!("[chromey test] Handler loop starting...");
            let mut count = 0u64;
            loop {
                match handler.next().await {
                    Some(Ok(())) => {
                        count += 1;
                        if count <= 5 || count % 100 == 0 {
                            eprintln!("[chromey test] Handler event #{count}");
                        }
                    }
                    Some(Err(err)) => {
                        eprintln!("[chromey test] Handler error after {count} events: {err}");
                    }
                    None => {
                        eprintln!("[chromey test] Handler stream ended after {count} events");
                        break;
                    }
                }
            }
        });

        let page = retried_open_start_page(&mut browser)
            .await
            .expect("browser startup should resolve");
        let url = page.url().await.expect("url()");
        assert_eq!(url.as_deref(), Some("about:blank"));
    }
}

/// Add background runtime churn and repeat the startup path to cover scheduler
/// pressure in the runtime.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn browser_like_about_blank_survives_tokio_churn() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let churn = (0..16)
        .map(|_| {
            tokio::spawn(async move {
                for _ in 0..2_000 {
                    tokio::task::yield_now().await;
                    tokio::time::sleep(Duration::from_micros(50)).await;
                }
            })
        })
        .collect::<Vec<_>>();

    for iter in 0..10 {
        let browser = launch_with_handler(browser_like_config(&format!("churn-{iter}"))).await;
        let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
            .await
            .unwrap_or_else(|_| panic!("iteration {iter}: new_page(about:blank) timed out"))
            .unwrap_or_else(|err| panic!("iteration {iter}: new_page(about:blank) failed: {err}"));
        let url = page
            .url()
            .await
            .unwrap_or_else(|err| panic!("iteration {iter}: url() failed: {err}"));
        assert_eq!(url.as_deref(), Some("about:blank"));
    }

    for handle in churn {
        let _ = handle.await;
    }
}

/// Verify that `set_content` works on an `about:blank` page.
///
/// This is the regression test for <https://github.com/spider-rs/chromey/issues/4>
/// where `set_content` failed with:
///   "Either objectId or executionContextId or uniqueContextId must be specified"
/// because the secondary (isolated) execution context was not available.
#[tokio::test]
async fn set_content_on_about_blank_succeeds() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch_with_handler(browser_like_config("set-content-about-blank")).await;

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page should not time out")
        .expect("new_page should resolve");

    let html = r#"<html><body><h1 id="greeting">Hello from set_content</h1></body></html>"#;

    timeout(Duration::from_secs(15), page.set_content(html))
        .await
        .expect("set_content should not time out")
        .expect("set_content should succeed");

    // Verify the content was actually set by reading it back.
    let content = timeout(Duration::from_secs(10), page.content())
        .await
        .expect("content() should not time out")
        .expect("content() should succeed");

    assert!(
        content.contains("Hello from set_content"),
        "page content should contain the HTML we set, got: {content}"
    );
}

/// Verify that calling `set_content` twice works (replaces prior content).
#[tokio::test]
async fn set_content_twice_replaces_content() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch_with_handler(browser_like_config("set-content-twice")).await;

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page should not time out")
        .expect("new_page should resolve");

    let html1 = r#"<html><body><h1>First</h1></body></html>"#;
    timeout(Duration::from_secs(15), page.set_content(html1))
        .await
        .expect("first set_content should not time out")
        .expect("first set_content should succeed");

    let html2 = r#"<html><body><p>Second</p></body></html>"#;
    timeout(Duration::from_secs(15), page.set_content(html2))
        .await
        .expect("second set_content should not time out")
        .expect("second set_content should succeed");

    let content = timeout(Duration::from_secs(10), page.content())
        .await
        .expect("content() should not time out")
        .expect("content() should succeed");

    assert!(
        content.contains("Second"),
        "page content should contain the second HTML, got: {content}"
    );
    assert!(
        !content.contains("First"),
        "page content should not contain the first HTML, got: {content}"
    );
}

/// Verify that many concurrent CDP commands on the same page do not deadlock.
///
/// This stresses the target channel (capacity 100) by issuing many commands
/// concurrently, covering the `CommandFuture` and `TargetMessageFuture`
/// send-backpressure paths.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_commands_do_not_deadlock() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch_with_handler(browser_like_config("concurrent-commands")).await;

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page should not time out")
        .expect("new_page should resolve");

    // Fire 200 concurrent evaluate calls — more than the channel capacity (100).
    let futs: Vec<_> = (0..200)
        .map(|i| {
            let page = page.clone();
            tokio::spawn(async move {
                timeout(
                    Duration::from_secs(30),
                    page.evaluate(format!("1 + {i}")),
                )
                .await
                .unwrap_or_else(|_| panic!("evaluate({i}) timed out — possible deadlock"))
                .unwrap_or_else(|err| panic!("evaluate({i}) failed: {err}"))
            })
        })
        .collect();

    for (i, fut) in futs.into_iter().enumerate() {
        fut.await
            .unwrap_or_else(|err| panic!("task {i} panicked: {err}"));
    }
}

/// Verify that rapid page.url() / page.mainframe() calls under concurrency
/// do not hang. These use raw TargetMessage sends (now protected by timeout).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_target_messages_do_not_deadlock() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch_with_handler(browser_like_config("concurrent-target-msgs")).await;

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page should not time out")
        .expect("new_page should resolve");

    // Mix different TargetMessage types concurrently.
    let futs: Vec<_> = (0..150)
        .map(|i| {
            let page = page.clone();
            tokio::spawn(async move {
                let result = match i % 3 {
                    0 => page
                        .url()
                        .await
                        .map(|_| ())
                        .map_err(|e| format!("url: {e}")),
                    1 => page
                        .mainframe()
                        .await
                        .map(|_| ())
                        .map_err(|e| format!("mainframe: {e}")),
                    _ => page
                        .evaluate("document.title")
                        .await
                        .map(|_| ())
                        .map_err(|e| format!("evaluate: {e}")),
                };
                timeout(Duration::from_secs(30), std::future::ready(result))
                    .await
                    .expect("should not time out")
                    .unwrap_or_else(|err| panic!("task {i} failed: {err}"));
            })
        })
        .collect();

    for fut in futs {
        fut.await.expect("task join");
    }
}

/// Verify that `goto` followed by `content()` completes without hanging.
/// This exercises the HttpFuture path (CommandFuture + TargetMessageFuture navigation).
#[tokio::test]
async fn goto_then_content_does_not_hang() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch_with_handler(browser_like_config("goto-content")).await;

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page should not time out")
        .expect("new_page should resolve");

    // Set content then read it back — exercises the full command pipeline.
    let html = r#"<html><body><p>deadlock-test</p></body></html>"#;
    timeout(Duration::from_secs(15), page.set_content(html))
        .await
        .expect("set_content should not time out")
        .expect("set_content should succeed");

    let content = timeout(Duration::from_secs(15), page.content())
        .await
        .expect("content() should not time out")
        .expect("content() should succeed");

    assert!(
        content.contains("deadlock-test"),
        "page should contain our test content"
    );
}

/// Open multiple pages concurrently and interact with all of them.
/// This tests that independent target channels don't interfere.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multiple_pages_concurrent_operations() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch_with_handler(browser_like_config("multi-page-concurrent")).await;

    // Create 4 pages sequentially (Browser is not Clone).
    let mut pages = Vec::new();
    for _ in 0..4 {
        let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
            .await
            .expect("new_page should not time out")
            .expect("new_page should resolve");
        pages.push(page);
    }

    // Run commands on all pages concurrently.
    let cmd_futs: Vec<_> = pages
        .iter()
        .enumerate()
        .map(|(i, page)| {
            let page = page.clone();
            tokio::spawn(async move {
                let html = format!(
                    r#"<html><body><p>page-{i}</p></body></html>"#
                );
                timeout(Duration::from_secs(15), page.set_content(&html))
                    .await
                    .expect("set_content should not time out")
                    .expect("set_content should succeed");

                let content = timeout(Duration::from_secs(15), page.content())
                    .await
                    .expect("content() should not time out")
                    .expect("content() should succeed");

                assert!(
                    content.contains(&format!("page-{i}")),
                    "page {i} should contain its content, got: {content}"
                );
            })
        })
        .collect();

    for fut in cmd_futs {
        fut.await.expect("task join");
    }
}

/// Generate a large HTML payload in Chrome via JS and read it back.
/// This exercises the full body streaming/transfer pipeline under load
/// and verifies no hang or timeout occurs for large content.
#[tokio::test]
async fn large_payload_set_content_and_read_back() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch_with_handler(browser_like_config("large-payload")).await;

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page should not time out")
        .expect("new_page should resolve");

    // Generate ~512 KiB of HTML content via JS (avoids sending it over CDP).
    timeout(
        Duration::from_secs(30),
        page.evaluate(
            r#"
            (() => {
                const chunk = '<p>' + 'A'.repeat(1024) + '</p>\n';
                document.body.innerHTML = chunk.repeat(512);
            })()
            "#,
        ),
    )
    .await
    .expect("evaluate should not time out")
    .expect("evaluate should succeed");

    let content = timeout(Duration::from_secs(30), page.content())
        .await
        .expect("content() should not time out — possible deadlock on large body")
        .expect("content() should succeed");

    // 512 chunks * ~1 KiB = ~512 KiB minimum
    assert!(
        content.len() > 400_000,
        "expected at least 400KB of content, got {} bytes",
        content.len()
    );
    eprintln!("large payload test: {} bytes read back", content.len());
}

/// Navigate multiple pages to real URLs concurrently.
/// This tests that the full pipeline (navigation + content extraction)
/// doesn't deadlock under concurrent real-world load.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_navigation_and_content_extraction() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch_with_handler(browser_like_config("concurrent-nav")).await;

    // Create 3 pages sequentially.
    let mut pages = Vec::new();
    for _ in 0..3 {
        let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
            .await
            .expect("new_page should not time out")
            .expect("new_page should resolve");
        pages.push(page);
    }

    // Navigate all pages concurrently with large generated content.
    let futs: Vec<_> = pages
        .into_iter()
        .enumerate()
        .map(|(i, page)| {
            tokio::spawn(async move {
                // Generate a ~256KB page via set_content.
                let filler = "X".repeat(1000);
                let body: String = (0..256)
                    .map(|j| format!("<p data-page='{i}' data-idx='{j}'>{filler}</p>\n"))
                    .collect();
                let html = format!("<html><body>{body}</body></html>");

                timeout(Duration::from_secs(30), page.set_content(&html))
                    .await
                    .unwrap_or_else(|_| panic!("page {i}: set_content timed out"))
                    .unwrap_or_else(|e| panic!("page {i}: set_content failed: {e}"));

                let content = timeout(Duration::from_secs(30), page.content())
                    .await
                    .unwrap_or_else(|_| panic!("page {i}: content() timed out"))
                    .unwrap_or_else(|e| panic!("page {i}: content() failed: {e}"));

                assert!(
                    content.len() > 200_000,
                    "page {i}: expected >200KB, got {} bytes",
                    content.len()
                );
                eprintln!("page {i}: {} bytes", content.len());

                // Also verify we can still run commands after the large transfer.
                let url = page.url().await.expect("url() after large content");
                assert!(url.is_some(), "page {i} should have a URL");
            })
        })
        .collect();

    for fut in futs {
        fut.await.expect("task join");
    }
}

/// Navigate to a real-world URL that may involve cross-origin redirects
/// (e.g. adding `www.` prefix or CDN routing). This exercises the fix for
/// navigation watchers losing track of the main frame when its ID changes
/// during a cross-origin redirect.
#[tokio::test]
async fn goto_cross_origin_redirect_url_loads() {
    if try_browser_config().is_none() {
        eprintln!("skipping: no Chrome/Chromium executable found");
        return;
    }

    let browser = launch_with_handler(browser_like_config("cross-origin-redirect")).await;

    let page = timeout(Duration::from_secs(30), browser.new_page("about:blank"))
        .await
        .expect("new_page should not time out")
        .expect("new_page should resolve");

    // Navigate to a real page that is known to redirect (clickz.com article).
    let target_url = "https://clickz.com/the-tiktok-perfume-effect-what-moroccanoils-measurement-gap-tells-every-senior-marketer";
    let result = timeout(Duration::from_secs(60), page.goto(target_url)).await;

    match result {
        Ok(Ok(_)) => {
            let url = page.url().await.expect("url()");
            eprintln!("navigated to: {url:?}");
            assert!(url.is_some(), "page should have a URL after navigation");

            // Verify we can actually extract HTML content from the page.
            let html = timeout(Duration::from_secs(15), page.content())
                .await
                .expect("content() should not time out")
                .expect("content() should succeed");
            assert!(
                !html.is_empty(),
                "page HTML should not be empty after navigation"
            );
            eprintln!("got {} bytes of HTML", html.len());
        }
        Ok(Err(err)) => {
            panic!("goto failed: {err}");
        }
        Err(_) => {
            panic!("goto timed out after 60s — navigation likely hung due to frame ID mismatch");
        }
    }
}
