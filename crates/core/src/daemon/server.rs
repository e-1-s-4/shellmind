//! The resident daemon: warm cache + background workers.
//!
//! Threading model: one OS thread per connection (completion requests are
//! tiny and short-lived), plus two background workers:
//!
//! * **indexer** – polls shell history files and imports new entries
//!   incrementally (byte offsets remembered per file),
//! * **embedder** – backfills vector embeddings through Ollama so hybrid
//!   history search lights up over time.
//!
//! The daemon holds one SQLite connection behind a mutex. This is a
//! deliberate v0.1 choice: an async runtime (tokio) would add ~40 crates
//! to the dependency tree for a single-user local socket service where
//! thread-per-connection is simpler, starts faster, and is plenty fast.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::Shutdown as SocketShutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rusqlite::Connection;

use crate::completions::{self, spec::SpecSet, CompletionResult};
use crate::config::Config;
use crate::context::Context;
use crate::history;
use crate::paths;
use crate::store;
use crate::ai::AiEngine;

use super::{Request, Response};

const INDEX_POLL: Duration = Duration::from_secs(30);
const EMBED_POLL: Duration = Duration::from_secs(15);
const SUGGESTION_TTL: Duration = Duration::from_secs(60);
const CONTEXT_TTL: Duration = Duration::from_secs(3);
const CACHE_CAP: usize = 512;

struct Daemon {
    started: Instant,
    cfg: Config,
    conn: Mutex<Connection>,
    specs: SpecSet,
    suggestions: Mutex<HashMap<String, (CompletionResult, Instant)>>,
    contexts: Mutex<HashMap<PathBuf, (Context, Instant)>>,
    shutdown: AtomicBool,
}

impl Daemon {
    fn history_count(&self) -> i64 {
        let conn = lock_conn(&self.conn);
        store::history_count(&conn)
    }

    fn context_for(&self, cwd: &PathBuf) -> Context {
        {
            let cache = self.contexts.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((ctx, at)) = cache.get(cwd) {
                if at.elapsed() < CONTEXT_TTL {
                    return ctx.clone();
                }
            }
        }
        let conn = lock_conn(&self.conn);
        let ctx = Context::collect(cwd, Some(&conn));
        drop(conn);
        let mut cache = self.contexts.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() > 64 {
            cache.clear();
        }
        cache.insert(cwd.clone(), (ctx.clone(), Instant::now()));
        ctx
    }

    fn complete(&self, buffer: &str, cursor: Option<usize>, shell: &str) -> (CompletionResult, bool) {
        {
            let cache = self.suggestions.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((res, at)) = cache.get(buffer) {
                if at.elapsed() < SUGGESTION_TTL {
                    return (res.clone(), true);
                }
            }
        }
        let q = crate::parser::parse_for_completion(buffer, cursor);
        let _ = shell; // shell-specific rendering happens in the plugin
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let ctx = self.context_for(&cwd);
        let conn = lock_conn(&self.conn);
        let result = completions::complete(&q, &ctx, Some(&conn), &self.specs, &self.cfg);
        drop(conn);
        let mut cache = self.suggestions.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() > CACHE_CAP {
            cache.clear(); // simple reset policy keeps memory bounded
        }
        cache.insert(buffer.to_string(), (result.clone(), Instant::now()));
        (result, false)
    }

    fn record(&self, command: &str, exit_code: i32, cwd: Option<&str>, shell: Option<&str>) {
        if store::is_internal_command(command) {
            return;
        }
        let secret = crate::redact::looks_secret(command);
        if secret && self.cfg.history.ignore_secret_commands {
            // Still recorded as a failure (for `sm fix`) but never indexed.
            if exit_code != 0 {
                let conn = lock_conn(&self.conn);
                let _ = store::record_failure(&conn, command, exit_code, None);
            }
            return;
        }
        let conn = lock_conn(&self.conn);
        let entry = store::HistoryEntry {
            command: command.to_string(),
            ts: crate::util::now_ts(),
            exit_code: Some(exit_code),
            cwd: cwd.map(|s| s.to_string()),
            shell: shell.map(|s| s.to_string()),
            secret,
        };
        let _ = store::record_command(&conn, &entry);
        if exit_code != 0 {
            let _ = store::record_failure(&conn, command, exit_code, None);
        }
        // The buffer changed → invalidate ghost cache.
        let mut cache = self.suggestions.lock().unwrap_or_else(|e| e.into_inner());
        cache.clear();
    }

