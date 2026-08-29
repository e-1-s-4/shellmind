//! Safety engine: destructive-command detection and safer alternatives.
//!
//! Commands are classified into five risk levels:
//!
//! | Level | Meaning |
//! |---|---|
//! | `Safe` | nothing to worry about |
//! | `Caution` | worth a warning, user should double-check |
//! | `Destructive` | deletes or overwrites data, usually recoverable |
//! | `Irreversible` | destroys data with no practical recovery path |
//! | `CredentialSensitive` | carries or exposes secrets |
//!
//! Detection combines structural parsing (robust against flag reordering
//! like `rm -fr`, `rm --recursive --force`) with regex rules for textual
//! patterns (`DROP TABLE`, fork bombs, `curl | bash`, ...).

use serde::Serialize;

use crate::config::SafetyConfig;
use crate::parser::{CommandLine, WRAPPERS};
use crate::redact;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Safe,
    Caution,
    CredentialSensitive,
    Destructive,
    Irreversible,
}

impl Risk {
    pub fn label(&self) -> &'static str {
        match self {
            Risk::Safe => "safe",
            Risk::Caution => "caution",
            Risk::CredentialSensitive => "credential-sensitive",
            Risk::Destructive => "destructive",
            Risk::Irreversible => "irreversible",
        }
    }

    /// Exit code used by `sm safety-check` for CI pipelines.
    pub fn exit_code(&self) -> i32 {
        match self {
            Risk::Safe => 0,
            Risk::Caution => 1,
            Risk::CredentialSensitive => 4,
            Risk::Destructive => 2,
            Risk::Irreversible => 3,
        }
    }
}

/// One matched rule.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub rule_id: String,
    pub risk: Risk,
    pub message: String,
    pub alternatives: Vec<String>,
    pub confirm_required: bool,
}

/// Full analysis result for a command line.
#[derive(Debug, Clone, Serialize)]
pub struct SafetyReport {
    pub command: String,
    pub risk: Risk,
    pub findings: Vec<Finding>,
}

impl SafetyReport {
    pub fn confirm_required(&self) -> bool {
        self.findings.iter().any(|f| f.confirm_required)
    }

    pub fn is_safe(&self) -> bool {
        self.risk == Risk::Safe
    }
}

fn finding(id: &str, risk: Risk, msg: &str, alts: Vec<&str>, confirm: bool) -> Finding {
    Finding {
        rule_id: id.to_string(),
        risk,
        message: msg.to_string(),
        alternatives: alts.iter().map(|s| s.to_string()).collect(),
        confirm_required: confirm,
    }
}

