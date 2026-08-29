#!/usr/bin/env bash
# shellmind demo — the three "wow" moments, plus a tour of the engine.
#
# Runs fully offline (deterministic rule engine; no model needed).
#   ./demo/demo.sh              interactive-friendly (pauses when on a TTY)
#   ./demo/demo.sh --no-pause   non-interactive (CI / scripting)
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(dirname "$HERE")"
SM="${SM_BIN:-$REPO/target/debug/sm}"

if [[ ! -x "$SM" ]]; then
  echo "shellmind binary not found at $SM"
  echo "build it first:  cargo build --workspace"
  exit 1
fi

PAUSE=1
[[ "${1:-}" == "--no-pause" ]] && PAUSE=0

# --- hermetic demo environment ------------------------------------------
export SHELLMIND_HOME="$(mktemp -d /tmp/shellmind-demo-XXXXXX)"
export SHELLMIND_HISTORY_FILE="$SHELLMIND_HOME/demo_history.zsh"
export SHELLMIND_KUBECONFIG="$SHELLMIND_HOME/kubeconfig"
export SHELL="/bin/zsh"
export NO_COLOR="${NO_COLOR:-0}"
DEMO_PROJECT="$SHELLMIND_HOME/project"
trap 'rm -rf "$SHELLMIND_HOME"' EXIT

# A realistic project: git repo + node + docker compose + k8s prod context.
mkdir -p "$DEMO_PROJECT/.git/refs/heads/feature" "$DEMO_PROJECT/src"
git_init_fixture() {
  printf 'ref: refs/heads/main\n' > "$DEMO_PROJECT/.git/HEAD"
  printf '[core]\n\tbare = false\n[remote "origin"]\n\turl = git@github.com:demo/app.git\n' > "$DEMO_PROJECT/.git/config"
  printf 'abc123\n' > "$DEMO_PROJECT/.git/refs/heads/main"
  printf 'def456\n' > "$DEMO_PROJECT/.git/refs/heads/feature/dashboard"
}
git_init_fixture
cat > "$DEMO_PROJECT/package.json" <<'JSON'
{
  "name": "demo-app",
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "test": "vitest run",
    "lint": "eslint src"
  }
}
JSON
cat > "$DEMO_PROJECT/docker-compose.yml" <<'YAML'
services:
  api:
    build: .
  worker:
    image: worker:latest
YAML
cat > "$SHELLMIND_HOME/kubeconfig" <<'YAML'
apiVersion: v1
kind: Config
current-context: prod
contexts:
  - name: prod
    context:
      cluster: demo
      namespace: production
YAML

# A history that makes the semantic search shine.
cat > "$SHELLMIND_HISTORY_FILE" <<'HIST'
: 1700000000:0;docker image prune -a
: 1700000001:0;pg_dump -U postgres -h localhost -F c -b -v -f backup.dump mydb
: 1700000002:0;tar -czvf archive.tar.gz folder/
: 1700000003:0;git log --oneline --graph --decorate
: 1700000004:0;kubectl get pods -n production
: 1700000005:0;du -h --max-depth=1 | sort -hr
: 1700000006:0;npm run dev
: 1700000007:0;npm run build
HIST

# Aliases the way a shell plugin would export them.
printf 'dps\tdocker ps --format "table {{.Names}}\\t{{.Status}}\\t{{.Ports}}"\nk\tkubectl\ngst\tgit status\n' \
  > "$SHELLMIND_HOME/aliases.txt"

cd "$DEMO_PROJECT"
"$SM" index --file "$SHELLMIND_HISTORY_FILE" --shell zsh > /dev/null

hr() { printf '\n\033[1;36m%s\033[0m\n' "────────────────────────────────────────────────────────────"; }
section() { printf '\n\033[1;35m  %s\033[0m\n\n' "$1"; }
maybe_pause() {
  if [[ $PAUSE == 1 ]]; then
    printf '\n\033[2m  — press Enter to continue —\033[0m' >&2
    read -r _ < /dev/tty 2>/dev/null || read -r _
    printf '\n'
  fi
}
run() { printf '\033[2m$ %s\033[0m\n' "$1"; }

