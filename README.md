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
EDITOR = "nvim"

[shell.alias]
ll = "ls -la"

[[configs]]
type = ".config"               # or "home"
name = "app"
source = "configs/app"         # relative path inside the zenops config repo
symlinks = ["config.toml"]     # listed files are symlinked; others are generated
```

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
