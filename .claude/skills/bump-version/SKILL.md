---
name: bump-version
description: Bump one or more workspace crates independently using per-crate SemVer, based on changes since each crate's last release tag. Runs the full pre-release procedure (`scripts/prerelease.sh`) before committing, and refuses to tag if anything fails. Use when the user asks to bump, release, or cut a new version.
---

# bump-version

Each crate in the workspace has its own `version`. This skill inspects what
has changed since each crate's last release tag and bumps only the crates
that need it, propagating through the internal dep graph when a breaking
change forces dependents to re-release.

## Helper script

Mechanical steps — workspace discovery, per-crate last-tag resolution with
the pre-split `v<version>` fallback, focused `git log`/`git diff`, and
GitHub release-URL construction — are wrapped by
[`scripts/bump-helper.sh`](../../../scripts/bump-helper.sh). Use its
subcommands below instead of re-implementing the pipelines inline. Run it
with no args to see the full list.

## Discovering the workspace

Do **not** hardcode the crate list — the workspace grows. Derive it fresh
each run:

```bash
./scripts/bump-helper.sh list
```

Emits one compact JSON object per crate with fields: `name`, `version`,
`manifest_path` (repo-relative), `crate_dir` (repo-relative; `.` for the
root crate), and `internal_deps` (array of workspace crate names this one
depends on — this gives the dep-graph edges for step 3).

The root `[workspace.dependencies]` in the top-level `Cargo.toml` pins each
internal crate with an explicit `version = "X.Y.Z"`. Any crate that appears
there must have its pin updated whenever the crate itself is bumped.

## Workflow

1. **Pick the target crate(s).**
   - If the user named one or more crates (e.g. `/bump-version <name>`),
     restrict the candidate set to those.
   - Otherwise, run `./scripts/bump-helper.sh candidates`. Each row is
     tab-separated: `name, version, last_tag, tag_source, commits_since`.
     - `tag_source=own` — crate has its own `<name>-v<version>` tag.
     - `tag_source=anchor` — fell back to the pre-split workspace tag
       `v<version>`. This is the one-time anchor for crates that were at
       `0.4.2` before the per-crate split.
     - `tag_source=none` — neither exists. Run `git pull --rebase` once
       and re-run the helper. If the row is still `none`, **stop** and
       tell the user no anchoring tag exists for that crate.
   - A crate is a **candidate** when `commits_since` is non-zero. For each
     candidate, use `./scripts/bump-helper.sh commits <crate>` to read the
     log before classifying. Path filters (subcrate-dir plus root
     `Cargo.toml`/`Cargo.lock`, and the root crate excludes `crates/` so
     subcrate churn doesn't bleed in) are applied automatically by the
     helper.

2. **Classify each candidate's bump level.**
   - Read the focused diff: `./scripts/bump-helper.sh diff <crate>`. It
     shows Rust sources plus `Cargo.toml` in the crate directory since
     the last tag — the subset that determines SemVer impact.
   - Focus on `pub` items in `<crate-dir>/src/lib.rs` and re-exports.
   - SemVer rules:
     - **Major** — breaking changes to any public API, CLI flag removals/
       renames, config schema breaks.
     - **Minor** — new public APIs, new CLI subcommands/flags, new config
       options, backwards-compatible feature additions.
     - **Patch** — bug fixes, docs, internal refactors, dependency bumps
       with no API impact.
   - **Pre-1.0 adjustment:** while a crate's version is `0.Y.Z`, treat
     breaking changes as a *minor* bump (`0.Y+1.0`) and everything else as
     a *patch*. Cargo's SemVer rules consider `0.x` minor bumps breaking,
     so this is the honest mapping.
   - State the rationale briefly: which commits or diffs drove the
     classification.

3. **Propagate through the internal dep graph (leaves-first).**
   - The `internal_deps` array on each `list` row already gives the
     workspace-internal edges (deps intersected with workspace crate
     names). Topologically order so leaves come first.
   - If crate `A` receives a **breaking** bump (for `0.x`, a minor bump),
     then every workspace crate `B` with an internal dep on `A` must also
     be bumped — at minimum a patch. `B` is re-releasing because its pin
     for `A` has to point at A's new version.
   - Non-breaking bumps of `A` do *not* force dependents to release. Their
     existing pins still resolve. Mention them, but don't add them to the
     bump plan.
   - Walking leaves-first lets propagation settle in one pass.

4. **Present the final plan** as a short table before editing:
   `crate | old version | new version | reason` where reason is either a
   summary of direct changes or "propagated from <crate>".

5. **Compose a user-facing changelog body per bumped crate.** Group each
   under `### Added`, `### Changed`, `### Fixed`, `### Removed` (omit
   empty groups). One bullet per item, user-facing, no commit hashes, no
   author names. Skip anything a user wouldn't care about (CI, doc-only,
   test-only, internal renames, unseen dep bumps). This body is reused
   three times — write it once: the new `CHANGELOG.md` section (step 6c),
   the annotated tag message (step 7), and the chat report (step 9).

