# shellmind-ai

Optional Python companion for [shellmind](../README.md).

The Rust CLI is self-contained — nothing here is required for normal use.
This module exists for:

* **experimenting** with prompts and models without recompiling,
* **batch embedding** history (one-off jobs, benchmarks, CI smoke tests),
* **reference**: `prompts.py` documents the model-output contract
  (`fenced code block = command`, prose = explanation) that the Rust
  engine also implements.

Everything is stdlib-only — no pip installs needed.

## Files

| file | purpose |
|---|---|
| `ollama.py` | dependency-free Ollama client (chat, embeddings, tags, pull) |
| `embeddings.py` | batch-embed the history index: `python3 embeddings.py` |
| `prompts.py` | canonical prompt templates + answer parser |

## Quick start

```bash
cd shellmind-ai
python3 ollama.py                      # list installed models
python3 embeddings.py --limit 200      # embed 200 most-used commands
python3 - <<'EOF'
from ollama import Ollama
from prompts import GENERATE_SYSTEM, generate_user, parse_llm_answer

ollama = Ollama()
ctx = "os: linux\nshell: zsh\ncwd: ~/projects/api\ngit branch: main"
raw = ollama.chat("qwen2.5-coder:3b", GENERATE_SYSTEM,
                  generate_user("show disk usage by folder", ctx))
print(parse_llm_answer(raw))
EOF
```

## Note on privacy

These scripts talk only to your local Ollama daemon
(`http://localhost:11434` by default). Command text is passed through
shellmind's redaction rules in the Rust CLI before it ever reaches a
model; when experimenting here, be mindful that raw commands are sent as
typed.
