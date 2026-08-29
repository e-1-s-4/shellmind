//! The `shellmind` / `sm` command-line interface.

use std::io::{Read, Write};
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use shellmind_core as core;

use core::completions::spec::SpecSet;
use core::config::Config;
use core::context::Context;
use core::daemon::client;
use core::daemon::{Request, Response};
use core::util::Style;

#[derive(Parser)]
#[command(
    name = "shellmind",
    version = core::VERSION,
    about = "AI-powered autocomplete and command intelligence for your terminal",
    long_about = "shellmind gives your terminal an intelligent memory.\n\n\
        It autocompletes commands, explains flags, fixes errors, searches your\n\
        history semantically, warns about destructive commands — and can run\n\
        fully locally through Ollama."
)]
pub struct Cli {
    /// Use an explicit config file instead of ~/.config/shellmind/config.toml
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum Shell {
    Zsh,
    Bash,
    Fish,
}

impl Shell {
    fn as_str(&self) -> &'static str {
        match self {
            Shell::Zsh => "zsh",
            Shell::Bash => "bash",
            Shell::Fish => "fish",
        }
    }
}

#[derive(Subcommand)]
enum Command {
    /// Print the shell integration script (pipe into eval)
    Init { shell: Shell },

    /// Show installation status
    Status,

    /// Import shell history into the local index
    Index {
        /// Import a specific history file
        #[arg(long)]
        file: Option<PathBuf>,
        /// Shell format of --file (zsh | bash | fish)
        #[arg(long)]
        shell: Option<String>,
        /// Drop the index and start over
        #[arg(long)]
        rebuild: bool,
    },

    /// Compute completions for a buffer (used by shell plugins)
    Complete {
        #[arg(long)]
        shell: String,
        #[arg(long, default_value = "")]
        buffer: String,
        #[arg(long)]
        cursor: Option<usize>,
        #[arg(long)]
        max: Option<usize>,
        /// Print only the ghost-text suffix
        #[arg(long)]
        ghost: bool,
        /// Print `insert<TAB>description` lines
        #[arg(long)]
        plain: bool,
        /// Interactive numbered menu; prints the chosen line to stdout
        #[arg(long)]
        menu: bool,
    },

    /// Natural language → command palette
    Palette {
        #[arg(long)]
        shell: Option<String>,
        #[arg(long, default_value = "")]
        buffer: String,
        /// Non-interactive: generate for this query
        #[arg(long)]
        query: Option<String>,
        /// With --query: print the top N commands (default 1)
        #[arg(long, default_value = "1")]
        top: usize,
    },

    /// Explain a command (or the last one you ran)
    Explain {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
        #[arg(long)]
        buffer: Option<String>,
    },

    /// Suggest a fix for the last failed command (or a given one)
    Fix {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
        /// Captured error output, if any
        #[arg(long)]
        error: Option<String>,
        #[arg(long)]
        buffer: Option<String>,
        /// Interactive selection; prints the chosen command to stdout
        #[arg(long)]
        menu: bool,
    },

    /// Semantic shell history search
    History {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        query: Vec<String>,
        #[arg(long)]
        limit: Option<usize>,
        /// Interactive selection; prints the chosen command to stdout
        #[arg(long)]
        menu: bool,
    },

    /// Save a reusable command snippet
    Save {
        name: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
        #[arg(long)]
        desc: Option<String>,
        #[arg(long)]
        tags: Vec<String>,
    },

    /// List available snippets (personal + team packs)
    Snippets {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        query: Vec<String>,
    },

    /// Use a snippet, filling {{placeholders}}
    Use {
        #[arg(allow_hyphen_values = false)]
        name: Vec<String>,
        /// Set placeholder values non-interactively (--set user=postgres)
        #[arg(long)]
        set: Vec<String>,
    },

    /// Analyze a command for destructive behavior
    ///
    /// Exit codes: 0 safe · 1 caution · 2 destructive · 3 irreversible · 4 credentials
    SafetyCheck {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
        #[arg(long)]
        json: bool,
    },

