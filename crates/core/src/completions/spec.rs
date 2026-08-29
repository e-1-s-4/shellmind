//! Static completion specs (YAML).
//!
//! Specs describe the static shape of a CLI: subcommand trees and flags
//! with human-readable descriptions. They are the highest-trust
//! completion source — before AI, before fuzzy history matching.
//!
//! Loading precedence (per binary name):
//!
//! 1. `~/.config/shellmind/completions/<name>.yaml` (user override),
//! 2. the spec embedded in the binary at build time
//!    (git, docker, kubectl, npm ship out of the box).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Embedded default specs (single source of truth: the repo `completions/`
/// directory — the same files users can copy and customize).
pub const EMBEDDED_SPECS: &[(&str, &str)] = &[
    ("git", include_str!("../../../../completions/git.yaml")),
    ("docker", include_str!("../../../../completions/docker.yaml")),
    ("kubectl", include_str!("../../../../completions/kubectl.yaml")),
    ("npm", include_str!("../../../../completions/npm.yaml")),
];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Spec {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub flags: Vec<Flag>,
    #[serde(default)]
    pub subcommands: Vec<Subcommand>,
    /// Dynamic completion key (`npm_scripts`, `docker_services`, ...).
    #[serde(default)]
    pub dynamic: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Subcommand {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub flags: Vec<Flag>,
    #[serde(default)]
    pub subcommands: Vec<Subcommand>,
    #[serde(default)]
    pub dynamic: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Flag {
    /// Full flag text as inserted: `--oneline`, `-a`, `--author=`.
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Value hint appended as `=` (`author`, `file`, `date`, ...).
    #[serde(default)]
    pub arg: Option<String>,
}

/// A resolved node in the spec tree: either the root spec or a nested
/// subcommand, with its own flags and children.
#[derive(Debug, Clone)]
pub struct Node<'a> {
    pub name: String,
    pub description: String,
    pub flags: &'a [Flag],
    pub subcommands: &'a [Subcommand],
    pub dynamic: Option<&'a str>,
    /// Global flags from the root spec (always available).
    pub global_flags: &'a [Flag],
}

/// The set of all loaded specs.
pub struct SpecSet {
    specs: Vec<Spec>,
    index: HashMap<String, usize>,
}

impl Default for SpecSet {
    fn default() -> Self {
        Self::load()
    }
}

impl SpecSet {
    /// Load all specs: embedded defaults overridden by user files.
    pub fn load() -> SpecSet {
        let mut specs: Vec<Spec> = Vec::new();
        let mut index = HashMap::new();

        for (name, text) in EMBEDDED_SPECS {
            if let Ok(mut spec) = serde_yaml::from_str::<Spec>(text) {
                if spec.name.is_empty() {
                    spec.name = name.to_string();
                }
                index.insert(spec.name.clone(), specs.len());
                specs.push(spec);
            }
        }

        // User overrides / additions.
        let dir = crate::paths::user_completions_dir();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            let mut files: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().map(|e| e == "yaml" || e == "yml").unwrap_or(false))
                .collect();
            files.sort();
            for f in files {
                if let Ok(text) = std::fs::read_to_string(&f) {
                    if let Ok(spec) = serde_yaml::from_str::<Spec>(&text) {
                        if spec.name.is_empty() {
                            continue;
                        }
                        if let Some(&i) = index.get(&spec.name) {
                            specs[i] = spec;
                        } else {
                            index.insert(spec.name.clone(), specs.len());
                            specs.push(spec);
                        }
                    }
                }
            }
        }

        SpecSet { specs, index }
    }

    pub fn get(&self, binary: &str) -> Option<&Spec> {
        self.index.get(binary).map(|&i| &self.specs[i])
    }

    pub fn binaries(&self) -> Vec<&str> {
        self.specs.iter().map(|s| s.name.as_str()).collect()
    }

    /// Resolve the deepest spec node matching the leading words.
    ///
    /// Returns the node plus the words that remain unmatched (the
    /// arguments). E.g. for `docker compose up --build` the node is
    /// `compose → up` and remaining words are `["--build"]`.
    pub fn resolve<'a>(&'a self, binary: &str, words: &[String]) -> Option<Node<'a>> {
        let spec = self.get(binary)?;
        let node = Node {
            name: spec.name.clone(),
            description: spec.description.clone(),
            flags: &spec.flags,
            subcommands: &spec.subcommands,
            dynamic: spec.dynamic.as_deref(),
            global_flags: &spec.flags,
        };
        Some(self.descend(node, words))
    }

    fn descend<'a>(&'a self, mut node: Node<'a>, words: &[String]) -> Node<'a> {
        for w in words {
            let Some(next) = node
                .subcommands
                .iter()
                .find(|s| s.name == *w)
            else {
                break;
            };
            node = Node {
                name: next.name.clone(),
                description: next.description.clone(),
                flags: &next.flags,
                subcommands: &next.subcommands,
                dynamic: next.dynamic.as_deref(),
                global_flags: node.global_flags,
            };
        }
        node
    }
}

impl Flag {
    /// Insert text for completion: `--author=` for value flags,
    /// `--oneline` otherwise.
    pub fn insert_text(&self) -> String {
        match &self.arg {
            Some(_) if !self.name.contains('=') => format!("{}=", self.name),
            _ => self.name.clone(),
        }
    }

    /// Bare flag name (`--author=X` → `--author`).
    pub fn bare_name(&self) -> &str {
        self.name.split('=').next().unwrap_or(&self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_specs_parse() {
        let set = SpecSet::load();
        let git = set.get("git").unwrap();
        assert!(!git.subcommands.is_empty());
        let log = git.subcommands.iter().find(|s| s.name == "log").unwrap();
        assert!(log.flags.iter().any(|f| f.name == "--oneline"));
        assert!(log.flags.iter().any(|f| f.name == "--graph"));
        let author = log.flags.iter().find(|f| f.name == "--author").unwrap();
        assert!(author.description.contains("author"));

        let docker = set.get("docker").unwrap();
        let compose = docker
            .subcommands
            .iter()
            .find(|s| s.name == "compose")
            .unwrap();
        assert!(compose.subcommands.iter().any(|s| s.name == "up"));

        let kubectl = set.get("kubectl").unwrap();
        let get = kubectl.subcommands.iter().find(|s| s.name == "get").unwrap();
        assert!(get.subcommands.iter().any(|s| s.name == "pods"));

        let npm = set.get("npm").unwrap();
        assert!(npm.subcommands.iter().any(|s| s.name == "run"));
        assert!(npm.dynamic.as_deref() == Some("npm_scripts"));
    }

    #[test]
    fn resolve_descends_chain() {
        let set = SpecSet::load();
        let words: Vec<String> = ["compose", "up"].iter().map(|s| s.to_string()).collect();
        let node = set.resolve("docker", &words).unwrap();
        assert_eq!(node.name, "up");

        let words2: Vec<String> = ["log", "--oneline", "HEAD"].iter().map(|s| s.to_string()).collect();
        let node2 = set.resolve("git", &words2).unwrap();
        assert_eq!(node2.name, "log");

        // Unknown binary.
        assert!(set.resolve("definitely-not-a-cli", &[]).is_none());
    }

    #[test]
    fn flag_insert_text() {
        let f = Flag {
            name: "--author".into(),
            description: String::new(),
            arg: Some("author".into()),
        };
        assert_eq!(f.insert_text(), "--author=");
        let f2 = Flag {
            name: "--oneline".into(),
            ..Default::default()
        };
        assert_eq!(f2.insert_text(), "--oneline");
    }
}
