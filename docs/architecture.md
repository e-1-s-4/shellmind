# Architecture

shellmind is a Rust workspace with one engine crate, two binaries, three
shell plugins and a pile of declarative YAML. This document explains how
the pieces fit together and why a few deliberate deviations from the
"obvious" design were made.

## Bird's-eye view

```
┌────────────────────────────────────────────────────────────────┐
│ Terminal shell: zsh · bash · fish                              │
│                                                                │
│  zsh:    zle widgets — POSTDISPLAY ghost text, palette, hooks  │
│  bash:   bind -x widgets — READLINE_LINE menu integration      │
│  fish:   Alt-key bindings — commandline replacement            │
└───────────────┬────────────────────────────────────────────────┘
                │  exec `sm …` (one short-lived process per action,
                │  or one Unix-socket round trip when the daemon runs)
┌───────────────▼────────────────────────────────────────────────┐
│ shellmind engine  (crates/core)                                │
│                                                                │
│  parser ──► completions ──► ranking                            │
│    │           ▲    ▲                                          │
│    │           │    └── dynamic context (files, manifests)     │
│    │           └── static specs (embedded YAML)                │
│    ├──► safety engine                                          │
│    ├──► history store (SQLite: BM25 tokens + embeddings)       │
│    └──► AI engine ──► Ollama (localhost) ──► offline fallback  │
└───────────────┬────────────────────────────────────────────────┘
                │ optional
┌───────────────▼────────────────────────────────────────────────┐
│ shellmind-daemon (crates/daemon)                               │
│  warm completion cache · incremental history indexing ·        │
│  background embedding backfill                                 │
└────────────────────────────────────────────────────────────────┘
```

## Components

### 1. Shell adapter (`shellmind-shell/`, embedded via `crates/core/src/plugins.rs`)

`sm init zsh` prints the whole plugin script inline, so
`eval "$(sm init zsh)"` needs no files on disk — the scripts are embedded
into the binary with `include_str!` at build time.

- **zsh** is the flagship: fish-style ghost text through `POSTDISPLAY`,
  refreshed on the `zle-line-pre-redraw` hook (zsh ≥ 5.9), accepted with
  `→`/`End` (wrapped widgets), word-by-word with `Ctrl+Right`, dismissed
  with `Ctrl+G`. Recording happens in `preexec`/`precmd` hooks and is
  fired-and-forgotten (`(sm record … &)!`).
- **bash** has no ghost-text layer in readline, so integration is a
  `Ctrl+Space` menu plus Alt-key widgets operating on `READLINE_LINE`.
  Command capture uses a `DEBUG` trap + `PROMPT_COMMAND`.
- **fish** binds Alt-key widgets (`commandline -r`), records through
  `fish_postexec`, and exports abbreviations for alias awareness.

The plugins never parse JSON: `sm complete --ghost` prints the raw ghost
suffix, `--menu` drives a numbered picker reading `/dev/tty`, so
`$(…)`-capture keeps stdout clean.

### 2. Command parser (`core/src/parser.rs`)

A hand-rolled tokenizer + structural parser, not tree-sitter. Reason:
completion must tolerate *broken* input (`git log --`, unterminated
quotes, trailing pipes) — precisely what real grammars reject. Supported:
quoting/escapes, env prefixes, wrappers (`sudo`, `nohup`, …), pipelines,
lists, redirects (`2>&1` folded into single redirect tokens).

`parse_for_completion(line, cursor)` additionally answers: what word is
being typed, is it a flag, which segment of the pipeline is the cursor in.

### 3. Completion engine (`core/src/completions/`)

Five ranked sources, merged and de-duplicated:

| source | examples | trust |
|---|---|---|
| static specs (YAML) | `--oneline — Show one commit per line` | highest |
| dynamic context | npm scripts, compose services, git branches, k8s namespaces | high |
| history | full-command ghost text you actually typed | high |
| aliases | your `dps` for `docker ps …` | high |
| PATH binaries | first-word completion | low |

Ranking favors declaration order (spec authors curate the common case),
usage frequency and recency. The inline path is **fully deterministic** —
the AI never blocks a keystroke.

### 4. History store (`core/src/store.rs`, `core/src/history.rs`)

SQLite (`rusqlite`, bundled) with four tables: `history` (deduplicated,
usage counts), `tokens` (per-command term frequencies for BM25),
`embeddings` (little-endian f32 blobs), `failed` (ring buffer powering
`sm fix`).

Importers parse native zsh (`: <ts>:<dur>;<cmd>`), bash and fish formats.
Secret-looking commands are refused entry (`redact::looks_secret`).

**Search** is hybrid: BM25 with a domain-synonym expansion ("remove
unused images" matches `docker image prune -a`) runs always; when an
embedding model answers, lexical candidates are re-ranked by cosine
similarity and merged with reciprocal-rank fusion. No model → pure
lexical, still useful.

### 5. Safety engine (`core/src/safety.rs`)

Structural rules (immune to `rm -fr` / `--recursive --force` reordering)
plus textual regexes (`DROP TABLE`, fork bombs, `curl | bash`). Each rule
carries a risk class, an explanation and **safer alternatives**.
`sm safety-check` maps risk classes to exit codes for CI pipelines.

### 6. AI engine (`core/src/ai/`)

One provider in v0.1: **Ollama** on localhost. Every method follows the
same rule — *try the model, fall back to the deterministic offline
engine*:

- `explain` → spec/Knowledge-base summary offline; LLM prose online
- `fix` → error-pattern knowledge base offline (18 curated failure
  classes with placeholder extraction); LLM online
- `generate` (palette) → intent library (~50 templates with OS
  awareness) + alias matching + semantic history offline; LLM online
- `embed` → Ollama embeddings for hybrid search

Text is redacted (`core/src/redact.rs`) before any HTTP call.

### 7. Daemon (`core/src/daemon/`)

A Unix-socket JSON-line server. Deliberately **threads, not tokio**: a
single-user local service with tiny request/response payloads doesn't
need an async runtime, and skipping one keeps the dependency tree (and
startup time) small. It exists for latency (warm cache: ghost text
without process spawn) and background work (incremental history import
by file-offset tracking; embedding backfill). The CLI transparently
falls back to direct computation when the daemon is absent.

## Deliberate deviations from the original stack sketch

| sketch | shipped | why |
|---|---|---|
| tree-sitter | custom tokenizer | completion needs broken-input tolerance, not full ASTs |
| tokio | std threads | single-user local socket; smaller tree, faster cold start |
| fastembed/ONNX | Ollama embeddings | one model server, one protocol, GPU-friendly, zero ML deps in the binary |
| OpenAI-compatible client | omitted in v0.1 | local-first is the promise; remote providers wait for the privacy-reviewed hybrid mode |

## Testing philosophy

- **Unit tests** live next to the code (parser, redaction, safety,
  ranking, offline AI — all table-driven).
- **Hermetic environment**: everything reads paths through
  `SHELLMIND_HOME` / `SHELLMIND_HISTORY_FILE` / `SHELLMIND_KUBECONFIG`
  overrides; tests never touch real user state, and env-mutating tests
  share a process-wide mutex.
- **End-to-end**: `crates/cli/tests/cli.rs` drives the real `sm` binary.
- **Daemon**: a test boots the real server on a temp socket and talks
  the actual protocol.
- **Shell-level**: `tests/run_tests.sh` syntax-checks the plugins
  (bash natively; zsh/fish when installed) and smoke-runs the demo.