    /// Manage the AI model (list | pull | use)
    Model {
        #[command(subcommand)]
        action: ModelAction,
    },

    /// Record an executed command (called by shell plugins)
    #[command(hide = true)]
    Record {
        exit_code: i32,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },

    /// Dump the current alias table (called by shell plugins)
    #[command(hide = true)]
    Aliases,

    /// Start / stop the resident daemon
    Daemon {
        #[arg(long)]
        stop: bool,
        #[arg(long)]
        status: bool,
    },

    /// Inspect configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Subcommand)]
enum ModelAction {
    /// List models installed in Ollama
    List,
    /// Pull the configured (or given) model
    Pull { name: Option<String> },
    /// Switch the model used by shellmind
    Use { name: String },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Print the config file path
    Path,
    /// Print the effective configuration
    Show,
    /// Write a fresh default config file
    Init,
}

pub fn run() {
    let cli = Cli::parse();
    let cfg = match &cli.config {
        Some(p) => Config::load_from(p),
        None => Config::load(),
    };
    let code = dispatch(cli.command, cfg);
    std::process::exit(code);
}

fn dispatch(cmd: Command, mut cfg: Config) -> i32 {
    let st = Style::new();
    match cmd {
        Command::Init { shell } => {
            if cfg.keybindings.accept == "Tab" {
                println!("export SHELLMIND_TAB_ACCEPT=1");
            }
            println!(
                "# shellmind {} — {} integration",
                core::VERSION,
                shell.as_str()
            );
            println!(
                "# add to ~/.{}rc: eval \"$(shellmind init {})\"",
                shell.as_str(),
                shell.as_str()
            );
            println!();
            print!("{}", core::plugins::script(shell.as_str()).unwrap_or(""));
            0
        }

        Command::Status => {
            let conn = core::store::open();
            let indexed = core::store::history_count(&conn);
            let engine = core::ai::AiEngine::new(cfg.ai.clone(), cfg.privacy.clone());
            let embedded = core::store::embedding_count(&conn, &engine.cfg.embedding_model);
            let daemon_up = client::ping(250);
            println!("shellmind v{}", core::VERSION);
            println!("shell: {}", cfg.effective_shell());
            println!("ai mode: {}", engine.mode_label());
            println!("model: {}", cfg.ai.model);
            if embedded > 0 {
                println!(
                    "embeddings: {} (model {})",
                    embedded, cfg.ai.embedding_model
                );
            }
            println!("history indexed: {} commands", indexed);
            println!(
                "daemon: {}",
                if daemon_up { "running".to_string() } else { "stopped".to_string() }
            );
            println!(
                "safety warnings: {}",
                if cfg.safety.warn_destructive { "enabled" } else { "disabled" }
            );
            println!(
                "telemetry: {}",
                if cfg.privacy.telemetry_enabled { "enabled" } else { "disabled" }
            );
            if !daemon_up {
                println!(
                    "{}",
                    st.dim("# tip: `sm daemon &!` keeps ghost text under 50ms")
                );
            }
            0
        }

        Command::Index {
            file,
            shell,
            rebuild,
        } => {
            if rebuild {
                let db = core::paths::db_path();
                let _ = std::fs::remove_file(&db);
                println!("index rebuilt from scratch");
            }
            let conn = core::store::open();
            let ignore = cfg.history.ignore_secret_commands;
            let count = if let Some(f) = file {
                let shell = shell.unwrap_or_else(|| "zsh".into());
                core::history::import_file(&conn, &f, &shell, ignore)
            } else {
                core::history::import_current(&conn, ignore)
            };
            let _ = core::store::trim_history(&conn, cfg.history.max_entries);
            println!(
                "{} commands imported ({} total indexed)",
                count,
                core::store::history_count(&conn)
            );
            0
        }

        Command::Complete {
            shell,
            buffer,
            cursor,
            max,
            ghost,
            plain,
            menu,
        } => {
            if let Some(m) = max {
                cfg.completions.max_suggestions = m;
            }
            let result = complete_via_daemon_or_local(&buffer, cursor, &shell, &cfg);
            if ghost {
                if let Some(g) = &result.ghost {
                    print!("{}", g);
                }
                return 0;
            }
            if menu {
                if let Some(line) = menu_select(
                    "suggestions",
                    result
                        .suggestions
                        .iter()
                        .map(|s| (s.line.clone(), s.description.clone(), s.kind.clone()))
                        .collect(),
                ) {
                    println!("{}", line);
                    return 0;
                }
                return 1;
            }
            if plain {
                for s in &result.suggestions {
                    println!("{}\t{}\t{}", s.insert, s.description, s.kind);
                }
                return 0;
            }
            match serde_json::to_string(&result) {
                Ok(json) => println!("{}", json),
                Err(_) => return 1,
            }
            0
        }

        Command::Palette {
            shell,
            buffer,
            query,
            top,
        } => {
            let _ = shell;
            let conn = core::store::open();
            let engine = core::ai::AiEngine::new(cfg.ai.clone(), cfg.privacy.clone());
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let ctx = Context::collect(&cwd, Some(&conn));

            let query = match query {
                Some(q) => q,
                None => {
                    if !buffer.trim().is_empty() {
                        buffer.trim().to_string()
                    } else {
                        match prompt_line("describe a command… ") {
                            Some(q) if !q.trim().is_empty() => q,
                            _ => return 1,
                        }
                    }
                }
            };
            let results = engine.generate(&query, &ctx, Some(&conn));
            if results.is_empty() {
                eprintln!("{}", st.dim("no suggestions — try rephrasing"));
                return 1;
            }
            if query_was_explicit() {
                // Non-interactive mode.
                for r in results.iter().take(top.max(1)) {
                    println!("{}", r.command);
                }
                return 0;
            }
            let items = results
                .iter()
                .map(|r| (r.command.clone(), r.explanation.clone(), r.source.to_string()))
                .collect();
            match menu_select("commands", items) {
                Some(cmd) => {
                    println!("{}", cmd);
                    0
                }
                None => 1,
            }
        }

        Command::Explain { command, buffer } => {
            let cmd = resolve_command(command, buffer);
            let Some(cmd) = cmd else {
                eprintln!("nothing to explain");
                return 1;
            };
            let conn = core::store::open();
            let engine = core::ai::AiEngine::new(cfg.ai.clone(), cfg.privacy.clone());
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let ctx = Context::collect(&cwd, Some(&conn));
            let specs = SpecSet::load();
            print!("{}", engine.explain(&cmd, &ctx, &specs));
            0
        }

        Command::Fix {
            command,
            error,
            buffer,
            menu,
        } => {
            let conn = core::store::open();
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let ctx = Context::collect(&cwd, Some(&conn));

            // Resolution order: explicit args / buffer → last FAILED →
            // last command.
            let explicit = command.join(" ");
            let buffer_cmd = buffer.filter(|b| !b.trim().is_empty());
            let (command, error) = if let Some(b) = buffer_cmd {
                (b, error)
            } else if !explicit.trim().is_empty() {
                (explicit, error)
            } else if let Some(f) = core::store::last_failed(&conn) {
                (f.command, f.stderr.or(error))
            } else if let Some(c) = core::store::last_user_command(&conn) {
                (c, error)
            } else {
                eprintln!("no failed command recorded yet");
                return 1;
            };

            let engine = core::ai::AiEngine::new(cfg.ai.clone(), cfg.privacy.clone());
            let fixes = engine.fix(&command, error.as_deref(), &ctx);
            if fixes.is_empty() {
                eprintln!(
                    "{}",
                    st.dim("no known fix — run `sm explain` or capture the error with --error")
                );
                return 1;
            }
            if menu {
                let items = fixes
                    .iter()
                    .map(|f| (f.command.clone(), f.explanation.clone(), "fix".to_string()))
                    .collect();
                return match menu_select("fixes", items) {
                    Some(c) => {
                        println!("{}", c);
                        0
                    }
                    None => 1,
                };
            }
            println!("{} {}", st.bold("command:"), command);
            println!();
            for (i, f) in fixes.iter().enumerate() {
                println!("  {}. {} {}", st.cyan(&format!("{}", i + 1)), st.green(&f.command), "");
                if !f.explanation.is_empty() {
                    println!("     {}", st.dim(&f.explanation));
                }
            }
            0
        }

        Command::History {
            query,
            limit,
            menu,
        } => {
            let conn = core::store::open();
            let query = match query.join(" ") {
                q if !q.trim().is_empty() => q,
                _ => match prompt_line("search history… ") {
                    Some(q) if !q.trim().is_empty() => q,
                    _ => return 1,
                },
            };
            let engine = core::ai::AiEngine::new(cfg.ai.clone(), cfg.privacy.clone());
            let model = engine.cfg.embedding_model.clone();
            let reachable = engine.ollama_reachable;
            let semantic = cfg.history.semantic_search && reachable;
            let result = core::history::hybrid_search(
                &conn,
                &query,
                limit.unwrap_or(10).min(50),
                semantic,
                |text| engine.embed(text),
                &model,
            );
            if result.hits.is_empty() {
                eprintln!("{}", st.dim("no matches"));
                return 1;
            }
            if menu {
                let items = result
                    .hits
                    .iter()
                    .map(|h| {
                        (
                            h.command.clone(),
                            format!(
                                "run {}× · {} · [{}]",
                                h.uses,
                                core::history::relative_time(h.ts),
                                h.source
                            ),
                            h.source.clone(),
                        )
                    })
                    .collect();
                return match menu_select("history", items) {
                    Some(c) => {
                        println!("{}", c);
                        0
                    }
                    None => 1,
                };
            }
            println!("{} \"{}\" {}", st.bold("history"), query, st.dim(&format!("({})", if result.used_vectors { "hybrid: bm25 + vectors" } else { "bm25" })));
            for (i, h) in result.hits.iter().enumerate() {
                println!(
                    "  {:>2}. {} {}",
                    i + 1,
                    h.command,
                    st.dim(&format!(
                        "— {}×, {}",
                        h.uses,
                        core::history::relative_time(h.ts)
                    ))
                );
            }
            0
        }

        Command::Save {
            name,
            command,
            desc,
            tags,
        } => {
            let command = command.join(" ");
            if command.trim().is_empty() {
                eprintln!("nothing to save");
                return 1;
            }
            match core::snippets::save(&name, &command, desc.as_deref().unwrap_or(""), &tags) {
                Ok(()) => {
                    println!("saved snippet {}{} {}", st.bold(&name), "", st.dim(&command));
                    0
                }
                Err(err) => {
                    eprintln!("save failed: {}", err);
                    1
                }
            }
        }

        Command::Snippets { query } => {
            let all = core::snippets::all();
            let q = query.join(" ").to_lowercase();
            let filtered: Vec<_> = if q.is_empty() {
                all
            } else {
                all.into_iter()
                    .filter(|s| {
                        s.name.to_lowercase().contains(&q)
                            || s.description.to_lowercase().contains(&q)
                            || s.command.to_lowercase().contains(&q)
                            || s.tags.iter().any(|t| t.to_lowercase().contains(&q))
                    })
                    .collect()
            };
            if filtered.is_empty() {
                eprintln!("{}", st.dim("no snippets found"));
                return 1;
            }
            for s in &filtered {
                println!(
                    "{} {} {}",
                    st.cyan(&s.name),
                    st.dim(&format!("[{}]", s.source)),
                    st.dim(&s.description)
                );
                println!("    {}", s.command);
            }
            0
        }

        Command::Use { name, set } => {
            let name = name.join(" ");
            let Some(snip) = core::snippets::find(&name) else {
                eprintln!("snippet not found: {}", name);
                return 1;
            };
            let mut values = std::collections::HashMap::new();
            for kv in &set {
                if let Some((k, v)) = kv.split_once('=') {
                    values.insert(k.trim().to_string(), v.trim().to_string());
                }
            }
            let missing: Vec<String> = core::snippets::placeholders(&snip.command)
                .into_iter()
                .filter(|k| !values.contains_key(k))
                .collect();
            if !missing.is_empty() && stdin_is_tty() {
                for key in &missing {
                    if let Some(v) = prompt_line(&format!("{}: ", key)) {
                        values.insert(key.clone(), v);
                    }
                }
            }
            let rendered = core::snippets::render(&snip.command, &values);
            if rendered.contains("{{") && !missing.is_empty() {
                eprintln!(
                    "missing placeholders: {} (use --set {}=value)",
                    missing.join(", "),
                    missing[0]
                );
                eprintln!("{}", rendered);
                return 1;
            }
            println!("{}", rendered);
            0
        }

        Command::SafetyCheck { command, json } => {
            let raw = command.join(" ");
            let report = core::safety::analyze(&raw, &cfg.safety);
            if json {
                match serde_json::to_string_pretty(&report) {
                    Ok(out) => println!("{}", out),
                    Err(_) => return 1,
                }
            } else {
                println!("{} {}", st.bold("safety:"), st.bold(report.risk.label()));
                print!("{}", core::safety::render_report(&report));
            }
            report.risk.exit_code()
        }

        Command::Model { action } => model_cmd(action, &mut cfg),

        Command::Record {
            exit_code,
            command,
        } => {
            let command = command.join(" ");
            if command.trim().is_empty() || core::store::is_internal_command(&command) {
                return 0;
            }
            let conn = core::store::open();
            let secret = core::redact::looks_secret(&command);
            if secret && cfg.history.ignore_secret_commands {
                if exit_code != 0 {
                    let _ = core::store::record_failure(&conn, &command, exit_code, None);
                }
                return 0;
            }
            let entry = core::store::HistoryEntry {
                command,
                ts: core::util::now_ts(),
                exit_code: Some(exit_code),
                cwd: std::env::current_dir().ok().map(|p| p.display().to_string()),
                shell: Some(cfg.effective_shell()),
                secret,
            };
            let _ = core::store::record_command(&conn, &entry);
            if exit_code != 0 {
                let _ = core::store::record_failure(&conn, &entry.command, exit_code, None);
            }
            0
        }

        Command::Aliases => {
            // Plugins pipe their alias tables here is NOT needed — the
            // plugin writes the cache file directly. This command exists
            // for debugging: dump what shellmind currently knows.
            for a in Context::collect(&std::env::current_dir().unwrap_or_default(), None).aliases {
                println!("{}\t{}", a.name, a.expansion);
            }
            0
        }

        Command::Daemon { stop, status } => {
            if stop {
                if client::shutdown() {
                    println!("daemon stopped");
                    0
                } else {
                    eprintln!("daemon is not running");
                    1
                }
            } else if status {
                match client::request(&Request::Ping, 500) {
                    Some(Response::Pong {
                        version,
                        pid,
                        uptime_secs,
                        history_count,
                    }) => {
                        println!(
                            "running v{} (pid {}, up {}s, {} commands)",
                            version, pid, uptime_secs, history_count
                        );
                        0
                    }
                    _ => {
                        println!("stopped");
                        1
                    }
                }
            } else {
                core::daemon::server::run_server();
                0
            }
        }

        Command::Config { action } => match action {
            ConfigAction::Path => {
                println!("{}", core::paths::config_path().display());
                0
            }
            ConfigAction::Show => {
                print!("{}", cfg.to_toml());
                0
            }
            ConfigAction::Init => match cfg.save() {
                Ok(()) => {
                    println!("wrote {}", core::paths::config_path().display());
                    0
                }
                Err(err) => {
                    eprintln!("failed: {}", err);
                    1
                }
            },
        },
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn model_cmd(action: ModelAction, cfg: &mut Config) -> i32 {
    let st = Style::new();
    let ollama = core::ai::ollama::Ollama::new(&cfg.ai.host, 600);
    match action {
        ModelAction::List => match ollama.tags() {
            Ok(models) => {
                if models.is_empty() {
                    println!("{}", st.dim("no models installed — try `sm model pull`"));
                    return 0;
                }
                for m in &models {
                    let marker = if *m == cfg.ai.model { " ← shellmind" } else { "" };
                    println!("{}{}", m, st.green(marker));
                }
                0
            }
            Err(err) => {
                eprintln!(
                    "cannot reach Ollama at {} ({})",
                    cfg.ai.host, err
                );
                1
            }
        },
        ModelAction::Pull { name } => {
            let model = name.unwrap_or_else(|| cfg.ai.model.clone());
            println!("pulling {} (this can take a while)…", model);
            match ollama.pull(&model) {
                Ok(()) => {
                    println!("{} {} ready", st.green("✓"), model);
                    0
                }
                Err(err) => {
                    eprintln!("pull failed: {}", err);
                    1
                }
            }
        }
        ModelAction::Use { name } => {
            cfg.ai.model = name.clone();
            match cfg.save() {
                Ok(()) => {
                    println!("model set to {}", st.green(&name));
                    0
                }
                Err(err) => {
                    eprintln!("failed to save config: {}", err);
                    1
                }
            }
        }
    }
}

/// Try the daemon first (warm cache), fall back to local computation.
fn complete_via_daemon_or_local(
    buffer: &str,
    cursor: Option<usize>,
    shell: &str,
    cfg: &Config,
) -> core::completions::CompletionResult {
    if let Some(Response::Complete { result, .. }) = client::request(
        &Request::Complete {
            buffer: buffer.to_string(),
            cursor,
            shell: shell.to_string(),
        },
        120,
    ) {
        return result;
    }
    let conn = core::store::open();
    let q = core::parser::parse_for_completion(buffer, cursor);
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let ctx = Context::collect(&cwd, Some(&conn));
    let specs = SpecSet::load();
    core::completions::complete(&q, &ctx, Some(&conn), &specs, cfg)
}

fn resolve_command(command: Vec<String>, buffer: Option<String>) -> Option<String> {
    if let Some(b) = buffer {
        if !b.trim().is_empty() {
            return Some(b);
        }
        // Empty buffer → fall through to last command.
        let conn = core::store::open();
        return core::store::last_user_command(&conn);
    }
    let joined = command.join(" ");
    if !joined.trim().is_empty() {
        return Some(joined);
    }
    let conn = core::store::open();
    core::store::last_user_command(&conn)
}

fn query_was_explicit() -> bool {
    // The palette subcommand sets this when --query was passed; simplest
    // reliable check is argv scanning.
    std::env::args().any(|a| a == "--query")
}

fn stdin_is_tty() -> bool {
    unsafe { libc::isatty(0) == 1 }
}

/// Read one line, preferring /dev/tty so `$()` captures keep stdout clean.
fn prompt_line(prompt: &str) -> Option<String> {
    eprint!("{}", prompt);
    let _ = std::io::stderr().flush();
    if let Ok(mut tty) = std::fs::File::open("/dev/tty") {
        let mut bytes = Vec::new();
        let mut buf = [0u8; 1];
        loop {
            match tty.read(&mut buf) {
                Ok(0) => break,
                Ok(_) => {
                    bytes.push(buf[0]);
                    if buf[0] == b'\n' {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        return Some(String::from_utf8_lossy(&bytes).trim().to_string());
    }
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).ok()?;
    Some(line.trim().to_string())
}

/// Render a numbered menu on stderr and read a selection from /dev/tty.
/// Returns the chosen primary line.
fn menu_select(title: &str, items: Vec<(String, String, String)>) -> Option<String> {
    let st = Style::new();
    if items.is_empty() {
        return None;
    }
    eprintln!("{}", st.bold(&format!("  {} — pick 1-{} (Enter to cancel):", title, items.len())));
    for (i, (line, desc, kind)) in items.iter().enumerate() {
        eprintln!("  {:>2}. {} {}", i + 1, st.cyan(line), st.dim(&format!("[{}]", kind)));
        if !desc.is_empty() {
            eprintln!("      {}", st.dim(desc));
        }
    }
    eprint!("  > ");
    let _ = std::io::stderr().flush();
    let choice = prompt_line("")?;
    let n: usize = choice.trim().parse().ok()?;
    if n >= 1 && n <= items.len() {
        Some(items[n - 1].0.clone())
    } else {
        None
    }
}
