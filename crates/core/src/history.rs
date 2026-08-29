//! Shell history import and semantic search.
//!
//! ## Import
//!
//! Parses native history formats:
//!
//! * zsh  – `~/.zsh_history` incl. extended format (`: <ts>:<dur>;<cmd>`),
//! * bash – `~/.bash_history` (plain lines, optional `#<epoch>` stamps),
//! * fish – `~/.local/share/fish/fish_history` (`- cmd:` / `when:` YAML).
//!
//! Commands that look secret (see [`crate::redact::looks_secret`]) are
//! never indexed when `history.ignore_secret_commands` is enabled.
//!
//! ## Search
//!
//! Ranking is hybrid:
//!
//! 1. **BM25** over command tokens, with a domain synonym expansion so
//!    natural queries match CLI vocabulary ("remove unused images" →
//!    `docker image prune -a`),
//! 2. **Vector re-ranking** — when an Ollama embedding model is reachable,
//!    the lexical top-N candidates are embedded and re-ranked by cosine
//!    similarity, merged with reciprocal-rank fusion.
//!
//! Both paths degrade gracefully: no model → pure lexical.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use crate::store::{self, HistoryEntry, HistoryRow};
use crate::util::tokenize;

/// One imported raw entry.
#[derive(Debug, Clone)]
pub struct RawEntry {
    pub ts: i64,
    pub command: String,
}

// ---------------------------------------------------------------------------
// Importers
// ---------------------------------------------------------------------------

/// Parse zsh history content. Handles both plain lines and the extended
/// `: <epoch>:<duration>;<command>` format.
pub fn parse_zsh_history(content: &str) -> Vec<RawEntry> {
    let mut out = Vec::new();
    let mut pending: Option<String> = None;
    for line in content.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let mut line = line.to_string();
        if let Some(p) = pending.take() {
            line = format!("{}{}", p, line);
        }
        if line.ends_with('\\') {
            pending = Some(line.trim_end_matches('\\').to_string());
            continue;
        }
        let (ts, cmd) = if let Some(rest) = line.strip_prefix(':') {
            // extended: ": 1699999999:0;docker ps"
            match rest.split_once(';') {
                Some((meta, cmd)) => {
                    let ts = meta
                        .split(':')
                        .next()
                        .and_then(|s| s.trim().parse::<i64>().ok())
                        .unwrap_or(0);
                    (ts, cmd.to_string())
                }
                None => (0, line.clone()),
            }
        } else {
            (0, line.clone())
        };
        if !cmd.trim().is_empty() {
            out.push(RawEntry { ts, command: cmd });
        }
    }
    out
}

/// Parse bash history content (plain lines, `#<epoch>` timestamps).
pub fn parse_bash_history(content: &str) -> Vec<RawEntry> {
    let mut out = Vec::new();
    let mut pending_ts: i64 = 0;
    for line in content.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(ts) = line.strip_prefix('#') {
            if let Ok(t) = ts.trim().parse::<i64>() {
                pending_ts = t;
                continue;
            }
        }
        if line.trim().is_empty() {
            continue;
        }
        out.push(RawEntry {
            ts: pending_ts,
            command: line.to_string(),
        });
        pending_ts = 0;
    }
    out
}

/// Parse fish history content (`- cmd: x` / `when: ts`).
pub fn parse_fish_history(content: &str) -> Vec<RawEntry> {
    let mut out = Vec::new();
    let mut cur_cmd: Option<String> = None;
    let mut cur_ts: i64 = 0;
    for line in content.lines() {
        let trimmed = line.trim_start();
        if let Some(cmd) = trimmed.strip_prefix("- cmd: ") {
            if let Some(c) = cur_cmd.take() {
                if !c.trim().is_empty() {
                    out.push(RawEntry { ts: cur_ts, command: c });
                }
            }
            cur_cmd = Some(cmd.trim_matches('"').to_string());
            cur_ts = 0;
        } else if let Some(ts) = trimmed.strip_prefix("when: ") {
            cur_ts = ts.trim().parse::<i64>().unwrap_or(0);
        } else if trimmed.starts_with("paths:") {
            // paths block follows a cmd; flush current entry
            if let Some(c) = cur_cmd.take() {
                if !c.trim().is_empty() {
                    out.push(RawEntry { ts: cur_ts, command: c });
                }
            }
            cur_ts = 0;
        }
    }
    if let Some(c) = cur_cmd {
        if !c.trim().is_empty() {
            out.push(RawEntry { ts: cur_ts, command: c });
        }
    }
    out
}

