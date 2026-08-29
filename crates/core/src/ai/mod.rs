//! The AI engine: Ollama-backed, offline-safe.
//!
//! v0.1.0 speaks exactly one LLM protocol — Ollama's local REST API —
//! because local-first is the product's promise. Every method follows the
//! same resilience rule:
//!
//! > try the model → on any failure, fall back to the deterministic
//! > offline engine (see [`offline`]) — the feature never hard-fails.
//!
//! Before any text is sent to the model it is passed through
//! [`crate::redact::redact`] (when `privacy.redact_secrets` is on, the
//! default). When `ai.mode = "offline"` no network call is attempted at
//! all, and `privacy.cloud_enabled = false` blocks remote providers.

pub mod kb;
pub mod offline;
pub mod ollama;
pub mod prompts;

use crate::completions::spec::SpecSet;
use crate::config::{AiConfig, PrivacyConfig};
use crate::context::Context;
use crate::store::Connection;
use offline::{Fix, NlResult};

pub struct AiEngine {
    pub cfg: AiConfig,
    pub privacy: PrivacyConfig,
    client: Option<ollama::Ollama>,
    /// Result of the availability probe at construction time.
    pub ollama_reachable: bool,
}

impl AiEngine {
    /// Build the engine and probe Ollama availability (fast timeout).
    pub fn new(cfg: AiConfig, privacy: PrivacyConfig) -> AiEngine {
        let wants_model = matches!(cfg.mode, crate::config::AiMode::Local | crate::config::AiMode::Hybrid)
            || (matches!(cfg.mode, crate::config::AiMode::Cloud) && privacy.cloud_enabled);
        if !wants_model || cfg.provider != "ollama" {
            return AiEngine {
                cfg,
                privacy,
                client: None,
                ollama_reachable: false,
            };
        }
        let client = ollama::Ollama::new(&cfg.host, cfg.timeout_secs);
        let reachable = client.ping(cfg.probe_timeout_ms);
        AiEngine {
            cfg,
            privacy,
            client: Some(client),
            ollama_reachable: reachable,
        }
    }