/// Analyze a single parsed command.
fn analyze_command(cmd: &CommandLine, cfg: &SafetyConfig) -> Vec<Finding> {
    let mut findings = Vec::new();
    let Some(binary) = cmd.binary_name() else {
        return findings;
    };
    let words = &cmd.words;
    let all_flags: Vec<String> = cmd.flags.iter().map(|f| f.to_string()).collect();
    let flag_names = cmd.flag_names();
    let has_flag = |needle: &str| flag_names.iter().any(|f| f == needle);

    // rm -rf family (structural: survives -rf, -fr, --recursive --force).
    if binary == "rm" {
        let recursive = has_flag("-r")
            || has_flag("-R")
            || has_flag("--recursive")
            || all_flags.iter().any(|f| {
                f.starts_with('-')
                    && !f.starts_with("--")
                    && f.len() > 1
                    && f[1..].contains('r')
            });
        let force = has_flag("-f") || has_flag("--force") || all_flags.iter().any(|f| {
            f.starts_with('-') && !f.starts_with("--") && f.len() > 1 && f[1..].contains('f')
        });
        let mut dangerous_targets: Vec<&String> = Vec::new();
        for a in &cmd.args {
            let t = a.as_str();
            if matches!(t, "/" | "/*" | "~" | "$HOME" | "$HOME/*" | "~/*") {
                dangerous_targets.push(a);
            }
        }
        let wildcard_or_dot = cmd
            .args
            .iter()
            .any(|a| a.contains('*') || a == "." || a == ".." || a == "./" || a.starts_with("./"));

        if recursive && !dangerous_targets.is_empty() {
            findings.push(finding(
                "rm_rf_root",
                Risk::Irreversible,
                "This deletes the root of your filesystem (or your entire home directory) recursively and forcefully. There is no undo.",
                vec!["# stop and list first: ls -la <target>"],
                true,
            ));
        } else if recursive && force && wildcard_or_dot {
            findings.push(finding(
                "rm_rf_wildcard",
                Risk::Destructive,
                "This recursively force-deletes everything matching in the current directory.",
                vec!["rm -i -r ./", "trash ./", "git clean -n"],
                cfg.confirm_rm_rf,
            ));
        } else if recursive && force {
            findings.push(finding(
                "rm_rf",
                Risk::Destructive,
                "rm -rf permanently deletes files without confirmation.",
                vec!["du -sh <target>  # check size first", "rm -i -r <target>", "trash <target>"],
                cfg.confirm_rm_rf,
            ));
        }
    }

    // git push --force
    if binary == "git" && words.first().map(|s| s.as_str()) == Some("push") {
        let force = has_flag("--force") || has_flag("-f");
        let with_lease = has_flag("--force-with-lease");
        if force && !with_lease {
            findings.push(finding(
                "git_force_push",
                Risk::Destructive,
                "Force-pushing rewrites remote history and can destroy teammates' work.",
                vec!["git push --force-with-lease <remote> <branch>"],
                cfg.confirm_force_push,
            ));
        }
    }

    // git reset --hard / git clean
    if binary == "git" {
        if words.first().map(|s| s.as_str()) == Some("reset") && has_flag("--hard") {
            findings.push(finding(
                "git_reset_hard",
                Risk::Destructive,
                "git reset --hard discards uncommitted changes in your working tree.",
                vec!["git stash", "git reset --soft HEAD~1", "git diff  # review first"],
                false,
            ));
        }
        if words.first().map(|s| s.as_str()) == Some("clean") {
            let dry = has_flag("-n") || has_flag("--dry-run");
            let aggressive =
                all_flags.iter().any(|f| f.contains('f') || f.contains('x') || f.contains('d'));
            if aggressive && !dry {
                findings.push(finding(
                    "git_clean",
                    Risk::Destructive,
                    "git clean with -f/-d/-x deletes untracked and ignored files permanently.",
                    vec!["git clean -nfdx  # dry-run first"],
                    false,
                ));
            }
        }
    }

    // kubectl delete
    if binary == "kubectl" && words.first().map(|s| s.as_str()) == Some("delete") {
        let dry = words.iter().any(|w| w.starts_with("--dry-run"));
        let ns_delete = cmd
            .args
            .iter()
            .chain(cmd.subcommands.iter())
            .any(|a| a == "namespace" || a == "namespaces" || a == "ns" || a == "-n");
        findings.push(if ns_delete {
            finding(
                "kubectl_delete_namespace",
                Risk::Destructive,
                "Deleting a Kubernetes namespace removes every resource inside it.",
                vec!["kubectl delete <resource> <name> --dry-run=client", "kubectl get all -n <namespace>  # inspect first"],
                true,
            )
        } else {
            finding(
                "kubectl_delete",
                Risk::Destructive,
                "This deletes live Kubernetes resources.",
                vec!["kubectl delete <resource> <name> --dry-run=client"],
                !dry,
            )
        });
        if dry {
            findings.pop();
        }
    }

    // docker prune family
    if binary == "docker" {
        let sub = words.first().map(|s| s.as_str());
        let is_prune = sub == Some("system")
            && words.get(1).map(|s| s.as_str()) == Some("prune")
            || sub == Some("image") && words.get(1).map(|s| s.as_str()) == Some("prune");
        if is_prune {
            let all = has_flag("-a") || has_flag("--all");
            findings.push(finding(
                "docker_prune",
                if all { Risk::Destructive } else { Risk::Caution },
                if all {
                    "docker system/image prune -a removes ALL unused images, not just dangling ones."
                } else {
                    "docker prune removes stopped containers / dangling images."
                },
                vec![
                    "docker image prune  # dangling only",
                    "docker image prune --filter until=24h",
                ],
                all,
            ));
        }
    }

    // terraform destroy
    if binary == "terraform" && words.first().map(|s| s.as_str()) == Some("destroy") {
        findings.push(finding(
            "terraform_destroy",
            Risk::Irreversible,
            "terraform destroy tears down real infrastructure resources.",
            vec!["terraform plan -destroy  # preview what would be destroyed"],
            true,
        ));
    }

    // chmod -R 777
    if binary == "chmod" {
        let recursive = has_flag("-R") || all_flags.iter().any(|f| f.starts_with('-') && !f.starts_with("--") && f.len() > 1 && f[1..].contains('R'));
        let mode777 = cmd
            .args
            .first()
            .map(|a| a == "777" || a == "a+rwx")
            .unwrap_or(false);
        if recursive && mode777 {
            findings.push(finding(
                "chmod_777",
                Risk::Caution,
                "chmod -R 777 makes every file world-writable — a common security hole.",
                vec!["chmod -R 755 <dir>", "chmod 700 <dir>  # private"],
                false,
            ));
        }
    }

    // kill -9 / shutdown
    if binary == "kill" || binary == "pkill" {
        if cmd.args.iter().any(|a| a == "-9" || a == "-KILL") {
            findings.push(finding(
                "kill_9",
                Risk::Caution,
                "SIGKILL (-9) prevents processes from cleaning up; try SIGTERM first.",
                vec!["kill <pid>", "kill -TERM <pid>"],
                false,
            ));
        }
    }

    // Redis flushall
    if binary == "redis-cli" && cmd.args.iter().any(|a| a.eq_ignore_ascii_case("flushall")) {
        findings.push(finding(
            "redis_flushall",
            Risk::Destructive,
            "FLUSHALL deletes every key in every database on the Redis server.",
            vec!["redis-cli --scan  # inspect keys first"],
            true,
        ));
    }

    // AWS S3 rb --force
    if binary == "aws" && cmd.subcommands.iter().any(|s| s == "s3") {
        if cmd.args.iter().any(|a| a == "rb") && has_flag("--force") {
            findings.push(finding(
                "aws_s3_rb_force",
                Risk::Destructive,
                "aws s3 rb --force deletes a bucket and ALL objects inside it.",
                vec!["aws s3 ls s3://<bucket>  # list contents first"],
                true,
            ));
        }
    }

    // sudo elevation combined with destructive verbs is worth a note,
    // but the underlying rule already fires — nothing extra needed here.

    findings
}

