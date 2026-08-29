//! Local context collection.
//!
//! The context snapshot is what makes shellmind suggestions feel native:
//! completions for `npm run` come from the actual `package.json` on disk,
//! `kubectl` picks up the namespace from kubeconfig, git branches come
//! from `.git/refs` — no shellouts where a file read will do.
//!
//! Privacy: the collector reads **names, never values**. Environment
//! variables are never captured; git status is optional and only produced
//! through the `git` binary when it exists.

use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::store::Connection;

/// A shell alias learned from the active shell or rc files.
#[derive(Debug, Clone, Serialize)]
pub struct Alias {
    pub name: String,
    pub expansion: String,
    #[serde(skip)]
    pub source: &'static str,
}

/// Git repository state (read directly from `.git` where possible).
#[derive(Debug, Clone, Serialize, Default)]
pub struct GitInfo {
    pub branch: String,
    pub detached: bool,
    pub remotes: Vec<String>,
    pub branches: Vec<String>,
    /// `Some(true)` when the current branch tracks an upstream.
    pub has_upstream: Option<bool>,
    /// `Some(true)` when there are uncommitted changes (needs git binary).
    pub dirty: Option<bool>,
}

/// Project type detection from well-known manifest files.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ProjectInfo {
    pub kind: Option<&'static str>,
    pub package_manager: Option<&'static str>,
    pub npm_scripts: Vec<String>,
    pub compose_services: Vec<String>,
    pub makefile_targets: Vec<String>,
}

/// Kubernetes context from kubeconfig.
#[derive(Debug, Clone, Serialize, Default)]
pub struct K8sInfo {
    pub context: String,
    pub namespace: String,
}

/// Everything the completion engine and AI prompts may know about the
/// current environment.
#[derive(Debug, Clone, Serialize)]
pub struct Context {
    pub cwd: PathBuf,
    pub shell: String,
    pub os: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git: Option<GitInfo>,
    pub project: ProjectInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub k8s: Option<K8sInfo>,
    pub aliases: Vec<Alias>,
    /// Entries in the current directory (capped).
    pub dir_entries: Vec<String>,
    /// Recently used commands (from the history store, when available).
    pub recent_commands: Vec<String>,
    /// Binaries discoverable on PATH (capped).
    pub installed_binaries: Vec<String>,
}

impl Context {
    /// Collect a snapshot for `cwd`. `conn` may be `None` (no history yet).
    pub fn collect(cwd: &Path, conn: Option<&Connection>) -> Context {
        let shell = std::env::var("SHELL")
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
            .unwrap_or_else(|| "zsh".into());
        let aliases = load_aliases();
        Context {
            cwd: cwd.to_path_buf(),
            shell,
            os: crate::util::os_name(),
            git: git_info(cwd),
            project: project_info(cwd),
            k8s: k8s_info(),
            dir_entries: dir_entries(cwd, 500),
            recent_commands: conn.map(recent_commands).unwrap_or_default(),
            installed_binaries: binaries_from_path(),
            aliases,
        }
    }

    /// Compact, redaction-safe text block used inside AI prompts.
    pub fn to_prompt_text(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!("os: {}", self.os));
        lines.push(format!("shell: {}", self.shell));
        lines.push(format!("cwd: {}", self.cwd.display()));
        if let Some(g) = &self.git {
            lines.push(format!("git branch: {}", g.branch));
            if let Some(d) = g.dirty {
                lines.push(format!(
                    "git dirty: {}",
                    if d { "yes" } else { "no" }
                ));
            }
        }
        if let Some(k) = &self.project.kind {
            lines.push(format!("project: {}", k));
        }
        if !self.project.npm_scripts.is_empty() {
            lines.push(format!("npm scripts: {}", self.project.npm_scripts.join(", ")));
        }
        if !self.project.compose_services.is_empty() {
            lines.push(format!(
                "compose services: {}",
                self.project.compose_services.join(", ")
            ));
        }
        if let Some(k) = &self.k8s {
            if !k.namespace.is_empty() {
                lines.push(format!("k8s namespace: {}", k.namespace));
            }
        }
        if !self.aliases.is_empty() {
            let names: Vec<&str> = self.aliases.iter().map(|a| a.name.as_str()).take(20).collect();
            lines.push(format!("aliases: {}", names.join(", ")));
        }
        lines.join("\n")
    }
}

