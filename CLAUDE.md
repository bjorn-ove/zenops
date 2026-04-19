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
```

## Architecture

ZenOps is a Rust (edition 2024) system configuration management tool. It reads a declarative TOML config from `~/.config/zenops/config.toml` and manages shell config and dotfiles on the local system.

**Workspace layout:**
- `src/` — main binary crate
- `crates/zenops-safe-relative-path` — custom path type that prevents `..` traversal; used throughout for all managed file paths
- `crates/zenops-safe-relative-path-macros` — `srpath!()` compile-time macro
- `crates/zenops-safe-relative-path-validator` — shared validation logic

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
