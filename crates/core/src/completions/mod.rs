//! The completion engine.
//!
//! Combines five ranked sources (highest trust first):
//!
//! 1. **static specs** — flag/subcommand knowledge from YAML,
//! 2. **dynamic context** — npm scripts, compose services, git branches,
//!    k8s namespaces, files in the working directory,
//! 3. **aliases** — your own shortcuts, suggested with an expansion preview,
//! 4. **history** — full-command ghost text from what you actually run,
//! 5. **PATH binaries** — first-word completion.
//!
//! The AI layer is intentionally *not* part of the inline path: ghost text
//! must appear in milliseconds, so it is deterministic and local. AI is
//! used for the natural-language palette instead (see [`crate::ai`]).

pub mod dynamic;
pub mod spec;

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::config::Config;
use crate::context::Context;
use crate::parser::CompletionQuery;
use crate::store::Connection;
use spec::SpecSet;

/// One completion candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suggestion {
    /// Text inserted at the cursor when accepted inline.
    pub insert: String,
    /// Byte offset in the buffer where a menu selection replaces from
    /// (`0` = replace the whole line, e.g. aliases and history).
    pub replace_from: usize,
    /// The full command line that results from accepting this suggestion.
    pub line: String,
    pub description: String,
    /// `flag` `subcommand` `script` `service` `branch` `remote` `resource`
    /// `namespace` `history` `alias` `binary` `file` `target`
    pub kind: String,
    pub score: i32,
    pub source: String,
}

/// Result of a completion request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResult {
    pub suggestions: Vec<Suggestion>,
    /// Ghost-text suffix to display after the cursor (without a leading
    /// space when the buffer already ends with one).
    pub ghost: Option<String>,
}

impl CompletionResult {
    pub fn empty() -> CompletionResult {
        CompletionResult {
            suggestions: Vec::new(),
            ghost: None,
        }
    }
}