    fn search(&self, query: &str, limit: usize) -> (Vec<history::HistoryHit>, bool) {
        let conn = lock_conn(&self.conn);
        let cfg = self.cfg.clone();
        let engine = AiEngine::new(cfg.ai.clone(), cfg.privacy.clone());
        let model = engine.cfg.embedding_model.clone();
        let reachable = engine.ollama_reachable;
        let result = history::hybrid_search(
            &conn,
            query,
            limit,
            cfg.history.semantic_search && reachable,
            |text| engine.embed(text),
            &model,
        );
        (result.hits, result.used_vectors)
    }
}

fn lock_conn(conn: &Mutex<Connection>) -> std::sync::MutexGuard<'_, Connection> {
    conn.lock().unwrap_or_else(|e| e.into_inner())
}

/// Run the daemon on the default socket. Blocks until `Shutdown` is
/// received. Prints the socket path on startup.
pub fn run_server() {
    let cfg = Config::load();
    let _ = paths::ensure_home();
    let socket = paths::socket_path();

    // Stale socket handling.
    if socket.exists() {
        if super::client::ping(300) {
            eprintln!("shellmind-daemon: already running at {}", socket.display());
            std::process::exit(0);
        }
        let _ = std::fs::remove_file(&socket);
    }

    let conn = store::open();
    // Initial (incremental) import.
    let imported = history::import_current(&conn, cfg.history.ignore_secret_commands);
    let _ = store::trim_history(&conn, cfg.history.max_entries);

    let listener = match UnixListener::bind(&socket) {
        Ok(l) => l,
        Err(err) => {
            eprintln!(
                "shellmind-daemon: cannot bind {}: {}",
                socket.display(),
                err
            );
            std::process::exit(1);
        }
    };

    let daemon = Arc::new(Daemon {
        started: Instant::now(),
        cfg,
        conn: Mutex::new(conn),
        specs: SpecSet::load(),
        suggestions: Mutex::new(HashMap::new()),
        contexts: Mutex::new(HashMap::new()),
        shutdown: AtomicBool::new(false),
    });

    println!(
        "shellmind-daemon v{} listening on {} (indexed {} new commands)",
        crate::VERSION,
        socket.display(),
        imported
    );

    // Background workers.
    {
        let d = daemon.clone();
        std::thread::spawn(move || indexer_loop(d));
    }
    {
        let d = daemon.clone();
        std::thread::spawn(move || embedder_loop(d));
    }

    // Accept loop (nonblocking so `Shutdown` can break it promptly).
    let _ = listener.set_nonblocking(true);
    loop {
        if daemon.shutdown.load(Ordering::SeqCst) {
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let d = daemon.clone();
                std::thread::spawn(move || handle_connection(d, stream));
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => break,
        }
    }
    let _ = std::fs::remove_file(&socket);
}

fn handle_connection(daemon: Arc<Daemon>, stream: UnixStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let writer = match stream.try_clone() {
        Ok(w) => w,
        Err(_) => return,
    };
    let mut writer = writer;
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(err) => {
                let _ = write_response(
                    &mut writer,
                    &Response::Error {
                        message: format!("bad request: {}", err),
                    },
                );
                continue;
            }
        };
        let resp = dispatch(&daemon, &req);
        if write_response(&mut writer, &resp).is_err() {
            break;
        }
        if matches!(req, Request::Shutdown) {
            daemon.shutdown.store(true, Ordering::SeqCst);
            let _ = writer.shutdown(SocketShutdown::Both);
            break;
        }
    }
}