hr
printf '  \033[1mshellmind\033[0m — AI-powered autocomplete and command intelligence\n'
printf '  demo (offline mode — no model required)\n'
hr

# ---------------------------------------------------------------- 1
section "WOW 1 — intelligent autocomplete (git log --)"
run 'git log --        ⇄ ghost text completes from your history'
printf '  \033[2mtype:\033[0m git log --\n'
printf '  \033[2mghost:\033[0m \033[32moneline --graph --decorate\033[0m\n'
GHOST="$("$SM" complete --shell zsh --buffer "git log --" --ghost)"
printf '  result: %s\n' "$GHOST"
echo
run 'git log --        ⇄ flags are explained too'
"$SM" complete --shell zsh --buffer "git log --" --plain | head -6
maybe_pause

section "… and dynamic context (npm scripts, compose services, k8s namespace)"
run 'npm run <TAB>'
"$SM" complete --shell zsh --buffer "npm run " --plain | head -4
echo
run 'docker compose up <TAB>'
"$SM" complete --shell zsh --buffer "docker compose up " --plain | head -2
echo
run 'kubectl get pods -n <TAB>      (from your kubeconfig)'
"$SM" complete --shell zsh --buffer "kubectl get pods -n " --plain | head -1
maybe_pause

# ---------------------------------------------------------------- 2
section "WOW 2 — natural language → command (Ctrl+Space palette)"
run '> find all files larger than 100MB and delete them'
"$SM" palette --query "find all files larger than 100MB and delete them" --top 3 | sed 's/^/  /'
printf '  \033[2m(shellmind also offers safer variants: -print, -exec rm -i)\033[0m\n'
echo
run '> show disk usage by folder'
"$SM" palette --query "show disk usage by folder" --top 1 | sed 's/^/  /'
echo
run '> show running containers      (matches YOUR alias: dps)'
"$SM" palette --query "show running containers" --top 3 | sed 's/^/  /'
maybe_pause

# ---------------------------------------------------------------- 3
section "WOW 3 — fix the last failed command (Ctrl+F)"
run 'git push origin main   ✗ fatal: The current branch main has no upstream branch.'
"$SM" fix --error "fatal: The current branch main has no upstream branch." git push origin main | sed 's/^/  /'
maybe_pause

# ---------------------------------------------------------------- bonus
section "BONUS — semantic history search (Ctrl+R)"
run '> that postgres backup command from last week'
"$SM" history "postgres backup command from last week" | sed 's/^/  /'
echo
run '> compress folder with tar'
"$SM" history "compress folder with tar" | sed 's/^/  /'
maybe_pause

section "BONUS — safety net for destructive commands"
run 'rm -rf ./'
"$SM" safety-check "rm -rf ./" | sed 's/^/  /'
echo
run 'git push --force origin main'
"$SM" safety-check "git push --force origin main" | sed 's/^/  /'
maybe_pause

section "BONUS — explanations (Ctrl+E) and snippets"
run 'sm explain tar'
"$SM" explain tar | sed 's/^/  /' | head -10
echo
run 'sm save "deploy staging" "./scripts/deploy.sh {{env}}"'
"$SM" save "deploy staging" "./scripts/deploy.sh {{env}}" --desc "Deploy current branch" > /dev/null
run 'sm use deploy staging --set env=staging'
"$SM" use deploy staging --set env=staging | sed 's/^/  /'
maybe_pause

hr
printf '  \033[1mThat was all offline.\033[0m With Ollama running, the palette and\n'
printf '  explanations upgrade to a local LLM:\n\n'
printf '    \033[36msm model pull\033[0m        # qwen2.5-coder:3b\n'
printf '    \033[36meval "$(sm init zsh)"\033[0m   # wire it into your shell\n'
hr
printf '\n'
