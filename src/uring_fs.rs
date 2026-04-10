//! Async file I/O with optional io_uring acceleration for chromey.
//!
//! On Linux with the `io_uring` feature, file operations are dispatched to a
//! dedicated worker thread that drives a raw io_uring ring. On all other
//! platforms, operations fall back to `tokio::fs`.
//!
//! The worker also supports TCP connect via io_uring Socket + Connect opcodes,
//! used for the initial CDP WebSocket connection.

// ── io_uring implementation ─────────────────────────────────────────────────

#[cfg(all(target_os = "linux", feature = "io_uring"))]
mod inner {
    use std::ffi::CString;
    use std::io;
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::{mpsc, oneshot};

    static URING_ENABLED: AtomicBool = AtomicBool::new(false);
    static URING_POOL: std::sync::OnceLock<mpsc::UnboundedSender<IoTask>> =
        std::sync::OnceLock::new();

    enum IoTask {
        WriteFile {
            path: String,
            data: Vec<u8>,
            tx: oneshot::Sender<io::Result<()>>,
        },
        ReadFile {
            path: String,
            tx: oneshot::Sender<io::Result<Vec<u8>>>,
        },
        TcpConnect {
            addr: SocketAddr,
            tx: oneshot::Sender<io::Result<std::net::TcpStream>>,
        },
    }

    fn probe_io_uring() -> Option<io_uring::IoUring> {
        match io_uring::IoUring::builder().build(64) {
            Ok(ring) => {
                tracing::info!("chromey: io_uring probe succeeded");
                Some(ring)
            }
            Err(e) => {
                tracing::info!("chromey: io_uring unavailable ({}), using tokio::fs", e);
                None
            }
        }
    }