6. **Apply the bumps.**
   - **6a. Pre-flight `git status`.** The bump commit must touch only
     `Cargo.toml` files (root + any crate manifests being bumped),
     `Cargo.lock`, and the bumped crates' `CHANGELOG.md` files.
     - If unrelated changes exist that clearly belong in their own commit,
       **stop and ask** — commit them first with their own message, stash
       them, or abandon the bump.
     - If the user already described a release-prep commit that should
       land first, commit those changes with an appropriate message, then
       proceed.
   - **6b. Edit manifests.** For each bumped crate, edit the
     `version = "X.Y.Z"` on its `[package]` block at its `manifest_path`.
     For each bumped crate that also appears in the root
     `[workspace.dependencies]`, update its pinned `version = "X.Y.Z"`
     there too, so consumers resolve to the new release.
   - **6c. Update `CHANGELOG.md`.** For each bumped crate, prepend a new
     section directly under the file's header, using the body from
     step 5:

     ```markdown
     ## [<X.Y.Z>] - <YYYY-MM-DD>

     <body from step 5>
     ```

     Date is today (`date +%Y-%m-%d`). The file lives at
     `<crate-dir>/CHANGELOG.md` (the root crate's is at the repo root).
     If the file doesn't exist (e.g. a new subcrate's first release),
     create it with a single `# Changelog` heading followed by a blank
     line and the new section. No explanatory boilerplate — readers come
     to a CHANGELOG for the changes, not for an introduction.

     Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
     section conventions and [SemVer](https://semver.org/spec/v2.0.0.html);
     the file itself does not link them.
   - **6d. Refresh `Cargo.lock`.** Run `cargo build`.
   - **6e. Run the pre-release gate.** Execute `./scripts/prerelease.sh`.
     This runs fmt/clippy/test/build-release/doc plus a packaging check
     for every publishable crate. If the script exits non-zero, **stop**.
     Report which step failed (the script prints a visible `==>` heading
     per step; surface the failing one). Do not commit and do not tag.
     Recovery paths to offer the user:
     - Fix forward — edit the offending code and re-run the skill from
       step 6d.
     - Abandon the bump —
       `git checkout -- Cargo.toml crates/*/Cargo.toml Cargo.lock CHANGELOG.md crates/*/CHANGELOG.md`.
   - **6f. Commit.** Commit only the manifests that changed, `Cargo.lock`,
     and the bumped crates' `CHANGELOG.md` files. Commit message:
     - Single crate: `Bumped <crate> to v<X.Y.Z>`.
     - Multiple: `Bumped <crate-a> to v<X.Y.Z>, <crate-b> to v<X.Y.Z>`
       (list each bumped crate).

7. **Create one signed annotated tag per bumped crate** pointing at the
   bump commit (HEAD). Use each crate's changelog body (step 5) as the
   tag message, with the crate name + version as a title line:

   ```bash
   git tag -s --cleanup=verbatim <crate>-v<X.Y.Z> -F - <<'EOF'
   <crate> v<X.Y.Z>

   ### Added
   - …

   ### Changed
   - …
   EOF
   ```

   - Use `-s`, not `-a`. Even with `tag.gpgsign = true` in git config,
     annotated tags created with `-a` are not auto-signed in all
     configurations; being explicit guarantees GitHub marks the tag
     Verified.
   - Use `--cleanup=verbatim`. The default `strip` mode treats lines
     starting with `#` as comments and removes them, which would eat the
     `### Added` / `### Changed` / `### Fixed` / `### Removed` headings
     from the changelog.

   One tag per crate bumped (direct or propagated). If any `<crate>-v<X.Y.Z>`
   tag already exists locally, **stop and ask** — something is out of sync.

8. **Push the tags.** Push all newly-created tags in a single invocation:

   ```bash
   git push origin <tag1> <tag2> …
   ```

   Also push the bump commit itself if the current branch is ahead of its
   upstream (`git status -sb` to check). Do not use `git push --tags`
   unqualified — be explicit about the tag set so stale local tags don't
   leak.

9. **Report** the per-crate before/after versions, bump types, and
   reasoning. Then, **for every tag created**, print the crate's changelog
   body (from step 5) as copy-pasteable markdown **immediately followed by**
   a ready-to-click GitHub "new release" URL for that tag. GitHub's
   `releases/new` form does **not** read the annotated tag message, so the
   changelog must be presented in the chat where the user can grab it and
   drop it into the release body field.

   Format per tag (repeat for each):

   ````
   ### <crate> v<X.Y.Z>

   ```markdown
   ### Added
   - …

   ### Changed
   - …
   ```

   https://github.com/<owner>/<repo>/releases/new?tag=<crate>-v<X.Y.Z>&title=<crate>%20v<X.Y.Z>
   ````

   URL construction:

   - Use `./scripts/bump-helper.sh release-url <crate> <X.Y.Z>` to get the
     URL. It parses `git remote get-url origin` (ssh and https forms),
     strips `.git`, and URL-encodes the title. If the remote is not a
     GitHub URL the helper exits non-zero — in that case, still print the
     changelog block but list the raw tag name instead of a URL.
   - Don't pre-fill `&body=` from the changelog. Release notes are often
     long and contain characters that would need aggressive URL-encoding
     (newlines, backticks, code fences); URLs also have practical length
     limits. Printing the body as a fenced code block in chat is more
     reliable and easier to copy.

## Notes

- Each crate has its own `CHANGELOG.md` next to its `Cargo.toml`
  (`<crate-dir>/CHANGELOG.md`; the root crate's lives at the repo root).
  Every bump prepends a new `## [X.Y.Z] - YYYY-MM-DD` section using the
  same body that becomes the tag annotation.
- Every workspace member has its own explicit `version` in its `[package]`
  block. There is no shared `[workspace.package] version`.
- Tag convention is `<crate>-v<X.Y.Z>` (leading `v` on the version). Do
  not accept bare `X.Y.Z` or the old workspace-style `v<X.Y.Z>` for new
  bumps.
- The pre-split tag `v0.4.2` remains as a one-time fallback anchor for the
  first per-crate bump of any crate that was at `0.4.2`.
- Publishing to crates.io is out of scope — the user runs `cargo publish`
  themselves. Creating and pushing the git tags is in scope and happens
  automatically as part of this skill; the GitHub "new release" URLs in
  the final report exist to make the remaining promote-to-release step a
  one-click action.