fn recent_commands(conn: &Connection) -> Vec<String> {
    let mut stmt = match conn.prepare("SELECT command FROM history WHERE secret = 0 ORDER BY ts DESC, id DESC LIMIT 20") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    stmt.query_map([], |r| r.get::<_, String>(0))
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Git
// ---------------------------------------------------------------------------

fn find_upwards(start: &Path, marker: &str) -> Option<PathBuf> {
    let mut dir = Some(start.to_path_buf());
    let mut hops = 0;
    while let Some(d) = dir {
        if d.join(marker).exists() {
            return Some(d);
        }
        hops += 1;
        if hops > 12 {
            return None;
        }
        dir = d.parent().map(|p| p.to_path_buf());
    }
    None
}

fn git_info(cwd: &Path) -> Option<GitInfo> {
    let root = find_upwards(cwd, ".git")?;
    let gitdir = root.join(".git");
    // `.git` can be a file (worktrees) pointing at the real git dir.
    let gitdir = if gitdir.is_file() {
        let text = std::fs::read_to_string(&gitdir).ok()?;
        let path = text.strip_prefix("gitdir:")?.trim();
        root.join(path)
    } else {
        gitdir
    };

    let mut info = GitInfo::default();

    // Branch from HEAD.
    if let Ok(head) = std::fs::read_to_string(gitdir.join("HEAD")) {
        let head = head.trim();
        if let Some(rest) = head.strip_prefix("ref: refs/heads/") {
            info.branch = rest.to_string();
        } else {
            info.detached = true;
            info.branch = head.chars().take(7).collect();
        }
    }

    // Local branches from refs/heads.
    let heads = gitdir.join("refs").join("heads");
    let mut branches = BTreeSet::new();
    collect_refs(&heads, "", &mut branches, 0);
    if branches.is_empty() {
        // packed-refs fallback
        if let Ok(packed) = std::fs::read_to_string(gitdir.join("packed-refs")) {
            for line in packed.lines() {
                if let Some(rest) = line.strip_prefix("ref: refs/heads/") {
                    if let Some((_, name)) = rest.split_once(' ') {
                        branches.insert(name.trim().to_string());
                    }
                }
            }
        }
    }
    if !info.branch.is_empty() {
        branches.insert(info.branch.clone());
    }
    info.branches = branches.into_iter().take(200).collect();

    // Remotes + upstream from config.
    if let Ok(cfg) = std::fs::read_to_string(gitdir.join("config")) {
        let mut current_remote: Option<String> = None;
        let mut current_branch: Option<String> = None;
        for line in cfg.lines() {
            let t = line.trim();
            if t.starts_with("[remote ") {
                let name = t
                    .trim_start_matches("[remote ")
                    .trim_end_matches(']')
                    .trim_matches('"');
                current_remote = Some(name.to_string());
                info.remotes.push(name.to_string());
            } else if t.starts_with("[branch ") {
                let name = t
                    .trim_start_matches("[branch ")
                    .trim_end_matches(']')
                    .trim_matches('"');
                current_branch = Some(name.to_string());
                if name == info.branch {
                    info.has_upstream = Some(false);
                }
            } else if t.contains('=') {
                let (k, v) = t.split_once('=').unwrap();
                let k = k.trim();
                let v = v.trim();
                if k == "remote" && current_branch.as_deref() == Some(info.branch.as_str()) {
                    let _ = v;
                    info.has_upstream = Some(true);
                }
                let _ = current_remote.take();
            } else {
                current_branch = None;
            }
        }
    }

    // A known branch with no [branch "<name>"] tracking section has no
    // upstream.
    if !info.branch.is_empty() && info.has_upstream.is_none() {
        info.has_upstream = Some(false);
    }

    // Dirty flag via the git binary (best-effort, cached by callers).
    info.dirty = git_dirty(&root);

    Some(info)
}

fn collect_refs(dir: &Path, prefix: &str, out: &mut BTreeSet<String>, depth: usize) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if e.path().is_dir() {
            collect_refs(&e.path(), &format!("{}{}/", prefix, name), out, depth + 1);
        } else {
            out.insert(format!("{}{}", prefix, name));
        }
    }
}

