#!/usr/bin/env bash
# Pre-release gate for the zenops workspace.
#
# Runs on the working tree (not HEAD), so the `bump-version` skill can invoke
# this after staging version edits but before committing. Fails fast on the
# first check that doesn't pass.
#
# Safe to run directly from a clean tree as well — every step is read-only
# from a repo-state perspective.

set -euo pipefail

cd "$(dirname "$0")/.."

step() {
    printf '\n==> %s\n' "$1"
}

step 'cargo fmt --all -- --check'
cargo fmt --all -- --check

step 'cargo clippy --workspace --all-targets --all-features -- -D warnings'
cargo clippy --workspace --all-targets --all-features -- -D warnings

step 'cargo test --workspace'
cargo test --workspace

step 'cargo build --workspace --release'
cargo build --workspace --release

step 'cargo doc --workspace --no-deps'
cargo doc --workspace --no-deps

step 'cargo package --workspace --allow-dirty --no-verify'
# --workspace:   cover every publishable workspace member in one invocation;
#                crates with `publish = false` are skipped automatically.
# --allow-dirty: bump-version invokes this with uncommitted version edits.
# --no-verify:   the verify-build resolves internal workspace deps from the
#                registry, which fails before siblings are published; the
#                preceding `cargo build --release --workspace` already
#                exercised the code with real resolution.
cargo package --workspace --allow-dirty --no-verify

printf '\nPre-release checks passed.\n'
