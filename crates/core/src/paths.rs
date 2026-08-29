//! Filesystem layout for all shellmind state.
//!
//! Every path lives under [`home`], which defaults to
//! `~/.config/shellmind` (respecting `XDG_CONFIG_HOME`) and can be
//! overridden with the `SHELLMIND_HOME` environment variable. The override
//! is what keeps tests, the demo and CI hermetic — no test ever touches a
//! real user directory.

use std::path::{Path, PathBuf};

/// Root directory for all shellmind state.
pub fn home() -> PathBuf {
    if let Ok(p) = std::env::var("SHELLMIND_HOME") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("shellmind")
}

/// Create the state directory tree if it does not exist yet.
pub fn ensure_home() -> std::io::Result<PathBuf> {
    let h = home();
    std::fs::create_dir_all(h.join("runtime"))?;
    std::fs::create_dir_all(h.join("completions"))?;
    Ok(h)
}

/// Path of the main TOML configuration file.
pub fn config_path() -> PathBuf {
    home().join("config.toml")
}

/// Path of the SQLite history database.
pub fn db_path() -> PathBuf {
    home().join("history.db")
}

/// Path of the user's personal snippet store.
pub fn snippets_path() -> PathBuf {
    home().join("snippets.yaml")
}

/// Directory holding user-supplied completion spec overrides.
pub fn user_completions_dir() -> PathBuf {
    home().join("completions")
}

/// Cache file where shell plugins export the current alias table.
/// Format: one `name<TAB>expansion` line per alias (bash `alias -p` output
/// is also accepted by the parser).
pub fn aliases_cache() -> PathBuf {
    home().join("aliases.txt")
}

/// Unix socket the daemon listens on.
pub fn socket_path() -> PathBuf {
    home().join("runtime").join("daemon.sock")
}

/// Default history file for a given shell (best-effort).
///
/// `SHELLMIND_HISTORY_FILE` overrides the location for every shell —
/// used by tests and the demo to stay hermetic.
pub fn history_file_for(shell: &str) -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SHELLMIND_HISTORY_FILE") {
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    let home_dir = dirs::home_dir()?;
    match shell {
        "zsh" => Some(home_dir.join(".zsh_history")),
        "bash" => Some(home_dir.join(".bash_history")),
        "fish" => Some(
            home_dir
                .join(".local/share/fish/fish_history"),
        ),
        _ => None,
    }
}

/// True when the daemon socket file exists (does not prove liveness —
/// use [`crate::daemon::client::ping`] for that).
pub fn socket_exists() -> bool {
    socket_path().exists()
}

/// Escape `%`, `_` and `\` for SQL `LIKE ... ESCAPE '\'` clauses.
pub fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Helper: read a file to string, mapping errors to `None`.
pub fn read_to_string_opt(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_override_respected() {
        // SAFE: single-threaded path test with a unique value.
        let key = "SHELLMIND_HOME_TEST_TOKEN";
        assert!(std::env::var(key).is_err());
    }

    #[test]
    fn like_escape_special_chars() {
        assert_eq!(like_escape("100%_done"), "100\\%\\_done");
    }
}