/// Compute inline completions for a parsed query.
pub fn complete(
    q: &CompletionQuery,
    ctx: &Context,
    conn: Option<&Connection>,
    specs: &SpecSet,
    cfg: &Config,
) -> CompletionResult {
    let mut out: Vec<Suggestion> = Vec::new();
    let word = q.current_word.as_str();
    // Byte offset where the current word starts in the buffer.
    let word_start = q.prefix.len();
    let buffer = format!("{}{}", q.prefix, q.current_word);
    let used_flags: HashSet<String> = q.cmdline.flag_names().into_iter().collect();

    match q.binary() {
        None => {
            // --- Completing the command name itself ---------------------
            if word.is_empty() {
                // Fresh prompt (or right after an operator): recent history.
                if let Some(conn) = conn {
                    for cmd in recent_commands(conn, 12) {
                        push(
                            &mut out,
                            Suggestion {
                                insert: cmd.clone(),
                                replace_from: 0,
                                line: cmd.clone(),
                                description: String::new(),
                                kind: "history".into(),
                                score: 400,
                                source: "history".into(),
                            },
                        );
                    }
                }
            }
            for b in &ctx.installed_binaries {
                if let Some(score) = match_word(word, b) {
                    push(
                        &mut out,
                        Suggestion {
                            insert: b.clone(),
                            replace_from: word_start,
                            line: format!("{}{}", q.prefix, b),
                            description: String::new(),
                            kind: "binary".into(),
                            score: score.saturating_sub(100),
                            source: "path".into(),
                        },
                    );
                }
            }
            for a in &ctx.aliases {
                if let Some(score) = match_word(word, &a.name) {
                    push(
                        &mut out,
                        Suggestion {
                            insert: a.name.clone(),
                            replace_from: word_start,
                            line: format!("{}{}", q.prefix, a.name),
                            description: format!("alias for: {}", a.expansion),
                            kind: "alias".into(),
                            score: score + 60,
                            source: "alias".into(),
                        },
                    );
                }
            }
        }
        Some(binary) => {
            // --- Spec-driven completion ---------------------------------
            if let Some(node) = specs.resolve(binary, &q.cmdline.words) {
                if q.completing_flag() {
                    for (idx, flag) in node.flags.iter().chain(node.global_flags.iter()).enumerate() {
                        let insert = flag.insert_text();
                        if insert == word {
                            continue;
                        }
                        if insert.starts_with(word) && !used_flags.contains(flag.bare_name()) {
                            let score = 1000 - (idx as i32) * 3
                                + if flag.description.is_empty() { 0 } else { 20 };
                            push(
                                &mut out,
                                Suggestion {
                                    insert: insert.clone(),
                                    replace_from: word_start,
                                    line: format!("{}{}", q.prefix, insert),
                                    description: flag.description.clone(),
                                    kind: "flag".into(),
                                    score,
                                    source: "spec".into(),
                                },
                            );
                        } else if word.len() >= 3 && !used_flags.contains(flag.bare_name()) {
                            if let Some(f) = crate::util::fuzzy_match(word, &flag.name) {
                                if f > 400 {
                                    push(
                                        &mut out,
                                        Suggestion {
                                            insert: insert.clone(),
                                            replace_from: word_start,
                                            line: format!("{}{}", q.prefix, insert),
                                            description: flag.description.clone(),
                                            kind: "flag".into(),
                                            score: f / 2,
                                            source: "spec".into(),
                                        },
                                    );
                                }
                            }
                        }
                    }
                } else {
                    // Subcommands.
                    for (idx, sub) in node.subcommands.iter().enumerate() {
                        if let Some(score) = match_word(word, &sub.name) {
                            push(
                                &mut out,
                                Suggestion {
                                    insert: sub.name.clone(),
                                    replace_from: word_start,
                                    line: format!("{}{}", q.prefix, sub.name),
                                    description: sub.description.clone(),
                                    kind: "subcommand".into(),
                                    score: 950 - (idx as i32) * 3 + score / 100,
                                    source: "spec".into(),
                                },
                            );
                        }
                    }
                    // Dynamic values from local context.
                    if let Some(key) = node.dynamic {
                        for (value, desc) in dynamic::values(key, ctx) {
                            if let Some(score) = match_word(word, &value) {
                                push(
                                    &mut out,
                                    Suggestion {
                                        insert: value.clone(),
                                        replace_from: word_start,
                                        line: format!("{}{}", q.prefix, value),
                                        description: desc,
                                        kind: dynamic::kind_for(key).to_string(),
                                        score: 900 + score / 100,
                                        source: "context".into(),
                                    },
                                );
                            }
                        }
                    }
                    // `kubectl ... -n <TAB>` → current namespace.
                    if binary == "kubectl" {
                        let prev = q.cmdline.words.last();
                        if matches!(prev.map(|s| s.as_str()), Some("-n") | Some("--namespace")) {
                            if let Some(k) = &ctx.k8s {
                                if !k.namespace.is_empty() {
                                    push(
                                        &mut out,
                                        Suggestion {
                                            insert: k.namespace.clone(),
                                            replace_from: word_start,
                                            line: format!("{}{}", q.prefix, k.namespace),
                                            description: format!("namespace in context {}", k.context),
                                            kind: "namespace".into(),
                                            score: 990,
                                            source: "context".into(),
                                        },
                                    );
                                }
                            }
                        }
                    }
                    // `git checkout/switch <TAB>` → branches (dynamic key
                    // already covers it); nothing extra here.
                }
            } else {
                // Unknown binary: complete file paths from the cwd.
                for f in &ctx.dir_entries {
                    if let Some(score) = match_word(word, f) {
                        push(
                            &mut out,
                            Suggestion {
                                insert: f.clone(),
                                replace_from: word_start,
                                line: format!("{}{}", q.prefix, f),
                                description: String::new(),
                                kind: "file".into(),
                                score: 300 + score / 10,
                                source: "context".into(),
                            },
                        );
                    }
                }
            }

            // --- History full-line ghost text (all binaries) ------------
            if let Some(conn) = conn {
                let rows = crate::history::suggest_completions(conn, &buffer, 8);
                for row in rows {
                    let mut score = 1050 + (row.uses as i32).min(20) * 10;
                    if row.ts > 0 {
                        // Recency bonus: up to +50 points, decaying over ~7 weeks.
                        let age_days =
                            (crate::util::now_ts() - row.ts).max(0) / 86_400;
                        score += (50 - age_days.min(50)) as i32;
                    }
                    if !q.after_space {
                        // The history command must extend the current word.
                        if !row.command.starts_with(&buffer) {
                            continue;
                        }
                    } else if !row.command.starts_with(buffer.trim_end()) {
                        continue;
                    }
                    push(
                        &mut out,
                        Suggestion {
                            insert: row.command[buffer.trim_end().len()..]
                                .trim_start()
                                .to_string(),
                            replace_from: buffer.trim_end().len(),
                            line: row.command.clone(),
                            description: format!(
                                "run {} time(s), {}",
                                row.uses,
                                crate::history::relative_time(row.ts)
                            ),
                            kind: "history".into(),
                            score,
                            source: "history".into(),
                        },
                    );
                }
            }

            // --- Alias awareness ----------------------------------------
            // If what is typed so far is the prefix of an alias expansion,
            // offer the alias as a full-line replacement.
            let typed = buffer.trim_end().to_string();
            if !typed.is_empty() {
                for a in &ctx.aliases {
                    let exp = a.expansion.trim();
                    if exp == typed {
                        continue;
                    }
                    if exp.starts_with(&typed)
                        || typed.starts_with(exp)
                        || (typed.len() >= 3
                            && crate::util::fuzzy_match(&typed, exp)
                                .map(|f| f > 350)
                                .unwrap_or(false))
                    {
                        push(
                            &mut out,
                            Suggestion {
                                insert: a.name.clone(),
                                replace_from: 0,
                                line: a.name.clone(),
                                description: format!("alias for: {}", a.expansion),
                                kind: "alias".into(),
                                score: 940,
                                source: "alias".into(),
                            },
                        );
                    }
                }
            }
        }
    }

    // Rank + dedupe + cap.
    out.sort_by(|a, b| b.score.cmp(&a.score));
    let mut seen: HashSet<(String, String)> = HashSet::new();
    out.retain(|s| {
        let key = (s.kind.to_string(), s.insert.clone());
        if s.insert.is_empty() {
            return false;
        }
        seen.insert(key)
    });
    out.truncate(cfg.completions.max_suggestions.max(1));

    // Ghost text: first suggestion that cleanly extends the cursor.
    let ghost = out.iter().find_map(|s| ghost_suffix(q, s, &buffer));

    CompletionResult { suggestions: out, ghost }
}