fn git_dirty(root: &Path) -> Option<bool> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(!out.stdout.is_empty())
}

// ---------------------------------------------------------------------------
// Project detection
// ---------------------------------------------------------------------------

fn project_info(cwd: &Path) -> ProjectInfo {
    let root = find_upwards(cwd, "package.json")
        .or_else(|| find_upwards(cwd, "Cargo.toml"))
        .or_else(|| find_upwards(cwd, "pyproject.toml"))
        .or_else(|| find_upwards(cwd, "go.mod"))
        .unwrap_or_else(|| cwd.to_path_buf());
    let mut p = ProjectInfo::default();

    // Node.
    if root.join("package.json").exists() {
        p.kind = Some("node");
        p.package_manager = Some(if root.join("pnpm-lock.yaml").exists() {
            "pnpm"
        } else if root.join("yarn.lock").exists() {
            "yarn"
        } else if root.join("bun.lockb").exists() {
            "bun"
        } else {
            "npm"
        });
        if let Ok(text) = std::fs::read_to_string(root.join("package.json")) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(scripts) = v.get("scripts").and_then(|s| s.as_object()) {
                    p.npm_scripts = scripts.keys().cloned().collect();
                }
            }
        }
    } else if root.join("Cargo.toml").exists() {
        p.kind = Some("rust");
        p.package_manager = Some("cargo");
    } else if root.join("pyproject.toml").exists() || root.join("requirements.txt").exists() {
        p.kind = Some("python");
        p.package_manager = Some("pip");
    } else if root.join("go.mod").exists() {
        p.kind = Some("go");
        p.package_manager = Some("go");
    }

    // Docker compose services.
    for name in [
        "docker-compose.yml",
        "docker-compose.yaml",
        "compose.yml",
        "compose.yaml",
    ] {
        let f = root.join(name);
        if f.exists() {
            if let Ok(text) = std::fs::read_to_string(&f) {
                if let Ok(v) = serde_yaml::from_str::<serde_yaml::Value>(&text) {
                    if let Some(services) = v.get("services").and_then(|s| s.as_mapping() ) {
                        p.compose_services = services
                            .keys()
                            .filter_map(|k| k.as_str().map(|s| s.to_string()))
                            .collect();
                        break;
                    }
                }
            }
        }
    }

    // Makefile targets (cheap line scan).
    if let Ok(text) = std::fs::read_to_string(root.join("Makefile")) {
        for line in text.lines() {
            if !line.starts_with('\t')
                && line.contains(':')
                && !line.starts_with('#')
                && !line.trim().is_empty()
            {
                let target = line.split(':').next().unwrap_or("").trim();
                if !target.is_empty()
                    && target
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
                    && !target.contains('=')
                {
                    p.makefile_targets.push(target.to_string());
                }
            }
        }
        p.makefile_targets.truncate(100);
    }

    p
}

// ---------------------------------------------------------------------------
// Kubernetes
// ---------------------------------------------------------------------------

fn k8s_info() -> Option<K8sInfo> {
    let path = std::env::var("SHELLMIND_KUBECONFIG")
        .ok()
        .or_else(|| std::env::var("KUBECONFIG").ok())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".kube").join("config")))?;
    let text = std::fs::read_to_string(path).ok()?;
    let v: serde_yaml::Value = serde_yaml::from_str(&text).ok()?;
    let current = v.get("current-context").and_then(|c| c.as_str())?.to_string();
    let mut namespace = String::new();
    if let Some(contexts) = v.get("contexts").and_then(|c| c.as_sequence()) {
        for ctx in contexts {
            let name = ctx.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if name == current {
                if let Some(c) = ctx.get("context") {
                    namespace = c
                        .get("namespace")
                        .and_then(|n| n.as_str())
                        .unwrap_or("default")
                        .to_string();
                }
                break;
            }
        }
    }
    if namespace.is_empty() {
        namespace = "default".into();
    }
    Some(K8sInfo {
        context: current,
        namespace,
    })
}

// ---------------------------------------------------------------------------
// Aliases
// ---------------------------------------------------------------------------

