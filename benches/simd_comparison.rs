//! A/B benchmark: str::find() vs memchr SIMD for network_utils hot paths.
//!
//! Inlines both old and new implementations so we can compare directly.

use criterion::{black_box, criterion_group, criterion_main, Criterion};

// ---- OLD implementation (str::find) ----

mod old {
    #[inline]
    fn strip_special_schemes(url: &str) -> &str {
        let url = url.strip_prefix("blob:").unwrap_or(url);
        url.strip_prefix("filesystem:").unwrap_or(url)
    }

    #[inline]
    pub fn host_and_rest(url: &str) -> Option<(&str, &str)> {
        let url = strip_special_schemes(url);

        let host_start = if let Some(pos) = url.find("://") {
            pos + 3
        } else if url.starts_with("//") {
            2
        } else {
            return None;
        };

        let mut rest_start = url.len();
        if let Some(i) = url[host_start..].find('/') {
            rest_start = host_start + i;
        }
        if let Some(i) = url[host_start..].find('?') {
            rest_start = rest_start.min(host_start + i);
        }
        if let Some(i) = url[host_start..].find('#') {
            rest_start = rest_start.min(host_start + i);
        }

        let authority = &url[host_start..rest_start];
        if authority.is_empty() {
            return None;
        }

        let authority = authority.rsplit('@').next().unwrap_or(authority);

        if authority.as_bytes().first() == Some(&b'[') {
            let close = authority.find(']')?;
            let host = &authority[1..close];
            return Some((host, &url[rest_start..]));
        }

        let host_end = authority.find(':').unwrap_or(authority.len());
        let host = &authority[..host_end];
        if host.is_empty() {
            return None;
        }

        Some((host, &url[rest_start..]))
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

        let mut i = 0usize;
        while i < hb.len() {
            while i < hb.len() && hb[i] == b'.' {
                i += 1;
            }
            if i >= hb.len() {
                break;
            }

            let start = i;
            while i < hb.len() && hb[i] != b'.' {
                i += 1;
            }
            let end = i;

            if end - start == lb.len() && hb[start..end].eq_ignore_ascii_case(lb) {
                return true;
            }
        }

        false
    }

    #[inline]
    pub fn first_label(host: &str) -> &str {
        let h = host.trim_end_matches('.');
        match h.find('.') {
            Some(i) => &h[..i],
            None => h,
        }
    }

    #[inline]
    fn is_common_cc_sld(sld: &str) -> bool {
        let s = sld.as_bytes();
        match s.len() {
            2 => matches!(
                [s[0].to_ascii_lowercase(), s[1].to_ascii_lowercase()],
                [b'c', b'o']
                    | [b'a', b'c']
                    | [b'g', b'o']
                    | [b'o', b'r']
                    | [b'n', b'e']
                    | [b'e', b'd']
                    | [b'g', b'r']
                    | [b'l', b'g']
                    | [b'a', b'd']
            ),
            3 => matches!(
                [
                    s[0].to_ascii_lowercase(),
                    s[1].to_ascii_lowercase(),
                    s[2].to_ascii_lowercase()
                ],
                [b'c', b'o', b'm']
                    | [b'n', b'e', b't']
                    | [b'o', b'r', b'g']
                    | [b'g', b'o', b'v']
                    | [b'e', b'd', b'u']
                    | [b'm', b'i', b'l']
                    | [b'n', b'i', b'c']
                    | [b's', b'c', b'h']
                    | [b'g', b'o', b'b']
            ),
            4 => matches!(
                [
                    s[0].to_ascii_lowercase(),
                    s[1].to_ascii_lowercase(),
                    s[2].to_ascii_lowercase(),
                    s[3].to_ascii_lowercase()
                ],
                [b'g', b'o', b'u', b'v']
            ),
            _ => false,
        }
    }

    #[inline]
    pub fn base_domain_from_host(host: &str) -> &str {
        let mut h = host.trim_end_matches('.');
        if let Some(x) = h.strip_prefix("www.") {
            h = x;
        }
        if let Some(x) = h.strip_prefix("m.") {
            h = x;
        }

        let last_dot = match h.rfind('.') {
            Some(p) => p,
            None => return h,
        };
        let prev_dot = match h[..last_dot].rfind('.') {
            Some(p) => p,
            None => return h,
        };

        let tld = &h[last_dot + 1..];
        let sld = &h[prev_dot + 1..last_dot];
        let mut base = &h[prev_dot + 1..];

        if tld.len() == 2 && is_common_cc_sld(sld) {
            if let Some(prev2_dot) = h[..prev_dot].rfind('.') {
                base = &h[prev2_dot + 1..];
            }
        }

        base
    }
}

// ---- NEW implementation (memchr SIMD) ----

mod new {
    use memchr::{memchr, memchr3, memrchr};

