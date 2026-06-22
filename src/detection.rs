use std::env;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct DetectionOptions {
    /// Detect Microsoft Edge
    pub msedge: bool,
    /// Detect unstable installations (beta, dev, unstable)
    pub unstable: bool,
    /// Prefer Microsoft Edge over Chrome/Chromium when both are installed.
    ///
    /// By default Chrome/Chromium binaries are checked first, so a system with
    /// both Chrome and Edge resolves to Chrome even when [`Self::msedge`] is
    /// set. Enabling this checks Edge binaries first and falls back to
    /// Chrome/Chromium only when no Edge install is found. Has no effect unless
    /// [`Self::msedge`] is also enabled.
    pub prefer_msedge: bool,
}

impl Default for DetectionOptions {
    fn default() -> Self {
        Self {
            msedge: true,
            unstable: false,
            prefer_msedge: false,
        }
    }
}

/// Returns the path to Chrome's executable.
///
/// The following elements will be checked:
///   - `CHROME` environment variable
///   - Usual filenames in the user path
///   - (Windows) Registry
///   - (Windows & MacOS) Usual installations paths
///     If all of the above fail, an error is returned.
pub fn default_executable(options: DetectionOptions) -> Result<std::path::PathBuf, String> {
    if let Some(path) = get_by_env_var() {
        return Ok(path);
    }

    if let Some(path) = get_by_name(&options) {
        return Ok(path);
    }

    #[cfg(windows)]
    if let Some(path) = get_by_registry() {
        return Ok(path);
    }

    if let Some(path) = get_by_path(&options) {
        return Ok(path);
    }

    Err("Could not auto detect a chrome executable".to_string())
}

fn get_by_env_var() -> Option<PathBuf> {
    if let Ok(path) = env::var("CHROME") {
        if Path::new(&path).exists() {
            return Some(path.into());
        }
    }

    None
}

/// Ordered list of `which`-resolvable binary names with their allowed flags.
///
/// Chrome/Chromium candidates come first by default; when
/// [`DetectionOptions::prefer_msedge`] is set the Edge candidates are moved
/// ahead of them. Disallowed entries are retained (flagged `false`) so the
/// caller skips them — keeping the relative order stable for either branch.
#[cfg(feature = "auto-detect-executable")]
fn name_candidates(options: &DetectionOptions) -> Vec<(&'static str, bool)> {
    let chrome_apps = [
        ("chrome", true),
        ("chrome-browser", true),
        ("google-chrome-stable", true),
        ("google-chrome-beta", options.unstable),
        ("google-chrome-dev", options.unstable),
        ("google-chrome-unstable", options.unstable),
        ("chromium", true),
        ("chromium-browser", true),
        ("brave", true),
    ];
    let edge_apps = [
        ("msedge", options.msedge),
        ("microsoft-edge", options.msedge),
        ("microsoft-edge-stable", options.msedge),
        ("microsoft-edge-beta", options.msedge && options.unstable),
        ("microsoft-edge-dev", options.msedge && options.unstable),
    ];

    if options.prefer_msedge {
        edge_apps.into_iter().chain(chrome_apps).collect()
    } else {
        chrome_apps.into_iter().chain(edge_apps).collect()
    }
}

#[cfg(feature = "auto-detect-executable")]
fn get_by_name(options: &DetectionOptions) -> Option<PathBuf> {
    for (app, allowed) in name_candidates(options) {
        if !allowed {
            continue;
        }
        if let Ok(path) = which::which(app) {
            return Some(path);
        }
    }

    None
}

#[cfg(not(feature = "auto-detect-executable"))]
fn get_by_name(_options: &DetectionOptions) -> Option<PathBuf> {
    None
}