fn write_response(w: &mut UnixStream, resp: &Response) -> std::io::Result<()> {
    let line = serde_json::to_string(resp).unwrap_or_else(|_| {
        serde_json::to_string(&Response::Error {
            message: "serialization failed".into(),
        })
        .unwrap()
    });
    w.write_all(line.as_bytes())?;
    w.write_all(b"\n")
}

fn dispatch(daemon: &Daemon, req: &Request) -> Response {
    match req {
        Request::Ping => Response::Pong {
            version: crate::VERSION.to_string(),
            pid: std::process::id(),
            uptime_secs: daemon.started.elapsed().as_secs(),
            history_count: daemon.history_count(),
        },
        Request::Complete {
            buffer,
            cursor,
            shell,
        } => {
            let (result, cached) = daemon.complete(buffer, *cursor, shell);
            Response::Complete { result, cached }
        }
        Request::Record {
            command,
            exit_code,
            cwd,
            shell,
        } => {
            daemon.record(command, *exit_code, cwd.as_deref(), shell.as_deref());
            Response::Ok
        }
        Request::Search { query, limit } => {
            let (hits, used_vectors) = daemon.search(query, limit.unwrap_or(10).min(50));
            Response::Search { hits, used_vectors }
        }
        Request::EmbedBackfill => {
            let n = embed_backfill_once(&daemon.cfg, &lock_conn(&daemon.conn));
            Response::Done { embedded: n }
        }
        Request::Shutdown => Response::Ok,
    }
}

// ---------------------------------------------------------------------------
// Background workers
// ---------------------------------------------------------------------------

fn indexer_loop(daemon: Arc<Daemon>) {
    loop {
        std::thread::sleep(INDEX_POLL);
        if daemon.shutdown.load(Ordering::SeqCst) {
            return;
        }
        let conn = lock_conn(&daemon.conn);
        let ignore = daemon.cfg.history.ignore_secret_commands;
        let n = incremental_import(&conn, ignore);
        let _ = store::trim_history(&conn, daemon.cfg.history.max_entries);
        drop(conn);
        if n > 0 {
            let mut cache = daemon
                .suggestions
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            cache.clear();
        }
    }
}

/// Import only the bytes appended to each history file since the last pass.
fn incremental_import(conn: &Connection, ignore_secrets: bool) -> usize {
    let mut total = 0;
    for shell in history::detect_available_shells() {
        let Some(path) = paths::history_file_for(&shell) else {
            continue;
        };
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        let size = meta.len() as i64;
        let key = format!("import_offset:{}", path.display());
        let offset: i64 = store::meta_get(conn, &key)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        if size <= offset {
            continue;
        }
        // Read the tail and only keep complete lines.
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let new_bytes = &content[(offset as usize).min(content.len())..];
        let last_nl = new_bytes.rfind('\n').map(|i| i + 1).unwrap_or(0);
        if last_nl == 0 {
            continue;
        }
        let fresh = &new_bytes[..last_nl];
        let entries = match shell.as_str() {
            "zsh" => history::parse_zsh_history(fresh),
            "bash" => history::parse_bash_history(fresh),
            "fish" => history::parse_fish_history(fresh),
            _ => continue,
        };
        for e in entries {
            if store::is_internal_command(&e.command) {
                continue;
            }
            let secret = crate::redact::looks_secret(&e.command);
            if secret && ignore_secrets {
                continue;
            }
            let entry = store::HistoryEntry {
                command: e.command,
                ts: e.ts,
                shell: Some(shell.clone()),
                secret,
                ..Default::default()
            };
            if store::record_command(conn, &entry).is_ok() {
                total += 1;
            }
        }
        store::meta_set(conn, &key, &(offset + last_nl as i64).to_string());
    }
    total
}

fn embedder_loop(daemon: Arc<Daemon>) {
    loop {
        std::thread::sleep(EMBED_POLL);
        if daemon.shutdown.load(Ordering::SeqCst) {
            return;
        }
        if !daemon.cfg.history.semantic_search {
            continue;
        }
        let conn = lock_conn(&daemon.conn);
        embed_backfill_once(&daemon.cfg, &conn);
        drop(conn);
    }
}

