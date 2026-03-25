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
