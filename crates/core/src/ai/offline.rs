//! The deterministic offline engine.
//!
//! Provides explain / fix / natural-language → command with **zero model
//! installed**. It is also the safety net the Ollama path falls back to
//! whenever the model is unreachable — every AI feature keeps working.

use regex::Regex;
use std::sync::OnceLock;

use crate::completions::spec::SpecSet;
use crate::context::Context;
use crate::parser::CommandLine;
use crate::store::Connection;
use crate::util::{is_typo_of, strip_stopwords, tokenize};

use super::kb;

/// One concrete fix for a failed command.
#[derive(Debug, Clone)]
pub struct Fix {
    pub command: String,
    pub explanation: String,
}

/// One natural-language → command result.
#[derive(Debug, Clone)]
pub struct NlResult {
    pub command: String,
    pub explanation: String,
    pub safer: Vec<String>,
    pub source: &'static str,
    pub score: i32,
}

fn cmd_or_default(line: &str) -> CommandLine {
    CommandLine::parse(line)
}

// ---------------------------------------------------------------------------
// Explain
// ---------------------------------------------------------------------------

/// Explain a command using static specs + the knowledge base.
pub fn explain(command: &str, ctx: &Context, specs: &SpecSet) -> String {
    let cmd = cmd_or_default(command);
    let mut out = String::new();
    let Some(binary) = cmd.binary_name().map(|s| s.to_string()) else {
        out.push_str("Nothing to explain yet — start typing a command.\n");
        return out;
    };

    // Spec-driven description of the chain + flags actually used.
    if let Some(spec) = specs.get(&binary) {
        if !spec.description.is_empty() {
            out.push_str(&format!("{} — {}\n", binary, spec.description));
        }
        // Walk the subcommand chain.
        let mut node = &spec.subcommands;
        let mut chain: Vec<String> = vec![binary.clone()];
        'walk: for w in &cmd.words {
            for sub in node {
                if sub.name == *w {
                    chain.push(sub.name.clone());
                    if !sub.description.is_empty() {
                        out.push_str(&format!(
                            "\n{} — {}\n",
                            chain.join(" "),
                            sub.description
                        ));
                    }
                    node = &sub.subcommands;
                    continue 'walk;
                }
            }
            break;
        }
        // Flags used.
        let mut flag_lines: Vec<String> = Vec::new();
        for f in &cmd.flags {
            let bare = f.split('=').next().unwrap_or(f);
            let desc = find_flag_description(spec, &cmd.words, bare);
            flag_lines.push(format!("  {:<18} {}", f, desc));
        }
        if !flag_lines.is_empty() {
            out.push_str("\nFlags in this command:\n");
            out.push_str(&flag_lines.join("\n"));
            out.push('\n');
        }
    }

    // Knowledge base entry with examples.
    if let Some(entry) = kb::EXPLAIN_KB.iter().find(|e| e.binary == binary) {
        if !out.contains(entry.summary) {
            out.push_str(&format!("\n{}\n", entry.summary));
        }
        out.push_str("\nCommon examples:\n");
        for (label, example) in entry.examples {
            out.push_str(&format!("  {:<24} {}\n", format!("{}:", label), example));
        }
    }

    if out.is_empty() {
        out.push_str(&format!(
            "{}: no offline knowledge for this command yet — start Ollama (see `sm model pull`) for AI explanations.\n",
            binary
        ));
    } else if let Some(g) = &ctx.git {
        if binary == "git" {
            out.push_str(&format!("\n(current branch: {})\n", g.branch));
        }
    }
    out.trim_end().to_string() + "\n"
}

