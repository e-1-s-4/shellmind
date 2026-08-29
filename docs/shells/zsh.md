# zsh integration

shellmind's flagship shell. Ghost text works like fish's autosuggestion,
plus AI-era keybindings.

## Install

```zsh
# ~/.zshrc
eval "$(shellmind init zsh)"
```

Requires `sm` (or `shellmind`) on `PATH` and zsh ≥ 5.9 for the ghost-text
redraw hook. On older zsh versions everything works except automatic
ghost refresh — use `Ctrl+Space` for the menu instead.

## Keybindings

| Key | Action |
|---|---|
| `→` / `End` | accept the ghost suggestion |
| `Ctrl+Right` | accept one word of the suggestion |
| `Ctrl+G` | dismiss the suggestion |
| `Ctrl+Space` | natural-language palette |
| `Ctrl+E` | explain the current command |
| `Ctrl+F` | fix the current command / last failure |
| `Ctrl+R` | semantic history search (falls back to zsh's incremental search when shellmind returns nothing) |
| `Tab` | accept suggestion **if** `SHELLMIND_TAB_ACCEPT=1` (off by default to respect `compinit`) |

## Customizing bindings

The plugin reads these environment variables before binding — set them
*before* the `eval` line:

```zsh
export SHELLMIND_KEY_PALETTE='^@'      # Ctrl+Space (NUL in most terminals)
export SHELLMIND_KEY_EXPLAIN='^[e'     # Alt+E
export SHELLMIND_KEY_FIX='^[f'         # Alt+F
export SHELLMIND_KEY_HISTORY='^R'      # Ctrl+R
export SHELLMIND_KEY_CANCEL='^G'       # Ctrl+G
export SHELLMIND_TAB_ACCEPT=1          # also accept on Tab
```

(`bindkey` syntax: `^X` = Ctrl+X, `^[x` = Alt+x. Check yours with
`bindkey -L`.)

## How ghost text works

1. The `zle-line-pre-redraw` hook fires after each edit.
2. The plugin calls `sm complete --shell zsh --buffer "$BUFFER" --ghost`
   — a Unix-socket round trip when the daemon runs, a short-lived
   process otherwise.
3. The returned suffix is placed in `POSTDISPLAY` (rendered after the
   cursor, dimmed by your ZLE highlighting).
4. Accepting appends it to `BUFFER`.

Buffers longer than 300 chars skip fetching to keep editing snappy.

## Command recording

`preexec` captures each command, `precmd` records its exit status via a
disowned background `sm record` call — recording never blocks your
prompt. The alias table is exported once a minute for alias-aware
completions.

## Troubleshooting

- **No ghost text**: check `sm status` (daemon?), zsh version
  (`echo $ZSH_VERSION`), and try `sm complete --shell zsh --buffer "git log --" --ghost`
  manually.
- **Ctrl+Space does nothing**: your terminal may send a different code —
  find it with `cat -v` then press the key, and set
  `SHELLMIND_KEY_PALETTE` accordingly.
- **Conflicts with other plugins**: shellmind wraps `forward-char` /
  `end-of-line`; if another plugin re-binds them later, re-`eval` the
  shellmind init line afterwards.
