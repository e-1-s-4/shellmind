//! Dynamic completion values derived from local context.
//!
//! Each key is referenced from a completion spec (`dynamic: npm_scripts`)
//! and resolved against the current [`Context`] snapshot. Everything here
//! is a local file read — no network, no shellouts.

use crate::context::Context;

/// Kubernetes resource names for `kubectl describe|delete|edit|explain`.
const K8S_RESOURCES: &[&str] = &[
    "pods",
    "deployments",
    "services",
    "configmaps",
    "secrets",
    "nodes",
    "namespaces",
    "ingresses",
    "persistentvolumes",
    "persistentvolumeclaims",
    "jobs",
    "cronjobs",
    "events",
    "serviceaccounts",
    "statefulsets",
    "daemonsets",
    "replicasets",
    "horizontalpodautoscalers",
    "endpoints",
];

/// Resolve a dynamic key to `(value, description)` pairs.
pub fn values(key: &str, ctx: &Context) -> Vec<(String, String)> {
    match key {
        "npm_scripts" => ctx
            .project
            .npm_scripts
            .iter()
            .map(|s| (s.clone(), "script from package.json".to_string()))
            .collect(),
        "docker_services" => ctx
            .project
            .compose_services
            .iter()
            .map(|s| (s.clone(), "service from docker-compose".to_string()))
            .collect(),
        "git_branches" => ctx
            .git
            .as_ref()
            .map(|g| {
                g.branches
                    .iter()
                    .map(|b| {
                        (
                            b.clone(),
                            if *b == g.branch {
                                "current branch".to_string()
                            } else {
                                "git branch".to_string()
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        "git_remotes" => ctx
            .git
            .as_ref()
            .map(|g| {
                g.remotes
                    .iter()
                    .map(|r| (r.clone(), "git remote".to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        "k8s_resources" => K8S_RESOURCES
            .iter()
            .map(|r| (r.to_string(), "kubernetes resource".to_string()))
            .collect(),
        "k8s_namespace" => ctx
            .k8s
            .as_ref()
            .filter(|k| !k.namespace.is_empty())
            .map(|k| vec![(k.namespace.clone(), format!("namespace in {}", k.context))])
            .unwrap_or_default(),
        "makefile_targets" => ctx
            .project
            .makefile_targets
            .iter()
            .map(|t| (t.clone(), "makefile target".to_string()))
            .collect(),
        "files" => ctx
            .dir_entries
            .iter()
            .map(|f| (f.clone(), String::new()))
            .collect(),
        _ => Vec::new(),
    }
}

/// Suggestion `kind` for a dynamic key.
pub fn kind_for(key: &str) -> &'static str {
    match key {
        "npm_scripts" => "script",
        "docker_services" => "service",
        "git_branches" => "branch",
        "git_remotes" => "remote",
        "k8s_resources" => "resource",
        "k8s_namespace" => "namespace",
        "makefile_targets" => "target",
        "files" => "file",
        _ => "value",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{GitInfo, K8sInfo, ProjectInfo};

    #[test]
    fn resolves_all_keys() {
        let ctx = Context {
            cwd: std::env::temp_dir(),
            shell: "zsh".into(),
            os: "linux",
            git: Some(GitInfo {
                branch: "main".into(),
                remotes: vec!["origin".into()],
                branches: vec!["main".into()],
                ..Default::default()
            }),
            project: ProjectInfo {
                npm_scripts: vec!["dev".into()],
                compose_services: vec!["api".into()],
                makefile_targets: vec!["build".into()],
                ..Default::default()
            },
            k8s: Some(K8sInfo {
                context: "prod".into(),
                namespace: "production".into(),
            }),
            aliases: vec![],
            dir_entries: vec!["file.txt".into()],
            recent_commands: vec![],
            installed_binaries: vec![],
        };
        assert_eq!(values("npm_scripts", &ctx)[0].0, "dev");
        assert_eq!(values("docker_services", &ctx)[0].0, "api");
        assert_eq!(values("git_branches", &ctx)[0].0, "main");
        assert_eq!(values("git_remotes", &ctx)[0].0, "origin");
        assert_eq!(values("k8s_namespace", &ctx)[0].0, "production");
        assert!(values("k8s_resources", &ctx).iter().any(|(v, _)| v == "pods"));
        assert_eq!(values("makefile_targets", &ctx)[0].0, "build");
        assert_eq!(values("files", &ctx)[0].0, "file.txt");
        assert!(values("unknown-key", &ctx).is_empty());
    }
}
