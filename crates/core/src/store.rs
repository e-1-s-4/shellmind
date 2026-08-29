//! SQLite persistence layer (rusqlite, bundled backend).
//!
//! Schema lives in a single database under `~/.config/shellmind/history.db`:
//!
//! * `history`   – deduplicated commands with usage counts,
//! * `tokens`    – per-command token frequencies powering BM25 search,
//! * `embeddings`– optional vector embeddings (BLOB of little-endian f32),
//! * `failed`    – ring buffer of recent failed commands (for `sm fix`),
//! * `meta`      – key/value state (import offsets, schema version).

use rusqlite::{params, OptionalExtension};

pub use rusqlite::Connection;
use std::collections::HashMap;
use std::path::Path;

use crate::util::tokenize;

pub const SCHEMA_VERSION: i64 = 1;

/// A history entry as imported or recorded.
#[derive(Debug, Clone, Default)]
pub struct HistoryEntry {
    pub command: String,
    pub ts: i64,
    pub exit_code: Option<i32>,
    pub cwd: Option<String>,
    pub shell: Option<String>,
    pub secret: bool,
}

/// A stored history row.
#[derive(Debug, Clone)]
pub struct HistoryRow {
    pub id: i64,
    pub command: String,
    pub ts: i64,
    pub uses: i64,
    pub secret: bool,
}

/// A recorded failure.
#[derive(Debug, Clone)]
pub struct FailedEntry {
    pub command: String,
    pub exit_code: i32,
    pub stderr: Option<String>,
    pub ts: i64,
}

/// Open (and migrate) the database at the default location.
pub fn open() -> Connection {
    if let Some(parent) = crate::paths::db_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match open_at(&crate::paths::db_path()) {
        Ok(conn) => conn,
        Err(err) => {
            eprintln!(
                "shellmind: cannot open database at {}: {}",
                crate::paths::db_path().display(),
                err
            );
            std::process::exit(1);
        }
    }
}

/// Open (and migrate) a database at an explicit path — used by tests.
pub fn open_at(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS history (
            id        INTEGER PRIMARY KEY AUTOINCREMENT,
            command   TEXT NOT NULL UNIQUE,
            ts        INTEGER NOT NULL DEFAULT 0,
            exit_code INTEGER,
            cwd       TEXT,
            shell     TEXT,
            secret    INTEGER NOT NULL DEFAULT 0,
            uses      INTEGER NOT NULL DEFAULT 1
        );
        CREATE INDEX IF NOT EXISTS idx_history_ts ON history(ts DESC);
        CREATE TABLE IF NOT EXISTS tokens (
            history_id INTEGER NOT NULL,
            token      TEXT NOT NULL,
            count      INTEGER NOT NULL DEFAULT 1,
            PRIMARY KEY (history_id, token)
        );
        CREATE INDEX IF NOT EXISTS idx_tokens_token ON tokens(token);
        CREATE TABLE IF NOT EXISTS embeddings (
            history_id INTEGER PRIMARY KEY,
            model      TEXT NOT NULL,
            dim        INTEGER NOT NULL,
            vector     BLOB NOT NULL
        );
        CREATE TABLE IF NOT EXISTS failed (
            id        INTEGER PRIMARY KEY AUTOINCREMENT,
            command   TEXT NOT NULL,
            exit_code INTEGER NOT NULL,
            stderr    TEXT,
            ts        INTEGER NOT NULL
        );
        INSERT OR IGNORE INTO meta(key, value) VALUES('schema_version', '1');
        "#,
    )?;
    Ok(conn)
}

/// Internal commands that should never be indexed as user history.
pub fn is_internal_command(cmd: &str) -> bool {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return true;
    }
    let first = trimmed.split_whitespace().next().unwrap_or("");
    let base = first.rsplit('/').next().unwrap_or(first);
    matches!(base, "sm" | "shellmind" | "shellmind-daemon" | "sshd")
        || trimmed.starts_with("eval")
}

/// Insert or refresh a history entry (deduplicated by command text).
pub fn record_command(conn: &Connection, e: &HistoryEntry) -> rusqlite::Result<()> {
    if e.command.trim().is_empty() {
        return Ok(());
    }
    conn.execute(
        r#"INSERT INTO history(command, ts, exit_code, cwd, shell, secret, uses)
           VALUES(?1, ?2, ?3, ?4, ?5, ?6, 1)
           ON CONFLICT(command) DO UPDATE SET
             ts = excluded.ts,
             exit_code = COALESCE(excluded.exit_code, history.exit_code),
             cwd = COALESCE(excluded.cwd, history.cwd),
             shell = COALESCE(excluded.shell, history.shell),
             uses = history.uses + 1"#,
        params![
            e.command.trim(),
            e.ts,
            e.exit_code,
            e.cwd,
            e.shell,
            e.secret as i64
        ],
    )?;
    let id: i64 = conn
        .query_row(
            "SELECT id FROM history WHERE command = ?1",
            params![e.command.trim()],
            |r| r.get(0),
        )
        .unwrap_or(0);
    for tok in tokenize(&e.command) {
        if tok.len() < 2 {
            continue;
        }
        conn.execute(
            r#"INSERT INTO tokens(history_id, token, count)
               VALUES(?1, ?2, 1)
               ON CONFLICT(history_id, token) DO UPDATE SET count = count + 1"#,
            params![id, tok],
        )?;
    }
    Ok(())
}

