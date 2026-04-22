---
name: coverage
description: Measure workspace test coverage with `cargo-llvm-cov`, propose several distinct options for improving it (each with target, rationale, current %, expected gain, effort), implement the user's pick, and report the before/after delta. Use when the user asks about coverage, wants to add tests, or says "where are we weak on tests".
---

# coverage

Measure → propose options → implement → re-measure. The skill never picks
an option on its own, never commits, and never installs tooling — it
prepares work and stops at decision points so the user can steer.

## Pre-flight

Stop with a one-line fix on any failure — do not auto-install, do not retry.

1. **Tool present.**
   ```bash
   command -v cargo-llvm-cov
   ```
   If missing, tell the user exactly one of:
   `brew install cargo-llvm-cov` · `cargo binstall cargo-llvm-cov` ·
   `cargo install cargo-llvm-cov --locked`. Stop.

2. **Rustup component.**
   ```bash
   rustup component list --installed | grep -q llvm-tools-preview
   ```
   If missing: `rustup component add llvm-tools-preview`. Stop.

3. **Workspace root.** Read `Cargo.toml` and confirm a `[workspace]` block.
   If absent, the skill was invoked from the wrong directory — stop.

4. **Dirty tree warning.** `git status --porcelain`. If uncommitted
   changes touch `src/` or `tests/`, warn once so the user can stash
   before the skill writes tests that would blend into their in-progress
   diff. Ask whether to continue anyway.

## Measure

Two commands, in this order:

```bash
cargo llvm-cov --workspace --summary-only \
  --ignore-filename-regex '/(tests|test_env)\.rs$'

cargo llvm-cov --workspace --json --summary-only \
  --ignore-filename-regex '/(tests|test_env)\.rs$' \
  --output-path target/llvm-cov-target/summary.json
```

- `--summary-only` keeps the JSON small — per-file summaries only, enough
  for ranking.
- `--ignore-filename-regex` excludes the test files themselves (they
  shouldn't earn coverage credit).
- `target/llvm-cov-target/` is `cargo-llvm-cov`'s default workdir and is
  already ignored by the existing `/target` entry in `.gitignore`.

**Cache.** Reuse `target/llvm-cov-target/summary.json` if it is newer
than every tracked `.rs` file and every `Cargo.toml`. Re-measure on demand
("fresh", "re-run coverage"). The post-implementation re-measure in the
final step is always fresh.

**Optional deeper runs** — only when the user asks:

```bash
# region/branch-level data for "show me uncovered lines in <file>"
cargo llvm-cov --workspace --json \
  --ignore-filename-regex '/(tests|test_env)\.rs$' \
  --output-path target/llvm-cov-target/full.json

# HTML report for local browsing
cargo llvm-cov --workspace --html \
  --ignore-filename-regex '/(tests|test_env)\.rs$' \
  --output-dir target/llvm-cov-target/html
```

If any test fails during the measure run, **stop**. Coverage over a
broken suite isn't honest — fix the test first.

## Analyze

Parse `summary.json`. The shape is:

```
data[0].totals.{lines,regions,functions,branches}.{count,covered,percent}
data[0].files[].filename
data[0].files[].summary.{lines,regions,functions,branches}.{count,covered,percent,notcovered}
```

Bucket each file into one of the option templates below; pick the
highest-value entry in each template to build the menu. Aim for **3–5
distinct options**. If fewer than 3 meaningful options exist (e.g.
coverage is already very high), say so honestly rather than pad.

Templates:

1. **Lowest-coverage file with enough lines to matter.** Sort ascending
   by `summary.lines.percent`; filter `summary.lines.count >= 50` so tiny
   files don't distort the ranking.

2. **Untested public API.** For `src/lib.rs` and every
   `crates/*/src/lib.rs`, grep `^pub (fn|struct|enum) `. Cross-reference
   the declaration line against uncovered regions (needs `full.json`,
   not `summary.json` — re-run without `--summary-only` if this template
   produces a candidate). Surface per crate with the worst offender.

3. **Unwrap / expect / panic on uncovered branches.** Grep
   `\.unwrap\(|\.expect\(|panic!\(` across `src/` and `crates/*/src/`.
   Intersect line numbers with uncovered regions. Surface as
   "Cover error paths in `<file>` (N unwrap/expect sites on uncovered
   branches)."

4. **Integration-test shaped gap.** A module in `src/` with
   `summary.functions.percent < 40` that's reachable from `Cmd::*`
   dispatch in `src/lib.rs::real_main` → propose a new integration test
   in `tests/basics.rs`.

5. **Subcrate lift.** A subcrate's total significantly below the
   workspace average → extend its own `tests/basics.rs`
   (`crates/zenops-safe-relative-path/tests/basics.rs` is an existing
   model).

Each option carries:

- **target** — files + approximate line ranges.
- **rationale** — why this matters (public API? error handling? reachable
  from a CLI subcommand?).