/// Textual patterns applied to the raw command line.
fn analyze_text(raw: &str, _cfg: &SafetyConfig) -> Vec<Finding> {
    let mut findings = Vec::new();
    let low = raw.to_lowercase();

    // SQL drops inside any client (psql/mysql -c "DROP TABLE ...").
    {
        use std::sync::OnceLock;
        static DROP: OnceLock<regex::Regex> = OnceLock::new();
        let re = DROP.get_or_init(|| {
            regex::Regex::new(r"(?i)\bdrop\s+(table|database|schema)\b").unwrap()
        });
        if re.is_match(raw) {
            findings.push(finding(
                "sql_drop",
                Risk::Irreversible,
                "DROP TABLE / DROP DATABASE permanently destroys database data.",
                vec!["# take a backup first: pg_dump <db> > backup.sql"],
                true,
            ));
        }
        static TRUNCATE: OnceLock<regex::Regex> = OnceLock::new();
        let re = TRUNCATE.get_or_init(|| regex::Regex::new(r"(?i)\btruncate\s+table\b").unwrap());
        if re.is_match(raw) {
            findings.push(finding(
                "sql_truncate",
                Risk::Destructive,
                "TRUNCATE TABLE empties the table immediately.",
                vec!["SELECT COUNT(*) FROM <table>  # check size first"],
                true,
            ));
        }
    }

    // Fork bomb.
    if low.contains(":(){ :|:& };:") || low.contains(":|:&};:") {
        findings.push(finding(
            "fork_bomb",
            Risk::Irreversible,
            "This is a fork bomb — it will hang the machine.",
            vec![],
            true,
        ));
    }

    // Raw device writes.
    {
        use std::sync::OnceLock;
        static DD: OnceLock<regex::Regex> = OnceLock::new();
        let re = DD.get_or_init(|| {
            regex::Regex::new(r"(?i)\bdd\b.*\bof=/dev/(sd|nvme|hd|vd|mmcblk)").unwrap()
        });
        if re.is_match(raw) {
            findings.push(finding(
                "dd_to_device",
                Risk::Irreversible,
                "dd writing to a raw device destroys its filesystem.",
                vec!["# triple-check the target: lsblk"],
                true,
            ));
        }
        static MKFS: OnceLock<regex::Regex> = OnceLock::new();
        let re = MKFS.get_or_init(|| regex::Regex::new(r"(?i)\bmkfs(\.\w+)?\s+/dev/").unwrap());
        if re.is_match(raw) {
            findings.push(finding(
                "mkfs",
                Risk::Irreversible,
                "mkfs formats the device — all data on it is lost.",
                vec![],
                true,
            ));
        }
        static REDIR_DEV: OnceLock<regex::Regex> = OnceLock::new();
        let re = REDIR_DEV.get_or_init(|| {
            regex::Regex::new(r">\s*/dev/(sd|nvme|hd|vd)").unwrap()
        });
        if re.is_match(raw) {
            findings.push(finding(
                "redirect_to_device",
                Risk::Irreversible,
                "Redirecting output to a raw device corrupts it.",
                vec![],
                true,
            ));
        }
    }

    // Remote scripts piped into a shell.
    {
        use std::sync::OnceLock;
        static PIPE_SH: OnceLock<regex::Regex> = OnceLock::new();
        let re = PIPE_SH.get_or_init(|| {
            regex::Regex::new(r"(?i)(curl|wget|fetch)\b[^\n|;]*\|\s*(sudo\s+)?(ba|z|da|fi)?sh\b")
                .unwrap()
        });
        if re.is_match(raw) {
            findings.push(finding(
                "pipe_to_shell",
                Risk::Caution,
                "Piping remote scripts straight into a shell executes unreviewed code.",
                vec![
                    "curl -fsSL <url> -o /tmp/install.sh  # download first",
                    "less /tmp/install.sh  # review",
                ],
                false,
            ));
        }
    }

    // Credential exposure.
    if redact::looks_secret(raw) {
        findings.push(finding(
            "credential_in_command",
            Risk::CredentialSensitive,
            "This command contains credentials in plain text. shellmind will redact them before any external call and (by default) will not index it.",
            vec!["# prefer: read secrets from a file or env at runtime"],
            false,
        ));
    }
    if redact::exposes_k8s_secrets(raw) {
        findings.push(finding(
            "k8s_secret_dump",
            Risk::CredentialSensitive,
            "Dumping Kubernetes secrets exposes base64-decoded credentials in your terminal.",
            vec!["kubectl get secret <name> -o jsonpath='{.data}'  # targeted field only"],
            false,
        ));
    }

    findings
}