/// Record a failed command (non-zero exit) into the failure ring buffer.
pub fn record_failure(
    conn: &Connection,
    command: &str,
    exit_code: i32,
    stderr: Option<&str>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO failed(command, exit_code, stderr, ts) VALUES(?1, ?2, ?3, ?4)",
        params![
            command,
            exit_code,
            stderr,
            crate::util::now_ts()
        ],
    )?;
    conn.execute(
        "DELETE FROM failed WHERE id NOT IN (SELECT id FROM failed ORDER BY id DESC LIMIT 50)",
        [],
    )?;
    Ok(())
}

/// Most recent failed command, if any.
pub fn last_failed(conn: &Connection) -> Option<FailedEntry> {
    conn.query_row(
        "SELECT command, exit_code, stderr, ts FROM failed ORDER BY id DESC LIMIT 1",
        [],
        |r| {
            Ok(FailedEntry {
                command: r.get(0)?,
                exit_code: r.get(1)?,
                stderr: r.get(2)?,
                ts: r.get(3)?,
            })
        },
    )
    .optional()
    .ok()
    .flatten()
}

/// Most recent non-internal command from history.
pub fn last_user_command(conn: &Connection) -> Option<String> {
    let mut stmt = match conn.prepare("SELECT command FROM history ORDER BY ts DESC, id DESC LIMIT 25")
    {
        Ok(s) => s,
        Err(_) => return None,
    };
    let rows: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .ok()?
        .filter_map(|r| r.ok())
        .collect();
    rows.into_iter()
        .find(|c| !is_internal_command(c))
}

/// Total number of indexed commands.
pub fn history_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM history WHERE secret = 0", [], |r| r.get(0))
        .unwrap_or(0)
}

/// Commands matching a literal prefix (used for ghost-text suggestions).
pub fn prefix_matches(conn: &Connection, prefix: &str, limit: usize) -> Vec<HistoryRow> {
    let like = format!("{}%", crate::paths::like_escape(prefix));
    let Ok(mut stmt) =
        conn.prepare("SELECT id, command, ts, uses, secret FROM history WHERE command LIKE ?1 ESCAPE '\\' AND secret = 0 ORDER BY uses DESC, ts DESC LIMIT ?2")
    else {
        return Vec::new();
    };
    let rows = stmt
        .query_map(params![like, limit as i64], |r| {
            Ok(HistoryRow {
                id: r.get(0)?,
                command: r.get(1)?,
                ts: r.get(2)?,
                uses: r.get(3)?,
                secret: r.get(4)?,
            })
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default();
    rows
}

/// All non-secret rows, for building the in-memory search index.
pub fn load_rows_for_search(conn: &Connection) -> Vec<HistoryRow> {
    let Ok(mut stmt) = conn
        .prepare("SELECT id, command, ts, uses, secret FROM history WHERE secret = 0")
    else {
        return Vec::new();
    };
    stmt.query_map([], |r| {
        Ok(HistoryRow {
            id: r.get(0)?,
            command: r.get(1)?,
            ts: r.get(2)?,
            uses: r.get(3)?,
            secret: r.get(4)?,
        })
    })
    .map(|rows| rows.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

/// Token frequencies for a set of history ids.
pub fn load_tokens(conn: &Connection, ids: &[i64]) -> HashMap<i64, HashMap<String, u32>> {
    let mut out: HashMap<i64, HashMap<String, u32>> = HashMap::new();
    if ids.is_empty() {
        return out;
    }
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT history_id, token, count FROM tokens WHERE history_id IN ({})",
        placeholders
    );
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return out;
    };
    let mut params_vec: Vec<&dyn rusqlite::ToSql> = ids.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
    if let Ok(rows) = stmt.query_map(params_vec.as_slice(), |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, u32>(2)?))
    }) {
        for row in rows.flatten() {
            out.entry(row.0)
                .or_default()
                .insert(row.1, row.2);
        }
    }
    out
}

