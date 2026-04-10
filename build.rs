fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(feature = "adblock_easylist")]
    easylist::fetch_lists();
}

#[cfg(feature = "adblock_easylist")]
mod easylist {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::path::Path;

    const LISTS: &[(&str, &str, &str)] = &[
        ("easylist.to", "/easylist/easylist.txt", "easylist.txt"),
        ("easylist.to", "/easylist/easyprivacy.txt", "easyprivacy.txt"),
    ];

    pub fn fetch_lists() {
        let out_dir = std::env::var("OUT_DIR").unwrap();

        for &(host, path, filename) in LISTS {
            let dest = Path::new(&out_dir).join(filename);

            // Cache: skip if already downloaded and non-trivial.
            if dest.exists()
                && std::fs::metadata(&dest)
                    .map(|m| m.len() > 1024)
                    .unwrap_or(false)
            {
                continue;
            }

            match fetch_https(host, path) {
                Ok(body) => {
                    let _ = std::fs::write(&dest, &body);
                    println!("cargo:warning=Downloaded {filename} ({} bytes)", body.len());
                }
                Err(e) => {
                    // Write empty fallback so include_str! compiles.
                    if !dest.exists() {
                        let _ = std::fs::write(&dest, "");
                    }
                    println!("cargo:warning=Failed to download {filename}: {e}");
                }
            }
        }
    }

    fn fetch_https(host: &str, path: &str) -> Result<String, Box<dyn std::error::Error>> {
        let connector = native_tls::TlsConnector::new()?;
        let stream = TcpStream::connect((host, 443))?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(30)))?;
        let mut tls = connector.connect(host, stream)?;

        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nAccept-Encoding: identity\r\n\r\n"
        );
        tls.write_all(request.as_bytes())?;

        let mut buf = Vec::with_capacity(4 * 1024 * 1024);
        tls.read_to_end(&mut buf)?;

        let response = String::from_utf8_lossy(&buf);

        // Follow a single 301/302 redirect if needed.
        if response.starts_with("HTTP/1.1 301") || response.starts_with("HTTP/1.1 302") {
            if let Some(loc) = extract_header(&response, "Location") {
                if let Some((rhost, rpath)) = parse_https_url(&loc) {
                    return fetch_https_direct(&rhost, &rpath);
                }
            }
        }

        // Strip HTTP headers to get body.
        match response.find("\r\n\r\n") {
            Some(idx) => Ok(response[idx + 4..].to_string()),
            None => Err("malformed HTTP response".into()),
        }
    }

    fn fetch_https_direct(host: &str, path: &str) -> Result<String, Box<dyn std::error::Error>> {
        let connector = native_tls::TlsConnector::new()?;
        let stream = TcpStream::connect((host, 443))?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(30)))?;
        let mut tls = connector.connect(host, stream)?;

        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nAccept-Encoding: identity\r\n\r\n"
        );
        tls.write_all(request.as_bytes())?;

        let mut buf = Vec::with_capacity(4 * 1024 * 1024);
        tls.read_to_end(&mut buf)?;

        let response = String::from_utf8_lossy(&buf);
        match response.find("\r\n\r\n") {
            Some(idx) => Ok(response[idx + 4..].to_string()),
            None => Err("malformed HTTP response".into()),
        }
    }

    fn extract_header<'a>(response: &'a str, name: &str) -> Option<&'a str> {
        // Search headers case-insensitively, line by line.
        let header_end = response.find("\r\n\r\n").unwrap_or(response.len());
        let headers_section = &response[..header_end];
        let prefix = format!("{}: ", name.to_ascii_lowercase());

        for line in headers_section.split("\r\n") {
            if line.to_ascii_lowercase().starts_with(&prefix) {
                return Some(line[prefix.len()..].trim());
            }
        }
        None
    }

    fn parse_https_url(url: &str) -> Option<(String, String)> {
        let rest = url.strip_prefix("https://")?;
        let (host, path) = match rest.find('/') {
            Some(i) => (rest[..i].to_string(), rest[i..].to_string()),
            None => (rest.to_string(), "/".to_string()),
        };
        Some((host, path))
    }
}
