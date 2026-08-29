//! shellmind-core: the engine behind the `sm` / `shellmind` CLI.
//!
//! This crate contains every piece of shellmind that is independent from the
//! command-line surface:
//!
//! * [`parser`]      – shell command line tokenizer / structural parser
//! * [`redact`]      – secret redaction before anything leaves the machine
//! * [`store`]       – SQLite persistence layer
//! * [`history`]     – shell history import + hybrid (BM25 + vector) search
//! * [`safety`]      – destructive command detection and safer alternatives
//! * [`completions`] – static specs + dynamic, context-aware completions
//! * [`context`]     – local environment snapshot (git, project, aliases, ...)
//! * [`ai`]          – Ollama-backed AI engine with a deterministic offline fallback
//! * [`snippets`]    – personal and team command templates
//! * [`daemon`]      – Unix-socket daemon (warm cache, background indexing)
//! * [`plugins`]     – embedded shell integration scripts (zsh / bash / fish)
//! * [`config`]      – TOML configuration with safe defaults
//!
//! Everything in this crate is local-first: no network access happens unless
//! the user explicitly configures it, and all outbound text passes through
//! [`redact`].

pub mod ai;
pub mod completions;
pub mod config;
pub mod context;
pub mod daemon;
pub mod history;
pub mod parser;
pub mod paths;
pub mod plugins;
pub mod redact;
pub mod safety;
pub mod snippets;
pub mod store;
pub mod util;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Cross-module mutex serializing tests that mutate environment variables
/// (tests share one process; `std::env::set_var` is process-global).
#[cfg(test)]
pub(crate) mod testutil {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    static ENV_MTX: OnceLock<Mutex<()>> = OnceLock::new();

    pub(crate) fn env_lock() -> MutexGuard<'static, ()> {
        ENV_MTX
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }
}
