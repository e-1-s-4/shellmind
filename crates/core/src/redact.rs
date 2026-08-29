//! Secret redaction.
//!
//! Everything that leaves the local machine — or gets printed to a log —
//! passes through this module first. It is intentionally conservative:
//! when in doubt, redact.
//!
//! Covered patterns include:
//!
//! * credentials embedded in URLs (`postgres://user:pass@host/db`),
//! * `--password=...`, `--token=...` style long flags,
//! * `VAR=value` assignments with sensitive names,
//! * AWS access keys (`AKIA…`), GitHub tokens (`ghp_…`), GitLab (`glpat-…`),
//! * JWTs (`eyJ…`), Slack tokens (`xox…`), `sk-…` API keys,
//! * `Bearer` / `Basic` authorization headers,
//! * private IPv4 ranges and `.internal` / `.local` hostnames,
//! * Kubernetes secret dumps (`kubectl get secret -o yaml`).
//!
//! Notable deliberate limitation: a bare `-p <value>` short flag is *not*
//! redacted, because `-p` overwhelmingly means "port" (`docker run -p
//! 8080:80`, `kubectl port-forward`). Use long flags for secrets.

use regex::Regex;
use std::sync::OnceLock;

pub const USER: &str = "[REDACTED_USER]";
pub const PASSWORD: &str = "[REDACTED_PASSWORD]";
pub const HOST: &str = "[REDACTED_HOST]";
pub const DB: &str = "[REDACTED_DB]";
pub const SECRET: &str = "[REDACTED]";

/// Sensitive environment-variable / flag name fragments.
const SENSITIVE_NAME_PARTS: &[&str] = &[
    "password", "passwd", "secret", "token", "api_key", "apikey", "access_key", "private_key",
    "auth", "credential", "session", "cookie", "signature",
];

fn sensitive_name(name: &str) -> bool {
    let n = name.to_lowercase();
    SENSITIVE_NAME_PARTS.iter().any(|p| n.contains(p))
}

fn static_regexes() -> &'static Vec<Regex> {
    static RE: OnceLock<Vec<Regex>> = OnceLock::new();
    RE.get_or_init(|| {
        vec![
            // AWS access key ids
            Regex::new(r"\bAKIA[0-9A-Z]{16}\b").unwrap(),
            // AWS secret keys
            Regex::new(r"(?i)\b(?:aws_secret_access_key|secret[_-]?key)\s*[:=]\s*\S+").unwrap(),
            // GitHub / GitLab / Slack / OpenAI-style tokens
            Regex::new(r"\bgh[pousr]_[A-Za-z0-9]{20,}\b").unwrap(),
            Regex::new(r"\bglpat-[A-Za-z0-9_\-]{15,}\b").unwrap(),
            Regex::new(r"\bxox[baprs]-[A-Za-z0-9\-]{10,}\b").unwrap(),
            Regex::new(r"\bsk-[A-Za-z0-9_\-]{20,}\b").unwrap(),
            // JWTs (three dot-separated base64url segments)
            Regex::new(r"\beyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\b")
                .unwrap(),
            // Authorization headers
            Regex::new(r"(?i)\b(bearer|basic)\s+[A-Za-z0-9._+/=\-]{8,}").unwrap(),
            // SSH private key blocks
            Regex::new(r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----")
                .unwrap(),
        ]
    })
}

fn sensitive_flag_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?i)((?:^|\s)--[a-z0-9_\-]*(?:password|passwd|token|secret|api[_-]?key|access[_-]?key|auth|credential)[a-z0-9_\-]*[ =]\s*)("[^"]*"|'[^']*'|\S+)"#,
        )
        .unwrap()
    })
}

fn env_assignment_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?i)(^|\s)((?:export\s+)?[A-Z_][A-Z0-9_]*(?:PASSWORD|PASSWD|SECRET|TOKEN|API_KEY|APIKEY|ACCESS_KEY|AUTH|CREDENTIALS?|SESSION[A-Z_]*))=("[^"]*"|'[^']*'|\S+)"#,
        )
        .unwrap()
    })
}

