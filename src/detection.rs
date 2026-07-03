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
///   - `CHROME` environment variable, plus `MSEDGE`/`EDGE` when
///     [`DetectionOptions::msedge`] is enabled
///   - Usual filenames in the user path
///   - (Windows) Registry
///   - (Windows & MacOS) Usual installations paths
///     If all of the above fail, an error is returned.
///
/// When [`DetectionOptions::prefer_msedge`] is enabled, every stage is swept
/// for an Edge install before the regular Chrome-first pipeline runs, so a
/// Chrome binary found early (e.g. on the `PATH`) cannot shadow an Edge
/// install that is only discoverable via the registry or known paths.
pub fn default_executable(options: DetectionOptions) -> Result<std::path::PathBuf, String> {
    if let Some(path) = get_by_env_var(&options) {
        return Ok(path);
    }

    if options.msedge && options.prefer_msedge {
        if let Some(path) = get_edge(&options) {
            return Ok(path);
        }
    }

    if let Some(path) = get_by_name(&options) {
        return Ok(path);
    }

    #[cfg(windows)]
    if let Some(path) = get_by_registry(&options) {
        return Ok(path);
    }

    if let Some(path) = get_by_path(&options) {
        return Ok(path);
    }

    Err("Could not auto detect a chrome executable".to_string())
}

/// Sweep every detection stage for an Edge install only, in the same stage
/// order as [`default_executable`]. Environment variables are handled by the
/// caller so an explicit `CHROME` override still outranks auto-detection.
fn get_edge(options: &DetectionOptions) -> Option<PathBuf> {
    if let Some(path) = get_edge_by_name(options) {
        return Some(path);
    }

    #[cfg(windows)]
    if let Some(path) = get_edge_by_registry() {
        return Some(path);
    }

    search_paths(&edge_path_candidates(options))
}

/// Executable path from an environment variable; requires a real file so a
/// stray variable (e.g. `EDGE` pointing at a directory) is never returned.
fn env_file(var: &str) -> Option<PathBuf> {
    let path = env::var(var).ok()?;
    if Path::new(&path).is_file() {
        Some(path.into())
    } else {
        None
    }
}

fn get_by_env_var(options: &DetectionOptions) -> Option<PathBuf> {
    // Historical behavior: any existing path set via `CHROME` is honored.
    let chrome = || {
        let path = env::var("CHROME").ok()?;
        if Path::new(&path).exists() {
            Some(PathBuf::from(path))
        } else {
            None
        }
    };
    let edge = || {
        if options.msedge {
            env_file("MSEDGE").or_else(|| env_file("EDGE"))
        } else {
            None
        }
    };

    if options.msedge && options.prefer_msedge {
        edge().or_else(chrome)
    } else {
        chrome().or_else(edge)
    }
}

