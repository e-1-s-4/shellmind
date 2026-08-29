//! Thin client used by the CLI to reach a running daemon.
//!
//! The client is best-effort by design: any failure (no socket, timeout,
//! malformed reply) maps to `None`, and callers fall back to computing
//! results directly. That keeps `sm` fully functional without the daemon.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use super::{Request, Response};
use crate::paths;

/// Send a request to the daemon. `timeout_ms` bounds connect + read.
pub fn request(req: &Request, timeout_ms: u64) -> Option<Response> {
    let path = paths::socket_path();
    if !path.exists() {
        return None;
    }
    let stream = UnixStream::connect(&path).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_millis(timeout_ms)))
        .ok()?;
    stream
        .set_write_timeout(Some(Duration::from_millis(timeout_ms)))
        .ok()?;
    let mut writer = stream.try_clone().ok()?;
    let line = serde_json::to_string(req).ok()?;
    writer.write_all(line.as_bytes()).ok()?;
    writer.write_all(b"\n").ok()?;
    writer.flush().ok()?;
    let mut reader = BufReader::new(stream);
    let mut buf = String::new();
    reader.read_line(&mut buf).ok()?;
    if buf.trim().is_empty() {
        return None;
    }
    serde_json::from_str(&buf).ok()
}

/// Is a daemon alive at the default socket?
pub fn ping(timeout_ms: u64) -> bool {
    matches!(request(&Request::Ping, timeout_ms), Some(Response::Pong { .. }))
}

/// Ask the daemon to shut down.
pub fn shutdown() -> bool {
    matches!(request(&Request::Shutdown, 3_000), Some(Response::Ok { .. }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_daemon_returns_none() {
        let _env = crate::testutil::env_lock();
        // A unique SHELLMIND_HOME ⇒ socket does not exist.
        let dir = std::env::temp_dir().join(format!("sm-cli-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("SHELLMIND_HOME", &dir);
        assert!(!ping(200));
        std::env::remove_var("SHELLMIND_HOME");
    }
}