#[allow(unused_variables)]
fn get_by_path(options: &DetectionOptions) -> Option<PathBuf> {
    #[cfg(all(unix, not(target_os = "macos")))]
    let chrome_paths: [(&str, bool); 3] = [
        ("/opt/chromium.org/chromium", true),
        ("/opt/google/chrome", true),
        // test for lambda
        ("/tmp/aws/lib", true),
    ];
    #[cfg(all(unix, not(target_os = "macos")))]
    let edge_paths: [(&str, bool); 0] = [];

    #[cfg(windows)]
    let chrome_paths: [(&str, bool); 0] = [];
    #[cfg(windows)]
    let edge_paths: [(&str, bool); 1] = [(
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        options.msedge,
    )];

    #[cfg(target_os = "macos")]
    let chrome_paths: [(&str, bool); 5] = [
        (
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            true,
        ),
        (
            "/Applications/Google Chrome Beta.app/Contents/MacOS/Google Chrome Beta",
            options.unstable,
        ),
        (
            "/Applications/Google Chrome Dev.app/Contents/MacOS/Google Chrome Dev",
            options.unstable,
        ),
        (
            "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
            options.unstable,
        ),
        ("/Applications/Chromium.app/Contents/MacOS/Chromium", true),
    ];
    #[cfg(target_os = "macos")]
    let edge_paths: [(&str, bool); 4] = [
        (
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            options.msedge,
        ),
        (
            "/Applications/Microsoft Edge Beta.app/Contents/MacOS/Microsoft Edge Beta",
            options.msedge && options.unstable,
        ),
        (
            "/Applications/Microsoft Edge Dev.app/Contents/MacOS/Microsoft Edge Dev",
            options.msedge && options.unstable,
        ),
        (
            "/Applications/Microsoft Edge Canary.app/Contents/MacOS/Microsoft Edge Canary",
            options.msedge && options.unstable,
        ),
    ];

    let search = |paths: &[(&str, bool)]| -> Option<PathBuf> {
        for &(path, allowed) in paths {
            if !allowed {
                continue;
            }
            if Path::new(path).exists() {
                return Some(path.into());
            }
        }
        None
    };

    if options.prefer_msedge {
        search(&edge_paths).or_else(|| search(&chrome_paths))
    } else {
        search(&chrome_paths).or_else(|| search(&edge_paths))
    }
}

#[cfg(windows)]
fn get_by_registry() -> Option<PathBuf> {
    winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
        .open_subkey("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\chrome.exe")
        .or_else(|_| {
            winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
                .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\chrome.exe")
        })
        .and_then(|key| key.get_value::<String, _>(""))
        .map(PathBuf::from)
        .ok()
}

#[cfg(all(test, feature = "auto-detect-executable"))]
mod tests {
    use super::*;

    fn names(options: &DetectionOptions) -> Vec<&'static str> {
        name_candidates(options)
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    }

    /// Regression guard: the default order checks Chrome before Edge, matching
    /// historical behavior so existing detections resolve identically.
    #[test]
    fn default_order_checks_chrome_before_edge() {
        let order = names(&DetectionOptions::default());
        let chrome = order.iter().position(|n| *n == "chrome").unwrap();
        let edge = order.iter().position(|n| *n == "msedge").unwrap();
        assert!(chrome < edge);
        assert_eq!(order.first(), Some(&"chrome"));
    }

    /// `prefer_msedge` moves every Edge candidate ahead of Chrome/Chromium.
    #[test]
    fn prefer_msedge_checks_edge_before_chrome() {
        let options = DetectionOptions {
            msedge: true,
            unstable: false,
            prefer_msedge: true,
        };
        let order = names(&options);
        let chrome = order.iter().position(|n| *n == "chrome").unwrap();
        let edge = order.iter().position(|n| *n == "msedge").unwrap();
        assert!(edge < chrome);
        assert_eq!(order.first(), Some(&"msedge"));
    }

    /// Reordering only swaps group order — no candidate is dropped, so Chrome
    /// remains a fallback when no Edge install is present.
    #[test]
    fn prefer_msedge_keeps_all_candidates() {
        let base = DetectionOptions {
            msedge: true,
            unstable: true,
            prefer_msedge: false,
        };
        let preferred = DetectionOptions {
            prefer_msedge: true,
            ..base.clone()
        };

        let mut base_sorted = names(&base);
        let mut preferred_sorted = names(&preferred);
        base_sorted.sort_unstable();
        preferred_sorted.sort_unstable();

        assert_eq!(base_sorted, preferred_sorted);
        assert!(preferred_sorted.contains(&"chrome"));
        assert!(preferred_sorted.contains(&"brave"));
    }
}
