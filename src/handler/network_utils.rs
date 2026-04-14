use std::borrow::Cow;

use memchr::{memchr, memchr3, memrchr};

#[inline]
fn strip_special_schemes(url: &str) -> &str {
    let url = url.strip_prefix("blob:").unwrap_or(url);
    url.strip_prefix("filesystem:").unwrap_or(url)
}

/// Returns (host_without_port, rest_starting_at_/ ? # or empty)
/// Robust: handles protocol-relative, userinfo, IPv6 literals, ports.
#[inline]
pub fn host_and_rest(url: &str) -> Option<(&str, &str)> {
    let url = strip_special_schemes(url);
    let bytes = url.as_bytes();

    let host_start = if let Some(pos) = memchr(b':', bytes) {
        if bytes.get(pos + 1) == Some(&b'/') && bytes.get(pos + 2) == Some(&b'/') {
            pos + 3
        } else if bytes.starts_with(b"//") {
            2
        } else {
            return None;
        }
    } else if bytes.starts_with(b"//") {
        2
    } else {
        return None;
    };

    // End of authority: find first of / ? # after host_start in one SIMD pass.
    let rest_start =
        memchr3(b'/', b'?', b'#', &bytes[host_start..]).map_or(url.len(), |i| host_start + i);

    let authority = &url[host_start..rest_start];
    if authority.is_empty() {
        return None;
    }

    // Drop userinfo if present: user:pass@host
    let authority = match memrchr(b'@', authority.as_bytes()) {
        Some(pos) => &authority[pos + 1..],
        None => authority,
    };

    let ab = authority.as_bytes();

    // IPv6: [::1]:8080
    if ab.first() == Some(&b'[') {
        let close = memchr(b']', ab)?;
        let host = &authority[1..close];
        return Some((host, &url[rest_start..]));
    }

    // IPv4/hostname: host:port
    let host_end = memchr(b':', ab).unwrap_or(ab.len());
    let host = &authority[..host_end];
    if host.is_empty() {
        return None;
    }

    Some((host, &url[rest_start..]))
}

#[inline]
fn eq_ignore_ascii_case(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

#[inline]
pub fn ends_with_ignore_ascii_case(hay: &str, suf: &str) -> bool {
    if suf.len() > hay.len() {
        return false;
    }
    hay[hay.len() - suf.len()..].eq_ignore_ascii_case(suf)
}

#[inline]
pub fn base_domain_from_any(s: &str) -> &str {
    if let Some((h, _)) = host_and_rest(s) {
        base_domain_from_host(h)
    } else {
        base_domain_from_host(s)
    }
}

#[inline]
pub fn first_label(host: &str) -> &str {
    let h = host.trim_end_matches('.');
    match memchr(b'.', h.as_bytes()) {
        Some(i) => &h[..i],
        None => h,
    }
}

#[inline]
pub fn host_contains_label_icase(host: &str, label: &str) -> bool {
    let host = host.trim_end_matches('.');
    let label = label.trim_matches('.');

    if host.is_empty() || label.is_empty() {
        return false;
    }

    let hb = host.as_bytes();
    let lb = label.as_bytes();

    // Use memchr to jump between dots instead of scanning byte-by-byte.
    let mut start = 0usize;

    // Skip leading dots.
    while start < hb.len() && hb[start] == b'.' {
        start += 1;
    }

    while start < hb.len() {
        let end = memchr(b'.', &hb[start..]).map_or(hb.len(), |i| start + i);

        if end - start == lb.len() && hb[start..end].eq_ignore_ascii_case(lb) {
            return true;
        }

        // Skip past the dot and any consecutive dots.
        start = end + 1;
        while start < hb.len() && hb[start] == b'.' {
            start += 1;
        }
    }

    false
}

/// Host matches base if host == base OR host ends with ".{base}" (case-insensitive),
/// with a required dot boundary to prevent "evil-mainr.com" matching "mainr.com".
#[inline]
pub fn host_is_subdomain_of(host: &str, base: &str) -> bool {
    let host = host.trim_end_matches('.');
    let base = base.trim_end_matches('.');

    if base.is_empty() {
        return false;
    }

    if eq_ignore_ascii_case(host, base) {
        return true;
    }

    if host.len() <= base.len() {
        return false;
    }

    let dot_pos = host.len() - base.len() - 1;
    host.as_bytes().get(dot_pos) == Some(&b'.') && ends_with_ignore_ascii_case(host, base)
}

/// Common subdomain labels.
static COMMON_SUBDOMAIN_LABELS: phf::Set<&'static str> = phf::phf_set! {
    "www","m","amp","api","cdn","static","assets","img","images","media","files",
    "login","auth","sso","id","account","accounts",
    "app","apps","dashboard","admin","portal","console",
    "status","support","help","docs","blog",
    "dev","staging","stage","test","qa","uat","beta","alpha","preview","demo","sandbox",
    "uploads","download","storage","origin","edge","cache",
    "mail","email","smtp","mx","webmail",
    "graphql","rpc","ws",
};

