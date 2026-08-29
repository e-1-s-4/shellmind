# bash integration

bash's readline has no ghost-text layer, so shellmind integrates through
`bind -x` widgets that read and write `READLINE_LINE`.

## Install

```bash
# ~/.bashrc
eval "$(shellmind init bash)"
```

## Keybindings

| Key | Action |
|---|---|
| `Ctrl+Space` | completion menu for the current line |
| `Alt+E` | explain the current command |
| `Alt+F` | fix the current command / last failure |
| `Alt+R` | semantic history search |

(`Ctrl+E` and `Ctrl+R` keep their readline defaults — end-of-line and
incremental history — since overriding them in bash is more disruptive
than in zsh. `Ctrl+Space` sends NUL in most terminals, which readline
binds as `\C-@`.)

## The menu

`Ctrl+Space` renders a numbered list of suggestions below your prompt:

```text
$ docker compose up  [Ctrl+Space]
  suggestions — pick 1-6 (Enter to cancel):
   1. api      [service]
      service from docker-compose
   2. worker   [service]
      service from docker-compose
   3. -d       [flag]
      Detached mode
  > 1
```

Typing a number replaces the current line with the completed command.

## Command recording

A `DEBUG` trap captures each command before execution; the exit status
is picked up by a `PROMPT_COMMAND` hook and recorded in the background.
Your aliases are exported to the cache once a minute.

## Notes & limitations

- Multi-line `PS1` setups that also manipulate `PROMPT_COMMAND` are
  respected — shellmind prepends its hook and preserves yours.
- If you use `set -o vi` mode, the Alt bindings still work; adjust them
  in `~/.inputrc` if you prefer different keys.
- Ghost text may arrive for bash in a future release via bracketed-paste
  tricks; the menu is the stable interface.
