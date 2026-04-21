---
name: bump-version
description: Bump one or more workspace crates independently using per-crate SemVer, based on changes since each crate's last release tag. Runs the full pre-release procedure (`scripts/prerelease.sh`) before committing, and refuses to tag if anything fails. Use when the user asks to bump, release, or cut a new version.
---

# bump-version

Each crate in the workspace has its own `version`. This skill inspects what
has changed since each crate's last release tag and bumps only the crates
that need it, propagating through the internal dep graph when a breaking
change forces dependents to re-release.

## Discovering the workspace

Do **not** hardcode the crate list — the workspace grows. Derive it fresh
each run:

```bash
cargo metadata --format-version=1 --no-deps
```

From the JSON, for each entry in `packages`:

- `name` — the crate name (used in the tag, commit message, and changelog)
- `version` — the crate's current version
- `manifest_path` — its `Cargo.toml` (edit this for the version bump)
- the crate's directory is `dirname(manifest_path)`, relative to the repo
  root — use this for `git log`/`git diff` path filters
- `dependencies[].name` intersected with the set of workspace package names
  gives the **internal** edges for dep-graph propagation

The root `[workspace.dependencies]` in the top-level `Cargo.toml` pins each
internal crate with an explicit `version = "X.Y.Z"`. Any crate that appears
there must have its pin updated whenever the crate itself is bumped.

## Workflow

1. **Pick the target crate(s).**
   - If the user named one or more crates (e.g. `/bump-version <name>`),
     restrict the candidate set to those.
   - Otherwise, for each workspace crate `C` discovered above:
     - Find the last tag `<C.name>-v<C.version>`:
       `git rev-parse -q --verify refs/tags/<C.name>-v<C.version>`.
     - If missing, run `git pull --rebase` and re-check.
     - If still missing, fall back to the pre-split workspace tag
       `v<C.version>` — this is the one-time anchor for crates that were at
       `0.4.2` before the per-crate split. If *that* also fails, **stop**
       and tell the user no anchoring tag exists for `C.name`.
     - Run `git log --oneline <tag>..HEAD -- <crate-dir>/ <root-manifests>`.
       - `<crate-dir>` is the crate's directory (from `manifest_path`).
       - `<root-manifests>` is `Cargo.toml Cargo.lock` for every crate,
         since root-level changes (workspace dep pins, resolver, etc.) can
         affect any member. For the root crate, pass the repo root itself
         — but exclude the `crates/` subtree so its changes don't bleed in
         (`:(exclude)crates/`).
     - A crate is a **candidate** if that log is non-empty.

2. **Classify each candidate's bump level.**
   - `git diff <tag>..HEAD -- <crate-dir>/'*.rs' <crate-dir>/Cargo.toml`
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
   - Build the internal dep graph from `cargo metadata` (intersect each
     crate's deps with the workspace package set). Topologically order it
     so leaves come first.
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

5. **Apply the bumps.**
   - **5a. Pre-flight `git status`.** The bump commit must touch only
     `Cargo.toml` files (root + any crate manifests being bumped) and
     `Cargo.lock`.
     - If unrelated changes exist that clearly belong in their own commit,
       **stop and ask** — commit them first with their own message, stash
       them, or abandon the bump.
     - If the user already described a release-prep commit that should
       land first, commit those changes with an appropriate message, then
       proceed.
   - **5b. Edit manifests.** For each bumped crate, edit the
     `version = "X.Y.Z"` on its `[package]` block at its `manifest_path`.
     For each bumped crate that also appears in the root
     `[workspace.dependencies]`, update its pinned `version = "X.Y.Z"`
     there too, so consumers resolve to the new release.
   - **5c. Refresh `Cargo.lock`.** Run `cargo build`.
   - **5d. Run the pre-release gate.** Execute `./scripts/prerelease.sh`.
     This runs fmt/clippy/test/build-release/doc plus a packaging check
     for every publishable crate. If the script exits non-zero, **stop**.
     Report which step failed (the script prints a visible `==>` heading
     per step; surface the failing one). Do not commit and do not tag.
     Recovery paths to offer the user:
     - Fix forward — edit the offending code and re-run the skill from
       step 5c.
     - Abandon the bump —
       `git checkout -- Cargo.toml crates/*/Cargo.toml Cargo.lock`.
   - **5e. Commit.** Commit only the manifests that changed and
     `Cargo.lock`. Commit message:
     - Single crate: `Bumped <crate> to v<X.Y.Z>`.
     - Multiple: `Bumped <crate-a> to v<X.Y.Z>, <crate-b> to v<X.Y.Z>`
       (list each bumped crate).

6. **Compose a user-facing changelog per bumped crate.** Group each under
   `### Added`, `### Changed`, `### Fixed`, `### Removed` (omit empty
   groups). One bullet per item, user-facing, no commit hashes, no author
   names. Skip anything a user wouldn't care about (CI, doc-only, test-only,
   internal renames, unseen dep bumps). These bodies become both the tag
   annotations and the output in step 9 — write them once, reuse.

7. **Create one annotated tag per bumped crate** pointing at the bump
   commit (HEAD). Use each crate's changelog as the tag message, with the
   crate name + version as a title line:

   ```bash
   git tag -a <crate>-v<X.Y.Z> -F - <<'EOF'
   <crate> v<X.Y.Z>

   ### Added
   - …

   ### Changed
   - …
   EOF
   ```

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
   reasoning, then list every tag created — each as a ready-to-click
   GitHub "new release" URL so the user can promote them in the UI:

   ```
   https://github.com/<owner>/<repo>/releases/new?tag=<crate>-v<X.Y.Z>
   ```

   Derive `<owner>/<repo>` from `git remote get-url origin` (handle both
   `git@github.com:owner/repo.git` and `https://github.com/owner/repo.git`
   forms; strip any trailing `.git`). If the remote is not a GitHub URL,
   list the raw tag names instead and skip the URL.

## Notes

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
