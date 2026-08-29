#!/usr/bin/env bash
# shellmind installer
#
#   curl -fsSL https://get.shellmind.dev | bash
#
# Builds the Rust binaries from source and wires them into your shell.
# Options:
#   SHELLMIND_SRC=/path/to/repo   install from a local checkout
#   PREFIX=~/.local               installation prefix (default ~/.local)
set -euo pipefail

BOLD=$'\033[1m'; DIM=$'\033[2m'; GREEN=$'\033[32m'; CYAN=$'\033[36m'; RESET=$'\033[0m'
say()  { printf '%s\n' "${CYAN}▸${RESET} $*"; }
ok()   { printf '%s\n' "${GREEN}✓${RESET} $*"; }
die()  { printf '%s\n' "✗ $*" >&2; exit 1; }

PREFIX="${PREFIX:-$HOME/.local}"
SRC="${SHELLMIND_SRC:-}"
SHELL_NAME="$(basename "${SHELL:-/bin/bash}")"
case "$SHELL_NAME" in
  *zsh)  SHELL_NAME=zsh ;;
  *bash) SHELL_NAME=bash ;;
  *fish) SHELL_NAME=fish ;;
  *)     SHELL_NAME="" ;;
esac

say "installing ${BOLD}shellmind${RESET} (prefix: $PREFIX)"

# --- 1. obtain the source ------------------------------------------------
if [[ -z "$SRC" ]]; then
  if command -v cargo >/dev/null 2>&1 && [[ -d "$PWD/crates" && -f "$PWD/Cargo.toml" ]]; then
    SRC="$PWD"   # already inside a checkout
  else
    die "no source found. Clone the repo first:
    git clone https://github.com/shellmind/shellmind && cd shellmind
then re-run this script, or set SHELLMIND_SRC=/path/to/shellmind"
  fi
fi
[[ -f "$SRC/Cargo.toml" ]] || die "$SRC does not look like the shellmind repository"

# --- 2. toolchain ----------------------------------------------------------
if ! command -v cargo >/dev/null 2>&1; then
  say "installing the Rust toolchain (rustup, minimal profile)…"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --no-modify-path
  export PATH="$HOME/.cargo/bin:$PATH"
fi
command -v cargo >/dev/null 2>&1 || die "cargo still unavailable — install Rust from https://rustup.rs"

# --- 3. build ---------------------------------------------------------------
say "building (this takes a couple of minutes on first run)…"
(cd "$SRC" && cargo build --release)

for bin in sm shellmind shellmind-daemon; do
  [[ -f "$SRC/target/release/$bin" ]] || die "expected binary $bin was not built"
done

# --- 4. install -------------------------------------------------------------
mkdir -p "$PREFIX/bin"
for bin in sm shellmind shellmind-daemon; do
  install -m 0755 "$SRC/target/release/$bin" "$PREFIX/bin/$bin"
done
ok "binaries installed in $PREFIX/bin"

case ":$PATH:" in
  *":$PREFIX/bin:"*) ;;
  *) printf '%s\n' "${DIM}note: $PREFIX/bin is not on your PATH — add it to use 'sm' directly${RESET}" ;;
esac

# --- 5. shell integration -----------------------------------------------------
INIT_LINE='eval "$(shellmind init SHELLNAME)"'
FISH_INIT_LINE='shellmind init fish | source'

if [[ -n "$SHELL_NAME" && "${SHELLMIND_NO_RC:-0}" != "1" ]]; then
  add_rc() {
    local rc="$1" line="$2"
    if [[ -f "$rc" ]] && ! grep -qF "$line" "$rc"; then
      printf '\n# shellmind\n%s\n' "$line" >> "$rc"
      ok "integration added to $rc"
    else
      ok "integration already present in $rc"
    fi
  }
  case "$SHELL_NAME" in
    zsh)  add_rc "$HOME/.zshrc"  "${INIT_LINE/SHELLNAME/zsh}" ;;
    bash) add_rc "$HOME/.bashrc" "${INIT_LINE/SHELLNAME/bash}" ;;
    fish) add_rc "$HOME/.config/fish/config.fish" "$FISH_INIT_LINE" ;;
  esac
fi

# --- 6. first-run setup ---------------------------------------------------------
export PATH="$PREFIX/bin:$PATH"
sm status || true

printf '%s\n' ""
printf '%s\n' "${BOLD}shellmind is ready.${RESET}"
[[ -n "$SHELL_NAME" ]] && printf '%s\n' "Restart your shell (or source your rc file) and try:"
printf '%s\n' "  ${CYAN}sm status${RESET}          installation overview"
printf '%s\n' "  ${CYAN}sm index${RESET}           import your shell history"
printf '%s\n' "  ${CYAN}sm model pull${RESET}      optional: local AI via Ollama"
printf '%s\n' ""
