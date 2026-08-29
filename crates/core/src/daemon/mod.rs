//! Daemon protocol: newline-delimited JSON over a Unix socket.
//!
//! Requests and responses are single-line JSON documents. The socket path
//! lives at `$SHELLMIND_HOME/runtime/daemon.sock` (see
//! [`crate::paths::socket_path`]).

pub mod client;
pub mod server;

use serde::{Deserialize, Serialize};

use crate::completions::CompletionResult;
use crate::history::HistoryHit;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Liveness check.
    Ping,
    /// Complete a buffer (ghost text + menu suggestions).
    Complete {
        buffer: String,
        #[serde(default)]
        cursor: Option<usize>,
        shell: String,
    },
    /// Record an executed command (called asynchronously by plugins).
    Record {
        command: String,
        exit_code: i32,
        #[serde(default)]
        cwd: Option<String>,
        shell: Option<String>,
    },
    /// Semantic history search.
    Search {
        query: String,
        #[serde(default)]
        limit: Option<usize>,
    },
    /// Background embed request (internal).
    EmbedBackfill,
    /// Terminate the daemon (only reachable by the local user).
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Response {
    Pong {
        version: String,
        pid: u32,
        uptime_secs: u64,
        history_count: i64,
    },
    Complete {
        #[serde(flatten)]
        result: CompletionResult,
        cached: bool,
    },
    Ok,
    Search {
        hits: Vec<HistoryHit>,
        used_vectors: bool,
    },
    Done {
        embedded: usize,
    },
    Error {
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_roundtrips_as_ndjson() {
        let req = Request::Complete {
            buffer: "git log --".into(),
            cursor: None,
            shell: "zsh".into(),
        };
        let line = serde_json::to_string(&req).unwrap();
        assert!(!line.contains('\n'));
        let back: Request = serde_json::from_str(&line).unwrap();
        match back {
            Request::Complete { buffer, shell, .. } => {
                assert_eq!(buffer, "git log --");
                assert_eq!(shell, "zsh");
            }
            _ => panic!("wrong variant"),
        }

        let resp = Response::Pong {
            version: "0.1.0".into(),
            pid: 42,
            uptime_secs: 5,
            history_count: 7,
        };
        let line = serde_json::to_string(&resp).unwrap();
        assert!(line.contains("\"ping\"") || line.contains("Pong") || line.contains("pong"));
        let back: Response = serde_json::from_str(&line).unwrap();
        assert!(matches!(back, Response::Pong { pid: 42, .. }));
    }

    #[test]
    fn record_request_roundtrip() {
        let req = Request::Record {
            command: "docker ps".into(),
            exit_code: 0,
            cwd: Some("/tmp".into()),
            shell: Some("zsh".into()),
        };
        let line = serde_json::to_string(&req).unwrap();
        let back: Request = serde_json::from_str(&line).unwrap();
        match back {
            Request::Record { command, exit_code, .. } => {
                assert_eq!(command, "docker ps");
                assert_eq!(exit_code, 0);
            }
            _ => panic!(),
        }
    }
}