/// Private IPv4 / internal hostname patterns.
fn host_regexes() -> &'static Vec<Regex> {
    static RE: OnceLock<Vec<Regex>> = OnceLock::new();
    RE.get_or_init(|| {
        vec![
            Regex::new(r"\b(?:10\.\d{1,3}\.\d{1,3}\.\d{1,3})\b").unwrap(),
            Regex::new(r"\b(?:192\.168\.\d{1,3}\.\d{1,3})\b").unwrap(),
            Regex::new(r"\b(?:172\.(?:1[6-9]|2\d|3[01])\.\d{1,3}\.\d{1,3})\b").unwrap(),
            Regex::new(r"\b(?:127\.0\.0\.1)\b").unwrap(),
            Regex::new(r"(?i)\b(?:[a-z0-9][a-z0-9\-]*\.(?:internal|local|corp|lan))\b").unwrap(),
        ]
    })
}

/// Redact credentials embedded in connection URLs while preserving the
/// command's shape, e.g.
/// `psql postgres://admin:password@db.internal:5432/prod` →
/// `psql postgres://[REDACTED_USER]:[REDACTED_PASSWORD]@[REDACTED_HOST]:5432/[REDACTED_DB]`.
fn redact_urls(input: &str) -> String {
    static URL: OnceLock<Regex> = OnceLock::new();
    let re = URL.get_or_init(|| {
        Regex::new(
            r"(?i)\b((?:postgres|postgresql|mysql|mongodb(?:\+srv)?|redis|rediss|amqp|ftp|sftp|http|https)://)([^/@\s:]+)(?::([^/@\s]*))?@([^/\s:]+)(?::(\d+))?(/([^\s]*))?",
        )
        .unwrap()
    });
    re.replace_all(input, |caps: &regex::Captures| {
        // Groups: 1 scheme, 2 user, 3 password?, 4 host, 5 port?, 6 path?, 7 db?
        let scheme = &caps[1];
        let user = caps.get(2).map(|m| m.as_str()).unwrap_or("");
        let pass = caps.get(3).map(|m| m.as_str());
        let port = caps.get(5).map(|m| m.as_str());
        let has_path = caps.get(6).is_some();
        let has_db = caps.get(7).map(|m| !m.as_str().is_empty()).unwrap_or(false);
        let _ = user;
        let mut out = String::new();
        out.push_str(scheme);
        out.push_str(USER);
        match pass {
            Some(p) if !p.is_empty() => {
                out.push(':');
                out.push_str(PASSWORD);
            }
            _ => {}
        }
        out.push('@');
        out.push_str(HOST);
        if let Some(p) = port {
            out.push(':');
            out.push_str(p);
        }
        if has_path {
            out.push('/');
            if has_db {
                out.push_str(DB);
            }
        }
        out
    })
    .to_string()
}

/// Redact a command (or any text) so it is safe to log or send externally.
pub fn redact(input: &str) -> String {
    if input.is_empty() {
        return input.to_string();
    }
    let mut out = redact_urls(input);
    for re in static_regexes() {
        out = re
            .replace_all(&out, |_: &regex::Captures| SECRET.to_string())
            .to_string();
    }
    out = sensitive_flag_regex()
        .replace_all(&out, |caps: &regex::Captures| {
            format!("{}{}", &caps[1], SECRET)
        })
        .to_string();
    out = env_assignment_regex()
        .replace_all(&out, |caps: &regex::Captures| {
            format!("{}{}={}", &caps[1], &caps[2], SECRET)
        })
        .to_string();
    for re in host_regexes() {
        out = re
            .replace_all(&out, |_: &regex::Captures| HOST.to_string())
            .to_string();
    }
    out
}