/// Analyze a full command line (all pipeline segments).
pub fn analyze(raw: &str, cfg: &SafetyConfig) -> SafetyReport {
    let mut findings = Vec::new();
    if raw.trim().is_empty() {
        return SafetyReport {
            command: raw.to_string(),
            risk: Risk::Safe,
            findings,
        };
    }
    let pipeline = crate::parser::parse_pipeline(raw);
    for seg in &pipeline.segments {
        // Strip leading wrappers for analysis, keeping sudo visibility.
        let mut seg = seg.clone();
        if !seg.wrappers.is_empty() {
            // `sudo` does not change the analyzed binary, but its presence
            // escalates Caution findings on system paths.
            seg.wrappers.retain(|w| !WRAPPERS.contains(&w.as_str()));
        }
        findings.extend(analyze_command(&seg, cfg));
    }
    findings.extend(analyze_text(raw, cfg));

    let risk = findings
        .iter()
        .map(|f| f.risk)
        .max()
        .unwrap_or(Risk::Safe);
    SafetyReport {
        command: raw.to_string(),
        risk,
        findings,
    }
}

/// Render a human-readable multi-line report.
pub fn render_report(report: &SafetyReport) -> String {
    use crate::util::Style;
    let st = Style::new();
    let mut out = String::new();
    if report.findings.is_empty() {
        out.push_str(&format!("{} no safety concerns detected\n", st.green("✓")));
        return out;
    }
    for f in &report.findings {
        let risk = match f.risk {
            Risk::Irreversible => st.red("IRREVERSIBLE"),
            Risk::Destructive => st.red("DESTRUCTIVE"),
            Risk::Caution => st.yellow("CAUTION"),
            Risk::CredentialSensitive => st.magenta("CREDENTIALS"),
            Risk::Safe => st.green("SAFE"),
        };
        out.push_str(&format!("{} [{}] {}\n", st.bold("!"), risk, f.message));
        if !f.alternatives.is_empty() {
            out.push_str(&format!("  {} alternatives:\n", st.dim("safer")));
            for alt in &f.alternatives {
                out.push_str(&format!("    {} {}\n", st.dim("•"), alt));
            }
        }
        if f.confirm_required {
            out.push_str(&format!(
                "  {} confirmation required before execution\n",
                st.yellow("⚠")
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analyze_cmd(s: &str) -> SafetyReport {
        analyze(s, &SafetyConfig::default())
    }

    #[test]
    fn rm_rf_current_dir_warns_with_alternatives() {
        let r = analyze_cmd("rm -rf ./");
        assert_eq!(r.risk, Risk::Destructive);
        let f = r.findings.iter().find(|f| f.rule_id == "rm_rf_wildcard").unwrap();
        assert!(f.alternatives.iter().any(|a| a.contains("rm -i")));
        assert!(f.alternatives.iter().any(|a| a.contains("trash")));
        assert!(f.alternatives.iter().any(|a| a.contains("git clean -n")));
    }

    #[test]
    fn rm_rf_root_is_irreversible() {
        assert_eq!(analyze_cmd("rm -rf /").risk, Risk::Irreversible);
        assert_eq!(analyze_cmd("sudo rm -rf /").risk, Risk::Irreversible);
        assert_eq!(analyze_cmd("rm -fr ~").risk, Risk::Irreversible);
    }

    #[test]
    fn rm_rf_flag_order_variants() {
        for c in [
            "rm -rf node_modules",
            "rm -fr node_modules",
            "rm --recursive --force node_modules",
            "rm -r -f node_modules",
        ] {
            let r = analyze_cmd(c);
            assert!(r.findings.iter().any(|f| f.rule_id == "rm_rf"), "cmd: {}", c);
        }
    }

    #[test]
    fn plain_rm_is_fine() {
        assert!(analyze_cmd("rm file.txt").is_safe());
        assert!(analyze_cmd("rm -r build").findings.is_empty());
    }

    #[test]
    fn git_force_push() {
        let r = analyze_cmd("git push --force origin main");
        assert_eq!(r.risk, Risk::Destructive);
        assert!(r.findings[0].alternatives[0].contains("--force-with-lease"));
        // force-with-lease itself is fine
        assert!(analyze_cmd("git push --force-with-lease origin main").is_safe());
    }

    #[test]
    fn kubectl_delete() {
        let r = analyze_cmd("kubectl delete pod api-7d9f");
        assert_eq!(r.risk, Risk::Destructive);
        assert!(r.findings[0].alternatives[0].contains("--dry-run"));
        // dry-run variant is acceptable
        let r2 = analyze_cmd("kubectl delete pod api-7d9f --dry-run=client");
        assert!(r2.findings.is_empty());
    }

    #[test]
    fn docker_prune_all() {
        let r = analyze_cmd("docker system prune -a");
        assert_eq!(r.risk, Risk::Destructive);
        let r2 = analyze_cmd("docker system prune");
        assert_eq!(r2.risk, Risk::Caution);
    }

    #[test]
    fn terraform_destroy() {
        let r = analyze_cmd("terraform destroy");
        assert_eq!(r.risk, Risk::Irreversible);
        assert!(r.findings[0].alternatives[0].contains("plan -destroy"));
    }

    #[test]
    fn chmod_recursive_777() {
        let r = analyze_cmd("chmod -R 777 /var/www");
        assert_eq!(r.risk, Risk::Caution);
        assert!(r.findings[0].alternatives[0].contains("755"));
    }

    #[test]
    fn drop_table() {
        let r = analyze_cmd("psql -c 'DROP TABLE users' postgres://localhost/app");
        assert_eq!(r.risk, Risk::Irreversible);
    }

    #[test]
    fn credential_detection() {
        let r = analyze_cmd("curl --token ghp_abcdefghijklmnopqrstuvwxyz123456 https://api.github.com");
        assert_eq!(r.risk, Risk::CredentialSensitive);
    }

    #[test]
    fn pipe_to_shell_caution() {
        let r = analyze_cmd("curl -fsSL https://get.example.dev | bash");
        assert!(r.findings.iter().any(|f| f.rule_id == "pipe_to_shell"));
    }

    #[test]
    fn safe_commands_stay_safe() {
        for c in [
            "git status",
            "docker compose up --build",
            "ls -la",
            "npm run build",
            "tar -czvf archive.tar.gz folder/",
        ] {
            assert!(analyze_cmd(c).is_safe(), "unexpected warning for {}", c);
        }
    }

    #[test]
    fn pipeline_segments_all_analyzed() {
        let r = analyze_cmd("echo hi && rm -rf ./ || docker system prune -a");
        assert_eq!(r.risk, Risk::Destructive);
        assert!(r.findings.len() >= 2);
    }

    #[test]
    fn exit_codes() {
        assert_eq!(Risk::Safe.exit_code(), 0);
        assert_eq!(Risk::Caution.exit_code(), 1);
        assert_eq!(Risk::Destructive.exit_code(), 2);
        assert_eq!(Risk::Irreversible.exit_code(), 3);
    }
}
