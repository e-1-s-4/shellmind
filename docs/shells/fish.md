# fish integration

fish ships its own (history-based) autosuggestion, which keeps working;
shellmind adds the AI layers on Alt keys.

## Install

```fish
# ~/.config/fish/config.fish
shellmind init fish | source
```

## Keybindings

| Key | Action |
|---|---|
| `Alt+P` | natural-language command palette |
| `Alt+E` | explain the current command |
| `Alt+F` | fix the current command / last failure |
| `Alt+R` | semantic history search |

Ctrl+Space can't be bound portably in fish across terminal emulators, so
the palette lives on `Alt+P`. Rebind with `bind` in
`fish_user_key_bindings` if you prefer.

## What you get

- **Palette**: describe a command in plain English, pick from numbered
  results; the command line is replaced.
- **Explain / fix**: output appears above the prompt; the fix picker
  replaces the command line.
- **Semantic search**: search your history by meaning — "that postgres
  backup command" finds your `pg_dump …` line even though the words
  don't appear in it.
- **fish's own autosuggestion** keeps working exactly as before.

## Alias awareness

Abbreviations (`abbr`) and aliases defined in your config are exported
to shellmind's cache once a minute, so completions and the palette can
suggest them.

## Recording

fish's `fish_postexec` event delivers executed commands but not their
exit codes; failures are recovered through `sm fix`'s inference instead.
When exit-code-aware fixes matter, zsh/bash capture them precisely.

## Known limitations

- No ghost text beyond fish's built-in (v0.3 explores the fish 4 plugin
  API).
- Keybindings are re-applied on mode-change events so they survive
  `fish_vi_key_bindings`.