/// Quick heuristic: does this command look like it contains a secret?
/// Used to decide whether a history entry should be indexed at all.
pub fn looks_secret(input: &str) -> bool {
    if sensitive_flag_regex().is_match(input) {
        return true;
    }
    if env_assignment_regex().is_match(input) {
        return true;
    }
    for re in static_regexes() {
        if re.is_match(input) {
            return true;
        }
    }
    // Credentials in URLs.
    static URL_CREDS: OnceLock<Regex> = OnceLock::new();
    let re = URL_CREDS.get_or_init(|| Regex::new(r"(?i)://[^/\s:@]+:[^/\s:@]+@").unwrap());
    re.is_match(input)
}

/// Does the command dump Kubernetes secrets?
pub fn exposes_k8s_secrets(input: &str) -> bool {
    let low = input.to_lowercase();
    low.contains("kubectl")
        && low.contains("secret")
        && (low.contains("-o yaml") || low.contains("--output yaml") || low.contains("decode"))
}

#[allow(dead_code)]
fn sensitive_name_check(name: &str) -> bool {
    sensitive_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_postgres_url_per_spec() {
        let red = redact("psql postgres://admin:password@db.internal:5432/prod");
        assert_eq!(
            red,
            "psql postgres://[REDACTED_USER]:[REDACTED_PASSWORD]@[REDACTED_HOST]:5432/[REDACTED_DB]"
        );
    }

    #[test]
    fn redacts_mysql_url_with_private_ip() {
        let red = redact("mysql://root:hunter2@10.0.0.4:3306/shop");
        assert!(red.contains(USER));
        assert!(red.contains(PASSWORD));
        assert!(red.contains(HOST));
        assert!(!red.contains("hunter2"));
        assert!(!red.contains("10.0.0.4"));
        // port preserved
        assert!(red.contains(":3306"));
    }

    #[test]
    fn redacts_tokens_and_keys() {
        let cases: Vec<String> = vec![
            "curl -H 'Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N65IWDpmNfXPU4HuXqoj0k'".into(),
            "export AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            "curl --header \"api-key: sk-abcdefghijklmnopqrstuvwxyz123456\"".into(),
            format!("git clone https://x-access-token:{}{}@github.com/me/repo.git", "gh", "p_abcdefghijklmnopqrstuvwxyz123456"),
            "psql --password=hunter2 -h localhost -U admin appdb".into(),
        ];
        for c in cases {
            let red = redact(&c);
            assert!(red.contains("[REDACTED"), "not redacted: {}", red);
        }
    }

    #[test]
    fn redacts_private_ips_and_internal_hosts() {
        let red = redact("ssh admin@192.168.1.10");
        assert!(red.contains(HOST));
        let red2 = redact("ping db.internal");
        assert!(red2.contains(HOST));
        // Public IPs stay visible.
        let red3 = redact("curl http://93.184.216.34/");
        assert!(!red3.contains(HOST));
    }

    #[test]
    fn redacts_sensitive_env_assignments() {
        let red = redact("DATABASE_PASSWORD=supersecret ./run.sh");
        assert!(red.contains(SECRET));
        assert!(!red.contains("supersecret"));
        // Harmless assignments stay.
        let ok = redact("ENV=production docker compose up");
        assert_eq!(ok, "ENV=production docker compose up");
    }

    #[test]
    fn keeps_port_flags_intact() {
        let out = redact("docker run -p 8080:80 nginx");
        assert_eq!(out, "docker run -p 8080:80 nginx");
    }

    #[test]
    fn detects_secret_commands() {
        assert!(looks_secret("psql postgres://admin:p@db/prod"));
        assert!(looks_secret("curl --token abc123 https://api.io"));
        assert!(looks_secret("export GITHUB_TOKEN=ghp_xxxxxxxxxxxxxxxxxxxx"));
        assert!(!looks_secret("docker compose up --build"));
        assert!(!looks_secret("git push origin main"));
        assert!(!looks_secret("kubectl port-forward svc/api 8080:80"));
    }

    #[test]
    fn detects_k8s_secret_dumps() {
        assert!(exposes_k8s_secrets(
            "kubectl get secret db-creds -n prod -o yaml"
        ));
        assert!(!exposes_k8s_secrets("kubectl get pods"));
    }
}
