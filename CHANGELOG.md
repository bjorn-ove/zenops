# Changelog

## [0.10.0] - 2026-04-27

### Added
- `zenops pkg [PATTERN]...` accepts positional patterns to narrow the listing to packages whose display name or map key matches any of the substrings (case-insensitive, multi-pattern OR). The aggregate-install footer follows the visible set.

### Changed
- **Breaking:** structured and human output (`zenops status`, `zenops pkg`, `-o json`, etc.) now goes to stdout instead of stderr, matching the `find . -name hello` convention. Log messages and fatal errors continue to go to stderr. Pipelines that previously read stderr to capture output need to read stdout (or merge with `2>&1`).

## [0.9.0] - 2026-04-24

### Added
- `zenops doctor` subcommand that diagnoses the environment.
- `zenops schema` subcommand that dumps the auto-generated JSON Schema bundle for the config file, so editors and validators can type-check `config.toml` without running zenops.
- `-o json` output format is now supported by `pkg`, `doctor`, and `init` (previously only a subset of commands).
- Split config reference documentation into `docs/`; `README.md` is now focused on onboarding.

### Changed
- **Breaking:** `zenops apply --yes` now aborts with `DirtyRepoRequiresAllowDirty` when the zenops config repo has uncommitted changes. Pass `--allow-dirty` to opt back in. The interactive prompt and `--dry-run` paths are unchanged. This makes CI and cron jobs fail loudly on divergence instead of applying silently from a dirty checkout.

## [0.8.3] - 2026-04-23

### Added
- `[user]` config section with `name` and `email`, also exposed as `${user.name}` and `${user.email}` template variables for any `${...}`-expanded config value.
- `[git]` section that manages `~/.gitconfig` on every `zenops apply` — writes `user.name`/`user.email`, and enables commit signing via `[git.signing]` with `type = "ssh"` (key path) or `type = "gpg"` (key ID/fingerprint).
- `[[ssh.allowed_signers]]` entries that generate `~/.ssh/allowed_signers` for verifying SSH-signed git commits. Supports `type = "github"` (fetches signing keys from `/users/<username>/ssh_signing_keys` via `curl`) and `type = "manual"`. With `[git.signing] type = "ssh"` configured, `gpg.ssh.allowedSignersFile` is pointed at the generated file automatically.

## [0.8.2] - 2026-04-23

### Added
- `zenops init <url>` subcommand clones an existing zenops config repo into `~/.config/zenops`, validates that it has a `config.toml`, and prints a short summary — so bootstrapping a fresh machine no longer requires crafting the config directory by hand.
- `--branch`/`-b` flag on `zenops init` to check out a specific branch or tag instead of the remote's default HEAD.
- `--apply` flag on `zenops init` to chain straight into `zenops apply` after a successful clone, with `--yes`/`-y` to skip prompts non-interactively.

## [0.8.1] - 2026-04-22

### Changed
- Publish `repository` metadata so crates.io links back to the GitHub repo.

## [0.8.0] - 2026-04-22

### Added
- `zenops status --all` / `-a` lists clean items too, so you can confirm zenops walked everything it manages instead of relying on silence.
- JSON output gains a `git_repo_clean` event kind.

### Changed
- Status output color scheme: green is now reserved for "no changes needed" states (`✓ ok`, `✓ clean`) and successful post-apply outcomes. Pending changes that were previously green (`+ missing`, `A added`) are now yellow, matching the existing `~ modified` convention.
- Symlink rows render with tiered styling: the shared zenops prefix uses an extra-dim style, the ` → ` arrow is bold, and the distinguishing tail renders at default weight. The same prefix split applies to any zenops-rooted path, including git rows.

### Removed
- **Breaking (JSON):** the `pkg_missing` event kind has been replaced by a unified `pkg` event carrying a nested `PkgStatus` enum. Consumers parsing zenops JSON output must update to the new shape.

## [0.7.0] - 2026-04-22

### Added
- `--output json` emits newline-delimited JSON events (one per line) to stderr for scripts and tooling.
- `zenops pkg` now prints a note when no supported package manager is detected on PATH (currently supported: brew).

### Changed
- Human output is redesigned with colored status markers, dimmed paths, and column-aligned rows.
- Color auto-detection now reflects whether stderr (where output is written) is a terminal, not stdout — fixing color suppression when stdout is piped.
- `ColorChoice::enabled` now requires a `stream_is_terminal: bool` argument so callers can resolve color for any specific stream.

### Removed
- `--output log` has been replaced by `--output human` (the new default). Scripts that parsed the old log-style output should migrate to `--output json`.

## [0.6.1] - 2026-04-22

### Changed
- Updated `zenops-expand` dependency to v0.5.0.

## [0.6.0] - 2026-04-21

### Added
- Explicit `any` and `all` combinators for `pkg.<name>.detect`, letting a pkg be considered installed when any / all of a set of sub-strategies matches.
- Optional `os = ["macos", "linux", ...]` gate on detection strategies — evaluates to false when the current OS isn't listed. Useful for GUI apps like Ghostty on macOS whose binary lives in `/Applications/*.app` and isn't always on PATH.

### Changed
- **Breaking:** `pkg.<name>.detect` is now a single detection strategy instead of an implicitly OR-ed list. Configs using `[[pkg.X.detect]]` or `detect = [...]` must migrate to `[pkg.X.detect]` or `detect = { ... }`; multiple signals must collapse to a single `which` or be wrapped in an explicit `type = "any"` / `type = "all"` with an `of = [...]` list.
- Simplified the built-in `sk` and `starship` detects to a single `which` lookup, removing hardcoded `~/.cargo/bin/…` and `/opt/homebrew/bin/…` fallback chains — if the binary is installed it belongs on PATH.

## [0.5.1] - 2026-04-21

### Added
- `apply` now detects uncommitted changes in the zenops config repo and offers a three-way prompt before proceeding: commit & push, continue without committing, or abort. `--yes` and `--dry-run` keep the warning but skip the prompt (and continue).

### Fixed
- `zenops status` on a config repo with deleted or added files no longer panics; the git porcelain parser now decodes the full XY status pair instead of just modified/untracked entries.

## [0.5.0] - 2026-04-21

### Added
- `zenops apply` and `zenops status` now warn when a pkg with `enable = "on"` is declared but not detected on the host, including a ready-to-run install command when a package manager (e.g. Homebrew) is available.

### Changed
- **Breaking: config schema.** Configs are now declared under the pkg that owns them as `[[pkg.<key>.configs]]`, replacing the old top-level `[[configs]]` array. The `name` field on a config is gone — the pkg key doubles as the config directory name, with an optional override. Configs and shell hooks only apply when the pkg is considered installed (`detect` strategies match, or no `detect` is specified).
