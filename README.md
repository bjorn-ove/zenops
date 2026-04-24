# zenops

Declarative system configuration management for shell config and dotfiles.

`zenops` reads a TOML config from `~/.config/zenops/config.toml` and uses it to
keep your shell environment, aliases, dotfile symlinks, and `$PATH` in sync
with what you've declared. Run `zenops apply` to make the system match the
config; run `zenops status` to see what would change without touching anything.

## Getting started

If you already have a zenops config repo:

```sh
cargo install zenops
zenops init git@github.com:you/dotfiles.git
zenops apply
```

`zenops init` clones the repo into `~/.config/zenops/` and validates that
it has a `config.toml`. Authentication (SSH key, HTTPS credential helper)
uses whatever git is already configured to use. Pass `zenops init <url>
--apply` to chain straight into `zenops apply`.

## Starting from scratch

If you don't have a config repo yet, create `~/.config/zenops/config.toml`
by hand:

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

## User identity

The `[user]` section holds your name and email — the single source of truth
for anything that needs them (git, SSH allowed-signer principals, and any
template string in the config):

```toml
[user]
name = "Ada Lovelace"
email = "ada@example.com"
```

Both fields are exposed as template variables `${user.name}` and
`${user.email}`, usable in any config value that supports `${...}`
expansion.

## SSH allowed signers

zenops can generate `~/.ssh/allowed_signers` — the file git consults when
verifying SSH-signed commits (`git config gpg.ssh.allowedSignersFile`). Each
entry is either a `github` lookup (zenops fetches the user's SSH *signing*
keys from `https://api.github.com/users/<username>/ssh_signing_keys` via `curl`
— note this is a different endpoint from the `.keys` file used for SSH
authentication) or a fully `manual` line:

```toml
[[ssh.allowed_signers]]
type = "github"
username = "octocat"
principal = "octocat@github.com"

[[ssh.allowed_signers]]
type = "manual"
principal = "bob@example.com"
key_type = "ssh-ed25519"
key = "AAAAC3NzaC1lZDI1NTE5AAAAIExampleKeyMaterial"
```

The file is regenerated on every `zenops apply`, so adding or rotating entries
in the config keeps the signers file in lockstep. `github` entries require
`curl` on `PATH`; if fetching fails, the run aborts — switch to `manual` for
offline stability. Pointing git at the file is handled automatically when
`[git.signing]` is set to `type = "ssh"` (see below).

## Git global config

zenops manages `~/.gitconfig` from the `[git]` section plus `[user]`. Name,
email, and — when configured — commit signing are written on every `zenops
apply`.

Enable commit signing with `[git.signing]`, tagged by backend. SSH-based
signing (git 2.34+):

```toml
[git.signing]
type = "ssh"
key = "~/.ssh/id_ed25519-github.pub"
```

Classic OpenPGP signing:

```toml
[git.signing]
type = "gpg"
key = "ABCD1234DEADBEEF"
```

Setting `[git.signing]` also sets `commit.gpgsign = true` and the matching
`gpg.format`. With `type = "ssh"` plus any `[[ssh.allowed_signers]]` entries
configured, `gpg.ssh.allowedSignersFile` points at the managed file
automatically.

## Commands

- `zenops init <git-url>` — clone an existing zenops config repo into `~/.config/zenops` and validate it. `--apply` chains into `zenops apply` after a successful clone; `--branch` picks a non-default branch or tag.
- `zenops apply` — apply the config (write generated files, create symlinks). Pass `--pull-config` to `git pull --rebase` the config repo first.
- `zenops status` — show what would change. `--diff` shows file diffs.
- `zenops pkg` — list configured packages and whether their dependencies are met. `--all` includes disabled packages; `--all-hints` shows every install hint.
- `zenops repo <git-subcommand>` — run a passthrough git command inside the zenops config repo.
- `zenops doctor` — diagnose the local environment (config dir, git, shell, package manager, package health). Read-only; keeps running even when `config.toml` is missing or fails to parse.
- `zenops schema` — dump a JSON Schema bundle covering both the `-o json` event stream and the `config.toml` input format.
- `zenops completions <bash|zsh|fish|elvish|powershell>` — print a shell completion script to stdout. Normally sourced automatically via the built-in `zenops` pkg; only needed for manual setup.

## Schema

`zenops schema` writes a single JSON document to stdout with two schemas inside `schemas.*`:

- `schemas.output_event` — every line of the NDJSON stream emitted by `-o json` (from `apply`, `status`, `pkg`, `doctor`, `init`).
- `schemas.config` — the TOML `config.toml` structure.

The bundle carries a `zenops_version` field and a `$id` that embeds the same version. The schema shape is part of the zenops crate's public API and follows the same SemVer promise as the crate version: breaking changes require a major bump, additive changes a minor bump.

### Editor autocomplete for `config.toml`

Point [taplo](https://taplo.tamasfe.dev/) (used by Even Better TOML in VS Code, by `taplo` LSP in Helix and Neovim) at the config schema. In `~/.config/taplo/taplo.toml`:

```
[[rule]]
include = ["**/zenops/config.toml", "~/.config/zenops/config.toml"]
[rule.schema]
url = "file:///path/to/zenops-schema.json#/schemas/config"
```

Generate the schema file with `zenops schema > zenops-schema.json`.

## License

Dual-licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.
