//! Shell command-line parsing.
//!
//! shellmind needs to understand a *partially typed* command line fast.
//! Full shell grammars (tree-sitter-bash and friends) are built for
//! complete, syntactically valid scripts; a completion engine sees broken
//! input by definition (`git log --`), so a small purpose-built tokenizer
//! + structural parser is both faster and more robust for this job.
//!
//! Supported constructs:
//!
//! * quoting (`'...'`, `"..."`, `\<char>` escapes),
//! * env prefixes (`ENV=production docker compose up`),
//! * wrapper commands (`sudo`, `nohup`, `time`, `env`, ...),
//! * pipelines and lists (`|`, `||`, `&&`, `;`, `&`),
//! * redirects (`> f`, `>> f`, `2>&1`, `< f`, `&> f`),
//! * command substitution is treated as an opaque word (we never need to
//!   look inside it for completion).

use serde::Serialize;

/// Quote state of a token as it was typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quote {
    #[default]
    None,
    Single,
    Double,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// A word with quotes/escapes already resolved.
    Word(String),
    /// A structural operator: `|`, `||`, `&&`, `;`, `&`.
    Op(String),
    /// A redirect such as `>`, `>>`, `2>&1`, `<`, `&>`, `>file`.
    Redirect(String),
}

/// One command (a single pipeline segment).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct CommandLine {
    /// The raw text of this segment.
    pub raw: String,
    /// `VAR=value` prefixes (values may be empty).
    pub env: Vec<(String, String)>,
    /// Wrappers consumed before the real binary (`sudo`, `nohup`, ...).
    pub wrappers: Vec<String>,
    /// The command name, once identified.
    pub binary: Option<String>,
    /// Every word after the binary, in order.
    pub words: Vec<String>,
    /// Leading non-flag words after the binary (`git log` → `["log"]`).
    pub subcommands: Vec<String>,
    /// All flags in order (`--oneline`, `-abc`, `--author=X`).
    pub flags: Vec<String>,
    /// Non-flag words after the subcommand chain.
    pub args: Vec<String>,
    /// Redirects attached to this segment.
    pub redirects: Vec<String>,
}

/// A full pipeline: segments joined by operators.
#[derive(Debug, Clone, Default)]
pub struct Pipeline {
    pub segments: Vec<CommandLine>,
    pub operators: Vec<String>,
}

/// Commands that may wrap the real binary without changing its meaning
/// for completion purposes.
pub const WRAPPERS: &[&str] = &[
    "sudo", "doas", "nohup", "time", "nice", "env", "command", "builtin", "watch", "strace",
    "gtimeout", "timeout",
];

fn is_env_assignment(w: &str) -> bool {
    if let Some(eq) = w.find('=') {
        let name = &w[..eq];
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
            && name.chars().next().map(|c| c.is_ascii_alphabetic() || c == '_').unwrap_or(false)
            && !w.starts_with('-')
    } else {
        false
    }
}