    #[inline]
    fn strip_special_schemes(url: &str) -> &str {
        let url = url.strip_prefix("blob:").unwrap_or(url);
        url.strip_prefix("filesystem:").unwrap_or(url)
    }

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

        let rest_start =
            memchr3(b'/', b'?', b'#', &bytes[host_start..]).map_or(url.len(), |i| host_start + i);

        let authority = &url[host_start..rest_start];
        if authority.is_empty() {
            return None;
        }

        let authority = match memrchr(b'@', authority.as_bytes()) {
            Some(pos) => &authority[pos + 1..],
            None => authority,
        };

        let ab = authority.as_bytes();

        if ab.first() == Some(&b'[') {
            let close = memchr(b']', ab)?;
            let host = &authority[1..close];
            return Some((host, &url[rest_start..]));
        }

        let host_end = memchr(b':', ab).unwrap_or(ab.len());
        let host = &authority[..host_end];
        if host.is_empty() {
            return None;
        }

        Some((host, &url[rest_start..]))
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

        let mut start = 0usize;
        while start < hb.len() && hb[start] == b'.' {
            start += 1;
        }

        while start < hb.len() {
            let end = memchr(b'.', &hb[start..]).map_or(hb.len(), |i| start + i);

            if end - start == lb.len() && hb[start..end].eq_ignore_ascii_case(lb) {
                return true;
            }

            start = end + 1;
            while start < hb.len() && hb[start] == b'.' {
                start += 1;
            }
        }

        false
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
    fn is_common_cc_sld(sld: &str) -> bool {
        let s = sld.as_bytes();
        match s.len() {
            2 => matches!(
                [s[0].to_ascii_lowercase(), s[1].to_ascii_lowercase()],
                [b'c', b'o']
                    | [b'a', b'c']
                    | [b'g', b'o']
                    | [b'o', b'r']
                    | [b'n', b'e']
                    | [b'e', b'd']
                    | [b'g', b'r']
                    | [b'l', b'g']
                    | [b'a', b'd']
            ),
            3 => matches!(
                [
                    s[0].to_ascii_lowercase(),
                    s[1].to_ascii_lowercase(),
                    s[2].to_ascii_lowercase()
                ],
                [b'c', b'o', b'm']
                    | [b'n', b'e', b't']
                    | [b'o', b'r', b'g']
                    | [b'g', b'o', b'v']
                    | [b'e', b'd', b'u']
                    | [b'm', b'i', b'l']
                    | [b'n', b'i', b'c']
                    | [b's', b'c', b'h']
                    | [b'g', b'o', b'b']
            ),
            4 => matches!(
                [
                    s[0].to_ascii_lowercase(),
                    s[1].to_ascii_lowercase(),
                    s[2].to_ascii_lowercase(),
                    s[3].to_ascii_lowercase()
                ],
                [b'g', b'o', b'u', b'v']
            ),
            _ => false,
        }
    }

    #[inline]
    pub fn base_domain_from_host(host: &str) -> &str {
        let mut h = host.trim_end_matches('.');
        if let Some(x) = h.strip_prefix("www.") {
            h = x;
        }
        if let Some(x) = h.strip_prefix("m.") {
            h = x;
        }

        let hb = h.as_bytes();
        let last_dot = match memrchr(b'.', hb) {
            Some(p) => p,
            None => return h,
        };
        let prev_dot = match memrchr(b'.', &hb[..last_dot]) {
            Some(p) => p,
            None => return h,
        };

        let tld = &h[last_dot + 1..];
        let sld = &h[prev_dot + 1..last_dot];
        let mut base = &h[prev_dot + 1..];

        if tld.len() == 2 && is_common_cc_sld(sld) {
            if let Some(prev2_dot) = memrchr(b'.', &hb[..prev_dot]) {
                base = &h[prev2_dot + 1..];
            }
        }

        base
    }
}

// ---- Benchmarks ----

const URLS: &[&str] = &[
    "https://user:pass@staging.mainr.com:8443/a.js?x=1#y",
    "https://example.com/path/to/resource",
    "http://[::1]:8080/path",
    "blob:https://example.com/path/to/blob",
    "https://cdn.assets.example.co.uk/js/app.min.js?v=42",
    "//protocol-relative.example.com/resource",
];

const HOSTS: &[&str] = &[
    "www.example.com",
    "staging.mainr.com",
    "a.b.example.co.uk",
    "mainr.chilipiper.com",
    "localhost",
    "cdn.assets.example.com",
];

fn bench_host_and_rest(c: &mut Criterion) {
    let mut group = c.benchmark_group("host_and_rest");

    group.bench_function("old (str::find)", |b| {
        b.iter(|| {
            for url in URLS {
                black_box(old::host_and_rest(black_box(url)));
            }
        });
    });

    group.bench_function("new (memchr SIMD)", |b| {
        b.iter(|| {
            for url in URLS {
                black_box(new::host_and_rest(black_box(url)));
            }
        });
    });

    group.finish();
}