/// Import a single history file into the database.
pub fn import_file(
    conn: &Connection,
    path: &Path,
    shell: &str,
    ignore_secrets: bool,
) -> usize {
    let Ok(content) = std::fs::read_to_string(path) else {
        return 0;
    };
    let entries = match shell {
        "zsh" => parse_zsh_history(&content),
        "bash" => parse_bash_history(&content),
        "fish" => parse_fish_history(&content),
        _ => return 0,
    };
    let mut count = 0usize;
    for e in entries {
        if store::is_internal_command(&e.command) {
            continue;
        }
        let secret = crate::redact::looks_secret(&e.command);
        if secret && ignore_secrets {
            continue;
        }
        let entry = HistoryEntry {
            command: e.command,
            ts: e.ts,
            shell: Some(shell.to_string()),
            secret,
            ..Default::default()
        };
        if store::record_command(conn, &entry).is_ok() {
            count += 1;
        }
    }
    count
}

/// Import the current shell's history file (auto-detected), plus any other
/// shell histories found on disk. Returns number of commands imported.
pub fn import_current(conn: &Connection, ignore_secrets: bool) -> usize {
    let mut total = 0;
    let shells = detect_available_shells();
    for shell in shells {
        if let Some(path) = crate::paths::history_file_for(&shell) {
            if path.exists() {
                total += import_file(conn, &path, &shell, ignore_secrets);
            }
        }
    }
    total
}

/// The shell the user is currently running, when recognizable.
fn preferred_shell() -> Option<String> {
    std::env::var("SHELL")
        .ok()
        .map(|s| {
            s.rsplit('/')
                .next()
                .unwrap_or("")
                .split('-')
                .next()
                .unwrap_or("")
                .to_lowercase()
        })
        .filter(|s| matches!(s.as_str(), "zsh" | "bash" | "fish"))
}

/// Which shell histories exist on this machine.
///
/// With `SHELLMIND_HISTORY_FILE` set (tests, demo), only the preferred
/// shell is considered — the override points at ONE file in ONE format.
pub fn detect_available_shells() -> Vec<String> {
    if let Ok(p) = std::env::var("SHELLMIND_HISTORY_FILE") {
        if !p.is_empty() {
            return vec![preferred_shell().unwrap_or_else(|| "zsh".into())];
        }
    }
    let mut shells = Vec::new();
    if let Some(p) = preferred_shell() {
        shells.push(p);
    }
    for s in ["zsh", "bash", "fish"] {
        if !shells.iter().any(|x| x == s) {
            if let Some(p) = crate::paths::history_file_for(s) {
                if p.exists() {
                    shells.push(s.to_string());
                }
            }
        }
    }
    shells
}

// ---------------------------------------------------------------------------
// BM25 search with synonym expansion
// ---------------------------------------------------------------------------