/// Embed up to 64 un-embedded commands. Returns how many were embedded.
fn embed_backfill_once(cfg: &Config, conn: &Connection) -> usize {
    let engine = AiEngine::new(cfg.ai.clone(), cfg.privacy.clone());
    if !engine.ollama_reachable {
        return 0;
    }
    let ids = store::ids_missing_embeddings(conn, &engine.cfg.embedding_model, 64);
    if ids.is_empty() {
        return 0;
    }
    let commands: Vec<String> = {
        let mut stmt = match conn.prepare("SELECT id, command FROM history WHERE id IN (
            SELECT id FROM history WHERE secret = 0 ORDER BY uses DESC LIMIT 512)") {
            Ok(s) => s,
            Err(_) => return 0,
        };
        let rows: Vec<(i64, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default();
        rows.into_iter()
            .filter(|(id, _)| ids.contains(id))
            .map(|(_, c)| c)
            .collect()
    };
    let refs: Vec<&str> = commands.iter().map(|s| s.as_str()).collect();
    match engine.embed_batch(&refs) {
        Some(vectors) => {
            let mut count = 0;
            for (id, vec) in ids.into_iter().zip(vectors) {
                if store::put_embedding(conn, id, &engine.cfg.embedding_model, &vec).is_ok() {
                    count += 1;
                }
            }
            count
        }
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::client;

    #[test]
    fn daemon_serves_completions_over_socket() {
        // Env vars are process-global: serialize with every other test
        // that mutates them.
        let _env = crate::testutil::env_lock();
        // Hermetic environment for the whole test.
        let dir = std::env::temp_dir().join(format!("sm-daemon-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("SHELLMIND_HOME", &dir);

        // Seed a history file so the initial import has something.
        let hist = dir.join("fake_zsh_history");
        std::fs::write(&hist, ": 1700000000:0;git log --oneline --graph --decorate\n").unwrap();
        std::env::set_var("SHELLMIND_HISTORY_FILE", &hist);
        std::env::set_var("SHELL", "/bin/zsh");

        let handle = std::thread::spawn(|| run_server());
        // Wait for the socket.
        let mut ok = false;
        for _ in 0..100 {
            if client::ping(500) {
                ok = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(ok, "daemon did not come up");

        // Ping response carries stats.
        if let Some(Response::Pong { history_count, .. }) = client::request(&Request::Ping, 1000) {
            assert!(history_count >= 1, "expected imported history, got {history_count}");
        } else {
            panic!("no ping response");
        }

        // Completion through the socket.
        if let Some(Response::Complete { result, .. }) = client::request(
            &Request::Complete {
                buffer: "git log --".into(),
                cursor: None,
                shell: "zsh".into(),
            },
            2000,
        ) {
            assert!(result
                .suggestions
                .iter()
                .any(|s| s.insert == "--oneline"));
            assert!(result.ghost.is_some());
        } else {
            panic!("no complete response");
        }

        // Record + search.
        assert!(matches!(
            client::request(
                &Request::Record {
                    command: "echo hello".into(),
                    exit_code: 0,
                    cwd: None,
                    shell: Some("zsh".into()),
                },
                1000
            ),
            Some(Response::Ok { .. })
        ));
        if let Some(Response::Search { hits, .. }) = client::request(
            &Request::Search {
                query: "git log".into(),
                limit: Some(5),
            },
            2000,
        ) {
            assert!(hits.iter().any(|h| h.command.contains("git log")));
        } else {
            panic!("no search response");
        }

        // Shutdown.
        assert!(matches!(
            client::request(&Request::Shutdown, 2000),
            Some(Response::Ok { .. })
        ));
        let _ = handle.join();
        std::env::remove_var("SHELLMIND_HOME");
        std::env::remove_var("SHELLMIND_HISTORY_FILE");
        std::env::remove_var("SHELL");
    }
}