/// Enforce `max_entries` by keeping the newest rows.
pub fn trim_history(conn: &Connection, max_entries: usize) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM history WHERE id NOT IN (SELECT id FROM history ORDER BY ts DESC, id DESC LIMIT ?1)",
        params![max_entries as i64],
    )?;
    conn.execute(
        "DELETE FROM tokens WHERE history_id NOT IN (SELECT id FROM history)",
        [],
    )?;
    conn.execute(
        "DELETE FROM embeddings WHERE history_id NOT IN (SELECT id FROM history)",
        [],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Embeddings
// ---------------------------------------------------------------------------

fn encode_vector(v: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(v.len() * 4);
    for f in v {
        bytes.extend_from_slice(&f.to_le_bytes());
    }
    bytes
}

fn decode_vector(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Store an embedding for a history entry.
pub fn put_embedding(
    conn: &Connection,
    history_id: i64,
    model: &str,
    vector: &[f32],
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO embeddings(history_id, model, dim, vector) VALUES(?1, ?2, ?3, ?4)
         ON CONFLICT(history_id) DO UPDATE SET model=excluded.model, dim=excluded.dim, vector=excluded.vector",
        params![history_id, model, vector.len() as i64, encode_vector(vector)],
    )?;
    Ok(())
}

/// Load embeddings for specific ids (for candidate re-ranking).
pub fn get_embeddings(
    conn: &Connection,
    model: &str,
    ids: &[i64],
) -> HashMap<i64, Vec<f32>> {
    let mut out = HashMap::new();
    if ids.is_empty() {
        return out;
    }
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT history_id, vector FROM embeddings WHERE model = ? AND history_id IN ({})",
        placeholders
    );
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return out;
    };
    let mut params_vec: Vec<&dyn rusqlite::ToSql> = vec![&model];
    params_vec.extend(ids.iter().map(|i| i as &dyn rusqlite::ToSql));
    if let Ok(rows) = stmt.query_map(params_vec.as_slice(), |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?))
    }) {
        for (id, blob) in rows.flatten() {
            out.insert(id, decode_vector(&blob));
        }
    }
    out
}

/// History ids that still lack an embedding for `model`.
pub fn ids_missing_embeddings(conn: &Connection, model: &str, limit: usize) -> Vec<i64> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT h.id FROM history h
         LEFT JOIN embeddings e ON e.history_id = h.id AND e.model = ?
         WHERE h.secret = 0 AND e.history_id IS NULL
         ORDER BY h.uses DESC, h.ts DESC LIMIT ?",
    ) else {
        return Vec::new();
    };
    stmt.query_map(params![model, limit as i64], |r| r.get::<_, i64>(0))
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

/// Number of stored embeddings for a model.
pub fn embedding_count(conn: &Connection, model: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM embeddings WHERE model = ?",
        params![model],
        |r| r.get(0),
    )
    .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Meta
// ---------------------------------------------------------------------------

pub fn meta_get(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
        r.get(0)
    })
    .optional()
    .ok()
    .flatten()
}

pub fn meta_set(conn: &Connection, key: &str, value: &str) {
    let _ = conn.execute(
        "INSERT INTO meta(key, value) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdb(name: &str) -> Connection {
        let dir = std::env::temp_dir().join(format!("shellmind-test-{}-{}", name, std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        open_at(&dir.join("t.db")).unwrap()
    }

    #[test]
    fn record_and_query() {
        let conn = tmpdb("record");
        record_command(
            &conn,
            &HistoryEntry {
                command: "docker ps".into(),
                ts: 100,
                ..Default::default()
            },
        )
        .unwrap();
        record_command(
            &conn,
            &HistoryEntry {
                command: "docker ps".into(),
                ts: 200,
                ..Default::default()
            },
        )
        .unwrap();
        record_command(
            &conn,
            &HistoryEntry {
                command: "git status".into(),
                ts: 150,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(history_count(&conn), 2);
        let rows = prefix_matches(&conn, "docker", 10);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].uses, 2);
        assert_eq!(rows[0].ts, 200);
    }

    #[test]
    fn failures_ring_buffer() {
        let conn = tmpdb("failed");
        record_failure(&conn, "git push origin main", 128, Some("fatal: The current branch main has no upstream branch.")).unwrap();
        record_failure(&conn, "false", 1, None).unwrap();
        let last = last_failed(&conn).unwrap();
        assert_eq!(last.command, "false");
        assert_eq!(last.exit_code, 1);
    }

    #[test]
    fn embeddings_roundtrip() {
        let conn = tmpdb("embed");
        record_command(&conn, &HistoryEntry { command: "tar -czvf a.tar.gz b/".into(), ts: 1, ..Default::default() }).unwrap();
        let id: i64 = conn
            .query_row("SELECT id FROM history", [], |r| r.get(0))
            .unwrap();
        let vec = vec![0.1f32, -0.5, 0.9];
        put_embedding(&conn, id, "nomic-embed-text", &vec).unwrap();
        let got = get_embeddings(&conn, "nomic-embed-text", &[id]);
        assert_eq!(got.get(&id).unwrap(), &vec);
        assert_eq!(embedding_count(&conn, "nomic-embed-text"), 1);
    }

    #[test]
    fn internal_commands_filtered() {
        assert!(is_internal_command("sm status"));
        assert!(is_internal_command("  "));
        assert!(!is_internal_command("git status"));
    }

    #[test]
    fn trim_keeps_newest() {
        let conn = tmpdb("trim");
        for i in 0..10 {
            record_command(
                &conn,
                &HistoryEntry {
                    command: format!("cmd{}", i),
                    ts: 100 + i,
                    ..Default::default()
                },
            )
            .unwrap();
        }
        trim_history(&conn, 5).unwrap();
        assert_eq!(history_count(&conn), 5);
        let last = last_user_command(&conn).unwrap();
        assert_eq!(last, "cmd9");
    }
}