/// Domain synonyms mapping natural-language vocabulary to CLI vocabulary.
/// One-hop expansion, applied to *queries* only.
pub fn synonym_expansion(tokens: &[String]) -> Vec<String> {
    const SYN: &[(&str, &[&str])] = &[
        ("remove", &["rm", "delete", "prune", "del", "clean"]),
        ("delete", &["rm", "remove", "drop", "prune"]),
        ("unused", &["dangling", "unused", "stale", "old"]),
        ("image", &["images", "img"]),
        ("images", &["image", "img"]),
        ("container", &["containers", "ps", "docker"]),
        ("containers", &["container", "ps", "docker"]),
        ("backup", &["dump", "pg_dump", "bak", "export"]),
        ("postgres", &["pg", "pgsql", "psql", "pg_dump"]),
        ("compress", &["tar", "gzip", "zip", "czvf"]),
        ("extract", &["untar", "unzip", "xzvf", "decompress"]),
        ("archive", &["tar", "zip", "gz"]),
        ("disk", &["du", "df", "storage", "space"]),
        ("folder", &["dir", "directory", "folders", "path"]),
        ("directory", &["dir", "folder", "folders"]),
        ("file", &["files", "find", "fd"]),
        ("files", &["file", "find", "fd"]),
        ("larger", &["size", "big", "large"]),
        ("large", &["big", "size", "largest"]),
        ("big", &["large", "size", "largest"]),
        ("kill", &["terminate", "stop", "pkill", "kill"]),
        ("port", &["lsof", "netstat", "listen", "ports"]),
        ("branch", &["branches", "ref", "checkout", "head"]),
        ("undo", &["reset", "revert", "rollback"]),
        ("commit", &["commits", "head"]),
        ("list", &["ls", "cat", "print", "ps", "show"]),
        ("logs", &["log", "tail", "journalctl"]),
        ("restart", &["reload", "stop", "start"]),
        ("pods", &["pod", "po"]),
        ("pod", &["pods", "po"]),
        ("deployments", &["deployment", "deploy", "deploys"]),
        ("deployment", &["deployments", "deploy", "deploys"]),
        ("production", &["prod"]),
        ("prod", &["production"]),
        ("search", &["grep", "find", "rg"]),
        ("sync", &["rsync", "scp", "copy", "cp"]),
        ("cpu", &["top", "usage", "processes"]),
        ("memory", &["mem", "top", "usage", "free"]),
        ("packages", &["package", "install", "deps", "dependencies"]),
        ("dependencies", &["deps", "packages", "install"]),
        ("update", &["upgrade", "update"]),
        ("env", &["environment", "env", "printenv"]),
        ("git", &["git"]),
        ("docker", &["docker"]),
        ("k8s", &["kubectl", "kubernetes"]),
        ("kubernetes", &["kubectl", "k8s"]),
        ("npm", &["npm", "node"]),
        ("node", &["npm", "node"]),
    ];
    let mut extra = Vec::new();
    for t in tokens {
        for (k, vals) in SYN {
            if t == k {
                for v in *vals {
                    if !tokens.iter().any(|x| x == v) && !extra.iter().any(|x| x == v) {
                        extra.push(v.to_string());
                    }
                }
            }
        }
    }
    extra
}

/// A searchable row with token frequencies.
#[derive(Debug, Clone)]
pub struct SearchRow {
    pub id: i64,
    pub command: String,
    pub ts: i64,
    pub uses: i64,
    pub tf: HashMap<String, u32>,
    pub len: usize,
}

/// In-memory BM25 index over the history store.
pub struct SearchIndex {
    pub rows: Vec<SearchRow>,
    df: HashMap<String, usize>,
    avg_len: f64,
    n: usize,
    oldest: i64,
    newest: i64,
}

/// A ranked search hit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryHit {
    pub command: String,
    pub score: f64,
    /// `lexical`, `vector` or `hybrid`
    pub source: String,
    pub uses: i64,
    pub ts: i64,
}

impl SearchIndex {
    /// Build the index from the database (non-secret rows).
    pub fn build(conn: &Connection) -> SearchIndex {
        let rows_db = store::load_rows_for_search(conn);
        let ids: Vec<i64> = rows_db.iter().map(|r| r.id).collect();
        let tokens = store::load_tokens(conn, &ids);
        let mut rows = Vec::with_capacity(rows_db.len());
        let mut df: HashMap<String, usize> = HashMap::new();
        for r in rows_db {
            let tf = tokens.get(&r.id).cloned().unwrap_or_default();
            for t in tf.keys() {
                *df.entry(t.clone()).or_insert(0) += 1;
            }
            let len = tf.values().map(|c| *c as usize).sum::<usize>().max(1);
            rows.push(SearchRow {
                id: r.id,
                command: r.command.clone(),
                ts: r.ts,
                uses: r.uses,
                tf,
                len,
            });
        }
        let n = rows.len();
        let avg_len = if n > 0 {
            rows.iter().map(|r| r.len as f64).sum::<f64>() / n as f64
        } else {
            1.0
        };
        let oldest = rows.iter().map(|r| r.ts).min().unwrap_or(0);
        let newest = rows.iter().map(|r| r.ts).max().unwrap_or(1);
        SearchIndex {
            rows,
            df,
            avg_len,
            n,
            oldest,
            newest,
        }
    }

