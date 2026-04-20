---
name: bump-version
description: Bump the workspace version in Cargo.toml based on SemVer-relevant changes since the tag matching the current version. Use when the user asks to bump, release, or cut a new version.
---

# bump-version

Bump the workspace `version` in `Cargo.toml` according to SemVer, based on what has actually changed since the last release tag.

## Workflow

1. **Read the current version.** Look at `[workspace.package] version = "X.Y.Z"` in the root `Cargo.toml`. This is the version of the *last* release.

2. **Locate the tag `vX.Y.Z`** matching that version.
   - `git rev-parse -q --verify refs/tags/vX.Y.Z` — succeeds if the tag exists locally.
   - If missing, run `git pull --rebase` to fetch any tags from upstream, then check again.
   - If still missing after the pull, **stop** and tell the user the tag `vX.Y.Z` is missing. Do not invent a bump.

3. **Inspect changes since the tag.** Look at both commit messages and diffs to classify the impact:
   - `git log --oneline vX.Y.Z..HEAD`
   - `git diff vX.Y.Z..HEAD -- '*.rs' 'Cargo.toml' 'crates/*/Cargo.toml'` for the shape of code changes.
   - For public-API impact, pay attention to `pub` items in `src/lib.rs` and each `crates/*/src/lib.rs`, as well as anything re-exported.

4. **Decide the bump** using SemVer:
   - **Major (X+1.0.0)** — breaking changes to any public API, CLI flag removals/renames, config schema breaks.
   - **Minor (X.Y+1.0)** — new public APIs, new CLI subcommands/flags, new config options, backwards-compatible feature additions.
   - **Patch (X.Y.Z+1)** — bug fixes, docs, internal refactors, dependency bumps with no API impact.
   - **Pre-1.0 note:** while the version is `0.Y.Z`, treat breaking changes as a *minor* bump (0.Y+1.0) and everything else as a *patch*. Cargo's SemVer rules consider `0.x` minor bumps breaking, so this is the honest mapping.

   State the rationale briefly: which commits or diffs drove the classification.

5. **Apply the bump.**
   - First, check `git status` for other modified or untracked files. The version-bump commit must touch only `Cargo.toml` and `Cargo.lock`.
     - If there are unrelated changes that clearly belong in their own commit (e.g. in-progress feature work, unstaged edits to source files), **stop and ask the user** how to handle them — commit them first with their own message, stash them, or abandon the bump.
     - If it's obvious the other changes should land before the bump (e.g. a release-prep commit the user already described), commit those first with an appropriate message, then proceed.
   - Edit `version = "X.Y.Z"` in `[workspace.package]` in the root `Cargo.toml`.
   - Also bump the hardcoded `version = "X.Y.Z"` strings on the internal `zenops-*` entries in `[workspace.dependencies]` so path deps stay in lockstep with the workspace version.
   - Run `cargo build` (or `cargo check`) to refresh `Cargo.lock` with the new version for every workspace crate.
   - Commit `Cargo.toml` and `Cargo.lock` (only those two files) with the message `Bumped version to vX.Y.Z`. Do **not** create the tag — that's on the user.

6. **Report** the old version, new version, bump type, and the reasoning. Mention that the bump was committed.

7. **Output a changelog** for the user to paste into the GitHub release notes. Keep it short and user-facing:
   - Group under `### Added`, `### Changed`, `### Fixed`, `### Removed` (omit empty groups).
   - One bullet per item, written from the perspective of an external user — what they can now do, what changed for them, what broke. No commit hashes, no author names, no internal refactors, no dependency bumps unless they affect users.
   - Skip anything a user wouldn't care about (CI tweaks, doc-only edits, test-only changes, internal renames).
   - Put the whole block inside a fenced ```markdown code block so it copies cleanly.

## Notes

- All workspace crates inherit `version.workspace = true`, so only the root `Cargo.toml` needs editing.
- Tag convention is `vX.Y.Z` (leading `v`). Do not accept `X.Y.Z` without the prefix.
- Publishing/releasing is out of scope for this skill — the user runs those steps themselves.
