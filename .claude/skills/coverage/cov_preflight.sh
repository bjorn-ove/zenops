#!/usr/bin/env bash
# Pre-flight checks for the coverage skill.
#
# Verifies cargo-llvm-cov is installed, the llvm-tools rustup component
# is present, the working directory is a Cargo workspace root, and warns
# (without failing) if src/ or tests/ have uncommitted changes.
#
# Exit codes:
#   0 — all required checks passed (the dirty-tree check is a warning).
#   1 — cargo-llvm-cov missing.
#   2 — llvm-tools rustup component missing.
#   3 — not at a Cargo workspace root.
#
# Stdout carries the structured status; stderr carries the one-line fix
# the skill should surface to the user. Each emitted line is prefixed
# with one of: OK / WARN / FAIL — easy to grep, easy to print.
#
# Flags:
#   --quiet      suppress OK lines (still prints WARN/FAIL).
#   --no-dirty   skip the dirty-tree warning entirely.

set -u

QUIET=0
NO_DIRTY=0
for arg in "$@"; do
    case "$arg" in
        --quiet) QUIET=1 ;;
        --no-dirty) NO_DIRTY=1 ;;
        -h|--help)
            sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "FAIL unknown flag: $arg" >&2
            exit 64
            ;;
    esac
done

ok()   { [ "$QUIET" -eq 1 ] || echo "OK   $*"; }
warn() { echo "WARN $*"; }
fail() { echo "FAIL $*"; echo "  fix: $2" >&2; }

# 1. cargo-llvm-cov on PATH
if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
    fail "cargo-llvm-cov not found" \
         "brew install cargo-llvm-cov  ·  cargo binstall cargo-llvm-cov  ·  cargo install cargo-llvm-cov --locked"
    exit 1
fi
ok "cargo-llvm-cov: $(command -v cargo-llvm-cov)"

# 2. llvm-tools rustup component (matches both 'llvm-tools-preview' and
#    the newer 'llvm-tools-<host-triple>' naming).
if ! rustup component list --installed 2>/dev/null \
        | grep -Eq '^llvm-tools(-preview)?(-|$)'; then
    fail "rustup component llvm-tools not installed" \
         "rustup component add llvm-tools-preview"
    exit 2
fi
ok "rustup component llvm-tools: installed"

# 3. Cargo workspace root
if [ ! -f Cargo.toml ] || ! grep -q '^\[workspace\]' Cargo.toml; then
    fail "not at a Cargo workspace root (no [workspace] in ./Cargo.toml)" \
         "cd to the workspace root and re-run"
    exit 3
fi
ok "workspace root: $(pwd)"

# 4. Dirty-tree warning (non-fatal)
if [ "$NO_DIRTY" -eq 0 ]; then
    if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        warn "not in a git work tree — skipping dirty-tree check"
    else
        dirty=$(git status --porcelain -- src tests 2>/dev/null \
                | grep -E '\.(rs|toml)$' || true)
        if [ -n "$dirty" ]; then
            warn "uncommitted changes under src/ or tests/:"
            printf '%s\n' "$dirty" | sed 's/^/       /'
            warn "consider stashing before the skill writes new tests"
        else
            ok "git tree clean under src/ and tests/"
        fi
    fi
fi

exit 0
