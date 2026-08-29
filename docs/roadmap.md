# Roadmap

Public roadmap. Dates are guesses; direction is a promise.

## v0.1.0 — first public release ✅

- zsh integration with inline ghost text (`POSTDISPLAY`), palette,
  explain, fix, semantic history keybindings
- bash and fish integrations (menu-style)
- static completion specs: git, docker, kubectl, npm
- dynamic context: npm scripts, compose services, git branches,
  kubeconfig namespaces, PATH binaries, cwd files
- alias awareness (suggestion + natural-language matching)
- hybrid semantic history search (BM25 + optional vectors via Ollama)
- offline engine: ~50 NL intents, 18 error-fix patterns, man-page-lite
  explanations
- safety engine with safer alternatives and CI-friendly exit codes
- personal + team snippets with `{{placeholders}}`
- optional daemon: warm cache, incremental indexing, embedding backfill
- redaction layer, zero telemetry, MIT license

## v0.2 — breadth

- richer parser (process substitution, here-docs, arithmetic contexts)
- completion packs: cargo, terraform, aws, gcloud, pnpm, make
- spec merging (user YAML *extends* embedded specs instead of replacing)
- bash improvements: prompt-level suggestions where readline allows
- team snippet sync (git-based, no server)
- `sm learn` — teach shellmind new intents/fixes from your own history

## v0.3 — depth

- fish ghost-text experiment (fish 4 plugin API)
- smarter local ranking (tiny gradient-boosted model over acceptance
  events — trained locally, of course)
- theme system + light/dark palettes
- Windows/PowerShell experiment behind a feature flag
- improved embeddings: incremental re-index on model switch

## v1.0 — stability

- plugin API for third-party completion packs and safety rules
- enterprise policy file (enforced offline mode, allow-listed commands)
- cross-platform installers (Homebrew tap, crates.io, static musl
  builds, npm wrapper)
- polished safety engine with a public rule-format spec

## Not planned

- A hosted AI service you can't turn off.
- Telemetry that isn't opt-in, local, and inspectable.
- Selling your command history. Ever.