#[cfg(feature = "auto-detect-executable")]
fn chrome_name_candidates(options: &DetectionOptions) -> [(&'static str, bool); 9] {
    [
        ("chrome", true),
        ("chrome-browser", true),
        ("google-chrome-stable", true),
        ("google-chrome-beta", options.unstable),
        ("google-chrome-dev", options.unstable),
        ("google-chrome-unstable", options.unstable),
        ("chromium", true),
        ("chromium-browser", true),
        ("brave", true),
    ]
}

#[cfg(feature = "auto-detect-executable")]
fn edge_name_candidates(options: &DetectionOptions) -> [(&'static str, bool); 5] {
    [
        ("msedge", options.msedge),
        ("microsoft-edge", options.msedge),
        ("microsoft-edge-stable", options.msedge),
        ("microsoft-edge-beta", options.msedge && options.unstable),
        ("microsoft-edge-dev", options.msedge && options.unstable),
    ]
}

/// Ordered list of `which`-resolvable binary names with their allowed flags.
///
/// Chrome/Chromium candidates come first by default; when
/// [`DetectionOptions::prefer_msedge`] is set the Edge candidates are moved
/// ahead of them. Disallowed entries are retained (flagged `false`) so the
/// caller skips them — keeping the relative order stable for either branch.
#[cfg(feature = "auto-detect-executable")]
fn name_candidates(options: &DetectionOptions) -> Vec<(&'static str, bool)> {
    let chrome_apps = chrome_name_candidates(options);
    let edge_apps = edge_name_candidates(options);

    if options.prefer_msedge {
        edge_apps.into_iter().chain(chrome_apps).collect()
    } else {
        chrome_apps.into_iter().chain(edge_apps).collect()
    }
}

#[cfg(feature = "auto-detect-executable")]
fn which_first(candidates: impl IntoIterator<Item = (&'static str, bool)>) -> Option<PathBuf> {
    for (app, allowed) in candidates {
        if !allowed {
            continue;
        }
        if let Ok(path) = which::which(app) {
            return Some(path);
        }
    }

    None
}

#[cfg(feature = "auto-detect-executable")]
fn get_by_name(options: &DetectionOptions) -> Option<PathBuf> {
    which_first(name_candidates(options))
}

#[cfg(not(feature = "auto-detect-executable"))]
fn get_by_name(_options: &DetectionOptions) -> Option<PathBuf> {
    None
}

#[cfg(feature = "auto-detect-executable")]
fn get_edge_by_name(options: &DetectionOptions) -> Option<PathBuf> {
    which_first(edge_name_candidates(options))
}

#[cfg(not(feature = "auto-detect-executable"))]
fn get_edge_by_name(_options: &DetectionOptions) -> Option<PathBuf> {
    None
}

#[allow(unused_variables)]
fn chrome_path_candidates(options: &DetectionOptions) -> Vec<(&'static str, bool)> {
    #[cfg(all(unix, not(target_os = "macos")))]
    return vec![
        ("/opt/chromium.org/chromium", true),
        ("/opt/google/chrome", true),
        // test for lambda
        ("/tmp/aws/lib", true),
    ];

    #[cfg(windows)]
    return Vec::new();

    #[cfg(target_os = "macos")]
    return vec![
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
}

#[allow(unused_variables)]
fn edge_path_candidates(options: &DetectionOptions) -> Vec<(&'static str, bool)> {
    #[cfg(all(unix, not(target_os = "macos")))]
    return Vec::new();

    #[cfg(windows)]
    return vec![
        // 64-bit default install location first, then the 32-bit fallback.
        (
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
            options.msedge,
        ),
        (
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            options.msedge,
        ),
    ];

    #[cfg(target_os = "macos")]
    return vec![
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
}

fn search_paths(paths: &[(&str, bool)]) -> Option<PathBuf> {
    for &(path, allowed) in paths {
        if !allowed {
            continue;
        }
        if Path::new(path).exists() {
            return Some(path.into());
        }
    }

    None
}

fn get_by_path(options: &DetectionOptions) -> Option<PathBuf> {
    let chrome_paths = chrome_path_candidates(options);
    let edge_paths = edge_path_candidates(options);

    if options.prefer_msedge {
        search_paths(&edge_paths).or_else(|| search_paths(&chrome_paths))
    } else {
        search_paths(&chrome_paths).or_else(|| search_paths(&edge_paths))
    }
}

/// Resolve an executable from the Windows `App Paths` registry entry,
/// checking HKLM first and then HKCU.
#[cfg(windows)]
fn registry_app_path(exe: &str) -> Option<PathBuf> {
    let subkey = format!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{exe}");

    winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
        .open_subkey(&subkey)
        .or_else(|_| winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).open_subkey(&subkey))
        .and_then(|key| key.get_value::<String, _>(""))
        .map(PathBuf::from)
        .ok()
}

/// Edge's `App Paths` entry, validated on disk so a stale key left behind by
/// an uninstall cannot shadow a working Chrome fallback.
#[cfg(windows)]
fn get_edge_by_registry() -> Option<PathBuf> {
    registry_app_path("msedge.exe").filter(|path| path.exists())
}

#[cfg(windows)]
fn get_by_registry(options: &DetectionOptions) -> Option<PathBuf> {
    let chrome = || registry_app_path("chrome.exe");
    let edge = || {
        if options.msedge {
            get_edge_by_registry()
        } else {
            None
        }
    };

    if options.prefer_msedge {
        edge().or_else(chrome)
    } else {
        chrome().or_else(edge)
    }
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

    /// The Edge-only sweep never resolves Chrome/Chromium names, and disabling
    /// [`DetectionOptions::msedge`] disables every Edge candidate.
    #[test]
    fn edge_candidates_respect_msedge_flag() {
        let enabled = DetectionOptions {
            msedge: true,
            unstable: false,
            prefer_msedge: true,
        };
        assert!(edge_name_candidates(&enabled)
            .iter()
            .all(|(name, _)| name.contains("edge")));

        let disabled = DetectionOptions {
            msedge: false,
            unstable: true,
            prefer_msedge: false,
        };
        assert!(edge_name_candidates(&disabled)
            .iter()
            .all(|(_, allowed)| !allowed));
        assert!(edge_path_candidates(&disabled)
            .iter()
            .all(|(_, allowed)| !allowed));
    }

    /// The 64-bit Edge install directory is probed first on Windows, with the
    /// 32-bit location kept as a fallback.
    #[test]
    #[cfg(windows)]
    fn windows_edge_paths_include_64_bit_install() {
        let paths: Vec<&str> = edge_path_candidates(&DetectionOptions::default())
            .into_iter()
            .map(|(path, _)| path)
            .collect();
        assert_eq!(
            paths.first(),
            Some(&r"C:\Program Files\Microsoft\Edge\Application\msedge.exe")
        );
        assert!(paths.contains(&r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"));
    }
}
