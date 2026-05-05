---
name: coverage
description: Measure workspace test coverage with `cargo-llvm-cov`, propose several distinct options for improving it (each with target, rationale, current %, expected gain, effort), implement the user's pick, and report the before/after delta. Use when the user asks about coverage, wants to add tests, or says "where are we weak on tests".
---

# coverage

Measure → propose options → implement → re-measure. The skill never picks
an option on its own, never commits, and never installs tooling — it
prepares work and stops at decision points so the user can steer.

## Helper tools

The skill ships three scripts so you don't write one-off shell or
`python3 <<EOF` blocks each run:

- **`cov_preflight.sh`** — verify `cargo-llvm-cov`, the `llvm-tools`
  rustup component, the workspace root, and the dirty-tree warning in
  one go. Use this for the **Pre-flight** step.
- **`cov_rank.py`** — parse a `cargo-llvm-cov` JSON report and surface
  ranked candidates: weakest files, low-function-coverage modules, and
  (with `--unwraps --full`) `unwrap`/`expect`/`panic!` sites on
  uncovered lines. Use this for the **Analyze** step.
- **`cov_diff.py`** — diff two summary JSONs and print the workspace +
  per-file delta. Use this for the **Re-measure and report** step.

All three live next to this file and are executable. Invoke them with
their absolute paths (the skill directory is not on `$PATH`):

```bash
.claude/skills/coverage/cov_preflight.sh
.claude/skills/coverage/cov_rank.py target/llvm-cov-target/summary.json
.claude/skills/coverage/cov_diff.py before.json after.json
```

Each script's `--help` documents every flag. Do not reimplement what
they already do — extend the scripts in place if a new shape is needed.

## Pre-flight

Run `cov_preflight.sh` and react to its output. The script does the
four checks below in one pass and emits one line per check, prefixed
`OK`, `WARN`, or `FAIL`:

```bash
.claude/skills/coverage/cov_preflight.sh
```

Exit codes — `0` (all required checks passed; dirty-tree is a warning),
`1` (`cargo-llvm-cov` missing), `2` (`llvm-tools` rustup component
missing), `3` (not at a Cargo workspace root). On any non-zero exit,
relay the `fix:` line the script printed to stderr to the user and
**stop** — do not auto-install, do not retry.

If the script prints a `WARN` about uncommitted changes under `src/` or
`tests/`, surface it once and ask whether to continue before writing
new tests that would blend into the user's in-progress diff. Pass
`--no-dirty` to skip that check entirely if the user has already
acknowledged it; pass `--quiet` to suppress the `OK` lines on a re-run.

The four checks the script performs:

1. **Tool present.** `cargo-llvm-cov` on `PATH`. Fix:
   `brew install cargo-llvm-cov` · `cargo binstall cargo-llvm-cov` ·
   `cargo install cargo-llvm-cov --locked`.
2. **Rustup component.** Matches both the old `llvm-tools-preview`
   naming and the newer `llvm-tools-<host-triple>` naming. Fix:
   `rustup component add llvm-tools-preview`.
3. **Workspace root.** `Cargo.toml` exists and contains a
   `[workspace]` block. Fix: cd to the workspace root.
4. **Dirty-tree warning.** `git status --porcelain` over `src/` and
   `tests/`, filtered to `.rs` / `.toml` files. Non-fatal.

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

**Optional deeper run** — needed only if you intend to use
`cov_rank.py --unwraps` or the user asks for HTML / region detail:

```bash
# region-level data — required by cov_rank.py --unwraps
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

Run `cov_rank.py` against the summary JSON. The default output is
already shaped for chat — print it as-is, then layer interpretation on
top. The two normal invocations:

```bash
# Quick ranking (no full.json needed)
.claude/skills/coverage/cov_rank.py \
  target/llvm-cov-target/summary.json --top 10

# With unwrap/expect/panic intersection (template 3)
.claude/skills/coverage/cov_rank.py \
  target/llvm-cov-target/summary.json \
  --full target/llvm-cov-target/full.json --unwraps --top 10