fn bench_host_contains_label(c: &mut Criterion) {
    let mut group = c.benchmark_group("host_contains_label_icase");

    let cases = [
        ("a.b.c.mainr.example.com", "mainr"),
        ("a.b.c.mainr.example.com", "EXAMPLE"),
        ("a.b.c.mainr.example.com", "notfound"),
        ("www.example.com", "example"),
        ("localhost", "localhost"),
    ];

    group.bench_function("old (byte-by-byte)", |b| {
        b.iter(|| {
            for (host, label) in &cases {
                black_box(old::host_contains_label_icase(
                    black_box(host),
                    black_box(label),
                ));
            }
        });
    });

    group.bench_function("new (memchr SIMD)", |b| {
        b.iter(|| {
            for (host, label) in &cases {
                black_box(new::host_contains_label_icase(
                    black_box(host),
                    black_box(label),
                ));
            }
        });
    });

    group.finish();
}

fn bench_base_domain_from_host(c: &mut Criterion) {
    let mut group = c.benchmark_group("base_domain_from_host");

    group.bench_function("old (str::rfind)", |b| {
        b.iter(|| {
            for host in HOSTS {
                black_box(old::base_domain_from_host(black_box(host)));
            }
        });
    });

    group.bench_function("new (memrchr SIMD)", |b| {
        b.iter(|| {
            for host in HOSTS {
                black_box(new::base_domain_from_host(black_box(host)));
            }
        });
    });

    group.finish();
}

fn bench_first_label(c: &mut Criterion) {
    let mut group = c.benchmark_group("first_label");

    let hosts = ["www.example.com", "localhost", "a.b.c.d.e.f.example.com."];

    group.bench_function("old (str::find)", |b| {
        b.iter(|| {
            for host in &hosts {
                black_box(old::first_label(black_box(host)));
            }
        });
    });

    group.bench_function("new (memchr SIMD)", |b| {
        b.iter(|| {
            for host in &hosts {
                black_box(new::first_label(black_box(host)));
            }
        });
    });

    group.finish();
}

fn bench_strip_query_fragment(c: &mut Criterion) {
    let mut group = c.benchmark_group("strip_query_fragment");

    let paths = [
        "/a/b.js?x=1#y",
        "/path/to/resource",
        "/long/path/to/some/deeply/nested/resource.min.js?version=42&cache=true",
        "/no-query-but-has#fragment",
        "/?only-query",
    ];

    group.bench_function("old (2x str::find)", |b| {
        b.iter(|| {
            for s in &paths {
                let q = s.find('?');
                let h = s.find('#');
                let result = match (q, h) {
                    (None, None) => *s,
                    (Some(i), None) => &s[..i],
                    (None, Some(i)) => &s[..i],
                    (Some(i), Some(j)) => &s[..i.min(j)],
                };
                black_box(result);
            }
        });
    });

    group.bench_function("new (memchr2 SIMD)", |b| {
        b.iter(|| {
            for s in &paths {
                let result = match memchr::memchr2(b'?', b'#', s.as_bytes()) {
                    Some(i) => &s[..i],
                    None => *s,
                };
                black_box(result);
            }
        });
    });

    group.finish();
}

fn bench_url_path_with_leading_slash(c: &mut Criterion) {
    let mut group = c.benchmark_group("url_path_with_leading_slash");

    let urls = [
        "https://cdn.example.net/js/app.js?x=y",
        "https://example.com/path/to/resource",
        "http://long-subdomain.very-long-domain-name.example.co.uk/deeply/nested/path/resource.js",
        "https://example.com",
    ];

    group.bench_function("old (2x str::find)", |b| {
        b.iter(|| {
            for url in &urls {
                let result = (|| {
                    let idx = url.find("//")?;
                    let after = idx + 2;
                    let slash_rel = url[after..].find('/')?;
                    let slash_idx = after + slash_rel;
                    if slash_idx < url.len() {
                        Some(&url[slash_idx..])
                    } else {
                        None
                    }
                })();
                black_box(result);
            }
        });
    });

    group.bench_function("new (memmem + memchr SIMD)", |b| {
        b.iter(|| {
            for url in &urls {
                let bytes = url.as_bytes();
                let result = (|| {
                    let idx = memchr::memmem::find(bytes, b"//")?;
                    let after = idx + 2;
                    let slash_rel = memchr::memchr(b'/', &bytes[after..])?;
                    let slash_idx = after + slash_rel;
                    if slash_idx < url.len() {
                        Some(&url[slash_idx..])
                    } else {
                        None
                    }
                })();
                black_box(result);
            }
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_host_and_rest,
    bench_host_contains_label,
    bench_base_domain_from_host,
    bench_first_label,
    bench_strip_query_fragment,
    bench_url_path_with_leading_slash,
);
criterion_main!(benches);
