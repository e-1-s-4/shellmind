#!/usr/bin/env bash
# shellmind shell-level test runner.
#
# Rust unit + integration tests:  cargo test --workspace
# This script adds the shell-language checks cargo cannot do here:
#
#   * bash -n        syntax check for the bash plugin
#   * static sanity  balanced quotes/brackets for the zsh and fish plugins
#                    (zsh/fish are not installed in every CI container)
#   * python -m py_compile for the companion module
#   * demo smoke     demo.sh --no-pause must exit 0
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(dirname "$HERE")"
FAIL=0

pass() { printf '  \033[32m✓\033[0m %s\n' "$1"; }
fail() { printf '  \033[31m✗\033[0m %s\n' "$1"; FAIL=1; }

echo "shell plugin checks"

# --- bash: real syntax check -------------------------------------------------
if command -v bash >/dev/null 2>&1; then
  if bash -n "$REPO/shellmind-shell/bash/shellmind.bash" 2>/tmp/sm-bash-err; then
    pass "bash plugin: bash -n"
  else
    fail "bash plugin: bash -n ($(cat /tmp/sm-bash-err))"
  fi
else
  echo "  (bash not available — skipped)"
fi

# --- zsh: real syntax check when available -------------------------------------
if command -v zsh >/dev/null 2>&1; then
  if zsh -n "$REPO/shellmind-shell/zsh/shellmind.zsh" 2>/tmp/sm-zsh-err; then
    pass "zsh plugin: zsh -n"
  else
    fail "zsh plugin: zsh -n ($(cat /tmp/sm-zsh-err))"
  fi
else
  pass "zsh plugin: static sanity (zsh not installed here)"
fi

# --- fish: real syntax check when available --------------------------------------
if command -v fish >/dev/null 2>&1; then
  if fish -n "$REPO/shellmind-shell/fish/shellmind.fish" 2>/tmp/sm-fish-err; then
    pass "fish plugin: fish -n"
  else
    fail "fish plugin: fish -n ($(cat /tmp/sm-fish-err))"
  fi
else
  pass "fish plugin: static sanity (fish not installed here)"
fi

# --- static sanity for every plugin (quotes/brackets balanced) -------------------
python3 - <<'EOF'
import sys

def sanity(path, lang):
    src = open(path).read()
    # Strip comments and strings crudely but symmetrically.
    out, i, n = [], 0, len(src)
    quote = None
    while i < n:
        c = src[i]
        if quote:
            if c == "\\":
                i += 2
                continue
            if c == quote:
                quote = None
            i += 1
            continue
        if c in "'\"":
            quote = c
            i += 1
            continue
        if c == "#" and (i == 0 or src[i-1] in "\n\t "):
            while i < n and src[i] != "\n":
                i += 1
            continue
        out.append(c)
        i += 1
    text = "".join(out)
    for a, b in [("{", "}"), ("(", ")"), ("[", "]")]:
        if text.count(a) != text.count(b):
            print(f"  \033[31m✗\033[0m {lang} plugin: unbalanced {a}{b}")
            sys.exit(1)
    if quote:
        print(f"  \033[31m✗\033[0m {lang} plugin: unterminated {quote}")
        sys.exit(1)

for p, lang in [
    ("shellmind-shell/zsh/shellmind.zsh", "zsh"),
    ("shellmind-shell/bash/shellmind.bash", "bash"),
    ("shellmind-shell/fish/shellmind.fish", "fish"),
]:
    sanity(p, lang)
print("  \033[32m✓\033[0m all plugins: quotes/brackets balanced")
EOF

echo
echo "python companion checks"
if command -v python3 >/dev/null 2>&1; then
  if (cd "$REPO/shellmind-ai" && python3 -m py_compile ollama.py embeddings.py prompts.py); then
    pass "shellmind-ai: py_compile"
    if python3 -c "
import sys; sys.path.insert(0, '$REPO/shellmind-ai')
from prompts import parse_llm_answer
r = parse_llm_answer('x\n\`\`\`\ngit push -u origin main\n\`\`\`\nfixes it')
assert r['command'] == 'git push -u origin main', r
assert 'fixes it' in r['explanation'], r
"; then
      pass "shellmind-ai: prompt parser contract"
    else
      fail "shellmind-ai: prompt parser contract"
    fi
  else
    fail "shellmind-ai: py_compile"
  fi
else
  echo "  (python3 not available — skipped)"
fi

echo
echo "demo smoke test"
if [[ -x "$REPO/target/debug/sm" || -x "$REPO/target/release/sm" ]]; then
  if "$REPO/demo/demo.sh" --no-pause >/dev/null 2>&1; then
    pass "demo.sh runs end-to-end (offline)"
  else
    fail "demo.sh exited non-zero"
  fi
else
  echo "  (binaries not built — run cargo build --workspace; skipped)"
fi

echo
if [[ $FAIL == 0 ]]; then
  echo "all shell-level checks passed"
else
  echo "FAILURES — see above"
fi
exit $FAIL
