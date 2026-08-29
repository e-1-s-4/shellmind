# Configuration

File: `~/.config/shellmind/config.toml` (override the whole directory
with the `SHELLMIND_HOME` environment variable — that's how tests and
the demo run hermetically).

`sm config show` prints the effective config; `sm config init` writes a
fresh default file. Missing keys fall back to the defaults below.

```toml
[core]
shell  = "auto"    # auto | zsh | bash | fish  ("auto" sniffs $SHELL)
theme  = "dark"    # dark | light
log_level = "info" # error | warn | info | debug

[ai]
mode            = "local"              # local | offline (| hybrid | cloud, gated)
provider        = "ollama"             # only ollama in v0.1.0
model           = "qwen2.5-coder:3b"   # chat / generation model
embedding_model = "nomic-embed-text"   # semantic history search
temperature     = 0.2
host            = "http://localhost:11434"
timeout_secs    = 60
probe_timeout_ms = 1500                # availability probe

[privacy]
redact_secrets    = true   # redact before any external call
cloud_enabled     = false  # blocks remote providers (none shipped yet)
telemetry_enabled = false  # no-op today: no telemetry endpoint exists

[history]
semantic_search        = true     # vector re-ranking when a model is up
max_entries            = 100000   # index cap (newest kept)
ignore_secret_commands = true     # never index secret-looking commands

[safety]
warn_destructive  = true
confirm_rm_rf     = true         # require confirmation for rm -rf
confirm_force_push = true

[completions]
inline_suggestions    = true
show_flag_descriptions = true
max_suggestions       = 12

[keybindings]
accept     = "Tab"         # informational; plugins read env overrides
palette    = "Ctrl+Space"
explain    = "Ctrl+E"
fix        = "Ctrl+F"
history    = "Ctrl+R"
cancel     = "Ctrl+G"
accept_word = "Ctrl+Right"
safer      = "Alt+S"
expand     = "Alt+Enter"

[snippets]
include = []   # extra team pack YAML files, e.g. ["/opt/company/snippets.yaml"]
```

## Environment variables

| variable | effect |
|---|---|
| `SHELLMIND_HOME` | root directory for all state (config, db, socket, caches) |
| `SHELLMIND_HISTORY_FILE` | single history file override (hermetic runs) |
| `SHELLMIND_KUBECONFIG` | kubeconfig override for k8s context detection |
| `SHELLMIND_TAB_ACCEPT` | `1` = zsh plugin also accepts suggestions on Tab |
| `SHELLMIND_KEY_*` | per-shell keybinding overrides (see [shells](./shells)) |
| `NO_COLOR` | disable ANSI styling |

## Completion spec overrides

Drop a YAML file named `<binary>.yaml` into
`~/.config/shellmind/completions/` to replace the embedded spec for that
binary:

```yaml
name: cargo
description: The Rust package manager
subcommands:
  - name: build
    description: Compile the current package
    flags:
      - name: --release
        description: Build with optimizations
      - name: --target
        arg: triple
        description: Build for a target triple
  - name: test
    description: Run tests
    flags:
      - name: --doc
        description: Test documentation only
dynamic: files
```

`dynamic` keys: `npm_scripts`, `docker_services`, `git_branches`,
`git_remotes`, `k8s_resources`, `k8s_namespace`, `makefile_targets`,
`files`.

## Team snippets

```toml
[snippets]
include = ["/opt/company/deploy-snippets.yaml"]
```

Pack format (identical to the shipped `snippets/*.yaml`):

```yaml
snippets:
  - name: deploy staging
    description: Deploy current branch to staging
    command: ./scripts/deploy.sh staging
  - name: logs production
    description: Tail production API logs
    command: kubectl logs -f deploy/api -n production
```