```

Flags worth knowing:

- `--min-lines N` (default 50) — drop tiny files that distort the
  ranking.
- `--max-pct P` (default 95.0) — cap on what counts as "weak".
- `--top N` — number of rows to print per section.
- `--format tsv|json` — for further scripting; `text` is the default.

The script's "Weakest files" section maps to template 1, the "Modules
with low function coverage" section maps to template 4, and the
"Unwrap/expect/panic on uncovered branches" section maps to template
3. Templates 2 (untested public API) and 5 (subcrate lift) you still
derive by hand — extend `cov_rank.py` if you find yourself recomputing
them often.

Templates:

1. **Lowest-coverage file with enough lines to matter.** Surfaced by
   `cov_rank.py` directly. Pick the entry with the highest absolute
   uncovered-line count among the worst few percentages.

2. **Untested public API.** For `src/lib.rs` and every
   `crates/*/src/lib.rs`, grep `^pub (fn|struct|enum) ` and
   cross-reference declaration lines against uncovered ranges from
   `full.json`. The `cov_rank.py` source already has an
   `uncovered_line_ranges` helper — reuse it if you script this out.

3. **Unwrap / expect / panic on uncovered branches.** Surfaced directly
   by `cov_rank.py --unwraps --full`. Each row is "Cover error paths in
   `<file>` (N unwrap/expect sites on uncovered branches)."

4. **Integration-test shaped gap.** A module in `src/` with low
   `funcs%` reachable from `Cmd::*` dispatch in
   `src/lib.rs::real_main` → propose a new integration test under
   `tests/`.

5. **Subcrate lift.** A subcrate's total significantly below the
   workspace average → extend that crate's own `tests/basics.rs`
   (`crates/zenops-safe-relative-path/tests/basics.rs` is an existing
   model).

Each option carries:

- **target** — files + approximate line ranges.
- **rationale** — why this matters (public API? error handling? reachable
  from a CLI subcommand?).
- **current coverage** — `%` and uncovered-line count (read directly
  from `cov_rank.py` output).
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

2. Add end-to-end test for `Cmd::Repo`
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
new patterns. Inspect `tests/` first to find the closest existing test
shape rather than assuming a fixed filename.

**Integration tests** land in the appropriate file under `tests/`
(currently split per surface area: `tests/apply.rs`, `tests/init.rs`,
`tests/doctor.rs`, `tests/pkg_list.rs`, `tests/git_status.rs`,
`tests/symlinks.rs`, …). Use the file whose name matches the surface
under test; only add a new file if no existing surface fits. The shared
fixture is `tests/test_env.rs`.

- `use similar_asserts::assert_eq;` — the existing files use it.
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
cargo test --test <file_stem> <new_test_name> -- --nocapture
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

**Do not commit.** The user reviews `git diff` and commits. Report which
files were touched so the review is cheap.

## Re-measure and report

Always fresh — the whole point of the delta is to reflect the new work.
Save the new summary to a sibling path and run `cov_diff.py`:

```bash
cargo llvm-cov --workspace --summary-only \
  --ignore-filename-regex '/(tests|test_env)\.rs$'

cargo llvm-cov --workspace --json --summary-only \
  --ignore-filename-regex '/(tests|test_env)\.rs$' \
  --output-path target/llvm-cov-target/summary.after.json

.claude/skills/coverage/cov_diff.py \
  target/llvm-cov-target/summary.json \
  target/llvm-cov-target/summary.after.json
```

`cov_diff.py` prints workspace totals (lines + regions + functions) and
the top per-file deltas (sorted by line-% change, hides churn below
`--min-delta`, default 0.5 pts). Read its output, then compose the
final report:

```
Coverage delta:
  Workspace total:  72.3% -> 75.1%  (+2.8 pts)
  src/git.rs:       41.2% -> 74.6%  (+33.4 pts, 91 lines now covered)

Tests added:
  tests/git_status.rs
    + git_status_on_detached_head
    + git_repo_with_missing_remote
  tests/apply.rs
    + apply_fails_when_git_rev_parse_errors

Next: review `git diff`, then commit if you're happy.
```

Outcome-focused — describe what's covered now and what's next, not
internals.

## Notes

- **Helper scripts are stdlib-only.** No `pip install`. If a future
  enhancement needs a third-party library, prefer extending the script
  with stdlib code over adding a dependency.
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
- **Coverage attributes to source files, not test location.** A test
  in `tests/apply.rs` that exercises `zenops-safe-relative-path` shows
  up under the subcrate in the JSON. Rank by `filename`, not by where
  the test lives.
- **Flaky integration tests distort coverage.** The `tests/*.rs` files
  spin up real git repos via `xshell`. If a test fails on the measure
  run, abort — don't rank options on a broken run.
- **Out of scope for v1:** a `--ci` threshold mode that slots into
  `scripts/prerelease.sh`. Wait until the user picks a threshold worth
  enforcing.
