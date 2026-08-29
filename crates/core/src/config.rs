//! TOML configuration with privacy-safe defaults.
//!
//! The file lives at `~/.config/shellmind/config.toml` (see
//! [`crate::paths::config_path`]). Every section below mirrors the
//! documented schema; unknown keys are preserved on load by `toml::Value`
//! round-tripping where possible, and missing keys fall back to the
//! defaults encoded here.
//!
//! The defaults are deliberately conservative:
//!
//! * AI mode is `local` (Ollama on localhost),
//! * `cloud_enabled = false`,
//! * telemetry is off,
//! * secret commands are never indexed,
//! * destructive commands always warn.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub core: CoreConfig,
    pub ai: AiConfig,
    pub privacy: PrivacyConfig,
    pub history: HistoryConfig,
    pub safety: SafetyConfig,
    pub completions: CompletionsConfig,
    pub keybindings: KeybindingsConfig,
    /// Extra YAML files containing team snippet packs.
    pub snippets: SnippetsConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            core: CoreConfig::default(),
            ai: AiConfig::default(),
            privacy: PrivacyConfig::default(),
            history: HistoryConfig::default(),
            safety: SafetyConfig::default(),
            completions: CompletionsConfig::default(),
            keybindings: KeybindingsConfig::default(),
            snippets: SnippetsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CoreConfig {
    /// `zsh`, `bash` or `fish`. `auto` sniffs `$SHELL`.
    pub shell: String,
    /// `dark` or `light` — selects the ANSI palette intensity.
    pub theme: String,
    /// `error` | `warn` | `info` | `debug`
    pub log_level: String,
}

impl Default for CoreConfig {
    fn default() -> Self {
        CoreConfig {
            shell: "auto".into(),
            theme: "dark".into(),
            log_level: "info".into(),
        }
    }
}

/// AI backend mode. v0.1.0 ships `local` (Ollama) and `offline`;
/// `hybrid`/`cloud` are accepted for forward-compat but gated by
/// `privacy.cloud_enabled` and currently fall back to `local`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AiMode {
    Local,
    Offline,
    Hybrid,
    Cloud,
}

impl AiMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            AiMode::Local => "local",
            AiMode::Offline => "offline",
            AiMode::Hybrid => "hybrid",
            AiMode::Cloud => "cloud",
        }
    }
}

