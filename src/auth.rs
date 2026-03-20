/// Which authentication challenges these credentials should respond to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthScope {
    /// Respond to both proxy (407) and server (401) challenges.
    #[default]
    Any,
    /// Respond only to proxy authentication challenges (407).
    Proxy,
    /// Respond only to server authentication challenges (401).
    Server,
}

/// Credentials for authentications
#[derive(Debug, Clone, PartialEq)]
pub struct Credentials {
    /// username
    pub username: String,
    /// password
    pub password: String,
    /// Which challenges to respond to (default: Any).
    pub scope: AuthScope,
}
