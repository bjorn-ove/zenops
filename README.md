# zenops

Declarative system configuration management for shell config and dotfiles.

`zenops` reads a TOML config from `~/.config/zenops/config.toml` and uses it to
keep your shell environment, aliases, dotfile symlinks, and `$PATH` in sync
with what you've declared. Run `zenops apply` to make the system match the
config; run `zenops status` to see what would change without touching anything.

## Install

```sh
cargo install zenops
```

## Configure

Create `~/.config/zenops/config.toml`:

```toml
[shell]
type = "bash"

[shell.environment]
EDITOR = "hx"

[shell.alias]
ll = "ls -la"

# A pkg is a tool zenops knows about: how to install it, how to detect it,
# which shell hooks it needs, and which dotfiles it owns. Configs live on
# the pkg that owns them, so `zenops pkg` and `zenops apply` stay in sync.
# `enable` defaults to "on" — the pkg is expected to be present. Use
# `enable = "detect"` for silent-miss or `enable = "disabled"` to skip.
[pkg.helix]
description = "modal editor"
install_hint.brew.packages = ["helix"]
detect = { type = "which", binary = "hx" }

# Dotfiles for helix. The `.config` directory name defaults to the pkg key
# (`helix`), so these land at `~/.config/helix/`. Override with an explicit
# `name = "..."` when the pkg key and config dir differ — e.g. a pkg keyed
# as `neovim` whose dir is `nvim`.
[[pkg.helix.configs]]
type = ".config"
source = "configs/helix"              # relative path inside the zenops config repo
symlinks = [                          # listed files are symlinked; others are generated
  "config.toml",
  "languages.toml",
  "themes/onedark-boh.toml",
]
```

A pkg's configs (and its shell hooks) only apply when the pkg is considered
installed — matching `detect` strategies on the current host, or no `detect`
specified at all (right for config-only or PATH-only pkgs). `enable` defaults
to `"on"` — "I expect this pkg to be here." If detect misses, `zenops apply`
and `zenops status` emit `<pkg> is missing — install with: …` so you notice
the drift. Use `enable = "detect"` for tools you may or may not have (silent
miss), or `enable = "disabled"` to skip a pkg entirely.

Use `type = "home"` with `dir = ".something"` instead of `.config` for
dotfiles that live directly under `~/`.

## Commands

- `zenops apply` — apply the config (write generated files, create symlinks). Pass `--pull-config` to `git pull --rebase` the config repo first.
- `zenops status` — show what would change. `--diff` shows file diffs.
- `zenops pkg` — list configured packages and whether their dependencies are met. `--all` includes disabled packages; `--all-hints` shows every install hint.
- `zenops repo <git-subcommand>` — run a passthrough git command inside the zenops config repo.
- `zenops completions <bash|zsh|fish|elvish|powershell>` — print a shell completion script to stdout. Normally sourced automatically via the built-in `zenops` pkg; only needed for manual setup.

## License

Dual-licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.