fn find_flag_description(
    spec: &crate::completions::spec::Spec,
    words: &[String],
    flag: &str,
) -> String {
    // Search the resolved subcommand chain first, then global flags.
    let mut node = &spec.subcommands;
    for w in words {
        if let Some(sub) = node.iter().find(|s| s.name == *w) {
            if let Some(f) = sub.flags.iter().find(|f| f.bare_name() == flag) {
                return f.description.clone();
            }
            node = &sub.subcommands;
        } else {
            break;
        }
    }
    spec.flags
        .iter()
        .find(|f| f.bare_name() == flag)
        .map(|f| f.description.clone())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Fix
// ---------------------------------------------------------------------------

/// Suggest fixes for `command` given captured `error` text (if any).
pub fn fix(command: &str, error: Option<&str>, ctx: &Context) -> Vec<Fix> {
    let mut fixes: Vec<Fix> = Vec::new();
    let cmd = cmd_or_default(command);
    let err = error.unwrap_or("");
    let values = PlaceholderValues::extract(err, &cmd, ctx);

    if !err.is_empty() {
        for pat in kb::ERROR_PATTERNS {
            let re = compiled(pat.regex);
            if let Some(caps) = re.captures(err) {
                match pat.id {
                    "command_not_found" => {
                        fixes.extend(command_not_found_fixes(err, ctx));
                    }
                    "module_not_found" => {
                        let module = caps
                            .get(1)
                            .map(|m| m.as_str())
                            .unwrap_or("module")
                            .to_string();
                        for (f, expl) in pat.fixes {
                            fixes.push(Fix {
                                command: f.replace("{module}", &module),
                                explanation: expl.to_string(),
                            });
                        }
                    }
                    _ => {
                        for (f, expl) in pat.fixes {
                            fixes.push(Fix {
                                command: render(f, &values),
                                explanation: render(expl, &values),
                            });
                        }
                    }
                }
                if !fixes.is_empty() {
                    break; // first matching pattern wins
                }
            }
        }
    } else {
        // Infer from the command + local context.
        fixes.extend(infer_fixes(&cmd, ctx));
    }
    fixes
}

fn command_not_found_fixes(err: &str, ctx: &Context) -> Vec<Fix> {
    let mut fixes = Vec::new();
    let missing = extract_missing_command(err);
    if let Some(name) = missing {
        // Typo suggestions from PATH.
        let mut typos: Vec<&String> = ctx
            .installed_binaries
            .iter()
            .filter(|b| is_typo_of(&name, b))
            .collect();
        typos.sort_by_key(|b| b.len());
        for t in typos.into_iter().take(3) {
            fixes.push(Fix {
                command: t.clone(),
                explanation: format!("'{}' is not installed, but '{}' is — did you mean this?", name, t),
            });
        }
        if let Some((_, hint)) = kb::INSTALL_HINTS.iter().find(|(b, _)| *b == name) {
            fixes.push(Fix {
                command: format!("# install {}:", name),
                explanation: hint.to_string(),
            });
        }
        if fixes.is_empty() {
            fixes.push(Fix {
                command: format!("# '{}' is not installed", name),
                explanation: "No close match on PATH either — install the tool or check the name.".into(),
            });
        }
    }
    fixes
}

fn extract_missing_command(err: &str) -> Option<String> {
    static P1: OnceLock<Regex> = OnceLock::new();
    static P2: OnceLock<Regex> = OnceLock::new();
    let p1 = P1.get_or_init(|| Regex::new(r"command not found: ([A-Za-z0-9_.\-/]+)").unwrap());
    if let Some(c) = p1.captures(err) {
        return Some(c[1].to_string());
    }
    let p2 = P2.get_or_init(|| {
        Regex::new(r"([A-Za-z0-9_.\-/]+): (command not found|not found)").unwrap()
    });
    p2.captures(err).map(|c| c[1].to_string())
}

fn infer_fixes(cmd: &CommandLine, ctx: &Context) -> Vec<Fix> {
    let mut fixes = Vec::new();
    let Some(binary) = cmd.binary_name().map(|s| s.to_string()) else {
        return fixes;
    };
    // git push without upstream — the flagship inference.
    if binary == "git"
        && cmd.words.first().map(|s| s.as_str()) == Some("push")
        && !cmd.flag_names().iter().any(|f| f == "--set-upstream" || f == "-u")
    {
        if let Some(g) = &ctx.git {
            if g.has_upstream == Some(false) && !g.branch.is_empty() {
                fixes.push(Fix {
                    command: format!("git push --set-upstream origin {}", g.branch),
                    explanation: format!(
                        "Your local {} branch is not tracking a remote branch. This pushes {} and sets it to track origin/{}.",
                        g.branch, g.branch, g.branch
                    ),
                });
            }
        }
    }
    // Binary not on PATH → typo / install hints.
    if !ctx.installed_binaries.is_empty()
        && !ctx.installed_binaries.iter().any(|b| b == &binary)
        && binary != "cd"
    {
        let mut typos: Vec<&String> = ctx
            .installed_binaries
            .iter()
            .filter(|b| is_typo_of(&binary, b))
            .collect();
        typos.sort_by_key(|b| b.len());
        for t in typos.into_iter().take(2) {
            fixes.push(Fix {
                command: t.clone(),
                explanation: format!(
                    "'{}' was not found on PATH, but '{}' is — did you mean this?",
                    binary, t
                ),
            });
        }
        if let Some((_, hint)) = kb::INSTALL_HINTS.iter().find(|(b, _)| *b == binary) {
            fixes.push(Fix {
                command: format!("# install {}:", binary),
                explanation: hint.to_string(),
            });
        }
    }
    fixes
}

// ---------------------------------------------------------------------------
// Placeholders
// ---------------------------------------------------------------------------

struct PlaceholderValues {
    port: Option<String>,
    branch: Option<String>,
    size: Option<String>,
    file: Option<String>,
    dir: Option<String>,
    namespace: Option<String>,
    user: Option<String>,
    host: Option<String>,
    command: String,
}

impl PlaceholderValues {
    fn extract(err: &str, cmd: &CommandLine, ctx: &Context) -> PlaceholderValues {
        static PORT: OnceLock<Regex> = OnceLock::new();
        static SIZE: OnceLock<Regex> = OnceLock::new();
        static BRANCH_ERR: OnceLock<Regex> = OnceLock::new();
        let port_re = PORT.get_or_init(|| Regex::new(r"(?i)(?:port\s*|:)(\d{2,5})").unwrap());
        let size_re = SIZE.get_or_init(|| {
            Regex::new(r"(?i)(\d+(?:\.\d+)?)\s*(kb|mb|gb|k|m|g)\b").unwrap()
        });
        let branch_re =
            BRANCH_ERR.get_or_init(|| Regex::new(r"(?i)branch (\S+?)[ .]").unwrap());
        PlaceholderValues {
            port: port_re.captures(err).map(|c| c[1].to_string()),
            branch: branch_re
                .captures(err)
                .map(|c| c[1].to_string())
                .or_else(|| ctx.git.as_ref().map(|g| g.branch.clone())),
            size: size_re.captures(err).map(|c| {
                let unit = c[2].to_uppercase().trim_end_matches('B').to_string();
                format!("{}{}", &c[1], unit)
            }),
            file: cmd.args.first().cloned().or_else(|| cmd.words.last().cloned()),
            dir: cmd.args.first().cloned(),
            namespace: ctx
                .k8s
                .as_ref()
                .map(|k| k.namespace.clone())
                .or_else(|| Some("default".into())),
            user: None,
            host: None,
            command: cmd.raw.split_whitespace().next().unwrap_or("").to_string(),
        }
    }

    fn get(&self, key: &str, default: &str) -> String {
        match key {
            "port" => self.port.clone().unwrap_or_else(|| default.into()),
            "branch" => self.branch.clone().unwrap_or_else(|| default.into()),
            "size" => self.size.clone().unwrap_or_else(|| default.into()),
            "file" => self.file.clone().unwrap_or_else(|| default.into()),
            "dir" => self.dir.clone().unwrap_or_else(|| default.into()),
            "namespace" => self.namespace.clone().unwrap_or_else(|| default.into()),
            "user" => self.user.clone().unwrap_or_else(|| default.into()),
            "host" => self.host.clone().unwrap_or_else(|| default.into()),
            "command" => self.command.clone(),
            _ => default.to_string(),
        }
    }
}

fn render(template: &str, v: &PlaceholderValues) -> String {
    static VAR: OnceLock<Regex> = OnceLock::new();
    let re = VAR.get_or_init(|| Regex::new(r"\{([a-z_]+)\}").unwrap());
    re.replace_all(template, |caps: &regex::Captures| {
        let key = &caps[1];
        let default = match key {
            "port" => "3000",
            "branch" => "main",
            "size" => "100M",
            "file" => "app",
            "dir" => ".",
            "namespace" => "default",
            "user" => "user",
            "host" => "host",
            "module" => "module",
            _ => "",
        };
        v.get(key, default)
    })
    .to_string()
}

fn compiled(pattern: &str) -> Regex {
    // Patterns are small and fix/generate are interactive-frequency calls,
    // so we compile per invocation and let the OS page cache absorb it.
    Regex::new(pattern).expect("static pattern must compile")
}

// ---------------------------------------------------------------------------
// Natural language → command
// ---------------------------------------------------------------------------

/// Words that never count as placeholder values.
const NON_VALUE_WORDS: &[&str] = &[
    "a", "an", "the", "all", "and", "or", "my", "me", "show", "list", "find", "delete", "remove",
    "clean", "prune", "kill", "free", "stop", "start", "restart", "reload", "files", "file",
    "large", "larger", "big", "bigger", "than", "size", "port", "process", "folder", "directory",
    "dir", "compress", "archive", "extract", "untar", "unzip", "disk", "usage", "space", "git",
    "branch", "create", "new", "switch", "checkout", "undo", "commit", "discard", "changes",
    "local", "stash", "pop", "sync", "update", "pull", "latest", "outdated", "packages",
    "package", "dependencies", "install", "requirements", "venv", "python", "environment",
    "ssh", "key", "copy", "rsync", "symlink", "link", "env", "variables", "serve", "http",
    "server", "static", "containers", "container", "images", "image", "dangling", "unused",
    "logs", "log", "tail", "follow", "watch", "pods", "pod", "deployment", "deployments",
    "namespace", "kubectl", "kubernetes", "k8s", "docker", "npm", "pip", "run", "with", "for",
    "of", "in", "on", "to", "from", "current", "last", "week", "please", "command", "give", "them", "they", "it",
    "want", "need", "how", "do", "i", "up", "out", "off", "down", "top", "cpu", "memory",
    "usage", "open", "listening", "ports", "main", "master", "amend", "edit", "change",
    "message", "revert", "reset", "unstage", "add", "service", "systemctl", "systemd",
];

/// Generate command candidates from a natural-language query.
pub fn generate(query: &str, ctx: &Context, conn: Option<&Connection>) -> Vec<NlResult> {
    let mut results: Vec<NlResult> = Vec::new();
    let tokens = tokenize(query);
    let terms = strip_stopwords(&tokens);

    // 1. Alias match (with synonym expansion, mirroring history search).
    let mut expanded = terms.clone();
    expanded.extend(crate::history::synonym_expansion(&terms));
    let query_lower = query.to_lowercase();
    for alias in &ctx.aliases {
        let alias_tokens = tokenize(&format!("{} {}", alias.name, alias.expansion));
        let overlap = crate::util::overlap(&expanded, &alias_tokens);
        let direct = alias_tokens.iter().any(|t| query_lower.contains(t.as_str())
            && t.len() >= 4)
            && overlap >= 1;
        if overlap >= 2 || direct {
            results.push(NlResult {
                command: alias.name.clone(),
                explanation: format!("your alias for: {}", alias.expansion),
                safer: vec![],
                source: "alias",
                score: 900 + overlap as i32 * 10,
            });
        }
    }

    // 2. Intent library.
    let placeholders = QueryPlaceholders::extract(query, ctx);
    let mut best_intents: Vec<(i32, &kb::Intent)> = Vec::new();
    for intent in kb::INTENTS {
        let mut best: i32 = 0;
        for set in intent.patterns {
            let hits = set.iter().filter(|k| expanded.iter().any(|t| t == *k)).count();
            if hits > 0 {
                let score = (hits as i32) * 100 / (set.len() as i32);
                best = best.max(score);
                if hits == set.len() {
                    best = best.max(200);
                }
            }
        }
        if best >= 100 {
            best_intents.push((best, intent));
        }
    }
    best_intents.sort_by(|a, b| b.0.cmp(&a.0));
    for (score, intent) in best_intents.into_iter().take(3) {
        let template = if intent.os_aware && crate::util::is_macos() {
            intent.macos
        } else {
            intent.command
        };
        let command = render(template, &placeholders.as_values(ctx));
        let explanation = render(intent.explanation, &placeholders.as_values(ctx));
        let safer = intent
            .safer
            .iter()
            .map(|s| render(s, &placeholders.as_values(ctx)))
            .collect();
        results.push(NlResult {
            command,
            explanation,
            safer,
            source: "intent",
            score: 600 + score,
        });
    }

    // 3. Semantic history search.
    if let Some(conn) = conn {
        let hits = crate::history::hybrid_search(conn, query, 3, false, |_| None, "m");
        for hit in hits.hits {
            if results.iter().any(|r| r.command == hit.command) {
                continue;
            }
            results.push(NlResult {
                command: hit.command,
                explanation: "from your command history".into(),
                safer: vec![],
                source: "history",
                score: 500,
            });
        }
    }

    results.sort_by(|a, b| b.score.cmp(&a.score));
    results.dedup_by(|a, b| a.command == b.command);
    results.truncate(8);
    results
}

/// Placeholders extracted from a natural-language query.
struct QueryPlaceholders {
    size: Option<String>,
    port: Option<String>,
    branch: Option<String>,
    file: Option<String>,
    dir: Option<String>,
    namespace: Option<String>,
    user: Option<String>,
    host: Option<String>,
}

impl QueryPlaceholders {
    fn extract(query: &str, ctx: &Context) -> QueryPlaceholders {
        static SIZE: OnceLock<Regex> = OnceLock::new();
        static PORT: OnceLock<Regex> = OnceLock::new();
        static QUOTED: OnceLock<Regex> = OnceLock::new();
        static USERHOST: OnceLock<Regex> = OnceLock::new();
        let size_re = SIZE.get_or_init(|| {
            Regex::new(r"(?i)(\d+(?:\.\d+)?)\s*(kb|mb|gb|k|m|g)\b").unwrap()
        });
        let port_re = PORT.get_or_init(|| Regex::new(r"(?i)port\s+(\d{2,5})").unwrap());
        let quoted_re = QUOTED.get_or_init(|| Regex::new(r#"'([^']+)'|"([^"]+)""#).unwrap());
        let userhost_re =
            USERHOST.get_or_init(|| Regex::new(r"([a-z0-9_.\-]+)@([a-z0-9.\-]+)").unwrap());

        let lower = query.to_lowercase();
        let tokens = tokenize(query);
        let last_value = tokens
            .iter()
            .rev()
            .find(|t| !NON_VALUE_WORDS.contains(&t.as_str()) && t.len() > 1)
            .cloned();

        let (user, host) = userhost_re
            .captures(query)
            .map(|c| (Some(c[1].to_string()), Some(c[2].to_string())))
            .unwrap_or((None, None));

        QueryPlaceholders {
            size: size_re.captures(&lower).map(|c| {
                let unit = c[2].to_uppercase().trim_end_matches('B').to_string();
                format!("{}{}", &c[1], unit)
            }),
            port: port_re.captures(&lower).map(|c| c[1].to_string()),
            branch: tokens
                .iter()
                .position(|t| t == "branch")
                .and_then(|i| tokens.get(i + 1))
                .filter(|t| !NON_VALUE_WORDS.contains(&t.as_str()))
                .cloned()
                .or_else(|| ctx.git.as_ref().map(|g| g.branch.clone())),
            file: quoted_re
                .captures(query)
                .map(|c| {
                    c.get(1)
                        .or_else(|| c.get(2))
                        .map(|m| m.as_str())
                        .unwrap_or("")
                        .to_string()
                })
                .filter(|s| !s.is_empty())
                .or(last_value.clone()),
            dir: tokens
                .iter()
                .position(|t| t == "folder" || t == "directory")
                .and_then(|i| tokens.get(i + 1))
                .filter(|t| !NON_VALUE_WORDS.contains(&t.as_str()))
                .cloned()
                .or_else(|| last_value.clone())
                .or(Some(".".into())),
            namespace: tokens
                .iter()
                .position(|t| t == "namespace")
                .and_then(|i| tokens.get(i + 1))
                .filter(|t| !NON_VALUE_WORDS.contains(&t.as_str()))
                .cloned()
                .or_else(|| ctx.k8s.as_ref().map(|k| k.namespace.clone())),
            user,
            host,
        }
    }

    fn as_values(&self, ctx: &Context) -> PlaceholderValues {
        PlaceholderValues {
            port: self.port.clone(),
            branch: self.branch
                .clone()
                .or_else(|| ctx.git.as_ref().map(|g| g.branch.clone())),
            size: self.size.clone(),
            file: self.file.clone(),
            dir: self.dir.clone(),
            namespace: self.namespace.clone(),
            user: self.user.clone(),
            host: self.host.clone(),
            command: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{Alias, GitInfo, K8sInfo};

    fn ctx_fixture() -> Context {
        Context {
            cwd: std::env::temp_dir(),
            shell: "zsh".into(),
            os: "linux",
            git: Some(GitInfo {
                branch: "main".into(),
                detached: false,
                remotes: vec!["origin".into()],
                branches: vec!["main".into()],
                has_upstream: Some(false),
                dirty: Some(false),
            }),
            project: Default::default(),
            k8s: Some(K8sInfo {
                context: "prod".into(),
                namespace: "production".into(),
            }),
            aliases: vec![Alias {
                name: "dps".into(),
                expansion: "docker ps --format 'table {{.Names}}'".into(),
                source: "shell",
            }],
            dir_entries: vec![],
            recent_commands: vec![],
            installed_binaries: vec![
                "git".into(),
                "docker".into(),
                "kubectl".into(),
                "python3".into(),
            ],
        }
    }

    #[test]
    fn explain_tar_like_spec() {
        let specs = SpecSet::load();
        let out = explain("tar -czvf archive.tar.gz folder/", &ctx_fixture(), &specs);
        assert!(out.contains("archive"), "{}", out);
        assert!(out.contains("Create archive"));
        assert!(out.contains("tar -xzvf archive.tar.gz"));
    }

    #[test]
    fn explain_git_log_flags() {
        let specs = SpecSet::load();
        let out = explain(
            "git log --oneline --graph",
            &ctx_fixture(),
            &specs,
        );
        assert!(out.contains("Show commit history"));
        assert!(out.contains("--oneline"));
        assert!(out.contains("one commit per line"));
    }

    #[test]
    fn fix_git_upstream_from_error() {
        let ctx = ctx_fixture();
        let fixes = fix(
            "git push origin main",
            Some("fatal: The current branch main has no upstream branch.\nTo push the current branch and set the remote as upstream, use\n\n    git push --set-upstream origin main"),
            &ctx,
        );
        assert!(fixes[0].command.contains("--set-upstream origin main"));
        assert!(fixes[0].explanation.contains("not tracking"));
    }

    #[test]
    fn fix_git_upstream_inferred_without_error() {
        let ctx = ctx_fixture();
        let fixes = fix("git push origin main", None, &ctx);
        assert!(fixes
            .iter()
            .any(|f| f.command == "git push --set-upstream origin main"));
    }

    #[test]
    fn fix_command_not_found_typo() {
        let ctx = ctx_fixture();
        let fixes = fix("gti status", Some("zsh: command not found: gti"), &ctx);
        assert!(fixes.iter().any(|f| f.command == "git"));
    }

    #[test]
    fn fix_command_not_found_install_hint() {
        let ctx = ctx_fixture();
        let fixes = fix("jq .", Some("bash: jq: command not found"), &ctx);
        assert!(fixes.iter().any(|f| f.explanation.contains("apt install jq")));
    }

    #[test]
    fn fix_module_not_found() {
        let ctx = ctx_fixture();
        let fixes = fix(
            "python manage.py migrate",
            Some("ModuleNotFoundError: No module named 'django'"),
            &ctx,
        );
        assert!(fixes.iter().any(|f| f.command == "source .venv/bin/activate"));
        assert!(fixes
            .iter()
            .any(|f| f.command == "pip install -r requirements.txt"));
        assert!(fixes.iter().any(|f| f.command == "pip install django"));
    }

    #[test]
    fn fix_port_in_use() {
        let ctx = ctx_fixture();
        let fixes = fix(
            "npm run dev",
            Some("Error: listen EADDRINUSE: address already in use :::3000"),
            &ctx,
        );
        assert!(fixes[0].command.contains(":3000"));
    }

    #[test]
    fn generate_find_large_files_with_size() {
        let ctx = ctx_fixture();
        let results = generate("find all files larger than 100MB and delete them", &ctx, None);
        assert!(results
            .iter()
            .any(|r| r.command == "find . -type f -size +100M -delete"));
        let del = results.iter().find(|r| r.command.contains("-delete")).unwrap();
        assert!(!del.safer.is_empty());
        assert!(del.safer[0].contains("-print"));
    }

    #[test]
    fn generate_du_os_aware() {
        let ctx = ctx_fixture();
        let results = generate("show disk usage by folder", &ctx, None);
        assert!(results
            .iter()
            .any(|r| r.command == "du -h --max-depth=1 | sort -hr"));
    }

    #[test]
    fn generate_compress_tar() {
        let ctx = ctx_fixture();
        let results = generate("compress folder with tar", &ctx, None);
        assert!(results.iter().any(|r| r.command.starts_with("tar -czvf")));
    }

    #[test]
    fn generate_extract_tar() {
        let ctx = ctx_fixture();
        let results = generate("how do I extract a tar.gz file?", &ctx, None);
        assert!(results
            .iter()
            .any(|r| r.command.starts_with("tar -xzvf")));
    }

    #[test]
    fn generate_docker_prune() {
        let ctx = ctx_fixture();
        let results = generate("docker command to remove unused images", &ctx, None);
        assert!(results
            .iter()
            .any(|r| r.command == "docker image prune -a" && !r.safer.is_empty()));
    }

    #[test]
    fn generate_show_running_containers_suggests_alias() {
        let ctx = ctx_fixture();
        let results = generate("show running containers", &ctx, None);
        let alias_hit = results.iter().find(|r| r.source == "alias");
        assert!(alias_hit.is_some(), "got {:?}", results.iter().map(|r| (&r.command, r.source)).collect::<Vec<_>>());
        assert_eq!(alias_hit.unwrap().command, "dps");
    }

    #[test]
    fn generate_k8s_namespace_from_context() {
        let ctx = ctx_fixture();
        let results = generate("list kubernetes pods", &ctx, None);
        assert!(results
            .iter()
            .any(|r| r.command.contains("kubectl get pods")));
    }

    #[test]
    fn generate_kill_port() {
        let ctx = ctx_fixture();
        let results = generate("kill process on port 8080", &ctx, None);
        assert!(results.iter().any(|r| r.command.contains(":8080")));
    }

    #[test]
    fn generate_git_undo_commit() {
        let ctx = ctx_fixture();
        let results = generate("undo last commit", &ctx, None);
        assert!(results
            .iter()
            .any(|r| r.command == "git reset --soft HEAD~1"));
    }

    #[test]
    fn generate_uses_history() {
        let dir = std::env::temp_dir().join(format!("sm-nl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conn = crate::store::open_at(&dir.join("t.db")).unwrap();
        crate::store::record_command(
            &conn,
            &crate::store::HistoryEntry {
                command: "pg_dump -U postgres -h localhost -F c -b -v -f backup.dump mydb".into(),
                ts: 1,
                ..Default::default()
            },
        )
        .unwrap();
        let ctx = ctx_fixture();
        let results = generate("postgres backup command from last week", &ctx, Some(&conn));
        assert!(results
            .iter()
            .any(|r| r.command.starts_with("pg_dump") && r.source == "history"));
    }
}