/// Tokenize one command line into words, operators and redirects.
pub fn tokenize(line: &str) -> Vec<Tok> {
    let mut tokens: Vec<Tok> = Vec::new();
    let mut cur = String::new();
    let mut has_word = false;
    let mut quote = Quote::None;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0usize;

    macro_rules! flush {
        () => {
            if has_word || !cur.is_empty() {
                tokens.push(Tok::Word(std::mem::take(&mut cur)));
                has_word = false;
            }
        };
    }

    while i < chars.len() {
        let c = chars[i];
        match quote {
            Quote::Single => {
                if c == '\'' {
                    quote = Quote::None;
                } else {
                    cur.push(c);
                }
                i += 1;
            }
            Quote::Double => {
                if c == '"' {
                    quote = Quote::None;
                } else if c == '\\' {
                    if let Some(&n) = chars.get(i + 1) {
                        if n == '"' || n == '\\' || n == '$' || n == '`' {
                            cur.push(n);
                            i += 2;
                            continue;
                        }
                    }
                    cur.push(c);
                    i += 1;
                } else {
                    cur.push(c);
                    i += 1;
                }
            }
            Quote::None => match c {
                '\'' => {
                    quote = Quote::Single;
                    has_word = true;
                    i += 1;
                }
                '"' => {
                    quote = Quote::Double;
                    has_word = true;
                    i += 1;
                }
                '\\' => {
                    if let Some(&n) = chars.get(i + 1) {
                        cur.push(n);
                        has_word = true;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                ' ' | '\t' | '\n' | '\r' => {
                    flush!();
                    i += 1;
                }
                ';' => {
                    flush!();
                    tokens.push(Tok::Op(";".into()));
                    i += 1;
                }
                '&' => {
                    flush!();
                    if chars.get(i + 1) == Some(&'&') {
                        tokens.push(Tok::Op("&&".into()));
                        i += 2;
                    } else if chars.get(i + 1) == Some(&'>') {
                        tokens.push(Tok::Redirect("&>".into()));
                        i += 2;
                    } else {
                        tokens.push(Tok::Op("&".into()));
                        i += 1;
                    }
                }
                '|' => {
                    flush!();
                    if chars.get(i + 1) == Some(&'|') {
                        tokens.push(Tok::Op("||".into()));
                        i += 2;
                    } else {
                        tokens.push(Tok::Op("|".into()));
                        i += 1;
                    }
                }
                '#' => {
                    // Comment outside quotes: ignore the rest of the line.
                    break;
                }
                '<' | '>' => {
                    flush!();
                    // Fold a leading fd digit that was already flushed as a
                    // standalone word ("ls 2>&1" → Redirect("2>&1")).
                    let mut op = String::new();
                    if c == '>' {
                        if let Some(Tok::Word(w)) = tokens.last() {
                            if !w.is_empty() && w.chars().all(|d| d.is_ascii_digit()) {
                                op.push_str(w);
                                tokens.pop();
                            }
                        }
                    }
                    op.push(c);
                    i += 1;
                    if c == '>' && chars.get(i) == Some(&'>') {
                        op.push('>');
                        i += 1;
                    }
                    if c == '>' && chars.get(i) == Some(&'&') {
                        op.push('&');
                        i += 1;
                        while let Some(&d) = chars.get(i) {
                            if d.is_ascii_digit() {
                                op.push(d);
                                i += 1;
                            } else {
                                break;
                            }
                        }
                    }
                    // Attached target: ">file" / ">>dir/file".
                    while let Some(&d) = chars.get(i) {
                        if d.is_whitespace() || d == ';' || d == '|' || d == '&' {
                            break;
                        }
                        op.push(d);
                        i += 1;
                    }
                    tokens.push(Tok::Redirect(op));
                }
                _ => {
                    cur.push(c);
                    has_word = true;
                    i += 1;
                }
            },
        }
    }
    if quote != Quote::None {
        // Unterminated quote: treat what we have as a word — completion
        // must cope with broken input.
        has_word = true;
    }
    flush!();
    tokens
}

fn looks_like_subcommand(w: &str) -> bool {
    !w.starts_with('-')
        && !w.contains('/')
        && !w.contains('=')
        && !w.starts_with('$')
        && !w.starts_with('.')
        && w.chars().next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false)
}

impl CommandLine {
    /// Parse a single command segment (no pipeline operators).
    pub fn parse(line: &str) -> CommandLine {
        let toks = tokenize(line);
        CommandLine::from_tokens(&toks)
    }

    pub fn from_tokens(toks: &[Tok]) -> CommandLine {
        let mut cmd = CommandLine {
            raw: toks
                .iter()
                .map(|t| match t {
                    Tok::Word(w) => w.clone(),
                    Tok::Op(o) => o.clone(),
                    Tok::Redirect(r) => r.clone(),
                })
                .collect::<Vec<_>>()
                .join(" "),
            ..Default::default()
        };
        let mut seen_flag = false;
        let mut pending_redirect: Option<String> = None;
        for tok in toks {
            match tok {
                Tok::Word(w) => {
                    // Attach the target word to a preceding bare redirect
                    // operator ("> out.txt" → ">out.txt").
                    if let Some(op) = pending_redirect.take() {
                        if let Some(last) = cmd.redirects.last_mut() {
                            *last = format!("{}{}", op, w);
                        }
                        continue;
                    }
                    if cmd.binary.is_none() {
                        if cmd.wrappers.is_empty() && cmd.env.is_empty() && is_env_assignment(w) {
                            let (k, v) = w.split_once('=').unwrap();
                            cmd.env.push((k.to_string(), v.to_string()));
                            continue;
                        }
                        if WRAPPERS.contains(&w.as_str()) {
                            cmd.wrappers.push(w.clone());
                            continue;
                        }
                        cmd.binary = Some(w.clone());
                        continue;
                    }
                    cmd.words.push(w.clone());
                    if w.starts_with('-') {
                        cmd.flags.push(w.clone());
                        seen_flag = true;
                    } else if !seen_flag && looks_like_subcommand(w) {
                        cmd.subcommands.push(w.clone());
                    } else {
                        cmd.args.push(w.clone());
                    }
                }
                Tok::Redirect(r) => {
                    cmd.redirects.push(r.clone());
                    pending_redirect = if is_pure_operator(r) { Some(r.clone()) } else { None };
                }
                Tok::Op(_) => {
                    pending_redirect = None;
                }
            }
        }
        cmd
    }

    /// Binary name with any path prefix removed (`/usr/bin/git` → `git`).
    pub fn binary_name(&self) -> Option<&str> {
        self.binary.as_deref().map(|b| b.rsplit('/').next().unwrap_or(b))
    }

    /// The flag name without any inline value (`--author=X` → `--author`).
    pub fn flag_names(&self) -> Vec<String> {
        self.flags
            .iter()
            .map(|f| f.split('=').next().unwrap_or(f).to_string())
            .collect()
    }

    /// True when the command has the `env` wrapper (its `VAR=x` arguments
    /// were parsed as env prefixes).
    pub fn has_env_wrapper(&self) -> bool {
        cmd_has_env_wrapper(&self.wrappers)
    }
}

fn cmd_has_env_wrapper(wrappers: &[String]) -> bool {
    wrappers.iter().any(|w| w == "env")
}

/// True for bare redirect operators (">", "2>&1", "<") that still need
/// their target word attached.
fn is_pure_operator(op: &str) -> bool {
    !op.is_empty()
        && op.chars().all(|c| c.is_ascii_digit() || c == '>' || c == '<' || c == '&')
        && op.contains(['>', '<'])
}

/// Split a line into pipeline segments and the operators joining them.
pub fn parse_pipeline(line: &str) -> Pipeline {
    let toks = tokenize(line);
    let mut pipeline = Pipeline::default();
    let mut current: Vec<Tok> = Vec::new();
    for tok in toks {
        match tok {
            Tok::Op(op) if matches!(op.as_str(), "|" | "||" | "&&" | ";") => {
                if !current.is_empty() {
                    pipeline.segments.push(CommandLine::from_tokens(&current));
                    current.clear();
                } else if pipeline.segments.is_empty() {
                    // Leading operator: ignore (broken input tolerance).
                    continue;
                }
                pipeline.operators.push(op);
            }
            Tok::Op(_) => {
                // `&` background operator: terminates the segment.
                if !current.is_empty() {
                    pipeline.segments.push(CommandLine::from_tokens(&current));
                    current.clear();
                }
            }
            other => current.push(other),
        }
    }
    if !current.is_empty() {
        pipeline.segments.push(CommandLine::from_tokens(&current));
    }
    pipeline
}

/// What the user is completing right now.
#[derive(Debug, Clone, Default)]
pub struct CompletionQuery {
    /// Full buffer.
    pub line: String,
    /// Buffer up to (not including) the word being typed.
    pub prefix: String,
    /// The word being typed; empty when the cursor follows a space.
    pub current_word: String,
    /// True when the cursor is right after whitespace (a fresh word).
    pub after_space: bool,
    /// Quote wrapping the current word.
    pub quote: Quote,
    /// Parsed structure of the current (last) pipeline segment.
    pub cmdline: CommandLine,
    /// All pipeline segments (the last one is `cmdline`).
    pub segments: Vec<CommandLine>,
}

impl CompletionQuery {
    /// True when the user is typing a flag (`--`, `--auth`, `-n`).
    pub fn completing_flag(&self) -> bool {
        self.current_word.starts_with('-') && self.current_word.len() >= 1
    }

    /// Binary of the segment being completed.
    pub fn binary(&self) -> Option<&str> {
        self.cmdline.binary_name()
    }
}

/// Parse a buffer + cursor position into a [`CompletionQuery`].
///
/// `cursor=None` means end-of-line. Everything after the cursor is
/// ignored — completion only looks backwards.
pub fn parse_for_completion(line: &str, cursor: Option<usize>) -> CompletionQuery {
    let cursor = cursor.unwrap_or(line.len()).min(line.len());
    let upto = &line[..cursor];

    // Determine current word + quote state by scanning forward.
    let mut word_start = 0usize;
    let mut quote = Quote::None;
    let chars: Vec<char> = upto.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            Quote::None => match c {
                '\'' | '"' => {
                    // Opening a quoted word.
                    if word_start == i || (word_start == 0 && i == 0) {
                        word_start = i;
                    }
                    quote = if c == '\'' { Quote::Single } else { Quote::Double };
                }
                '\\' => {
                    i += 1;
                }
                ' ' | '\t' | '\n' | '\r' | ';' | '|' | '&' => {
                    word_start = i + 1;
                }
                _ => {}
            },
            Quote::Single => {
                if c == '\'' {
                    quote = Quote::None;
                }
            }
            Quote::Double => {
                if c == '"' {
                    quote = Quote::None;
                } else if c == '\\' {
                    i += 1;
                }
            }
        }
        i += 1;
    }
    // word_start is a char index; convert to byte offset.
    let word_start_byte: usize = chars[..word_start].iter().map(|c| c.len_utf8()).sum();
    let prefix = upto.get(..word_start_byte).unwrap_or("").to_string();
    let current_word = upto.get(word_start_byte..).unwrap_or("").to_string();

    // Strip surrounding quotes from the typed word for matching purposes.
    let bare_word = strip_quotes(&current_word);
    let after_space = current_word.is_empty() || current_word.chars().all(|c| c.is_whitespace());

    // Parse the full pipeline of the prefix (completion applies to the
    // last segment before the cursor).
    let mut pipeline = parse_pipeline(&prefix);
    let trimmed = prefix.trim_end();
    if trimmed.ends_with('|') || trimmed.ends_with(';') || trimmed.ends_with('&') {
        // The cursor starts a brand-new command after an operator.
        pipeline.segments.push(CommandLine::default());
    }
    let segments = pipeline.segments.clone();
    let cmdline = segments.last().cloned().unwrap_or_default();

    CompletionQuery {
        line: line.to_string(),
        prefix,
        current_word: bare_word,
        after_space,
        quote,
        cmdline,
        segments,
    }
}

