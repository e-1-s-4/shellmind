//! Small shared helpers: lexical tokenization, fuzzy matching, typo
//! correction and terminal styling.

use std::collections::HashSet;

/// Split a string (command or natural-language query) into normalized,
/// lowercase alphanumeric terms for lexical matching.
pub fn tokenize(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect()
}

/// Remove common English stop-words from a natural-language query so the
/// remaining terms carry the actual intent ("show disk usage by folder" →
/// `disk usage folder`).
pub fn strip_stopwords(tokens: &[String]) -> Vec<String> {
    const STOP: &[&str] = &[
        "a", "an", "the", "of", "to", "for", "in", "on", "at", "and", "or", "my", "me", "please",
        "show", "how", "do", "i", "all", "that", "with", "from", "command", "cmd", "run", "use",
        "using", "want", "need", "can", "you", "give", "make", "get", "what", "whats", "is",
        "are", "find", "list", "up", "out",
    ];
    tokens
        .iter()
        .filter(|t| !STOP.contains(&t.as_str()))
        .cloned()
        .collect()
}

/// Token set intersection size — used as a cheap relevance signal.
pub fn overlap(a: &[String], b: &[String]) -> usize {
    let set: HashSet<&str> = b.iter().map(|s| s.as_str()).collect();
    a.iter().filter(|t| set.contains(t.as_str())).count()
}

/// Case-insensitive fuzzy subsequence match.
///
/// Returns `Some(score)` when every character of `needle` appears in
/// `haystack` in order. Higher scores mean better matches: exact prefixes,
/// word boundaries and contiguous runs score highest, scattered matches
/// score lower.
pub fn fuzzy_match(needle: &str, haystack: &str) -> Option<i32> {
    let needle = needle.to_lowercase();
    let haystack = haystack.to_lowercase();
    if needle.is_empty() {
        return Some(0);
    }
    if haystack.contains(&needle) {
        // Contiguous substring: strong signal, reward short haystacks.
        return Some(500 - (haystack.len() as i32).min(400));
    }
    let hc: Vec<char> = haystack.chars().collect();
    let mut hi = 0usize;
    let mut score = 0i32;
    let mut prev_match: Option<usize> = None;
    for nc in needle.chars() {
        let mut found = None;
        while hi < hc.len() {
            if hc[hi] == nc {
                found = Some(hi);
                hi += 1;
                break;
            }
            hi += 1;
        }
        let pos = found?;
        let bonus = match prev_match {
            Some(p) if pos == p + 1 => 4, // contiguous run
            _ => {
                if pos == 0 {
                    5 // prefix
                } else if !hc[pos - 1].is_alphanumeric() {
                    3 // word boundary
                } else {
                    1
                }
            }
        };
        score += bonus;
        prev_match = Some(pos);
    }
    Some(score - (haystack.len() as i32) / 8)
}

/// Jaro–Winkler string similarity in `[0, 1]`. Used for typo correction
/// (`gti` → `git`) and near-miss history matching.
pub fn jaro_winkler(a: &str, b: &str) -> f64 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let max_dist = (a.len().max(b.len()) / 2).saturating_sub(1);
    let mut a_match = vec![false; a.len()];
    let mut b_match = vec![false; b.len()];
    let mut matches = 0usize;
    for (i, ca) in a.iter().enumerate() {
        let start = i.saturating_sub(max_dist);
        let end = (i + max_dist + 1).min(b.len());
        for j in start..end {
            if !b_match[j] && *ca == b[j] {
                a_match[i] = true;
                b_match[j] = true;
                matches += 1;
                break;
            }
        }
    }
    if matches == 0 {
        return 0.0;
    }
    let mut transpositions = 0usize;
    let mut k = 0usize;
    for i in 0..a.len() {
        if a_match[i] {
            while !b_match[k] {
                k += 1;
            }
            if a[i] != b[k] {
                transpositions += 1;
            }
            k += 1;
        }
    }
    let m = matches as f64;
    let jaro = (m / a.len() as f64 + m / b.len() as f64 + (m - transpositions as f64 / 2.0) / m)
        / 3.0;
    // Winkler prefix boost
    let prefix = a
        .iter()
        .zip(b.iter())
        .take(4)
        .take_while(|(x, y)| x == y)
        .count() as f64;
    jaro + 0.1 * prefix * (1.0 - jaro)
}