#[inline]
/// Common sub domains.
fn is_common_subdomain_label(lbl: &str) -> bool {
    if lbl.is_empty() {
        return false;
    }
    let lower = lbl.to_ascii_lowercase();
    COMMON_SUBDOMAIN_LABELS.contains(lower.as_str())
}

#[inline]
pub fn base_domain_from_url(main_url: &str) -> Option<&str> {
    let (host, _) = host_and_rest(main_url)?;
    Some(base_domain_from_host(host))
}

/// Given a base domain (already computed) and a URL, returns the “relative” path
/// for same-site/subdomain URLs, otherwise returns the original URL.
#[inline]
pub fn rel_for_ignore_script<'a>(main_host_or_base: &str, url: &'a str) -> Cow<'a, str> {
    if url.starts_with('/') {
        return Cow::Borrowed(url);
    }

    let base = base_domain_from_host(main_host_or_base.trim_end_matches('.'));
    let base = base.trim_end_matches('.');
    if base.is_empty() {
        return Cow::Borrowed(url);
    }

    let brand = first_label(base);

    if let Some((host, rest)) = host_and_rest(url) {
        if host_is_subdomain_of(host, base) || host_contains_label_icase(host, brand) {
            if rest.starts_with('/') {
                return Cow::Borrowed(rest);
            }
            return Cow::Borrowed("/");
        }
    }

    Cow::Borrowed(url)
}

#[inline]
/// Common cc.
fn is_common_cc_sld(sld: &str) -> bool {
    let s = sld.as_bytes();
    match s.len() {
        2 => matches!(
            [s[0].to_ascii_lowercase(), s[1].to_ascii_lowercase()],
            [b'c', b'o'] | // co
            [b'a', b'c'] | // ac
            [b'g', b'o'] | // go
            [b'o', b'r'] | // or
            [b'n', b'e'] | // ne
            [b'e', b'd'] | // ed
            [b'g', b'r'] | // gr
            [b'l', b'g'] | // lg
            [b'a', b'd'] // ad
        ),
        3 => matches!(
            [
                s[0].to_ascii_lowercase(),
                s[1].to_ascii_lowercase(),
                s[2].to_ascii_lowercase()
            ],
            // globally common
            [b'c', b'o', b'm'] | // com
            [b'n', b'e', b't'] | // net
            [b'o', b'r', b'g'] | // org
            [b'g', b'o', b'v'] | // gov
            [b'e', b'd', b'u'] | // edu
            [b'm', b'i', b'l'] | // mil
            [b'n', b'i', b'c'] | // nic
            [b's', b'c', b'h'] | // sch
            // MX / some LATAM
            [b'g', b'o', b'b'] // gob
        ),
        4 => matches!(
            [
                s[0].to_ascii_lowercase(),
                s[1].to_ascii_lowercase(),
                s[2].to_ascii_lowercase(),
                s[3].to_ascii_lowercase()
            ],
            [b'g', b'o', b'u', b'v'] // gouv (seen in some places)
        ),
        _ => false,
    }
}