- **current coverage** — `%` and uncovered-line count.
- **expected gain** — crude estimate:
  `uncovered_lines_addressed / total_workspace_lines * 100`, rounded.
- **effort** — S / M / L based on how many tests and fixtures are
  plausibly needed.

## Present options

Print the workspace summary first, then a numbered list. No wide tables —
they wrap badly in the terminal.

```
Workspace coverage: 72.3% lines (1,847 / 2,553)

Weakest files:
  src/git.rs              41.2%  (112 / 272 uncovered)
  src/pkg_list.rs         58.0%  (83 / 197)
  crates/zenops-expand    68.5%  (48 / 152)

Options (pick one or combine):

1. Cover error paths in src/git.rs
   Rationale: 14 unwrap/expect sites on uncovered branches; git.rs is
              user-facing.
   Current:   41.2% (112 uncovered lines)
   Gain:      ~3.5% workspace
   Effort:    M — 4-6 integration tests using TestEnv with broken
              git repos.

2. Add end-to-end test for `Cmd::Repo` in tests/basics.rs
   Rationale: Repo subcommand is reachable from main but has no direct
              integration coverage.
   Current:   0% (dispatch uncovered)
   Gain:      ~2.1% workspace
   Effort:    S — one new test following the `missing_config` pattern.

3. …
```

Then **stop and ask which option(s) to implement**. Never auto-pick.
This is the user-approval gate — same shape as `bump-version`'s pre-tag
pause.

## Implement

Once the user picks, follow the repo's existing idiom — do not invent
new patterns.

**Integration tests** land in `tests/basics.rs`:

- `use similar_asserts::assert_eq;` — the file already uses it.
- `let env = test_env::TestEnv::load();` — the canonical setup.
- Helpers available on `TestEnv`: `.run(&Cmd)`, `.resolve_path()`,
  `.cfpath()`, `.write_file()`, `.append_file()`, `.write_zenops_file()`,
  `.append_zenops_file()`, `.delete_file()`, `.delete_dir_all()`,
  `.create_dir()`, `.create_symlink()`, `.init_config()`,
  `.run_pkg_list()`.
- Paths use `srpath!("…")` (compile-time validated). Never build paths
  by string concatenation.
- Model closed-domain test inputs as enums, not stringly-typed fields.

**Subcrate tests** land in `crates/<crate>/tests/basics.rs` — separate
`tests/` directory, matching the existing layout.

Iterate fast, then widen:

```bash
cargo test --test basics <new_test_name> -- --nocapture
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

**Do not commit.** The user reviews `git diff` and commits. Report which
files were touched so the review is cheap.

## Re-measure and report

Always fresh — the whole point of the delta is to reflect the new work.

```bash
cargo llvm-cov --workspace --summary-only \
  --ignore-filename-regex '/(tests|test_env)\.rs$'

cargo llvm-cov --workspace --json --summary-only \
  --ignore-filename-regex '/(tests|test_env)\.rs$' \
  --output-path target/llvm-cov-target/summary.after.json
```

Diff against `summary.json` and emit:

```
Coverage delta:
  Workspace total:  72.3% -> 75.1%  (+2.8 pts)
  src/git.rs:       41.2% -> 74.6%  (+33.4 pts, 91 lines now covered)

Tests added:
  tests/basics.rs
    + git_status_on_detached_head
    + git_repo_with_missing_remote
    + apply_fails_when_git_rev_parse_errors

Next: review `git diff`, then commit if you're happy.
```

Outcome-focused — describe what's covered now and what's next, not
internals.

## Notes

- **First instrumented run is slow** (2–5× a normal `cargo test`).
  Instrumented binaries are large; `cargo-llvm-cov` keeps its own
  `target/llvm-cov-target/` so the normal incremental cache isn't
  invalidated. Warn the user before the first run so the wait isn't a
  surprise.
- **`llvm-tools-preview` must match active rustc.** After a toolchain
  change, `cargo-llvm-cov` may error out. Surface the real error and
  suggest `rustup component add llvm-tools-preview`. Do not retry
  blindly.
- **Doctests are not covered** by a plain `cargo llvm-cov` run
  (`--doctests` is nightly-only). Low doc-test volume in this repo; note
  the gap, do not enable.
- **Coverage attributes to source files, not test location.** A test in
  `tests/basics.rs` that exercises `zenops-safe-relative-path` shows up
  under the subcrate in the JSON. Rank by `filename`, not by where the
  test lives.
- **Flaky integration tests distort coverage.** `tests/basics.rs` spins
  up real git repos via `xshell`. If a test fails on the measure run,
  abort — don't rank options on a broken run.
- **Out of scope for v1:** a `--ci` threshold mode that slots into
  `scripts/prerelease.sh`. Wait until the user picks a threshold worth
  enforcing.