/// Load aliases from the plugin-maintained cache, falling back to parsing
/// rc files directly.
pub fn load_aliases() -> Vec<Alias> {
    let mut aliases = Vec::new();
    if let Ok(text) = std::fs::read_to_string(crate::paths::aliases_cache()) {
        for line in text.lines() {
            if let Some(a) = parse_alias_line(line) {
                aliases.push(a);
            }
        }
    }
    if aliases.is_empty() {
        for rc in rc_files() {
            if let Ok(text) = std::fs::read_to_string(&rc) {
                for line in text.lines() {
                    if let Some(a) = parse_alias_line(line) {
                        aliases.push(a);
                    }
                }
            }
        }
    }
    aliases.truncate(300);
    aliases
}

fn rc_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Some(home) = dirs::home_dir() {
        for f in [
            ".zshrc",
            ".bashrc",
            ".bash_aliases",
            ".config/fish/config.fish",
        ] {
            files.push(home.join(f));
        }
    }
    files
}

/// Accepts:
/// * `name<TAB>expansion`          (zsh plugin cache)
/// * `alias name='expansion'`      (bash `alias -p`, rc files)
/// * `abbr -a name expansion...`   (fish abbreviations)
fn parse_alias_line(line: &str) -> Option<Alias> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    if let Some(rest) = line.strip_prefix("alias ") {
        let rest = rest.trim();
        let (name, value) = rest.split_once('=')?;
        let value = value.trim().trim_matches('\'').trim_matches('"');
        return Some(Alias {
            name: name.trim().to_string(),
            expansion: value.to_string(),
            source: "shell",
        });
    }
    if let Some(rest) = line.strip_prefix("abbr -a ") {
        let rest = rest.trim();
        let mut parts = rest.splitn(2, ' ');
        let name = parts.next()?.trim();
        let value = parts.next().unwrap_or("").trim();
        return Some(Alias {
            name: name.to_string(),
            expansion: value.to_string(),
            source: "shell",
        });
    }
    if let Some((name, value)) = line.split_once('\t') {
        if !name.is_empty() && !value.is_empty() {
            return Some(Alias {
                name: name.to_string(),
                expansion: value.to_string(),
                source: "shell",
            });
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Files & binaries
// ---------------------------------------------------------------------------

fn dir_entries(cwd: &Path, cap: usize) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(cwd) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let name = if e.path().is_dir() {
                format!("{}/", name)
            } else {
                name
            };
            out.push(name);
            if out.len() >= cap {
                break;
            }
        }
    }
    out.sort();
    out
}

/// List binary names found on PATH (deduplicated, capped).
pub fn binaries_from_path() -> Vec<String> {
    let path = std::env::var("PATH").unwrap_or_default();
    let dirs: Vec<PathBuf> = path
        .split(':')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect();
    binaries_from_path_dirs(&dirs)
}