    fn submit_and_reap(ring: &mut io_uring::IoUring) -> io::Result<i32> {
        ring.submit_and_wait(1)?;
        let cqe = ring
            .completion()
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "io_uring: no CQE after wait"))?;
        Ok(cqe.result())
    }

    fn uring_close(ring: &mut io_uring::IoUring, fd: i32) -> io::Result<()> {
        let close_e = io_uring::opcode::Close::new(io_uring::types::Fd(fd))
            .build()
            .user_data(0xC105E);
        unsafe {
            ring.submission()
                .push(&close_e)
                .map_err(|_| io::Error::new(io::ErrorKind::Other, "io_uring: SQ full on close"))?;
        }
        let res = submit_and_reap(ring)?;
        if res < 0 {
            return Err(io::Error::from_raw_os_error(-res));
        }
        Ok(())
    }

    fn uring_write_file(ring: &mut io_uring::IoUring, path: &str, data: &[u8]) -> io::Result<()> {
        let c_path =
            CString::new(path).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

        let open_e =
            io_uring::opcode::OpenAt::new(io_uring::types::Fd(libc::AT_FDCWD), c_path.as_ptr())
                .flags(libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC)
                .mode(0o644)
                .build()
                .user_data(0x0BE4);
        unsafe {
            ring.submission()
                .push(&open_e)
                .map_err(|_| io::Error::new(io::ErrorKind::Other, "io_uring: SQ full on open"))?;
        }
        let fd = submit_and_reap(ring)?;
        if fd < 0 {
            return Err(io::Error::from_raw_os_error(-fd));
        }

        let write_result = uring_write_all(ring, fd, data);
        let close_result = uring_close(ring, fd);
        write_result?;
        close_result
    }

    fn uring_write_all(ring: &mut io_uring::IoUring, fd: i32, data: &[u8]) -> io::Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        let mut offset: u64 = 0;
        while (offset as usize) < data.len() {
            let remaining = &data[offset as usize..];
            let chunk_len = remaining.len().min(u32::MAX as usize) as u32;
            let write_e = io_uring::opcode::Write::new(
                io_uring::types::Fd(fd),
                remaining.as_ptr(),
                chunk_len,
            )
            .offset(offset)
            .build()
            .user_data(0x1417E);
            unsafe {
                ring.submission().push(&write_e).map_err(|_| {
                    io::Error::new(io::ErrorKind::Other, "io_uring: SQ full on write")
                })?;
            }
            let written = submit_and_reap(ring)?;
            if written < 0 {
                return Err(io::Error::from_raw_os_error(-written));
            }
            if written == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "io_uring: write returned 0",
                ));
            }
            offset += written as u64;
        }
        Ok(())
    }

    fn uring_read_file(ring: &mut io_uring::IoUring, path: &str) -> io::Result<Vec<u8>> {
        let meta = std::fs::metadata(path)?;
        let len = meta.len() as usize;
        if len == 0 {
            return Ok(Vec::new());
        }

        let c_path =
            CString::new(path).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        let open_e =
            io_uring::opcode::OpenAt::new(io_uring::types::Fd(libc::AT_FDCWD), c_path.as_ptr())
                .flags(libc::O_RDONLY)
                .build()
                .user_data(0x0BE4);
        unsafe {
            ring.submission()
                .push(&open_e)
                .map_err(|_| io::Error::new(io::ErrorKind::Other, "io_uring: SQ full on open"))?;
        }
        let fd = submit_and_reap(ring)?;
        if fd < 0 {
            return Err(io::Error::from_raw_os_error(-fd));
        }

        let mut buf = vec![0u8; len];
        let read_result = uring_read_exact(ring, fd, &mut buf);
        let close_result = uring_close(ring, fd);
        read_result?;
        close_result?;
        Ok(buf)
    }

    fn uring_read_exact(ring: &mut io_uring::IoUring, fd: i32, buf: &mut [u8]) -> io::Result<()> {
        let mut offset: u64 = 0;
        while (offset as usize) < buf.len() {
            let remaining = &mut buf[offset as usize..];
            let chunk_len = remaining.len().min(u32::MAX as usize) as u32;
            let read_e = io_uring::opcode::Read::new(
                io_uring::types::Fd(fd),
                remaining.as_mut_ptr(),
                chunk_len,
            )
            .offset(offset)
            .build()
            .user_data(0x4EAD);
            unsafe {
                ring.submission().push(&read_e).map_err(|_| {
                    io::Error::new(io::ErrorKind::Other, "io_uring: SQ full on read")
                })?;
            }
            let n = submit_and_reap(ring)?;
            if n < 0 {
                return Err(io::Error::from_raw_os_error(-n));
            }
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "io_uring: read returned 0",
                ));
            }
            offset += n as u64;
        }
        Ok(())
    }

    fn uring_tcp_connect(
        ring: &mut io_uring::IoUring,
        addr: SocketAddr,
    ) -> io::Result<std::net::TcpStream> {
        use std::os::unix::io::FromRawFd;

        let domain = match addr {
            SocketAddr::V4(_) => libc::AF_INET,
            SocketAddr::V6(_) => libc::AF_INET6,
        };

        let socket_e = io_uring::opcode::Socket::new(
            domain,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        )
        .build()
        .user_data(0x50CE7);
        unsafe {
            ring.submission()
                .push(&socket_e)
                .map_err(|_| io::Error::new(io::ErrorKind::Other, "io_uring: SQ full on socket"))?;
        }
        let fd = submit_and_reap(ring)?;
        if fd < 0 {
            return Err(io::Error::from_raw_os_error(-fd));
        }

        let (sa_ptr, sa_len) = match addr {
            SocketAddr::V4(v4) => {
                let sa = libc::sockaddr_in {
                    sin_family: libc::AF_INET as libc::sa_family_t,
                    sin_port: v4.port().to_be(),
                    sin_addr: libc::in_addr {
                        s_addr: u32::from_ne_bytes(v4.ip().octets()),
                    },
                    sin_zero: [0; 8],
                };
                let ptr = &sa as *const libc::sockaddr_in as *const libc::sockaddr;
                (ptr, std::mem::size_of::<libc::sockaddr_in>() as u32)
            }
            SocketAddr::V6(v6) => {
                let sa = libc::sockaddr_in6 {
                    sin6_family: libc::AF_INET6 as libc::sa_family_t,
                    sin6_port: v6.port().to_be(),
                    sin6_flowinfo: v6.flowinfo(),
                    sin6_addr: libc::in6_addr {
                        s6_addr: v6.ip().octets(),
                    },
                    sin6_scope_id: v6.scope_id(),
                };
                let ptr = &sa as *const libc::sockaddr_in6 as *const libc::sockaddr;
                (ptr, std::mem::size_of::<libc::sockaddr_in6>() as u32)
            }
        };

        let connect_e = io_uring::opcode::Connect::new(io_uring::types::Fd(fd), sa_ptr, sa_len)
            .build()
            .user_data(0xC044);
        unsafe {
            ring.submission().push(&connect_e).map_err(|_| {
                libc::close(fd);
                io::Error::new(io::ErrorKind::Other, "io_uring: SQ full on connect")
            })?;
        }

        let res = submit_and_reap(ring)?;
        if res < 0 && res != -libc::EINPROGRESS {
            let _ = uring_close(ring, fd);
            return Err(io::Error::from_raw_os_error(-res));
        }

        let stream = unsafe { std::net::TcpStream::from_raw_fd(fd) };
        Ok(stream)
    }

    fn worker_loop(mut rx: mpsc::UnboundedReceiver<IoTask>, mut ring: io_uring::IoUring) {
        while let Some(task) = rx.blocking_recv() {
            match task {
                IoTask::WriteFile { path, data, tx } => {
                    let _ = tx.send(uring_write_file(&mut ring, &path, &data));
                }
                IoTask::ReadFile { path, tx } => {
                    let _ = tx.send(uring_read_file(&mut ring, &path));
                }
                IoTask::TcpConnect { addr, tx } => {
                    let _ = tx.send(uring_tcp_connect(&mut ring, addr));
                }
            }
        }
        drop(ring);
    }

    // ── Public API ──────────────────────────────────────────────────────────

    pub fn init() -> bool {
        if URING_ENABLED.load(Ordering::Acquire) {
            return true;
        }
        let ring = match probe_io_uring() {
            Some(r) => r,
            None => return false,
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let builder = std::thread::Builder::new().name("chromey-uring-worker".into());
        match builder.spawn(move || worker_loop(rx, ring)) {
            Ok(_) => {
                if URING_POOL.set(tx).is_ok() {
                    URING_ENABLED.store(true, Ordering::Release);
                }
            }
            Err(e) => {
                tracing::warn!("Failed to spawn chromey io_uring worker: {}", e);
                return false;
            }
        }
        URING_ENABLED.load(Ordering::Acquire)
    }

    async fn try_uring<T>(
        make_task: impl FnOnce(oneshot::Sender<io::Result<T>>) -> IoTask,
    ) -> Option<io::Result<T>> {
        if !URING_ENABLED.load(Ordering::Acquire) {
            return None;
        }
        let sender = URING_POOL.get()?;
        let (tx, rx) = oneshot::channel();
        if sender.send(make_task(tx)).is_err() {
            return Some(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "chromey io_uring worker channel closed",
            )));
        }
        match rx.await {
            Ok(result) => Some(result),
            Err(_) => Some(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "chromey io_uring worker dropped the response",
            ))),
        }
    }

    pub async fn write_file(path: String, data: Vec<u8>) -> io::Result<()> {
        if URING_ENABLED.load(Ordering::Acquire) {
            if let Some(sender) = URING_POOL.get() {
                let (tx, rx) = oneshot::channel();
                if sender.send(IoTask::WriteFile { path, data, tx }).is_err() {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "chromey io_uring worker channel closed",
                    ));
                }
                return match rx.await {
                    Ok(result) => result,
                    Err(_) => Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "chromey io_uring worker dropped the response",
                    )),
                };
            }
        }
        tokio::fs::write(path, data).await
    }

    pub async fn read_file(path: String) -> io::Result<Vec<u8>> {
        if URING_ENABLED.load(Ordering::Acquire) {
            if let Some(sender) = URING_POOL.get() {
                let (tx, rx) = oneshot::channel();
                if sender.send(IoTask::ReadFile { path, tx }).is_err() {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "chromey io_uring worker channel closed",
                    ));
                }
                return match rx.await {
                    Ok(result) => result,
                    Err(_) => Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "chromey io_uring worker dropped the response",
                    )),
                };
            }
        }
        tokio::fs::read(path).await
    }

    pub async fn tcp_connect(addr: SocketAddr) -> io::Result<std::net::TcpStream> {
        if let Some(result) = try_uring(|tx| IoTask::TcpConnect { addr, tx }).await {
            return result;
        }
        tokio::task::spawn_blocking(move || std::net::TcpStream::connect(addr))
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
    }

    pub fn is_enabled() -> bool {
        URING_ENABLED.load(Ordering::Acquire)
    }
}

