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

## First run — clone an existing config repo

If you already have a zenops config repo (yours or someone else's):

```sh
zenops init git@github.com:you/dotfiles.git --apply
```

`zenops init` clones the repo into `~/.config/zenops/` and validates that it
has a `config.toml`. Authentication (SSH key, HTTPS credential helper) uses
whatever git is already configured to use. `--apply` chains straight into
`zenops apply`; drop it to inspect the repo first, then run `zenops apply`
manually. Use `--branch` to check out a non-default branch or tag.

## First run — start from scratch

If you don't have a config repo yet, create `~/.config/zenops/config.toml`
by hand:

```toml
[user]
name = "Ada Lovelace"
email = "ada@example.com"

[shell]
type = "bash"

[shell.environment]
EDITOR = "hx"

[shell.alias]
ll = "ls -la"

[pkg.helix]
description = "modal editor"
install_hint.brew.packages = ["helix"]
detect = { type = "which", binary = "hx" }

[[pkg.helix.configs]]
type = ".config"
source = "configs/helix"
symlinks = [
  "config.toml",
  "languages.toml",
  "themes/onedark-boh.toml",
]
```

This covers the three primitives: user identity, the shell zenops manages, and
a `pkg` — a tool with an install hint, a detect check, and dotfiles it owns.
Run `zenops apply` to materialize it. See
[docs/config.md](docs/config.md) for every field and variant.

## Commands

- `zenops init <git-url>` — clone a config repo into `~/.config/zenops` and validate it. `--apply` chains into apply; `--branch` picks a branch or tag.
- `zenops apply` — apply the config (write generated files, create symlinks). `--pull-config` pulls the config repo first.
- `zenops status` — show what would change. `--diff` shows file diffs.
- `zenops pkg` — list configured packages and whether their dependencies are met. `--all` includes disabled packages; `--all-hints` shows every install hint.
- `zenops repo <git-subcommand>` — passthrough git command inside the zenops config repo.
- `zenops doctor` — diagnose the local environment (config dir, git, shell, package manager, package health). Read-only; keeps running even when `config.toml` is missing or fails to parse. First thing to run when something is off.
- `zenops schema` — dump a JSON Schema bundle covering both the `-o json` event stream and the `config.toml` input format.
- `zenops completions <bash|zsh|fish|elvish|powershell>` — print a shell completion script to stdout. Normally sourced automatically via the built-in `zenops` pkg; only needed for manual setup.

Use `zenops <cmd> --help` for the authoritative flag list on any subcommand.
Add `-o json` to any command for NDJSON output suitable for scripting.

## Reference

- [docs/config.md](docs/config.md) — full `config.toml` field reference.
- [docs/schema.md](docs/schema.md) — JSON Schema bundle and editor autocomplete integration.

## License

Dual-licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.
