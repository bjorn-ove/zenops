# `config.toml` reference

Full field-by-field reference for `~/.config/zenops/config.toml`. For a
gentler introduction, see the [README](../README.md).

The top-level sections are:

- [`[user]`](#user) — identity (name, email)
- [`[shell]`](#shell) — shell environment, aliases
- [`[pkg.*]`](#pkg) — package definitions (detect, install_hint, shell hooks, dotfiles)
- [`[ssh]`](#ssh) — allowed signers for SSH commit signing
- [`[git]`](#git) — `~/.gitconfig` management, including signing

All sections are optional. Unknown top-level keys are rejected at load time.

---

## `[user]`

Identity fields that aren't git-specific. Both are optional, but setting
them enables the matching features (git identity, template expansion).

```toml
[user]
name = "Ada Lovelace"
email = "ada@example.com"
```

| Field   | Type   | Default | Notes |
| ------- | ------ | ------- | ----- |
| `name`  | string | none    | Also exposed as `${user.name}` for template expansion. |
| `email` | string | none    | Also exposed as `${user.email}`. |

See [Template variables](#template-variables) for where `${user.*}` can be used.

---

## `[shell]`

Controls the shell zenops manages. The `type` field is tagged:

```toml
[shell]
type = "bash"

[shell.environment]
EDITOR = "hx"
PAGER = "less -R"

[shell.alias]
ll = "ls -la"
gs = "git status"
```

| Field         | Type                              | Default | Notes |
| ------------- | --------------------------------- | ------- | ----- |
| `type`        | `"none"` / `"bash"` / `"zsh"`     | `"none"` | Selects the shell. With `"none"`, zenops doesn't touch shell config. |
| `environment` | map of string → string            | `{}`    | Emitted as `export NAME=value` in the generated init script. Only when `type` is `"bash"` or `"zsh"`. |
| `alias`       | map of string → string            | `{}`    | Emitted as `alias name=value`. Only when `type` is `"bash"` or `"zsh"`. |

Per-pkg shell hooks (env init, login init, interactive init) are configured under
`[pkg.*.shell]` — see [pkg shell hooks](#shell-hooks).

---

## `[pkg.*]`

A pkg is a tool zenops knows about: how to install it, how to detect whether
it's present, which shell init lines it needs, and which dotfiles it owns.

Each pkg is a map entry keyed by an arbitrary identifier:

```toml
[pkg.helix]
description = "modal editor"
[pkg.helix.install_hint.brew]
packages = ["helix"]
[pkg.helix.detect]
type = "which"
binary = "hx"
```

### Core fields

| Field              | Type                              | Default | Notes |
| ------------------ | --------------------------------- | ------- | ----- |
| `name`             | string                            | map key | Display label. Useful when two OS-gated entries (e.g. `brew-linux` / `brew-macos`) should share a single user-facing name. |
| `description`      | string                            | none    | Free-form human description. |
| `enable`           | `"on"` / `"detect"` / `"disabled"` | `"on"`  | See [enable states](#enable-states). |
| `detect`           | detect strategy                   | none    | See [detect strategies](#detect-strategies). |
| `install_hint`     | object                            | **required** | See [install hints](#install-hints). |
| `inputs`           | map of string → string            | `{}`    | Template variables scoped to this pkg. Shadow system inputs with the same key. See [template variables](#template-variables). |
| `supported_os`     | array of `"linux"` / `"macos"`   | `[]`    | Empty means any OS. Otherwise the pkg is treated as not-installed on OSes not in the list. |
| `supported_shells` | array of `"bash"` / `"zsh"`      | `[]`    | Empty means any shell. Relevance filter — hides the pkg's init actions when the user's configured shell isn't listed. Doesn't affect `is_installed`. |
| `shell`            | object                            | `{}`    | Shell init hooks. See [shell hooks](#shell-hooks). |
| `configs`          | array                             | `[]`    | Dotfiles owned by this pkg. See [configs](#configs). |

### Enable states

- **`on`** (default) — "I expect this pkg to be here." Runs the detect
  check; if it misses, `zenops apply` and `zenops status` surface
  `<pkg> is missing — install with: …` so you notice the drift. A bare
  `[pkg.x]` reads as "I want this."
- **`detect`** — Use the pkg when detect matches, silent otherwise. Right
  variant for tooling you may or may not have installed (a miss is a
  non-event).
- **`disabled`** — Skip the pkg entirely. Never installed, never surfaces.

A pkg's shell hooks and configs only take effect when the pkg is considered
installed — either detect matches on the current host, or there's no
`detect` field at all (which is right for config-only or PATH-only pkgs).

### Detect strategies

`detect` expresses "is this pkg present on this host?" with four kinds.
Every strategy accepts an optional `os` array that short-circuits the check
to "not installed" when the current OS isn't listed — useful for composing
OS-specific checks inside combinators.

**`which`** — binary is on `PATH`:

```toml
[pkg.sk]
[pkg.sk.install_hint.brew]
packages = ["sk"]
[pkg.sk.detect]
type = "which"
binary = "sk"
```

**`file`** — a path exists. Supports `${...}` expansion and leading `~`
(expanded to `$HOME`):

```toml
[pkg.cargo]
[pkg.cargo.install_hint.brew]
packages = ["rust"]
[pkg.cargo.detect]
type = "file"
path = "~/.cargo/bin/cargo"
```

**`any`** — matches when *any* child strategy matches (short-circuits):

```toml
[pkg.editor]
[pkg.editor.install_hint.brew]
packages = ["neovim"]
[pkg.editor.detect]
type = "any"
of = [
  { type = "which", binary = "nvim" },
  { type = "which", binary = "vim" },
]
```

**`all`** — matches when *every* child matches. An empty `of` is vacuously
true; prefer omitting `detect` entirely to express "no check required".

```toml
[pkg.toolchain]
[pkg.toolchain.install_hint.brew]
packages = ["llvm"]
[pkg.toolchain.detect]
type = "all"
of = [
  { type = "which", binary = "clang" },
  { type = "which", binary = "lld" },
]
```

OS gating nests naturally — a child strategy gated with `os = ["linux"]`
inside an `any` participates only on Linux.

### Install hints

`install_hint` tells `zenops pkg` (and the "<pkg> is missing" warning) how
to install the pkg. Currently only Homebrew is supported; the field is
required so every pkg documents at least one install path.

```toml
[pkg.helix]
[pkg.helix.install_hint.brew]
packages = ["helix"]
```

| Field          | Type             | Notes |
| -------------- | ---------------- | ----- |
| `brew.packages` | array of string | Homebrew formula names. May be empty for pkgs that aren't brew-installable (e.g. meta-pkgs like `bashrc-chain`). |

Future package managers (apt, pacman, etc.) will live alongside `brew`
under `install_hint`.

### Shell hooks

Shell init actions, grouped by stage and keyed by shell. Only emitted when
the pkg is installed *and* the user's shell is in `supported_shells` (or
`supported_shells` is empty).

```toml
[pkg.starship]
[pkg.starship.install_hint.brew]
packages = ["starship"]
[pkg.starship.detect]
type = "which"
binary = "starship"

[[pkg.starship.shell.interactive_init.bash]]
type = "eval_output"
command = ["starship", "init", "bash"]

[[pkg.starship.shell.interactive_init.zsh]]
type = "eval_output"
command = ["starship", "init", "zsh"]
```

Stages (run in this order in the generated init script):

| Stage              | When it runs |
| ------------------ | ------------ |
| `env_init`         | Environment-only setup — sourced early, before login. |
| `login_init`       | Login-shell setup — after env, before interactive. |
| `interactive_init` | Interactive shell only — prompts, keybindings, completion. |

Each stage has per-shell arrays: `bash` and `zsh`.

Every action entry has an optional `optional` flag:

| Field      | Type    | Default | Notes |
| ---------- | ------- | ------- | ----- |
| `optional` | bool    | `false` | When `true`, a `${...}` placeholder that doesn't resolve skips the action silently instead of failing the run. |

Plus one of the kinds below (tagged by `type`):

| `type`           | Fields                    | Emits |
| ---------------- | ------------------------- | ----- |
| `comment`        | `text`                    | `# <text>` |
| `source`         | `path`                    | `. "<path>"` (leading `~/` → `$HOME/`) |
| `eval_output`    | `command` (array)         | `eval "$(<cmd>)"` |
| `source_output`  | `command` (array)         | `source <(<cmd>)` |
| `export`         | `name`, `value`           | `export NAME="VALUE"` |
| `line`           | `line`                    | Literal line, no wrapping. |
| `path_prepend`   | `value`                   | `export PATH="<value>:$PATH"` |
| `path_append`    | `value`                   | `export PATH="$PATH:<value>"` |

All string fields support `${...}` expansion. `path`, `path_prepend.value`,
and `path_append.value` also accept `~/…` (translated to `$HOME/…` in the
emitted script).

### Configs

`configs` lists dotfiles the pkg owns. Each entry targets either
`~/.config/<name>/` or `~/<dir>/`:

```toml
[pkg.helix]
[pkg.helix.install_hint.brew]
packages = ["helix"]

[[pkg.helix.configs]]
type = ".config"
source = "configs/helix"
symlinks = [
  "config.toml",
  "languages.toml",
  "themes/onedark-boh.toml",
]
```

**`.config` variant** — lands at `~/.config/<name>/`:

| Field      | Type                   | Default | Notes |
| ---------- | ---------------------- | ------- | ----- |
| `type`     | `".config"`            | —       | Tag. |
| `name`     | single path component  | pkg key | Override when the pkg key and the config dir differ (e.g. pkg `neovim` whose dir is `nvim`). |
| `source`   | safe relative path     | —       | Path inside the zenops config repo to pull files from. |
| `symlinks` | array of safe relative paths | `[]` | Files listed here are symlinked; every other file under `source` is copied as a generated file. |

**`home` variant** — lands at `~/<dir>/`:

```toml
[pkg.starship]
[pkg.starship.install_hint.brew]
packages = ["starship"]

[[pkg.starship.configs]]
type = "home"
dir = ".config/starship"
source = "configs/starship"
symlinks = ["starship.toml"]
```

| Field      | Type                   | Default | Notes |
| ---------- | ---------------------- | ------- | ----- |
| `type`     | `"home"`               | —       | Tag. |
| `dir`      | safe relative path     | —       | Directory under `~/`. |
| `source`   | safe relative path     | —       | Path inside the zenops config repo. |
| `symlinks` | array of safe relative paths | `[]` | Listed = symlink; rest = generated file. |

Safe relative paths reject `..` traversal at parse time.

The listed-vs-generated split lets you keep the frequently edited files as
live symlinks into the config repo (edits survive `zenops apply`) while
letting zenops regenerate the derived ones (the shell init script is
rendered per-host; it wouldn't make sense to edit it in place).

---

## `[ssh]`

Manages `~/.ssh/allowed_signers`, the file git consults when verifying
SSH-signed commits (`git config gpg.ssh.allowedSignersFile`). Regenerated
on every `zenops apply`.

```toml
[[ssh.allowed_signers]]
type = "github"
username = "octocat"
principal = "octocat@example.com"

[[ssh.allowed_signers]]
type = "manual"
principal = "bob@example.com"
key_type = "ssh-ed25519"
key = "AAAAC3NzaC1lZDI1NTE5AAAAIExampleKeyMaterial"
```

Two entry shapes (tagged by `type`):

**`github`** — zenops fetches the user's SSH *signing* keys from
`https://api.github.com/users/<username>/ssh_signing_keys` via `curl`. Note
this is a different endpoint from `https://github.com/<username>.keys`,
which lists SSH *authentication* keys.

| Field       | Type   | Notes |
| ----------- | ------ | ----- |
| `username`  | string | GitHub username. |
| `principal` | string | Principal git records for the signature (commonly an email). |

Requires `curl` on `PATH`. A failed fetch aborts the apply — switch to
`manual` entries for offline stability.

**`manual`** — the full key material is in the config:

| Field       | Type   | Notes |
| ----------- | ------ | ----- |
| `principal` | string | Principal. |
| `key_type`  | string | e.g. `"ssh-ed25519"`, `"ssh-rsa"`. |
| `key`       | string | Public key material (the base64 blob). |

---

## `[git]`

Generates `~/.gitconfig` from `[git]` plus `[user]`. Only writes the file
when there's something to record — identity, signing, or both.

Enable commit signing via `[git.signing]`, tagged by backend.

**SSH signing** (git 2.34+):

```toml
[git.signing]
type = "ssh"
key = "~/.ssh/id_ed25519-github.pub"
```

**Classic OpenPGP signing**:

```toml
[git.signing]
type = "gpg"
key = "ABCD1234DEADBEEF"
```

| Variant   | Field | Notes |
| --------- | ----- | ----- |
| `ssh`     | `key` | Path to an SSH public key. Passed through verbatim — git expands `~` itself. |
| `gpg`     | `key` | OpenPGP key ID or full fingerprint. |

Setting `[git.signing]` also writes `commit.gpgsign = true` and the matching
`gpg.format` (`ssh` or `openpgp`). With `type = "ssh"` *and* at least one
`[[ssh.allowed_signers]]` entry, zenops also writes
`gpg.ssh.allowedSignersFile = ~/.ssh/allowed_signers` so verification
picks up the managed file automatically.

---

## Template variables

Any field documented as supporting `${...}` expansion goes through the same
lookup. Two scopes:

**System inputs** (auto-populated):

| Variable        | Value |
| --------------- | ----- |
| `${os}`         | `"linux"` or `"macos"` (the current host). |
| `${brew_prefix}` | Homebrew install root, if detected (e.g. `/opt/homebrew`, `/usr/local`, `/home/linuxbrew/.linuxbrew`). Absent on hosts without Homebrew. |
| `${user.name}`  | `[user].name`, when set. |
| `${user.email}` | `[user].email`, when set. |

**Per-pkg inputs** — every key-value pair in `[pkg.<name>.inputs]` is a
template variable visible inside that pkg's detect, shell actions, and
nested inputs:

```toml
[pkg.rustup]
[pkg.rustup.install_hint.brew]
packages = ["rustup-init"]
[pkg.rustup.inputs]
bin_dir = "~/.cargo/bin"
[pkg.rustup.detect]
type = "file"
path = "${bin_dir}/rustup"
```

**Shadowing** — when a per-pkg input and a system input share a key, the
per-pkg value wins. This lets a pkg override `${os}`, `${brew_prefix}`, etc.
for its own detect/init logic without affecting other pkgs.

**Unresolved placeholders** — a detect check with an unresolved `${...}`
reports "not installed" (the pkg silently misses, same as a failed leaf
check). For shell actions, an unresolved placeholder aborts the run unless
the action is marked `optional = true`, in which case the action is skipped.
