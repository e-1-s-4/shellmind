# Tests

shellmind's test suite has three layers:

| layer | where | what it covers |
|---|---|---|
| unit | inline `#[cfg(test)]` modules next to the code | parser (`crates/core/src/parser.rs`), redaction, safety rules, BM25 ranking, offline AI, snippets, daemon protocol |
| integration | `crates/cli/tests/cli.rs` | drives the real `sm` binary end-to-end (hermetic `SHELLMIND_HOME`) |
| shell-level | `tests/run_tests.sh` | plugin syntax checks (bash natively; zsh/fish when installed), Python companion compile checks, demo smoke test |

Run everything:

```bash
cargo test --workspace
./tests/run_tests.sh
```

## Directory conventions

The top-level `parser/`, `safety/` and `completions/` directories under
`tests/` hold **fixture material** for tricky inputs that deserves to be
data rather than code:

- `parser/*.txt` — command lines with expected structural parses,
- `safety/*.txt` — command lines labeled with expected risk classes,
- `completions/*.txt` — buffer + expected top suggestions.

The Rust suites load them where table-driven inline tests end and
regression corpora begin. When you fix a bug, drop the offending input
into the right directory *and* write the assertion — fixtures catch
rewordings, inline tests catch logic.

All tests are hermetic: they set `SHELLMIND_HOME` to a temp directory and
never touch real user state (see `testutil::env_lock` for the
process-wide env mutex).
