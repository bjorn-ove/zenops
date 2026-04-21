# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo build                          # debug build
cargo build --release                # release build
cargo test                           # all tests
cargo test --test basics             # integration tests only
cargo test --test basics <test_name> -- --nocapture  # single test with output
cargo fmt                            # format
cargo clippy                         # lint
./scripts/prerelease.sh              # pre-release gate (fmt/clippy/test/build/doc + cargo package --workspace)
```

The `bump-version` skill runs `./scripts/prerelease.sh` automatically before committing the version bump and refuses to tag if anything fails. Run it directly any time you want the same gate outside the skill.

## Architecture

ZenOps is a Rust (edition 2024) system configuration management tool. It reads a declarative TOML config from `~/.config/zenops/config.toml` and manages shell config and dotfiles on the local system.

**Workspace layout:**
- `src/` — main binary crate
- `crates/zenops-expand` — `ExpandStr` newtype for strings with `${name}` placeholders that must be `.expand(lookup)`ed before use
- `crates/zenops-safe-relative-path` — custom path type that prevents `..` traversal; used throughout for all managed file paths
- `crates/zenops-safe-relative-path-macros` — `srpath!()` compile-time macro
- `crates/zenops-safe-relative-path-validator` — shared validation logic

## Conventions

- `SmolStr`: use `SmolStr::new_static(s)` when `s` is `&'static str` (string literals, `std::env::consts::*`, etc.); reserve `SmolStr::new` for runtime-owned values. `new_static` avoids the allocation check and stores the literal directly.
- License files in subcrates: each published crate needs `LICENSE-APACHE` and `LICENSE-MIT` at its root (cargo packages each crate independently), but they must be **symlinks** to the workspace-root files (`ln -s ../../LICENSE-APACHE LICENSE-APACHE`), not real copies. Cargo resolves symlinks when building the `.crate` tarball, so each published crate ships the license text without duplicating bytes on disk. When adding a new subcrate, create the symlinks — do not copy the files.
- Per-crate versioning: every crate has its own explicit `version = "X.Y.Z"` in its `[package]` block — the workspace does not share a version. The root `[workspace.dependencies]` pins each internal crate with `version = "X.Y.Z"`; bumping an internal crate means editing both its own `[package] version` and that pin. Release tags are `<crate>-v<X.Y.Z>` (e.g. `zenops-expand-v0.4.3`). The pre-split `v0.4.2` workspace tag is kept as a one-time fallback anchor for the first per-crate bump. The `bump-version` skill automates this.

**Command flow (`src/`):**
1. `main.rs` — clap CLI, calls into `lib.rs`
2. `lib.rs` — dispatches to one of three commands: `Apply`, `Status`, `Repo`
3. `config.rs` — loads and deserializes `config.toml`
4. `config_files.rs` — applies config files: symlinks or generates content under `~/.config/` or `~/`
5. `git.rs` — checks git status of zenops config repo; also passes through raw git subcommands
6. `output.rs` — `Output` trait abstraction for reporting actions (current impl: `Log`)

**Config format (TOML):**
```toml
[shell]
type = "bash"
[shell.environment]
KEY = "value"
[shell.alias]
alias = "command"

[[configs]]
type = ".config"               # or "home"
name = "app"
source = "configs/app"         # relative path in zenops repo
symlinks = ["config.toml"]     # files to symlink (others are generated)
```

**Integration tests** (`tests/basics.rs`) use `tempfile` for isolation and spin up minimal git repos via `xshell`. `TestEnv` in `tests/test_env.rs` provides helpers for file creation, git init (with `gpgsign=false`), and `ConfigFilePath` assertions.