pub fn binaries_from_path_dirs(dirs: &[PathBuf]) -> Vec<String> {
    let mut set = BTreeSet::new();
    for dir in dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_file() {
                    if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                        if !name.starts_with('.') {
                            set.insert(name.to_string());
                        }
                    }
                }
            }
        }
        if set.len() > 4000 {
            break;
        }
    }
    set.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
        fn fixture(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sm-ctx-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_git_repo(dir: &Path) {
        let gd = dir.join(".git");
        std::fs::create_dir_all(gd.join("refs").join("heads").join("feature")).unwrap();
        std::fs::write(gd.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(
            gd.join("config"),
            "[core]\n\trepositoryformatversion = 0\n\
             [remote \"origin\"]\n\turl = git@github.com:me/repo.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n\
             [branch \"main\"]\n\tremote = origin\n\tmerge = refs/heads/main\n",
        )
        .unwrap();
        std::fs::write(gd.join("refs").join("heads").join("main"), "abc123\n").unwrap();
        std::fs::write(gd.join("refs").join("heads").join("feature").join("x"), "def456\n").unwrap();
    }

    #[test]
    fn git_info_from_fixture() {
        let dir = fixture("git");
        make_git_repo(&dir);
        let g = git_info(&dir).unwrap();
        assert_eq!(g.branch, "main");
        assert!(!g.detached);
        assert!(g.remotes.contains(&"origin".to_string()));
        assert!(g.branches.contains(&"main".to_string()));
        assert!(g.branches.contains(&"feature/x".to_string()));
        assert_eq!(g.has_upstream, Some(true));
    }

    #[test]
    fn git_info_no_upstream() {
        let dir = fixture("git2");
        let gd = dir.join(".git");
        std::fs::create_dir_all(gd.join("refs").join("heads")).unwrap();
        std::fs::write(gd.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(gd.join("config"), "[core]\n\tbare = false\n").unwrap();
        std::fs::write(gd.join("refs").join("heads").join("main"), "abc\n").unwrap();
        let g = git_info(&dir).unwrap();
        // No [branch] section at all → definitively no upstream.
        assert_eq!(g.has_upstream, Some(false));
    }

    #[test]
    fn project_info_node_and_compose() {
        let dir = fixture("node");
        std::fs::write(
            dir.join("package.json"),
            r#"{"name":"app","scripts":{"dev":"vite","build":"vite build","test":"vitest"}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("docker-compose.yml"),
            "services:\n  api:\n    build: .\n  worker:\n    image: worker:latest\n",
        )
        .unwrap();
        let p = project_info(&dir);
        assert_eq!(p.kind, Some("node"));
        assert_eq!(p.package_manager, Some("npm"));
        assert!(p.npm_scripts.contains(&"dev".to_string()));
        assert!(p.npm_scripts.contains(&"build".to_string()));
        assert!(p.compose_services.contains(&"api".to_string()));
        assert!(p.compose_services.contains(&"worker".to_string()));
    }

    #[test]
    fn project_info_rust() {
        let dir = fixture("rust");
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        let p = project_info(&dir);
        assert_eq!(p.kind, Some("rust"));
        assert_eq!(p.package_manager, Some("cargo"));
    }

    #[test]
    fn k8s_namespace_from_kubeconfig() {
        let _m = crate::testutil::env_lock();
        let dir = fixture("k8s");
        let kc = dir.join("kubeconfig");
        std::fs::write(
            &kc,
            "apiVersion: v1\nkind: Config\ncurrent-context: prod\ncontexts:\n  - name: prod\n    context:\n      cluster: c1\n      namespace: production\n  - name: dev\n    context:\n      cluster: c2\n      namespace: dev\n",
        )
        .unwrap();
        std::env::set_var("SHELLMIND_KUBECONFIG", &kc);
        let k = k8s_info().unwrap();
        assert_eq!(k.context, "prod");
        assert_eq!(k.namespace, "production");
        std::env::remove_var("SHELLMIND_KUBECONFIG");
    }

    #[test]
    fn alias_lines_all_formats() {
        assert_eq!(
            parse_alias_line("dps\tdocker ps --format 'table'").map(|a| a.name),
            Some("dps".into())
        );
        assert_eq!(
            parse_alias_line("alias k='kubectl'").map(|a| a.expansion),
            Some("kubectl".into())
        );
        assert_eq!(
            parse_alias_line("abbr -a gs git status").map(|a| a.expansion),
            Some("git status".into())
        );
        assert!(parse_alias_line("# comment").is_none());
        assert!(parse_alias_line("ls -la").is_none());
    }

    #[test]
    fn binaries_from_dirs() {
        let dir = fixture("bin");
        std::fs::write(dir.join("git"), "#!/bin/sh").unwrap();
        std::fs::write(dir.join("docker"), "#!/bin/sh").unwrap();
        std::fs::write(dir.join(".hidden"), "").unwrap();
        let bins = binaries_from_path_dirs(&[dir.clone()]);
        assert!(bins.contains(&"git".to_string()));
        assert!(bins.contains(&"docker".to_string()));
        assert!(!bins.contains(&".hidden".to_string()));
    }

    #[test]
    fn dir_entries_listed_with_slash_for_dirs() {
        let dir = fixture("ls");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("README.md"), "hi").unwrap();
        let entries = dir_entries(&dir, 100);
        assert!(entries.contains(&"src/".to_string()));
        assert!(entries.contains(&"README.md".to_string()));
    }

    #[test]
    fn context_prompt_text_is_compact() {
        let dir = fixture("prompt");
        make_git_repo(&dir);
        std::fs::write(
            dir.join("package.json"),
            r#"{"scripts":{"dev":"vite"}}"#,
        )
        .unwrap();
        let ctx = Context::collect(&dir, None);
        let text = ctx.to_prompt_text();
        assert!(text.contains("git branch: main"));
        assert!(text.contains("npm scripts: dev"));
        assert!(text.contains("project: node"));
    }
}