// ── Fallback (non-Linux / no io_uring feature) ──────────────────────────────

#[cfg(not(all(target_os = "linux", feature = "io_uring")))]
mod inner {
    use std::io;
    use std::net::SocketAddr;

    pub fn init() -> bool {
        false
    }

    pub async fn write_file(path: String, data: Vec<u8>) -> io::Result<()> {
        tokio::fs::write(&path, &data).await
    }

    pub async fn read_file(path: String) -> io::Result<Vec<u8>> {
        tokio::fs::read(&path).await
    }

    pub async fn tcp_connect(addr: SocketAddr) -> io::Result<std::net::TcpStream> {
        tokio::task::spawn_blocking(move || std::net::TcpStream::connect(addr))
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
    }

    pub fn is_enabled() -> bool {
        false
    }
}

// ── Re-exports ──────────────────────────────────────────────────────────────

pub use inner::init;
pub use inner::is_enabled;
pub use inner::read_file;
pub use inner::tcp_connect;
pub use inner::write_file;

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_write_read_roundtrip() {
        let path = std::env::temp_dir()
            .join("chromey_uring_test_roundtrip")
            .display()
            .to_string();
        let payload = b"chromey uring test".to_vec();

        write_file(path.clone(), payload.clone()).await.unwrap();
        let read_back = read_file(path.clone()).await.unwrap();
        assert_eq!(read_back, payload);

        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn test_tcp_connect_loopback() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let accept = tokio::spawn(async move { listener.accept().await });
        let connect = tokio::spawn(async move { tcp_connect(addr).await });

        let (a, c) = tokio::join!(accept, connect);
        assert!(a.unwrap().is_ok());
        assert!(c.unwrap().is_ok());
    }

    #[tokio::test]
    async fn test_tcp_connect_refused() {
        let addr: std::net::SocketAddr = "127.0.0.1:1".parse().unwrap();
        assert!(tcp_connect(addr).await.is_err());
    }

    #[tokio::test]
    async fn test_init_idempotent() {
        let r1 = init();
        let r2 = init();
        assert_eq!(r1, r2);
    }
}
