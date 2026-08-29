//! Personal and team command snippets.
//!
//! Two sources:
//!
//! * the user's snippet store at `~/.config/shellmind/snippets.yaml`
//!   (written by `sm save`),
//! * team packs — YAML files listed in `config.toml` under
//!   `[snippets] include = [...]` plus the packs shipped with shellmind.
//!
//! Commands may contain `{{placeholders}}` that `sm use` fills in —
//! interactively on a TTY, or non-interactively via `--set key=value`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SnippetsFile {
    #[serde(default)]
    pub snippets: Vec<Snippet>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snippet {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub command: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// `user` or `team`
    #[serde(skip)]
    pub source: &'static str,
}

impl SnippetsFile {
    pub fn load(path: &Path) -> SnippetsFile {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_yaml::from_str(&text).unwrap_or_default(),
            Err(_) => SnippetsFile::default(),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let yaml = serde_yaml::to_string(self).unwrap_or_default();
        std::fs::write(path, yaml)
    }
}

/// Embedded team packs shipped with the binary (repo `snippets/` dir).
pub const EMBEDDED_PACKS: &[(&str, &str)] = &[
    ("git", include_str!("../../../snippets/git.yaml")),
    ("docker", include_str!("../../../snippets/docker.yaml")),
    ("postgres", include_str!("../../../snippets/postgres.yaml")),
];

/// All snippets visible to the user (personal first, then team packs).
pub fn all() -> Vec<Snippet> {
    let mut out: Vec<Snippet> = Vec::new();

    let user_path = crate::paths::snippets_path();
    let user_file = SnippetsFile::load(&user_path);
    for mut s in user_file.snippets {
        s.source = "user";
        out.push(s);
    }

    // Team packs from config includes.
    let cfg = crate::config::Config::load();
    for inc in &cfg.snippets.include {
        let p = PathBuf::from(inc);
        let f = SnippetsFile::load(&p);
        for mut s in f.snippets {
            s.source = "team";
            out.push(s);
        }
    }

    // Embedded packs (marked team; read-only).
    for (_, text) in EMBEDDED_PACKS {
        if let Ok(f) = serde_yaml::from_str::<SnippetsFile>(text) {
            for mut s in f.snippets {
                s.source = "team";
                out.push(s);
            }
        }
    }

    out
}

/// Save (or replace) a personal snippet.
pub fn save(name: &str, command: &str, description: &str, tags: &[String]) -> std::io::Result<()> {
    let path = crate::paths::snippets_path();
    let mut file = SnippetsFile::load(&path);
    file.snippets.retain(|s| s.name != name);
    file.snippets.push(Snippet {
        name: name.to_string(),
        description: description.to_string(),
        command: command.to_string(),
        tags: tags.to_vec(),
        source: "user",
    });
    file.snippets.sort_by(|a, b| a.name.cmp(&b.name));
    file.save(&path)
}

/// Delete a personal snippet by name.
pub fn delete(name: &str) -> std::io::Result<bool> {
    let path = crate::paths::snippets_path();
    let mut file = SnippetsFile::load(&path);
    let before = file.snippets.len();
    file.snippets.retain(|s| s.name != name);
    if file.snippets.len() == before {
        return Ok(false);
    }
    file.save(&path)?;
    Ok(true)
}

/// Find a snippet by (fuzzy-tolerant) name.
pub fn find(name: &str) -> Option<Snippet> {
    let all_snippets = all();
    if let Some(exact) = all_snippets.iter().find(|s| s.name == name) {
        return Some(exact.clone());
    }
    let lower = name.to_lowercase();
    all_snippets
        .into_iter()
        .find(|s| s.name.to_lowercase().replace([' ', '-'], "") == lower.replace([' ', '-'], ""))
}

/// Placeholder names in a snippet command, in order of appearance.
pub fn placeholders(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = command;
    while let Some(start) = rest.find("{{") {
        if let Some(end) = rest[start..].find("}}") {
            let key = rest[start + 2..start + end].trim().to_string();
            if !key.is_empty() && !out.contains(&key) {
                out.push(key);
            }
            rest = &rest[start + end + 2..];
        } else {
            break;
        }
    }
    out
}

/// Fill placeholders with values. Missing values keep the placeholder text
/// so the user can see what still needs replacing.
pub fn render(command: &str, values: &std::collections::HashMap<String, String>) -> String {
    let mut out = command.to_string();
    for key in placeholders(command) {
        let pat = format!("{{{{{}}}}}", key);
        if let Some(v) = values.get(&key) {
            out = out.replace(&pat, v);
        }
    }
    out
}

/// Interactive prompt for missing placeholders (TTY only).
pub fn prompt_missing(command: &str) -> String {
    use std::io::Write;
    let mut values = std::collections::HashMap::new();
    for key in placeholders(command) {
        print!("{}: ", key);
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_ok() {
            let v = line.trim();
            if !v.is_empty() {
                values.insert(key.clone(), v.to_string());
            }
        }
    }
    render(command, &values)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sm-snip-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn with_home<T>(name: &str, f: impl FnOnce(&Path) -> T) -> T {
        // Serialize env access across ALL tests process-wide.
        let _g = crate::testutil::env_lock();
        let dir = tmp(name);
        std::env::set_var("SHELLMIND_HOME", &dir);
        let result = f(&dir);
        std::env::remove_var("SHELLMIND_HOME");
        result
    }

    #[test]
    fn save_find_render_roundtrip() {
        with_home("roundtrip", |dir| {
            save(
                "postgres backup",
                "pg_dump -U {{user}} -h {{host}} -F c -b -v -f {{file}} {{db}}",
                "Full custom-format dump",
                &["postgres".to_string()],
            )
            .unwrap();
            let path = dir.join("snippets.yaml");
            assert!(path.exists());
            let snip = find("postgres backup").unwrap();
            assert_eq!(snip.source, "user");
            assert_eq!(
                placeholders(&snip.command),
                vec!["user", "host", "file", "db"]
            );
            let mut values = std::collections::HashMap::new();
            values.insert("user".to_string(), "postgres".to_string());
            values.insert("host".to_string(), "localhost".to_string());
            values.insert("file".to_string(), "backup.dump".to_string());
            values.insert("db".to_string(), "mydb".to_string());
            assert_eq!(
                render(&snip.command, &values),
                "pg_dump -U postgres -h localhost -F c -b -v -f backup.dump mydb"
            );
        });
    }

    #[test]
    fn fuzzy_name_lookup() {
        with_home("fuzzy", |_| {
            save("reset local branch", "git fetch origin && git reset --hard origin/main", "", &[])
                .unwrap();
            assert!(find("Reset Local Branch").is_some());
            assert!(find("reset-local-branch").is_some());
            assert!(find("nope").is_none());
        });
    }

    #[test]
    fn embedded_team_packs_load() {
        let all_snippets = all();
        assert!(all_snippets
            .iter()
            .any(|s| s.source == "team" && s.command.contains("pg_dump")));
        assert!(all_snippets
            .iter()
            .any(|s| s.source == "team" && s.command.contains("docker compose up")));
    }

    #[test]
    fn missing_placeholders_kept() {
        let mut values = std::collections::HashMap::new();
        values.insert("user".to_string(), "postgres".to_string());
        assert_eq!(
            render("pg_dump -U {{user}} -h {{host}}", &values),
            "pg_dump -U postgres -h {{host}}"
        );
    }

    #[test]
    fn delete_snippet() {
        with_home("delete", |_| {
            save("temp", "echo hi", "", &[]).unwrap();
            assert!(delete("temp").unwrap());
            assert!(!delete("temp").unwrap());
        });
    }
}
