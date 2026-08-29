# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-08-30

First public release.

### Added

- **zsh integration** with inline ghost-text suggestions (`POSTDISPLAY`
  based), a completion menu on `Ctrl+Space`, and post-command hooks.
- **bash integration** with a `Ctrl+Space` completion menu, `Ctrl+E`
  explain, and `Ctrl+F` fix widgets (bash has no ghost-text layer; the
  menu is the primary UI).
- **fish integration** with Alt-key bindings for palette, explain, fix
  and semantic history search.
- **Intelligent command autocomplete** driven by static completion specs
  (git, docker, kubectl, npm), dynamic local context (package.json
  scripts, docker compose services, git branches, kubeconfig namespace),
  shell aliases, and history frequency.
- **Natural language command generation** (`Ctrl+Space` palette) with a
  deterministic offline intent engine and optional Ollama backend.
- **Error explanation and fix suggestions** for the last failed command
  (`sm fix`, `Ctrl+F`) with a built-in knowledge base of common failures
  (missing upstream, command not found + typo correction, missing Python
  modules, port conflicts, permission errors, ...).
- **Semantic shell history search** (`sm history`, `Ctrl+R`): BM25 lexical
  ranking in SQLite, upgraded with vector embeddings (cosine + reciprocal
  rank fusion) whenever an Ollama embedding model is reachable.
- **Alias awareness**: suggests your own aliases (with expansion preview)
  when they match what you are typing or asking for.
- **Safety engine**: classifies commands into safe / caution /
  destructive / irreversible / credential-sensitive and proposes safer
  alternatives (`rm -rf`, `git push --force`, `kubectl delete`,
  `docker system prune -a`, `terraform destroy`, `chmod -R 777`,
  `DROP TABLE`, ...).
- **Man page / command explanation** (`sm explain`) with common examples
  for tar, git, docker, kubectl, npm and friends, offline by default.
- **Personal and team snippets** (`sm save`, `sm snippets`, `sm use`)
  with `{{placeholder}}` prompts and YAML team packs.
- **Privacy layer**: secret redaction (tokens, passwords, JWTs, AWS keys,
  connection strings, private hosts) applied before any external call;
  telemetry disabled by default; local-only mode by default.
- **Optional daemon** (`shellmind-daemon` / `sm daemon`): Unix-socket
  warm cache for sub-50ms ghost text, incremental history indexing and
  background embedding backfill.
- **Ollama support** (`sm model list | pull | use`) for chat and
  embeddings, defaulting to `qwen2.5-coder:3b` + `nomic-embed-text`.
- Cross-platform builds (Linux, macOS) and an `install.sh` installer.

### Notes

- The AI backend in 0.1.0 is Ollama-only (local-first). Every AI feature
  degrades gracefully to the deterministic offline engine when no model is
  reachable, so shellmind is fully usable with zero models installed.
- Cloud / hybrid providers are on the roadmap but intentionally disabled
  in this release (see `docs/privacy.md`).
