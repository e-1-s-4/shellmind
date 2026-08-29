//! End-to-end tests for the `sm` binary.
//!
//! Every test runs against a hermetic `SHELLMIND_HOME` temp directory so
//! no real user state is ever touched.

use std::path::PathBuf;
use std::process::{Command, Output};

fn test_home() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sm-e2e-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn sm_for(home: &PathBuf) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_sm"));
    c.env("SHELLMIND_HOME", home);
    c.env("SHELLMIND_HISTORY_FILE", home.join("history.zsh"));
    c.env("SHELL", "/bin/zsh");
    c.env("NO_COLOR", "1");
    c
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

fn seed_history(home: &PathBuf) {
    let hist = home.join("history.zsh");
    std::fs::write(
        &hist,
        ": 1700000000:0;docker image prune -a\n\
         : 1700000001:0;pg_dump -U postgres -h localhost -F c -b -v -f backup.dump mydb\n\
         : 1700000002:0;git log --oneline --graph --decorate\n\
         : 1700000003:0;tar -czvf archive.tar.gz folder/\n",
    )
    .unwrap();
}

#[test]
fn init_prints_sourceable_scripts() {
    let home = test_home();
    for shell in ["zsh", "bash", "fish"] {
        let o = sm_for(&home).args(["init", shell]).output().unwrap();
        assert!(o.status.success(), "{} init failed", shell);
        let script = out(&o);
        assert!(script.contains("shellmind"), "{} init missing content", shell);
        assert!(script.len() > 500);
    }
    // bash plugin must be valid bash syntax when eval'd in syntax-check mode.
    let o = sm_for(&home).args(["init", "bash"]).output().unwrap();
    let dir = std::env::temp_dir().join(format!("sm-init-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("plugin.bash");
    std::fs::write(&path, out(&o)).unwrap();
    let check = Command::new("bash")
        .arg("-n")
        .arg(&path)
        .output()
        .expect("bash available");
    assert!(
        check.status.success(),
        "bash -n failed: {}",
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn status_reports_fields() {
    let home = test_home();
    let o = sm_for(&home).args(["status"]).output().unwrap();
    let text = out(&o);
    assert!(text.contains("shellmind v"));
    assert!(text.contains("shell:"));
    assert!(text.contains("ai mode:"));
    assert!(text.contains("model: qwen2.5-coder:3b"));
    assert!(text.contains("history indexed: 0 commands"));
    assert!(text.contains("telemetry: disabled"));
}

#[test]
fn complete_returns_json_for_plugins() {
    let home = test_home();
    let o = sm_for(&home)
        .args(["complete", "--shell", "zsh", "--buffer", "git log --"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    assert!(json["ghost"].is_string());
    let suggestions = json["suggestions"].as_array().unwrap();
    assert!(suggestions.iter().any(|s| s["insert"] == "--oneline"
        && s["description"]
            .as_str()
            .unwrap()
            .contains("one commit per line")));
    assert!(suggestions.iter().all(|s| s["line"].is_string()));
}

#[test]
fn complete_ghost_flag_suffix() {
    let home = test_home();
    let o = sm_for(&home)
        .args([
            "complete",
            "--shell",
            "zsh",
            "--buffer",
            "git log --",
            "--ghost",
        ])
        .output()
        .unwrap();
    let ghost = out(&o);
    assert!(!ghost.is_empty());
    assert!(!ghost.starts_with("--"));
}

#[test]
fn complete_history_ghost_after_index() {
    let home = test_home();
    seed_history(&home);
    sm_for(&home).args(["index"]).output().unwrap();
    let o = sm_for(&home)
        .args([
            "complete",
            "--shell",
            "zsh",
            "--buffer",
            "git log --",
            "--ghost",
        ])
        .output()
        .unwrap();
    assert_eq!(out(&o).trim(), "oneline --graph --decorate");
}

#[test]
fn index_imports_and_counts() {
    let home = test_home();
    seed_history(&home);
    let o = sm_for(&home).args(["index"]).output().unwrap();
    assert!(out(&o).contains("4 commands imported (4 total indexed)"));
    let s = sm_for(&home).args(["status"]).output().unwrap();
    assert!(out(&s).contains("history indexed: 4 commands"));
}

#[test]
fn history_semantic_search() {
    let home = test_home();
    seed_history(&home);
    sm_for(&home).args(["index"]).output().unwrap();
    let o = sm_for(&home)
        .args(["history", "docker", "remove", "unused", "images"])
        .output()
        .unwrap();
    assert!(out(&o).contains("docker image prune -a"));
}

#[test]
fn palette_nl_to_command() {
    let home = test_home();
    let o = sm_for(&home)
        .args([
            "palette",
            "--query",
            "show disk usage by folder",
            "--top",
            "1",
        ])
        .output()
        .unwrap();
    assert_eq!(out(&o).trim(), "du -h --max-depth=1 | sort -hr");
}

#[test]
fn palette_nl_destructive_includes_safer() {
    let home = test_home();
    let o = sm_for(&home)
        .args([
            "palette",
            "--query",
            "find all files larger than 100MB and delete them",
            "--top",
            "3",
        ])
        .output()
        .unwrap();
    let text = out(&o);
    assert!(text.lines().any(|l| l.contains("-delete")));
}

#[test]
fn explain_tar_examples() {
    let home = test_home();
    let o = sm_for(&home).args(["explain", "tar"]).output().unwrap();
    let text = out(&o);
    assert!(text.contains("Create archive"));
    assert!(text.contains("tar -czvf archive.tar.gz folder/"));
    assert!(text.contains("tar -xzvf archive.tar.gz"));
}

#[test]
fn fix_from_explicit_error() {
    let home = test_home();
    let o = sm_for(&home)
        .args([
            "fix",
            "--error",
            "fatal: The current branch main has no upstream branch.",
            "git",
            "push",
            "origin",
            "main",
        ])
        .output()
        .unwrap();
    let text = out(&o);
    assert!(text.contains("git push --set-upstream origin main"));
    assert!(text.contains("main branch is not tracking"));
}

#[test]
fn fix_last_recorded_failure() {
    let home = test_home();
    // Fixture git repo whose main branch has no upstream.
    let gd = home.join(".git");
    std::fs::create_dir_all(gd.join("refs").join("heads")).unwrap();
    std::fs::write(gd.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(gd.join("config"), "[core]\n\tbare = false\n").unwrap();
    std::fs::write(gd.join("refs").join("heads").join("main"), "abc123\n").unwrap();

    let mut rec = sm_for(&home);
    rec.current_dir(&home);
    rec.args(["record", "128", "--", "git", "push", "origin", "main"])
        .output()
        .unwrap();
    let mut fix = sm_for(&home);
    fix.current_dir(&home);
    let o = fix.args(["fix"]).output().unwrap();
    assert!(
        out(&o).contains("--set-upstream"),
        "output was: {}\nerr: {}",
        out(&o),
        err(&o)
    );
}

#[test]
fn record_ignores_internal_commands() {
    let home = test_home();
    sm_for(&home)
        .args(["record", "0", "--", "sm", "status"])
        .output()
        .unwrap();
    let o = sm_for(&home).args(["status"]).output().unwrap();
    assert!(out(&o).contains("history indexed: 0 commands"));
}

#[test]
fn safety_check_exit_codes() {
    let home = test_home();
    let safe = sm_for(&home).args(["safety-check", "ls", "-la"]).output().unwrap();
    assert_eq!(safe.status.code(), Some(0));
    let caution = sm_for(&home)
        .args(["safety-check", "chmod", "-R", "777", "/var/www"])
        .output()
        .unwrap();
    assert_eq!(caution.status.code(), Some(1));
    let destructive = sm_for(&home)
        .args(["safety-check", "rm", "-rf", "./"])
        .output()
        .unwrap();
    assert_eq!(destructive.status.code(), Some(2));
    assert!(out(&destructive).contains("rm -i -r ./"));
    let force_push = sm_for(&home)
        .args(["safety-check", "git", "push", "--force", "origin", "main"])
        .output()
        .unwrap();
    assert_eq!(force_push.status.code(), Some(2));
    assert!(out(&force_push).contains("--force-with-lease"));
    let json = sm_for(&home)
        .args(["safety-check", "--json", "rm", "-rf", "/"])
        .output()
        .unwrap();
    assert!(out(&json).starts_with("{"));
}

#[test]
fn snippet_save_list_use_roundtrip() {
    let home = test_home();
    // Flags come BEFORE the trailing command (trailing_var_arg semantics).
    sm_for(&home)
        .args([
            "save",
            "--desc",
            "Full custom-format dump",
            "postgres backup",
            "pg_dump -U {{user}} -h {{host}} -F c -b -v -f {{file}} {{db}}",
        ])
        .output()
        .unwrap();

    let list = sm_for(&home).args(["snippets"]).output().unwrap();
    assert!(out(&list).contains("postgres backup"));

    let used = sm_for(&home)
        .args([
            "use",
            "postgres",
            "backup",
            "--set",
            "user=postgres",
            "--set",
            "host=localhost",
            "--set",
            "file=backup.dump",
            "--set",
            "db=mydb",
        ])
        .output()
        .unwrap();
    assert_eq!(
        out(&used).trim(),
        "pg_dump -U postgres -h localhost -F c -b -v -f backup.dump mydb"
    );
}

#[test]
fn snippet_use_missing_placeholder_fails() {
    let home = test_home();
    sm_for(&home)
        .args(["save", "deploy", "./scripts/deploy.sh {{env}}"])
        .output()
        .unwrap();
    let o = sm_for(&home)
        .args(["use", "deploy", "--set", "env=staging"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0));
    let missing = sm_for(&home).args(["use", "deploy"]).output().unwrap();
    assert_eq!(missing.status.code(), Some(1));
    assert!(err(&missing).contains("missing placeholders"));
}

#[test]
fn config_show_and_path() {
    let home = test_home();
    let o = sm_for(&home).args(["config", "path"]).output().unwrap();
    assert!(out(&o).contains("config.toml"));
    let show = sm_for(&home).args(["config", "show"]).output().unwrap();
    assert!(out(&show).contains("[ai]"));
    assert!(out(&show).contains("qwen2.5-coder:3b"));
}

#[test]
fn explain_git_log_flags() {
    let home = test_home();
    let o = sm_for(&home)
        .args(["explain", "git", "log", "--oneline"])
        .output()
        .unwrap();
    let text = out(&o);
    assert!(text.contains("Show commit history"));
    assert!(text.contains("Show one commit per line"));
}

#[test]
fn daemon_stop_when_not_running() {
    let home = test_home();
    let o = sm_for(&home).args(["daemon", "--stop"]).output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("not running"));
}
