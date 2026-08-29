//! Embedded shell integration scripts.
//!
//! `sm init <shell>` prints these verbatim, so `eval "$(sm init zsh)"`
//! works with zero filesystem setup — the plugin lives inside the binary.
//! The same files exist in the repo under `shellmind-shell/` for editing
//! and review; they are embedded at build time via `include_str!`.

pub const ZSH: &str = include_str!("../../../shellmind-shell/zsh/shellmind.zsh");
pub const BASH: &str = include_str!("../../../shellmind-shell/bash/shellmind.bash");
pub const FISH: &str = include_str!("../../../shellmind-shell/fish/shellmind.fish");

/// Return the integration script for a shell name.
pub fn script(shell: &str) -> Option<&'static str> {
    match shell {
        "zsh" => Some(ZSH),
        "bash" => Some(BASH),
        "fish" => Some(FISH),
        _ => None,
    }
}

/// All supported shells.
pub const SUPPORTED: &[&str] = &["zsh", "bash", "fish"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_embedded() {
        for sh in SUPPORTED {
            let s = script(sh).unwrap();
            assert!(s.len() > 500, "{} plugin looks too small", sh);
            assert!(s.contains("shellmind"), "{} plugin missing marker", sh);
        }
    }

    #[test]
    fn zsh_plugin_has_key_widgets() {
        let zsh = script("zsh").unwrap();
        assert!(zsh.contains("_sm_palette"));
        assert!(zsh.contains("_sm_explain"));
        assert!(zsh.contains("_sm_fix"));
        assert!(zsh.contains("_sm_history_search"));
        assert!(zsh.contains("POSTDISPLAY"));
        assert!(zsh.contains("preexec_functions"));
    }

    #[test]
    fn bash_plugin_binds_readline() {
        let bash = script("bash").unwrap();
        assert!(bash.contains("READLINE_LINE"));
        assert!(bash.contains("bind -x"));
        assert!(bash.contains("PROMPT_COMMAND"));
    }

    #[test]
    fn fish_plugin_binds() {
        let fish = script("fish").unwrap();
        assert!(fish.contains("commandline"));
        assert!(fish.contains("__sm_palette"));
        assert!(fish.contains("fish_postexec"));
    }

    #[test]
    fn unknown_shell_rejected() {
        assert!(script("powershell").is_none());
    }
}