    /// Label shown in `sm status`.
    pub fn mode_label(&self) -> &'static str {
        if self.ollama_reachable {
            "local (ollama)"
        } else if self.cfg.mode == crate::config::AiMode::Offline {
            "offline"
        } else {
            "local (ollama unreachable — offline fallback)"
        }
    }

    /// Redact text according to privacy settings before external use.
    fn scrub(&self, text: &str) -> String {
        if self.privacy.redact_secrets {
            crate::redact::redact(text)
        } else {
            text.to_string()
        }
    }

    fn chat(&self, system: &str, user: &str) -> Option<String> {
        let client = self.client.as_ref()?;
        if !self.ollama_reachable {
            return None;
        }
        client
            .chat(&self.cfg.model, system, user, self.cfg.temperature)
            .ok()
    }

    // -- explain ---------------------------------------------------------

    pub fn explain(&self, command: &str, ctx: &Context, specs: &SpecSet) -> String {
        if let Some(raw) = self.chat(
            prompts::EXPLAIN_SYSTEM,
            &prompts::explain_user(&self.scrub(command), &ctx.to_prompt_text()),
        ) {
            let answer = prompts::parse_llm_answer(&raw);
            let mut out = String::new();
            if let Some(cmd) = &answer.command {
                out.push_str(&format!("{}\n", cmd));
            }
            if !answer.explanation.is_empty() {
                out.push_str(&format!("\n{}\n", answer.explanation));
            }
            if !out.trim().is_empty() {
                return out;
            }
        }
        offline::explain(command, ctx, specs)
    }

    // -- fix -------------------------------------------------------------

    pub fn fix(&self, command: &str, error: Option<&str>, ctx: &Context) -> Vec<Fix> {
        if let Some(err) = error {
            if let Some(raw) = self.chat(
                prompts::FIX_SYSTEM,
                &prompts::fix_user(&self.scrub(command), &self.scrub(err), &ctx.to_prompt_text()),
            ) {
                let answer = prompts::parse_llm_answer(&raw);
                if let Some(cmd) = answer.command {
                    let mut fixes = vec![Fix {
                        command: cmd,
                        explanation: answer.explanation.clone(),
                    }];
                    for alt in answer.alternatives {
                        fixes.push(Fix {
                            command: alt,
                            explanation: "safer alternative (from model)".into(),
                        });
                    }
                    return fixes;
                }
            }
        }
        offline::fix(command, error, ctx)
    }

    // -- natural language → command ---------------------------------------

    pub fn generate(&self, query: &str, ctx: &Context, conn: Option<&Connection>) -> Vec<NlResult> {
        let mut results: Vec<NlResult> = Vec::new();
        if let Some(raw) = self.chat(
            prompts::GENERATE_SYSTEM,
            &prompts::generate_user(&self.scrub(query), &ctx.to_prompt_text()),
        ) {
            let answer = prompts::parse_llm_answer(&raw);
            if let Some(cmd) = answer.command {
                results.push(NlResult {
                    command: cmd,
                    explanation: answer.explanation.clone(),
                    safer: answer.alternatives.clone(),
                    source: "ollama",
                    score: 1000,
                });
            }
        }
        // Offline candidates always follow (deduped) — deterministic
        // results are ranked above the model's when confidence is high.
        for r in offline::generate(query, ctx, conn) {
            if !results.iter().any(|x| x.command == r.command) {
                results.push(r);
            }
        }
        results
    }

    // -- embeddings --------------------------------------------------------

    /// Embed a single text (for hybrid history search). `None` when the
    /// model is unreachable — callers degrade to lexical search.
    pub fn embed(&self, text: &str) -> Option<Vec<f32>> {
        let client = self.client.as_ref()?;
        if !self.ollama_reachable {
            return None;
        }
        client
            .embed(&self.cfg.embedding_model, &[text])
            .ok()
            .and_then(|mut v| v.pop())
    }

    /// Batch embed (for the daemon's background indexer).
    pub fn embed_batch(&self, texts: &[&str]) -> Option<Vec<Vec<f32>>> {
        let client = self.client.as_ref()?;
        if !self.ollama_reachable {
            return None;
        }
        client.embed(&self.cfg.embedding_model, texts).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offline_engine() -> AiEngine {
        AiEngine::new(
            AiConfig {
                mode: crate::config::AiMode::Offline,
                ..Default::default()
            },
            PrivacyConfig::default(),
        )
    }

    #[test]
    fn offline_mode_never_touches_network() {
        let e = offline_engine();
        assert!(!e.ollama_reachable);
        assert_eq!(e.mode_label(), "offline");
        assert!(e.client.is_none());
        assert!(e.embed("hello").is_none());
    }

    #[test]
    fn offline_engine_still_explains_and_fixes() {
        use crate::context::{GitInfo, K8sInfo};
        let e = offline_engine();
        let ctx = crate::context::Context {
            cwd: std::env::temp_dir(),
            shell: "zsh".into(),
            os: "linux",
            git: Some(GitInfo {
                branch: "main".into(),
                has_upstream: Some(false),
                ..Default::default()
            }),
            project: Default::default(),
            k8s: Some(K8sInfo::default()),
            aliases: vec![],
            dir_entries: vec![],
            recent_commands: vec![],
            installed_binaries: vec!["git".into()],
        };
        let specs = SpecSet::load();
        let out = e.explain("tar -czvf a.tar.gz b/", &ctx, &specs);
        assert!(out.contains("Create archive"));
        let fixes = e.fix("git push origin main", None, &ctx);
        assert!(fixes
            .iter()
            .any(|f| f.command == "git push --set-upstream origin main"));
    }

    #[test]
    fn unreachable_ollama_falls_back() {
        // Point at a port with no listener; construction succeeds but the
        // probe fails and the engine runs in offline fallback.
        let e = AiEngine::new(
            AiConfig {
                host: "http://127.0.0.1:1".into(),
                probe_timeout_ms: 300,
                ..Default::default()
            },
            PrivacyConfig::default(),
        );
        assert!(!e.ollama_reachable);
        assert!(e.mode_label().contains("offline fallback"));
        let ctx = crate::context::Context::collect(&std::env::temp_dir(), None);
        let results = e.generate("show disk usage by folder", &ctx, None);
        assert!(results
            .iter()
            .any(|r| r.command == "du -h --max-depth=1 | sort -hr"));
    }

    #[test]
    fn scrub_redacts_secrets() {
        let e = offline_engine();
        let ghp = format!("{}{}", "gh", "p_abcdefghijklmnopqrstuvwxyz123456");
        let scrubbed = e.scrub(&format!("curl --token {} https://api.io", ghp));
        assert!(scrubbed.contains("[REDACTED"));
    }
}