/// Ghost suffix for a suggestion, if it can be shown inline.
fn ghost_suffix(q: &CompletionQuery, s: &Suggestion, _buffer: &str) -> Option<String> {
    if s.kind == "history" {
        // History suggestions carry the continuation suffix in `insert`
        // (already relative to the whole typed buffer).
        return if s.insert.is_empty() { None } else { Some(s.insert.clone()) };
    }
    if s.replace_from != q.prefix.len() {
        // Full-line replacements (aliases) are menu-only.
        return None;
    }
    if q.after_space {
        if s.insert.is_empty() {
            None
        } else {
            Some(s.insert.clone())
        }
    } else {
        s.insert
            .strip_prefix(&q.current_word)
            .filter(|rest| !rest.is_empty())
            .map(|rest| rest.to_string())
    }
}

fn push(out: &mut Vec<Suggestion>, s: Suggestion) {
    out.push(s);
}

fn recent_commands(conn: &Connection, limit: usize) -> Vec<String> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT command FROM history WHERE secret = 0 ORDER BY ts DESC, id DESC LIMIT ?1",
    ) else {
        return Vec::new();
    };
    stmt.query_map([limit as i64], |r| r.get::<_, String>(0))
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

/// Word match with score: exact > prefix > substring > fuzzy.
fn match_word(word: &str, candidate: &str) -> Option<i32> {
    if word.is_empty() {
        return Some(500);
    }
    if candidate == word {
        return Some(1500);
    }
    if candidate.starts_with(word) {
        return Some(1000 - (candidate.len() as i32).min(300));
    }
    if word.len() >= 3 && candidate.contains(word) {
        return Some(600);
    }
    if word.len() >= 2 {
        crate::util::fuzzy_match(word, candidate).filter(|f| *f > 250)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{Alias, GitInfo, K8sInfo, ProjectInfo};

    fn testdb(name: &str) -> Connection {
        let dir = std::env::temp_dir().join(format!("sm-comp-{}-{}", name, std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        crate::store::open_at(&dir.join("t.db")).unwrap()
    }

    fn ctx_fixture() -> Context {
        Context {
            cwd: std::env::temp_dir(),
            shell: "zsh".into(),
            os: "linux",
            git: Some(GitInfo {
                branch: "main".into(),
                detached: false,
                remotes: vec!["origin".into()],
                branches: vec!["main".into(), "feature/x".into(), "develop".into()],
                has_upstream: Some(true),
                dirty: Some(false),
            }),
            project: ProjectInfo {
                kind: Some("node"),
                package_manager: Some("npm"),
                npm_scripts: vec!["dev".into(), "build".into(), "test".into()],
                compose_services: vec!["api".into(), "worker".into()],
                makefile_targets: vec![],
            },
            k8s: Some(K8sInfo {
                context: "prod".into(),
                namespace: "production".into(),
            }),
            aliases: vec![Alias {
                name: "dps".into(),
                expansion: "docker ps --format 'table {{.Names}}'".into(),
                source: "shell".into(),
            }],
            dir_entries: vec!["README.md".into(), "src/".into()],
            recent_commands: vec![],
            installed_binaries: vec!["git".into(), "docker".into(), "grep".into()],
        }
    }

    fn cfg() -> Config {
        Config::default()
    }

    fn complete_line(line: &str, ctx: &Context, conn: Option<&Connection>) -> CompletionResult {
        let q = crate::parser::parse_for_completion(line, None);
        let specs = SpecSet::load();
        complete(&q, ctx, conn, &specs, &cfg())
    }

    fn has<'a>(res: &'a CompletionResult, kind: &str, insert: &str) -> &'a Suggestion {
        res.suggestions
            .iter()
            .find(|s| s.kind == kind && s.insert == insert)
            .unwrap_or_else(|| panic!("no {} suggestion {:?} in {:?}", kind, insert, res.suggestions.iter().map(|s| (&s.kind, &s.insert)).collect::<Vec<_>>()))
    }

    #[test]
    fn git_log_flags_with_descriptions() {
        let ctx = ctx_fixture();
        let res = complete_line("git log --", &ctx, None);
        let oneline = has(&res, "flag", "--oneline");
        assert!(oneline.description.contains("one commit per line"));
        assert!(res.suggestions.iter().any(|s| s.insert == "--graph"));
        assert!(res.suggestions.iter().any(|s| s.insert == "--author="));
        assert_eq!(res.ghost.as_deref(), Some("oneline"));
        // Display line is the full command.
        assert_eq!(has(&res, "flag", "--oneline").line, "git log --oneline");
    }

    #[test]
    fn kubectl_get_suggests_resources() {
        let ctx = ctx_fixture();
        let res = complete_line("kubectl get ", &ctx, None);
        assert!(res.suggestions.iter().any(|s| s.insert == "pods" && s.kind == "subcommand"));
        assert!(res.suggestions.iter().any(|s| s.insert == "deployments"));
        assert!(res.suggestions.iter().any(|s| s.insert == "services"));
        assert!(res.suggestions.iter().any(|s| s.insert == "configmaps"));
        assert!(res.suggestions.iter().any(|s| s.insert == "ingresses"));
    }

    #[test]
    fn kubectl_namespace_after_n() {
        let ctx = ctx_fixture();
        let res = complete_line("kubectl get pods -n ", &ctx, None);
        let ns = has(&res, "namespace", "production");
        assert!(ns.description.contains("prod"));
    }

    #[test]
    fn npm_run_suggests_package_scripts() {
        let ctx = ctx_fixture();
        let res = complete_line("npm run ", &ctx, None);
        has(&res, "script", "dev");
        has(&res, "script", "build");
        has(&res, "script", "test");
    }

    #[test]
    fn docker_compose_up_suggests_services() {
        let ctx = ctx_fixture();
        let res = complete_line("docker compose up ", &ctx, None);
        has(&res, "service", "api");
        has(&res, "service", "worker");
    }

    #[test]
    fn docker_images_filter_flag() {
        let ctx = ctx_fixture();
        let res = complete_line("docker images --", &ctx, None);
        assert!(res
            .suggestions
            .iter()
            .any(|s| s.insert == "--filter=" && s.description.contains("dangling")));
    }

    #[test]
    fn git_checkout_suggests_branches() {
        let ctx = ctx_fixture();
        let res = complete_line("git checkout ", &ctx, None);
        has(&res, "branch", "main");
        has(&res, "branch", "feature/x");
    }

    #[test]
    fn git_push_suggests_remotes() {
        let ctx = ctx_fixture();
        let res = complete_line("git push ", &ctx, None);
        has(&res, "remote", "origin");
    }

    #[test]
    fn alias_suggested_for_expansion_prefix() {
        let ctx = ctx_fixture();
        let res = complete_line("docker ps ", &ctx, None);
        let alias = res
            .suggestions
            .iter()
            .find(|s| s.kind == "alias" && s.insert == "dps")
            .expect("alias dps should be suggested");
        assert!(alias.description.contains("docker ps"));
        assert_eq!(alias.replace_from, 0);
    }

    #[test]
    fn alias_suggested_by_name_prefix() {
        let ctx = ctx_fixture();
        let res = complete_line("dp", &ctx, None);
        assert!(res.suggestions.iter().any(|s| s.kind == "alias" && s.insert == "dps"));
    }

    #[test]
    fn history_ghost_full_command() {
        let conn = testdb("ghost");
        crate::store::record_command(
            &conn,
            &crate::store::HistoryEntry {
                command: "git log --oneline --graph --decorate".into(),
                ts: crate::util::now_ts(),
                ..Default::default()
            },
        )
        .unwrap();
        let ctx = ctx_fixture();
        let res = complete_line("git log --", &ctx, Some(&conn));
        assert_eq!(res.ghost.as_deref(), Some("oneline --graph --decorate"));
        let hist = has(&res, "history", "oneline --graph --decorate");
        assert_eq!(hist.line, "git log --oneline --graph --decorate");
    }

    #[test]
    fn used_flags_not_resuggested() {
        let ctx = ctx_fixture();
        let res = complete_line("git log --oneline --", &ctx, None);
        assert!(!res.suggestions.iter().any(|s| s.insert == "--oneline"));
        assert!(res.suggestions.iter().any(|s| s.insert == "--graph"));
    }

    #[test]
    fn binary_completion_from_path() {
        let ctx = ctx_fixture();
        let res = complete_line("git", &ctx, None);
        assert!(res.suggestions.iter().any(|s| s.kind == "binary" && s.insert == "git"));
    }

    #[test]
    fn unknown_binary_gets_files() {
        let ctx = ctx_fixture();
        let res = complete_line("grep ", &ctx, None);
        assert!(res.suggestions.iter().any(|s| s.kind == "file" && s.insert == "README.md"));
    }

    #[test]
    fn subcommand_completion_prefix() {
        let ctx = ctx_fixture();
        let res = complete_line("git che", &ctx, None);
        assert!(res.suggestions.iter().any(|s| s.insert == "checkout" && s.kind == "subcommand"));
    }

    #[test]
    fn respects_max_suggestions() {
        let mut c = cfg();
        c.completions.max_suggestions = 3;
        let ctx = ctx_fixture();
        let q = crate::parser::parse_for_completion("git log --", None);
        let specs = SpecSet::load();
        let res = complete(&q, &ctx, None, &specs, &c);
        assert!(res.suggestions.len() <= 3);
    }

    #[test]
    fn empty_query_recent_commands() {
        let conn = testdb("recent");
        crate::store::record_command(
            &conn,
            &crate::store::HistoryEntry {
                command: "docker compose up -d".into(),
                ts: crate::util::now_ts(),
                ..Default::default()
            },
        )
        .unwrap();
        let ctx = ctx_fixture();
        let res = complete_line("", &ctx, Some(&conn));
        assert!(res.suggestions.iter().any(|s| s.insert == "docker compose up -d"));
    }
}
