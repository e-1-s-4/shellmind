<div align="center">

# shellmind

**AI-powered autocomplete and command intelligence for your terminal.**

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org)
[![Shells](https://img.shields.io/badge/shell-zsh%20%7C%20bash%20%7C%20fish-green.svg)](./docs/shells)
[![AI](https://img.shields.io/badge/AI-ollama%20%7C%20offline-purple.svg)](./docs/privacy.md)
[![Tests](https://img.shields.io/badge/tests-151%20passing-brightgreen.svg)](./tests)

*Your terminal, but with a memory and a safety net.*

</div>

---

shellmind gives your terminal an intelligent memory.

It autocompletes commands, explains flags, fixes errors, searches your
history **semantically**, warns you before destructive commands — and can
run **fully locally** through [Ollama](https://ollama.com), with a
deterministic offline engine as the floor. No cloud. No telemetry. No
command history leaving your machine.

```text
$ git log --                                            ← you type
$ git log --oneline --graph --decorate                  ← shellmind ghosts

$ sm palette                                            ← Ctrl+Space
❯ find all files larger than 100MB and delete them
  1. find . -type f -size +100M -exec ls -lh {} + | sort -k5 -h
  2. find . -type f -size +100M -delete
     safer: find . -type f -size +100M -print

$ git push origin main
fatal: The current branch main has no upstream branch.
$ sm fix                                                ← Ctrl+F
  1. git push --set-upstream origin main
     Your local main branch is not tracking a remote branch. …
```

## Features

- **AI-powered command autocomplete** — flags with descriptions, subcommand
  trees, scripts from `package.json`, services from `docker-compose.yml`,
  branches from `.git`, namespaces from kubeconfig, your own aliases.
- **Natural language → command** (`Ctrl+Space`) — with safer alternatives
  offered for destructive intents.
- **Error explanation & fix suggestions** (`Ctrl+F`) — missing upstream,
  `command not found` (with typo detection), missing Python modules, port
  conflicts, permission errors, merge conflicts, and more.
- **Semantic shell history search** (`Ctrl+R`) — BM25 + synonym expansion
  always; vector re-ranking when an embedding model is available.
- **Alias awareness** — typing `docker ps` suggests your `dps` alias;
  asking "show running containers" finds it too.
- **Safety net** — `rm -rf`, `git push --force`, `kubectl delete`,
  `docker system prune -a`, `terraform destroy`, `chmod -R 777`,
  `DROP TABLE` … classified, explained, and paired with safer variants.
- **Local-first privacy** — redaction before any external call, secrets
  never indexed, telemetry off by default.
- **Offline by design** — every AI feature works with **zero models
  installed** through a deterministic rule engine.
- **Optional daemon** — warm Unix-socket cache for sub-50 ms ghost text,
  incremental history indexing, background embeddings.

## Quick start

```bash
git clone https://github.com/shellmind/shellmind
cd shellmind
./install.sh          # or: cargo build --release and use target/release/sm
```

Then add the integration line to your shell:

```bash
# zsh  (~/.zshrc)
eval "$(shellmind init zsh)"

# bash (~/.bashrc)
eval "$(shellmind init bash)"

# fish (~/.config/fish/config.fish)
shellmind init fish | source
```

Restart your shell and run:

```bash
sm index              # import your existing shell history
sm status             # installation overview
```

Optional — upgrade to a local LLM:

```bash
ollama serve &        # https://ollama.com
sm model pull         # qwen2.5-coder:3b by default
sm model use llama3.2:3b
```

> Prefer to watch first? `./demo/demo.sh` walks through all three "wow"
> moments offline, no model required.

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| `Tab` / `→` | Accept the inline suggestion (ghost text) |
| `Ctrl+Right` | Accept one word of the suggestion |
| `Ctrl+Space` | Natural-language command palette |
| `Ctrl+E` | Explain the current command |
| `Ctrl+F` | Fix the last failed command |
| `Ctrl+R` | Semantic history search |
| `Ctrl+G` | Dismiss the suggestion |
| `Alt+S` | Suggest a safer alternative |

All bindings are configurable in `config.toml` — see
[docs/shells](./docs/shells) for per-shell details (bash and fish use
Alt-key variants where the terminal can't deliver Ctrl combos).

## The `sm` CLI

```bash
sm status                    # version, shell, AI mode, indexed commands
sm index [--rebuild]         # import shell history (zsh/bash/fish formats)
sm explain "git rebase -i HEAD~3"
sm fix [--error "…"]         # fix the last failed command
sm history "docker remove unused images"
sm safety-check "rm -rf ."   # exit code = risk level (CI-friendly)
sm save "reset local branch" "git fetch origin && git reset --hard origin/main"
sm use "postgres backup" --set user=postgres --set db=mydb
sm model list | pull | use
sm daemon [--stop|--status]  # optional warm-cache daemon
```

Full reference: [docs/cli-reference.md](./docs/cli-reference.md).

## Privacy

**By default, shellmind does not send your command history to the cloud.**

- The AI backend is Ollama on `localhost` — nothing else ships in v0.1.
- Secrets are redacted before any model call (`psql postgres://admin:hunter2@…`
  becomes `psql postgres://[REDACTED_USER]:[REDACTED_PASSWORD]@[REDACTED_HOST]/…`).
- Commands containing credentials are never indexed.
- Telemetry: **off**, and there is no telemetry server at all.

Details: [docs/privacy.md](./docs/privacy.md).

## How it works

```
┌───────────────────────────────────────────────┐
│ Terminal (zsh / bash / fish)                  │
│  ghost text · palette · keybindings · hooks   │
└───────────────────┬───────────────────────────┘
                    │ sm complete / record / …
┌───────────────────▼───────────────────────────┐
│ shellmind engine (Rust, single binary)        │
│  parser · safety · completions · snippets     │
│  SQLite history store (BM25 + vectors)        │
└───────────┬───────────────────────┬───────────┘
            │ optional daemon       │ optional
            ▼                       ▼
     Unix-socket cache        Ollama (localhost)
```

Deep dive: [docs/architecture.md](./docs/architecture.md).

## Repository layout

```
crates/core          engine: parser, history, safety, completions, AI, daemon
crates/cli           the sm / shellmind binaries
crates/daemon        the resident shellmind-daemon binary
shellmind-shell/     zsh · bash · fish integration plugins (embedded at build)
completions/         YAML completion specs (git, docker, kubectl, npm)
snippets/            team snippet packs (git, docker, postgres)
shellmind-ai/        optional Python companion (batch embedding, prompts)
demo/                offline demo of the three wow moments
tests/               shell-level test runner (plugin syntax checks)
```

## Development

```bash
cargo build --workspace     # build sm, shellmind, shellmind-daemon
cargo test  --workspace     # 151 tests: unit + e2e + daemon socket
./tests/run_tests.sh        # plugin syntax checks + demo smoke test
./demo/demo.sh              # watch it work, offline
```

Adding a completion spec = dropping a YAML file into `~/.config/shellmind/completions/`.
Adding a safety rule, intent, or error fix = one entry in a static table
(`crates/core/src/safety.rs`, `crates/core/src/ai/kb.rs`).

## Roadmap

- **v0.2** — bash polish, richer parser, more completion packs (cargo,
  terraform, aws), team snippet sync
- **v0.3** — fish ghost text, improved local embeddings, theme system
- **v1.0** — plugin system, enterprise policy controls, cross-platform
  installers

Full plan: [docs/roadmap.md](./docs/roadmap.md).

## Contributing

Issues and PRs are welcome — [CONTRIBUTING](./.github/CONTRIBUTING.md) has
the details, [good first issues](https://github.com/shellmind/shellmind/labels/good%20first%20issue)
are labeled. Be kind; read the [code of conduct](./.github/CODE_OF_CONDUCT.md).

## License

[MIT](./LICENSE) © shellmind contributors
