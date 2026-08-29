# Privacy

shellmind is local-first. This document is the complete, honest list of
what happens to your data.

## TL;DR

- **No telemetry.** Off by default, and there is no server to send it to.
- **No cloud AI.** The only AI backend in v0.1.0 is Ollama on
  `localhost:11434`.
- **Secrets are redacted** before any text reaches a model.
- **Secret-looking commands are never indexed** at all.
- Everything — history index, embeddings, config — lives in
  `~/.config/shellmind/`.

## Data flow by feature

| Feature | Where it runs | What leaves the machine |
|---|---|---|
| Inline completions | 100% local (specs + SQLite + files) | nothing |
| Safety checks | 100% local | nothing |
| Snippets | 100% local files | nothing |
| History search (BM25) | 100% local | nothing |
| History search (vectors) | embedding request to **localhost** Ollama | the query text, redacted |
| `sm explain` / `fix` / palette | offline engine locally, or a chat request to **localhost** Ollama | command/query + context summary, redacted |
| `sm model pull` | download from ollama.com | model name |

## Redaction rules

Before any text is handed to the model, [`redact`](../crates/core/src/redact.rs)
replaces:

- credentials in URLs → `scheme://[REDACTED_USER]:[REDACTED_PASSWORD]@[REDACTED_HOST]:port/[REDACTED_DB]`
- AWS access keys (`AKIA…`), GitHub (`ghp_…`), GitLab (`glpat-…`), Slack (`xox…`), `sk-…` API keys
- JWTs (`eyJ…`) and `Authorization: Bearer …` headers
- `--password=…`, `--token=…` style long flags
- `PASSWORD=…`, `SECRET_TOKEN=…` style environment assignments
- private IPv4 ranges (`10.x`, `192.168.x`, `172.16–31.x`) and
  `.internal` / `.local` / `.corp` hostnames
- SSH private key blocks

Known limitation: a bare short flag (`-p <value>`) is *not* redacted,
because `-p` overwhelmingly means "port". Use long flags for secrets.

## What gets indexed

`sm index` and the daemon import your shell history files into a local
SQLite database. Before a command is stored:

1. internal commands (`sm …`, `shellmind …`, empty lines) are dropped,
2. commands matching the secret heuristics above are dropped entirely
   (configurable via `history.ignore_secret_commands`, default `true`),
3. everything else is stored with its timestamp, exit code (when
   recorded by a plugin hook), and a token-frequency table for BM25.

Embeddings (only when an embedding model is installed) are stored as
vectors next to the commands in the same database — still local.

## Configuration reference

```toml
[privacy]
redact_secrets = true     # redact before any external call
cloud_enabled = false     # blocks remote providers (none shipped yet)
telemetry_enabled = false # no-op today: there is no telemetry endpoint

[history]
semantic_search = true        # enable vector re-ranking when a model is up
ignore_secret_commands = true # never index secret-looking commands
```

## Air-gapped use

shellmind works with zero network access:

- the offline engine covers explain / fix / palette / history search,
- completions never needed the network,
- set `ai.mode = "offline"` to disable probing entirely.

## Verifying these claims

- `grep -ri "http" crates/` — the only HTTP client is
  `crates/core/src/ai/ollama.rs`, pointed at your configured local host.
- The test suite asserts redaction behavior (`redact::tests`), secret
  non-indexing (`history::tests::secret_commands_not_indexed`) and that
  offline mode never constructs a client (`ai::tests`).
