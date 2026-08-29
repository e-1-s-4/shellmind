# CLI reference

`shellmind` and `sm` are the same binary — `sm` is the short form used
throughout. Run `sm help` for the always-current list.

## Global option

| flag | description |
|---|---|
| `--config <PATH>` | use an explicit config file |

## Shell integration

### `sm init <zsh|bash|fish>`

Prints the integration script for the shell. Pipe it into `eval`
(zsh/bash) or `source` (fish). When `keybindings.accept = "Tab"` in the
config, the emitted script also enables Tab-acceptance in zsh.

### `sm status`

```text
shellmind v0.1.0
shell: zsh
ai mode: local (ollama)          ← or "offline" / "… unreachable — offline fallback"
model: qwen2.5-coder:3b
history indexed: 18,342 commands
daemon: running
safety warnings: enabled
telemetry: disabled
```

## History

### `sm index [--file <path>] [--shell <zsh|bash|fish>] [--rebuild]`

Import shell history into the local index. Without options it imports
every history file it can find (`$SHELL`'s first, then the others).
`--rebuild` drops the index first.

### `sm history <query…> [--limit N] [--menu]`

Semantic search (BM25 + synonym expansion, vector re-ranking when a
model is reachable). `--menu` turns it into an interactive picker that
prints the chosen command to stdout (used by shell keybindings).

## Completions

### `sm complete --shell <s> --buffer <b> [--cursor <n>]`

Computes completions for a buffer. Output modes:

| mode | output |
|---|---|
| *(default)* | JSON: `{ suggestions: [...], ghost: "…" }` for plugin consumption |
| `--ghost` | just the ghost-text suffix (may be empty) |
| `--plain` | `insert<TAB>description<TAB>kind` lines |
| `--menu` | interactive numbered picker → chosen line on stdout |

Options: `--max N` caps the suggestion count.

### `sm palette [--shell <s>] [--buffer <b>] [--query <q>] [--top N]`

Natural language → command. Interactive by default (reads the query on
`/dev/tty`); with `--query` prints the top N commands for scripting
(e.g. the demo). Sources: your aliases, the intent library, semantic
history, and — when Ollama is up — the local LLM.

## Explain & fix

### `sm explain [command…] [--buffer <b>]`

Explains a command: spec-driven chain + flag descriptions, plus
man-page-lite examples for common binaries. With no arguments it
explains the last command you ran.

### `sm fix [command…] [--error <text>] [--buffer <b>] [--menu]`

Suggests fixes. Resolution order: explicit args → `--buffer` → last
*failed* command → last command. `--error` supplies captured stderr
(recommended for precise matching); without it shellmind infers from the
command + local context (e.g. a branch with no upstream).

> **Argument order note:** flags must come *before* the command —
> `sm fix --error "…" git push origin main`, not `sm fix git push … --error "…"`.
> (Everything after the first command word is treated as the command.)

Exit code: `0` when fixes were found, `1` otherwise.

## Safety

### `sm safety-check <command…> [--json]`

Classifies a command and prints findings with safer alternatives.

| exit code | meaning |
|---|---|
| 0 | safe |
| 1 | caution |
| 2 | destructive |
| 3 | irreversible |
| 4 | credential-sensitive |

`--json` emits a machine-readable report (CI-friendly).

## Snippets

### `sm save <name> <command…> [--desc <d>] [--tags <t>…]`

Save a personal snippet. `{{placeholders}}` become prompts at use time.

### `sm snippets [query…]`

List personal + team snippets, optionally filtered.

### `sm use <name…> [--set <key>=<value>…]`

Render a snippet. Missing placeholders are prompted interactively on a
TTY; non-interactive runs with missing placeholders fail with exit code
1 and list what's needed.

## AI model

### `sm model list` · `sm model pull [name]` · `sm model use <name>`

Manage the Ollama backend: list installed models, pull the configured
(or given) model, switch the active model. Requires `ollama serve`
running on the configured host.

## Daemon

### `sm daemon [--stop] [--status]`

Start the daemon in the foreground (use `sm daemon &!` to background
it), stop it, or query it. The daemon serves completions from a warm
cache over a Unix socket, imports history incrementally, and backfills
embeddings in the background. Everything works without it.

## Configuration

### `sm config path` · `sm config show` · `sm config init`

Print the config path, the effective TOML, or write a fresh default
file. See [configuration.md](./configuration.md) for every key.

## Hidden plugin helpers

`sm record <exit-code> -- <command…>` (called by shell hooks) and
`sm aliases` (dump the current alias cache) exist for the shell plugins
and are hidden from help output.