/// True when `candidate` looks like a typo of `target` (high similarity but
/// not equal). Short adjacent transpositions (`gti` → `git`) are handled
/// explicitly because Jaro scores them low for very short strings.
pub fn is_typo_of(candidate: &str, target: &str) -> bool {
    if candidate == target || candidate.len().abs_diff(target.len()) > 2 {
        return false;
    }
    if candidate.len() <= 5 {
        let mut a: Vec<char> = candidate.chars().collect();
        let mut b: Vec<char> = target.chars().collect();
        a.sort();
        b.sort();
        if a == b {
            return true; // same characters, different order → transposition
        }
    }
    jaro_winkler(candidate, target) >= 0.84
}

/// Truncate a single-line summary to `max` chars, appending an ellipsis
/// when it does not fit.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", cut.trim_end())
    }
}

/// Pad a string to `width` display columns (character count is a good
/// approximation for the ASCII-only descriptions we emit).
pub fn pad(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - len))
    }
}

/// ANSI styling helper. Detects `NO_COLOR` and dumb terminals once and
/// degrades to plain text when either is present.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    pub enabled: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self::new()
    }
}

impl Style {
    pub fn new() -> Self {
        let enabled = std::env::var_os("NO_COLOR").is_none()
            && std::env::var("TERM").map(|t| t != "dumb").unwrap_or(true);
        Style { enabled }
    }

    fn wrap(&self, code: &str, s: &str) -> String {
        if self.enabled {
            format!("{}{}\x1b[0m", code, s)
        } else {
            s.to_string()
        }
    }

    pub fn dim(&self, s: &str) -> String {
        self.wrap("\x1b[2m", s)
    }
    pub fn bold(&self, s: &str) -> String {
        self.wrap("\x1b[1m", s)
    }
    pub fn green(&self, s: &str) -> String {
        self.wrap("\x1b[32m", s)
    }
    pub fn yellow(&self, s: &str) -> String {
        self.wrap("\x1b[33m", s)
    }
    pub fn red(&self, s: &str) -> String {
        self.wrap("\x1b[31m", s)
    }
    pub fn cyan(&self, s: &str) -> String {
        self.wrap("\x1b[36m", s)
    }
    pub fn magenta(&self, s: &str) -> String {
        self.wrap("\x1b[35m", s)
    }
    pub fn blue(&self, s: &str) -> String {
        self.wrap("\x1b[34m", s)
    }
}

/// Current operating system as a lowercase slug (`linux`, `macos`, ...).
pub fn os_name() -> &'static str {
    std::env::consts::OS
}

/// True on macOS, where several coreutils flags differ (`du -d 1` instead
/// of `du --max-depth=1`, BSD sed, ...).
pub fn is_macos() -> bool {
    std::env::consts::OS == "macos"
}

/// Current unix timestamp in seconds.
pub fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_splits_on_non_alnum() {
        assert_eq!(
            tokenize("docker ps --format 'table {{.Names}}'"),
            vec!["docker", "ps", "--format", "table", "names"]
        );
    }

    #[test]
    fn fuzzy_prefers_prefix_and_word_boundaries() {
        let prefix = fuzzy_match("graph", "--graph").unwrap();
        let scattered = fuzzy_match("gaph", "--graph").unwrap();
        assert!(prefix > scattered);
        assert!(fuzzy_match("zzz", "--graph").is_none());
    }

    #[test]
    fn jaro_winkler_detects_typos() {
        assert!(is_typo_of("gti", "git"));
        assert!(is_typo_of("dcoker", "docker"));
        assert!(!is_typo_of("git", "grep"));
        assert!(!is_typo_of("git", "git"));
    }

    #[test]
    fn stopwords_removed() {
        let t = tokenize("show disk usage by folder");
        let kept = strip_stopwords(&t);
        assert!(kept.contains(&"disk".to_string()));
        assert!(kept.contains(&"folder".to_string()));
        assert!(!kept.contains(&"show".to_string()));
    }
}