    fn idf(&self, term: &str) -> f64 {
        let df = *self.df.get(term).unwrap_or(&0) as f64;
        let n = self.n as f64;
        (1.0 + (n - df + 0.5) / (df + 0.5)).ln()
    }

    /// Lexical BM25 search with synonym expansion, fuzzy and recency
    /// boosts. Returns rows sorted best-first.
    pub fn search(&self, query: &str, limit: usize) -> Vec<(usize, f64)> {
        // (row_index, score)
        let q_tokens = tokenize(query);
        let mut terms: Vec<String> = q_tokens.clone();
        terms.extend(synonym_expansion(&q_tokens));
        // Whole-phrase presence is a strong signal.
        let ql = query.to_lowercase();
        let mut scored: Vec<(usize, f64)> = Vec::with_capacity(self.n);
        let k1 = 1.5f64;
        let b = 0.75f64;
        let now = crate::util::now_ts();
        let span = (self.newest - self.oldest).max(1) as f64;
        for (i, row) in self.rows.iter().enumerate() {
            let mut score = 0.0f64;
            let mut matched = 0usize;
            for t in &terms {
                if let Some(&tf) = row.tf.get(t) {
                    let tf = tf as f64;
                    let denom = tf + k1 * (1.0 - b + b * (row.len as f64 / self.avg_len));
                    score += self.idf(t) * (tf * (k1 + 1.0)) / denom;
                    matched += 1;
                }
            }
            if matched == 0 {
                // Fuzzy fallback: does the whole query fuzzy-match the command?
                if let Some(fs) = crate::util::fuzzy_match(&ql, &row.command.to_lowercase()) {
                    score += (fs as f64) / 40.0;
                } else {
                    continue;
                }
            }
            // Substring of the full command text.
            if row.command.to_lowercase().contains(&ql) {
                score += 6.0;
            }
            // Usage frequency (logarithmic).
            score += (1.0 + row.uses as f64).ln();
            // Recency, relative to the index's own time span.
            if row.ts > 0 {
                let rec = (now - row.ts).max(0) as f64;
                score += 1.5 * (-rec / (span * 6.0)).exp();
            }
            scored.push((i, score));
        }
        scored.sort_by(|a, c| c.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        scored
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
}

/// Cosine similarity of two vectors.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Result of a hybrid search merge.
pub struct HybridSearchResult {
    pub hits: Vec<HistoryHit>,
    pub used_vectors: bool,
}

/// Hybrid semantic search: BM25 always, vector re-rank when an embedding
/// function is available. `embed` returns `None` when the model is
/// unreachable — search then degrades to lexical only.
pub fn hybrid_search<F>(
    conn: &Connection,
    query: &str,
    limit: usize,
    semantic: bool,
    mut embed: F,
    embedding_model: &str,
) -> HybridSearchResult
where
    F: FnMut(&str) -> Option<Vec<f32>>,
{
    let index = SearchIndex::build(conn);
    if index.is_empty() {
        return HybridSearchResult {
            hits: Vec::new(),
            used_vectors: false,
        };
    }
    let lexical = index.search(query, 200);

    let mut used_vectors = false;
    let mut vector_ranked: Vec<usize> = Vec::new();
    if semantic && !lexical.is_empty() {
        if let Some(qv) = embed(query) {
            let candidate_ids: Vec<i64> = lexical
                .iter()
                .map(|(i, _)| index.rows[*i].id)
                .collect();
            let embeddings = store::get_embeddings(conn, embedding_model, &candidate_ids);
            if !embeddings.is_empty() {
                let mut sims: Vec<(usize, f32)> = lexical
                    .iter()
                    .enumerate()
                    .filter_map(|(rank, (i, _))| {
                        embeddings
                            .get(&index.rows[*i].id)
                            .map(|v| (rank, cosine(&qv, v)))
                    })
                    .collect();
                sims.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                vector_ranked = sims.iter().map(|(rank, _)| *rank).collect();
                used_vectors = true;
            }
        }
    }

    // Reciprocal rank fusion across the ranked lists.
    let mut fused: HashMap<usize, f64> = HashMap::new();
    const K: f64 = 60.0;
    for (rank, (i, score)) in lexical.iter().enumerate() {
        *fused.entry(*i).or_insert(0.0) += 1.0 / (K + rank as f64) + score * 0.0001;
    }
    if used_vectors {
        for (rank, lex_rank) in vector_ranked.iter().enumerate() {
            let i = lexical[*lex_rank].0;
            *fused.entry(i).or_insert(0.0) += 1.0 / (K + rank as f64);
        }
    }
    let mut merged: Vec<(usize, f64)> = fused.into_iter().collect();
    merged.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    merged.truncate(limit);

    let hits = merged
        .into_iter()
        .map(|(i, score)| HistoryHit {
            command: index.rows[i].command.clone(),
            score,
            source: if used_vectors { "hybrid".into() } else { "lexical".into() },
            uses: index.rows[i].uses,
            ts: index.rows[i].ts,
        })
        .collect();
    HybridSearchResult { hits, used_vectors }
}

/// History-based inline completions: commands in the store that extend
/// `prefix`. Returns full commands; callers derive the ghost suffix.
pub fn suggest_completions(conn: &Connection, prefix: &str, limit: usize) -> Vec<HistoryRow> {
    if prefix.trim().len() < 3 {
        return Vec::new();
    }
    store::prefix_matches(conn, prefix, limit)
        .into_iter()
        .filter(|r| r.command.len() > prefix.len())
        .collect()
}

/// Approximate last-run time of a command (for display).
pub fn relative_time(ts: i64) -> String {
    if ts == 0 {
        return "unknown".into();
    }
    let now = crate::util::now_ts();
    let diff = now.saturating_sub(ts);
    match diff {
        0..=60 => "just now".into(),
        61..=3600 => format!("{}m ago", diff / 60),
        3601..=86400 => format!("{}h ago", diff / 3600),
        86401..=604800 => format!("{}d ago", diff / 86400),
        _ => format!("{}w ago", diff / 604800),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::jaro_winkler;

    fn testdb(name: &str) -> Connection {
        let dir = std::env::temp_dir().join(format!("sm-hist-{}-{}", name, std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        store::open_at(&dir.join("t.db")).unwrap()
    }

    fn seed(conn: &Connection) {
        let cmds = [
            "docker image prune -a",
            "docker ps --format 'table {{.Names}}'",
            "pg_dump -U postgres -h localhost -F c -b -v -f backup.dump mydb",
            "tar -czvf archive.tar.gz folder/",
            "git log --oneline --graph --decorate",
            "kubectl get pods -n production",
            "du -h --max-depth=1 | sort -hr",
            "find . -type f -size +100M -delete",
            "npm run dev",
            "npm run build",
            "git push origin main",
        ];
        for (i, c) in cmds.iter().enumerate() {
            store::record_command(
                conn,
                &HistoryEntry {
                    command: c.to_string(),
                    ts: 1_700_000_000 + i as i64,
                    ..Default::default()
                },
            )
            .unwrap();
        }
    }

    #[test]
    fn zsh_extended_history() {
        let content = ": 1699999999:0;docker ps\n: 1700000000:5;git status\nplain command\n";
        let entries = parse_zsh_history(content);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].ts, 1699999999);
        assert_eq!(entries[0].command, "docker ps");
        assert_eq!(entries[2].ts, 0);
        assert_eq!(entries[2].command, "plain command");
    }

    #[test]
    fn zsh_multiline_backslash() {
        let content = ": 1700000000:0;tar -czvf a.tar.gz \\\nfolder/";
        let entries = parse_zsh_history(content);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].command, "tar -czvf a.tar.gz folder/");
    }

    #[test]
    fn bash_history_with_timestamps() {
        let content = "#1700000001\nls -la\n#1700000002\ngit status\npwd\n";
        let entries = parse_bash_history(content);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].ts, 1700000001);
        assert_eq!(entries[2].command, "pwd");
    }

    #[test]
    fn fish_history() {
        let content = "- cmd: docker ps\n  when: 1700000001\n- cmd: git status\n  when: 1700000002\n  paths:\n    - /path\n";
        let entries = parse_fish_history(content);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].command, "docker ps");
        assert_eq!(entries[1].ts, 1700000002);
    }

    #[test]
    fn lexical_search_matches_intent() {
        let conn = testdb("lex");
        seed(&conn);
        let result = hybrid_search(&conn, "docker remove unused images", 3, false, |_| None, "m");
        assert!(!result.hits.is_empty());
        assert!(
            result.hits.iter().any(|h| h.command.contains("docker image prune")),
            "expected prune suggestion, got {:?}",
            result.hits.iter().map(|h| &h.command).collect::<Vec<_>>()
        );
    }

    #[test]
    fn lexical_search_postgres_backup() {
        let conn = testdb("pg");
        seed(&conn);
        let result = hybrid_search(&conn, "postgres backup command", 3, false, |_| None, "m");
        assert!(
            result.hits.iter().any(|h| h.command.starts_with("pg_dump")),
            "got {:?}",
            result.hits.iter().map(|h| &h.command).collect::<Vec<_>>()
        );
    }

    #[test]
    fn lexical_search_compress_tar() {
        let conn = testdb("tar");
        seed(&conn);
        let result = hybrid_search(&conn, "compress folder with tar", 3, false, |_| None, "m");
        assert!(
            result.hits
                .iter()
                .any(|h| h.command.starts_with("tar -czvf"))
        );
    }

    #[test]
    fn vector_rerank_changes_ranking() {
        let conn = testdb("vec");
        seed(&conn);
        // Deterministic fake embeddings: map each command to a vector that
        // encodes its first character code.
        let embed = |q: &str| -> Option<Vec<f32>> {
            let mut v = vec![0.0f32; 8];
            for (i, b) in q.bytes().take(8).enumerate() {
                v[i] = (b as f32) / 255.0;
            }
            Some(v)
        };
        // Store matching embeddings for all rows.
        let rows = store::load_rows_for_search(&conn);
        for r in rows {
            let mut v = vec![0.0f32; 8];
            for (i, b) in r.command.bytes().take(8).enumerate() {
                v[i] = (b as f32) / 255.0;
            }
            store::put_embedding(&conn, r.id, "test-embed", &v).unwrap();
        }
        let result = hybrid_search(&conn, "docker ps", 5, true, embed, "test-embed");
        assert!(result.used_vectors);
        assert!(result.hits[0].command.contains("docker ps"));
    }

    #[test]
    fn secret_commands_not_indexed() {
        let dir = std::env::temp_dir().join(format!("sm-sec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let hist = dir.join("zsh_hist");
        std::fs::write(
            &hist,
            ": 1700000000:0;docker ps\n: 1700000001:0;curl --token abc123def456 https://api.io\n",
        )
        .unwrap();
        let conn = store::open_at(&dir.join("t.db")).unwrap();
        let imported = import_file(&conn, &hist, "zsh", true);
        assert_eq!(imported, 1);
        assert_eq!(store::history_count(&conn), 1);
    }

    #[test]
    fn cosine_basics() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        assert!(cosine(&[1.0], &[1.0, 2.0]) == 0.0);
    }

    #[test]
    fn jaro_similarity_sane() {
        assert!(jaro_winkler("docker", "docker") > 0.99);
        assert!(jaro_winkler("docker", "kubectl") < 0.6);
    }
}