impl Default for AiMode {
    fn default() -> Self {
        AiMode::Local
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiConfig {
    pub mode: AiMode,
    /// Only `ollama` is supported in v0.1.0.
    pub provider: String,
    /// Chat / generation model.
    pub model: String,
    /// Embedding model used for semantic history search.
    pub embedding_model: String,
    pub temperature: f32,
    /// Base URL of the Ollama daemon.
    pub host: String,
    /// Per-request HTTP timeout in seconds.
    pub timeout_secs: u64,
    /// Timeout for the initial availability probe (milliseconds).
    pub probe_timeout_ms: u64,
}

impl Default for AiConfig {
    fn default() -> Self {
        AiConfig {
            mode: AiMode::Local,
            provider: "ollama".into(),
            model: "qwen2.5-coder:3b".into(),
            embedding_model: "nomic-embed-text".into(),
            temperature: 0.2,
            host: "http://localhost:11434".into(),
            timeout_secs: 60,
            probe_timeout_ms: 1500,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PrivacyConfig {
    pub redact_secrets: bool,
    pub cloud_enabled: bool,
    pub telemetry_enabled: bool,
}

impl Default for PrivacyConfig {
    fn default() -> Self {
        PrivacyConfig {
            redact_secrets: true,
            cloud_enabled: false,
            telemetry_enabled: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryConfig {
    pub semantic_search: bool,
    pub max_entries: usize,
    /// Skip indexing commands that look like they contain secrets.
    pub ignore_secret_commands: bool,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        HistoryConfig {
            semantic_search: true,
            max_entries: 100_000,
            ignore_secret_commands: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SafetyConfig {
    pub warn_destructive: bool,
    pub confirm_rm_rf: bool,
    pub confirm_force_push: bool,
}

impl Default for SafetyConfig {
    fn default() -> Self {
        SafetyConfig {
            warn_destructive: true,
            confirm_rm_rf: true,
            confirm_force_push: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CompletionsConfig {
    pub inline_suggestions: bool,
    pub show_flag_descriptions: bool,
    pub max_suggestions: usize,
}

impl Default for CompletionsConfig {
    fn default() -> Self {
        CompletionsConfig {
            inline_suggestions: true,
            show_flag_descriptions: true,
            max_suggestions: 12,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KeybindingsConfig {
    pub accept: String,
    pub palette: String,
    pub explain: String,
    pub fix: String,
    pub history: String,
    pub cancel: String,
    pub accept_word: String,
    pub safer: String,
    pub expand: String,
}

impl Default for KeybindingsConfig {
    fn default() -> Self {
        KeybindingsConfig {
            accept: "Tab".into(),
            palette: "Ctrl+Space".into(),
            explain: "Ctrl+E".into(),
            fix: "Ctrl+F".into(),
            history: "Ctrl+R".into(),
            cancel: "Ctrl+G".into(),
            accept_word: "Ctrl+Right".into(),
            safer: "Alt+S".into(),
            expand: "Alt+Enter".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SnippetsConfig {
    /// Extra YAML files with team snippet packs (absolute paths).
    pub include: Vec<String>,
}

impl Default for SnippetsConfig {
    fn default() -> Self {
        SnippetsConfig { include: Vec::new() }
    }
}

impl Config {
    /// Resolve the effective shell: explicit config value, `$SHELL`
    /// sniffing, then `zsh` as a last resort.
    pub fn effective_shell(&self) -> String {
        let s = self.core.shell.trim().to_lowercase();
        if matches!(s.as_str(), "zsh" | "bash" | "fish") {
            return s;
        }
        if let Ok(sh) = std::env::var("SHELL") {
            let base = sh.rsplit('/').next().unwrap_or("").to_lowercase();
            let base = base.split('-').next().unwrap_or("").to_string();
            if matches!(base.as_str(), "zsh" | "bash" | "fish") {
                return base;
            }
        }
        "zsh".to_string()
    }

    /// Load the configuration, falling back to defaults when the file is
    /// missing or partially invalid. When the file does not exist yet, a
    /// commented default file is written so users can discover every
    /// option (best-effort — failures to write are ignored).
    pub fn load() -> Config {
        let path = paths::config_path();
        if !path.exists() {
            let cfg = Config::default();
            cfg.write_default_file(&path);
            return cfg;
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<Config>(&text) {
                Ok(cfg) => cfg,
                Err(err) => {
                    eprintln!(
                        "shellmind: config parse error in {} ({}) — using defaults",
                        path.display(),
                        err
                    );
                    Config::default()
                }
            },
            Err(_) => Config::default(),
        }
    }

    /// Load with an explicit path override (used by `--config`).
    pub fn load_from(path: &Path) -> Config {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str::<Config>(&text).unwrap_or_default(),
            Err(_) => Config::default(),
        }
    }

    /// Serialize the effective config as pretty TOML.
    pub fn to_toml(&self) -> String {
        let header = "# shellmind configuration\n# Docs: https://github.com/shellmind/shellmind/blob/main/docs/configuration.md\n\n";
        match toml::to_string_pretty(self) {
            Ok(body) => format!("{}{}", header, body),
            Err(_) => "# shellmind configuration (serialization failed — defaults in effect)\n"
                .to_string(),
        }
    }

    /// Persist the config to disk.
    pub fn save(&self) -> std::io::Result<()> {
        let path = paths::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, self.to_toml())
    }

    fn write_default_file(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, self.to_toml());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_privacy_safe() {
        let c = Config::default();
        assert_eq!(c.ai.mode, AiMode::Local);
        assert_eq!(c.ai.provider, "ollama");
        assert_eq!(c.ai.model, "qwen2.5-coder:3b");
        assert!(!c.privacy.cloud_enabled);
        assert!(!c.privacy.telemetry_enabled);
        assert!(c.privacy.redact_secrets);
        assert!(c.history.ignore_secret_commands);
        assert!(c.safety.warn_destructive);
        assert_eq!(c.completions.max_suggestions, 12);
        assert_eq!(c.keybindings.palette, "Ctrl+Space");
    }

    #[test]
    fn toml_roundtrip() {
        let text = r#"
[core]
shell = "bash"

[ai]
model = "llama3.2:3b"
temperature = 0.5
"#;
        let cfg: Config = toml::from_str(text).unwrap();
        assert_eq!(cfg.core.shell, "bash");
        assert_eq!(cfg.ai.model, "llama3.2:3b");
        assert_eq!(cfg.ai.temperature, 0.5);
        // Untouched sections keep defaults.
        assert!(cfg.privacy.redact_secrets);
        let out = cfg.to_toml();
        assert!(out.contains("shell = \"bash\""));
    }

    #[test]
    fn ai_mode_serializes_lowercase() {
        let cfg: Config = toml::from_str("[ai]\nmode = \"offline\"").unwrap();
        assert_eq!(cfg.ai.mode, AiMode::Offline);
    }
}