#[inline]
/// Get the base “site” domain from a host.
///
/// - Normal sites: `staging.mainr.com` -> `mainr.com`
/// - ccTLD-ish: `a.b.example.co.uk` -> `example.co.uk` (existing heuristic)
/// - Multi-tenant SaaS: `mainr.chilipiper.com` -> `mainr.chilipiper.com`
///   (keeps one extra label when it looks like a tenant, not `www`/`cdn`/etc.)
pub fn base_domain_from_host(host: &str) -> &str {
    let mut h = host.trim_end_matches('.');
    if let Some(x) = h.strip_prefix("www.") {
        h = x;
    }
    if let Some(x) = h.strip_prefix("m.") {
        h = x;
    }

    // Find last two dots using SIMD-accelerated reverse search.
    let hb = h.as_bytes();
    let last_dot = match memrchr(b'.', hb) {
        Some(p) => p,
        None => return h,
    };
    let prev_dot = match memrchr(b'.', &hb[..last_dot]) {
        Some(p) => p,
        None => return h, // only 1 dot
    };

    let tld = &h[last_dot + 1..];
    let sld = &h[prev_dot + 1..last_dot];

    let mut base = &h[prev_dot + 1..]; // "example.com" or "co.uk"

    if tld.len() == 2 && is_common_cc_sld(sld) {
        if let Some(prev2_dot) = memrchr(b'.', &hb[..prev_dot]) {
            base = &h[prev2_dot + 1..]; // "example.co.uk"
        }
    }

    if h.len() > base.len() + 1 {
        let base_start = h.len() - base.len();
        let boundary = base_start - 1;
        if hb.get(boundary) == Some(&b'.') {
            let left_part = &h[..boundary];
            // label immediately to the left of base
            let (lbl_start, lbl) = match memrchr(b'.', left_part.as_bytes()) {
                Some(p) => (p + 1, &left_part[p + 1..]),
                None => (0, left_part),
            };

            if !lbl.is_empty() && !is_common_subdomain_label(lbl) {
                // return "tenant.base" => slice starting at lbl_start
                return &h[lbl_start..];
            }
        }
    }

    base
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_domain_match_basic_and_subdomains() {
        let base = "mainr.com";

        assert!(host_is_subdomain_of("mainr.com", base));
        assert!(host_is_subdomain_of("staging.mainr.com", base));
        assert!(host_is_subdomain_of("a.b.c.mainr.com", base));

        // case-insensitive
        assert!(host_is_subdomain_of("StAgInG.mainr.CoM", "mainr.COM"));
    }

    #[test]
    fn test_domain_match_no_false_positives() {
        let base = "mainr.com";

        // must be dot-boundary
        assert!(!host_is_subdomain_of("evil-mainr.com", base));
        assert!(!host_is_subdomain_of("mainr.com.evil.com", base));
        assert!(!host_is_subdomain_of("stagingmainr.com", base));
        assert!(!host_is_subdomain_of("mainr.co", base));
    }

    #[test]
    fn test_host_and_rest_handles_userinfo_port_ipv6() {
        let (h, rest) =
            host_and_rest("https://user:pass@staging.mainr.com:8443/a.js?x=1#y").unwrap();
        assert_eq!(h, "staging.mainr.com");
        assert_eq!(rest, "/a.js?x=1#y");

        let (h, rest) = host_and_rest("http://[::1]:8080/path").unwrap();
        assert_eq!(h, "::1");
        assert_eq!(rest, "/path");
    }

    #[test]
    fn test_rel_for_ignore_script_mainr_example() {
        let base = "mainr.com";

        let main = "https://mainr.com/careers";
        assert_eq!(rel_for_ignore_script(base, main).as_ref(), "/careers");

        let script = "https://staging.mainr.com/mainr.min.js";
        assert_eq!(
            rel_for_ignore_script(base, script).as_ref(),
            "/mainr.min.js"
        );

        // Different site stays absolute
        let other = "https://cdn.other.com/app.js";
        assert_eq!(rel_for_ignore_script(base, other).as_ref(), other);

        // Root-relative stays as-is
        assert_eq!(
            rel_for_ignore_script(base, "/static/app.js").as_ref(),
            "/static/app.js"
        );
    }

    #[test]
    fn test_rel_for_ignore_script_query_only_same_site() {
        let base = "example.com";
        let u = "https://sub.example.com?x=1";
        assert_eq!(rel_for_ignore_script(base, u).as_ref(), "/");
    }

    #[test]
    fn test_rel_for_ignore_script_special_schemes() {
        let base = "example.com";
        let u = "blob:https://example.com/path/to/blob";
        assert_eq!(rel_for_ignore_script(base, u).as_ref(), "/path/to/blob");
    }

    #[test]
    fn test_base_domain_tenant_subdomain() {
        let base = base_domain_from_host("mainr.chilipiper.com");
        assert_eq!(base, "mainr.chilipiper.com");

        // same tenant (subdomain) becomes relative
        let u = "https://assets.mainr.chilipiper.com/a.js";
        assert_eq!(rel_for_ignore_script(base, u).as_ref(), "/a.js");

        // different tenant must NOT match
        let other = "https://othertenant.chilipiper.com/a.js";
        assert_eq!(rel_for_ignore_script(base, other).as_ref(), other);
    }

    #[test]
    fn test_brand_label_allows_vendor_subdomain() {
        let base = "mainr.com";
        let u = "https://mainr.chilipiper.com/concierge-js/cjs/concierge.js";
        assert_eq!(
            rel_for_ignore_script(base, u).as_ref(),
            "/concierge-js/cjs/concierge.js"
        );

        // Important: not a substring match
        let bad = "https://evil-mainr.com/x.js";
        assert_eq!(rel_for_ignore_script(base, bad).as_ref(), bad);
    }

    #[test]
    fn test_allows_vendor_host_when_brand_label_matches_main_site() {
        // main page host is www.mainr.com
        let main_host = "www.mainr.com";

        let u = "https://mainr.chilipiper.com/concierge-js/cjs/concierge.js";
        assert_eq!(
            rel_for_ignore_script(main_host, u).as_ref(),
            "/concierge-js/cjs/concierge.js"
        );
    }

    // --- Additional edge-case tests for SIMD-accelerated paths ---

    #[test]
    fn test_host_and_rest_edge_cases() {
        // Protocol-relative URL
        let (h, rest) = host_and_rest("//example.com/path").unwrap();
        assert_eq!(h, "example.com");
        assert_eq!(rest, "/path");

        // No path, query, or fragment
        let (h, rest) = host_and_rest("https://example.com").unwrap();
        assert_eq!(h, "example.com");
        assert_eq!(rest, "");

        // Query only (no path)
        let (h, rest) = host_and_rest("https://example.com?q=1").unwrap();
        assert_eq!(h, "example.com");
        assert_eq!(rest, "?q=1");

        // Fragment only (no path)
        let (h, rest) = host_and_rest("https://example.com#frag").unwrap();
        assert_eq!(h, "example.com");
        assert_eq!(rest, "#frag");

        // No scheme returns None
        assert!(host_and_rest("example.com/path").is_none());
        assert!(host_and_rest("").is_none());

        // blob: + filesystem: schemes
        let (h, _) = host_and_rest("filesystem:https://example.com/path").unwrap();
        assert_eq!(h, "example.com");

        // Port only, no path
        let (h, rest) = host_and_rest("https://example.com:8080").unwrap();
        assert_eq!(h, "example.com");
        assert_eq!(rest, "");

        // Userinfo with port
        let (h, _) = host_and_rest("https://user@example.com:443/x").unwrap();
        assert_eq!(h, "example.com");

        // IPv6 without port
        let (h, rest) = host_and_rest("http://[::1]/path").unwrap();
        assert_eq!(h, "::1");
        assert_eq!(rest, "/path");

        // Empty authority
        assert!(host_and_rest("http:///path").is_none());
    }

    #[test]
    fn test_host_contains_label_icase_edge_cases() {
        // Basic match
        assert!(host_contains_label_icase("www.example.com", "example"));
        assert!(host_contains_label_icase("www.example.com", "EXAMPLE"));
        assert!(host_contains_label_icase("www.example.com", "www"));
        assert!(host_contains_label_icase("www.example.com", "com"));

        // Exact single-label host
        assert!(host_contains_label_icase("localhost", "localhost"));
        assert!(host_contains_label_icase("LOCALHOST", "localhost"));

        // No partial matches
        assert!(!host_contains_label_icase("www.example.com", "exam"));
        assert!(!host_contains_label_icase("www.example.com", "ample"));

        // Empty inputs
        assert!(!host_contains_label_icase("", "example"));
        assert!(!host_contains_label_icase("example.com", ""));

        // Trailing dots
        assert!(host_contains_label_icase("example.com.", "com"));
        assert!(host_contains_label_icase("example.com.", "example"));
    }

    #[test]
    fn test_first_label_edge_cases() {
        assert_eq!(first_label("www.example.com"), "www");
        assert_eq!(first_label("example.com"), "example");
        assert_eq!(first_label("localhost"), "localhost");
        assert_eq!(first_label("example.com."), "example");
    }

    #[test]
    fn test_base_domain_from_host_edge_cases() {
        // Simple two-label
        assert_eq!(base_domain_from_host("example.com"), "example.com");

        // Strip www/m
        assert_eq!(base_domain_from_host("www.example.com"), "example.com");
        assert_eq!(base_domain_from_host("m.example.com"), "example.com");

        // ccTLD
        assert_eq!(base_domain_from_host("example.co.uk"), "example.co.uk");
        assert_eq!(base_domain_from_host("www.example.co.uk"), "example.co.uk");

        // Single label
        assert_eq!(base_domain_from_host("localhost"), "localhost");

        // Trailing dot
        assert_eq!(base_domain_from_host("example.com."), "example.com");
    }

    #[test]
    fn test_host_is_subdomain_of_edge_cases() {
        // Trailing dots
        assert!(host_is_subdomain_of("example.com.", "example.com."));
        assert!(host_is_subdomain_of("sub.example.com.", "example.com."));

        // Empty base
        assert!(!host_is_subdomain_of("example.com", ""));

        // Exact match
        assert!(host_is_subdomain_of("example.com", "example.com"));

        // Shorter host than base
        assert!(!host_is_subdomain_of("com", "example.com"));
    }
}