fn strip_quotes(w: &str) -> String {
    let mut s = w.to_string();
    if s.len() >= 2 {
        let bytes = s.as_bytes();
        let first = bytes[0];
        let last = bytes[s.len() - 1];
        if (first == b'\'' && last == b'\'') || (first == b'"' && last == b'"') {
            s = s[1..s.len() - 1].to_string();
        }
    }
    // Unterminated opening quote: strip just the leading quote so
    // completion can match the raw text being typed.
    if s.starts_with('\'') || s.starts_with('"') {
        s = s[1..].to_string();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_env_prefix_binary_subcommands_flags() {
        // Exact example from the architecture spec (§9.3).
        let cmd = CommandLine::parse("ENV=production docker compose up --build");
        assert_eq!(cmd.env, vec![("ENV".to_string(), "production".to_string())]);
        assert_eq!(cmd.binary_name(), Some("docker"));
        assert_eq!(cmd.subcommands, vec!["compose", "up"]);
        assert_eq!(cmd.flags, vec!["--build"]);
        assert!(cmd.args.is_empty());
    }

    #[test]
    fn parses_git_log_chain() {
        let cmd = CommandLine::parse("git log --oneline --graph HEAD~3");
        assert_eq!(cmd.binary_name(), Some("git"));
        assert_eq!(cmd.subcommands, vec!["log"]);
        assert_eq!(cmd.flags, vec!["--oneline", "--graph"]);
        assert_eq!(cmd.args, vec!["HEAD~3"]);
    }

    #[test]
    fn handles_wrappers() {
        let cmd = CommandLine::parse("sudo rm -rf ./");
        assert_eq!(cmd.wrappers, vec!["sudo"]);
        assert_eq!(cmd.binary_name(), Some("rm"));
        assert_eq!(cmd.flags, vec!["-rf"]);

        let cmd2 = CommandLine::parse("nohup npm run build &");
        assert_eq!(cmd2.wrappers, vec!["nohup"]);
        assert_eq!(cmd2.binary_name(), Some("npm"));
    }

    #[test]
    fn preserves_quoted_words() {
        let cmd = CommandLine::parse("git commit -m \"some message with spaces\"");
        assert!(cmd
            .words
            .iter()
            .any(|w| w == "some message with spaces"));
        let cmd2 = CommandLine::parse("echo 'a b'\\ c");
        assert!(cmd2.words.iter().any(|w| w == "a b c"));
    }

    #[test]
    fn parses_pipelines() {
        let p = parse_pipeline("docker ps | grep nginx && echo done");
        assert_eq!(p.segments.len(), 3);
        assert_eq!(p.operators, vec!["|", "&&"]);
        assert_eq!(p.segments[0].binary_name(), Some("docker"));
        assert_eq!(p.segments[1].binary_name(), Some("grep"));
        assert_eq!(p.segments[2].binary_name(), Some("echo"));
    }

    #[test]
    fn parses_redirects() {
        let cmd = CommandLine::parse("du -h > out.txt");
        assert_eq!(cmd.redirects, vec![">out.txt"]);
        let cmd2 = CommandLine::parse("ls 2>&1 | less");
        assert_eq!(cmd2.redirects, vec!["2>&1"]);
        let cmd3 = CommandLine::parse("sort < in.txt >> out.log");
        assert!(cmd3.redirects.contains(&"<in.txt".to_string()));
        assert!(cmd3.redirects.contains(&">>out.log".to_string()));
    }

    #[test]
    fn completion_query_partial_flag() {
        let q = parse_for_completion("git log --", None);
        assert_eq!(q.current_word, "--");
        assert!(!q.after_space);
        assert!(q.completing_flag());
        assert_eq!(q.binary(), Some("git"));
        assert_eq!(q.cmdline.subcommands, vec!["log"]);
    }

    #[test]
    fn completion_query_after_space() {
        let q = parse_for_completion("kubectl get ", None);
        assert_eq!(q.current_word, "");
        assert!(q.after_space);
        assert!(!q.completing_flag());
        assert_eq!(q.binary(), Some("kubectl"));
        assert_eq!(q.cmdline.subcommands, vec!["get"]);
    }

    #[test]
    fn completion_query_mid_line_cursor() {
        let q = parse_for_completion("git log --oneline --gr", Some("git log --oneline --gr".len()));
        assert_eq!(q.current_word, "--gr");
        let q2 = parse_for_completion("npm run   dev", Some(8)); // "npm run "
        assert_eq!(q2.current_word, "");
        assert!(q2.after_space);
    }

    #[test]
    fn completion_query_pipeline_tail() {
        // Typing a fresh command after a pipe: the current word IS the
        // (partial) binary, no binary is committed yet.
        let q = parse_for_completion("docker ps --format x | gre", None);
        assert_eq!(q.binary(), None);
        assert_eq!(q.current_word, "gre");
        assert!(q.after_space == false);
    }

    #[test]
    fn completion_query_quoted_word() {
        let q = parse_for_completion("git commit -m \"fix bug", None);
        assert_eq!(q.quote, Quote::Double);
        assert_eq!(q.current_word, "fix bug");
    }

    #[test]
    fn completion_query_empty() {
        let q = parse_for_completion("", None);
        assert!(q.after_space);
        assert_eq!(q.binary(), None);
        let q2 = parse_for_completion("git", None);
        assert_eq!(q2.current_word, "git");
        assert_eq!(q2.binary(), None);
    }

    #[test]
    fn flag_names_split_inline_values() {
        let cmd = CommandLine::parse("git log --author=alice --since=2024-01-01");
        assert_eq!(
            cmd.flag_names(),
            vec!["--author".to_string(), "--since".to_string()]
        );
    }

    #[test]
    fn args_after_flags_are_args() {
        let cmd = CommandLine::parse("kubectl get pods -n prod web");
        assert_eq!(cmd.binary_name(), Some("kubectl"));
        // Leading word-like tokens form the subcommand chain; fine-grained
        // resource/arg classification is the completion engine's job.
        assert_eq!(cmd.subcommands, vec!["get", "pods"]);
        assert_eq!(cmd.args, vec!["prod", "web"]);
    }

    #[test]
    fn comments_ignored() {
        let q = parse_for_completion("git status # what now", None);
        // Everything after # is a comment; completion sees the prefix.
        assert_eq!(q.cmdline.binary_name(), Some("git"));
    }
}
