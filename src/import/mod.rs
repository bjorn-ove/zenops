//! Implementation of `zenops import`.
//!
//! Takes a path under `$HOME` (`~/.config/<x>` or `~/.<x>`), moves the file
//! or directory into `~/.config/zenops/configs/<pkg>/`, replaces the
//! original with a symlink, and appends a `[[pkg.<pkg>.configs]]` block to
//! `config.toml`. Path classification is strict: anything that isn't one
//! of those two shapes is rejected, since silently re-routing a surprising
//! layout produces a mis-managed config.
//!
//! Filesystem changes are applied before the TOML update so a partial
//! failure can be re-driven by re-running `import` (idempotent for
//! already-symlinked files) — the alternative ordering would leave
//! `config.toml` pointing at non-existent repo files.

mod error;

pub use error::Error as ImportError;

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use smol_str::{SmolStr, ToSmolStr};
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, value};
use zenops_safe_relative_path::SinglePathComponent;

use crate::{
    Args,
    config_files::{ConfigFileDirs, ConfigFilePath},
    error::Error,
    line_prompter::{LineOutcome, LinePrompter, RustylinePrompter},
    output::{
        AppliedAction, Event, ImportApplied, ImportFileAction, ImportMode, ImportPlan,
        ImportTomlChange, ImportType, Output,
    },
};

/// Entry point for `zenops import`. Builds a [`LinePrompter`] when one is
/// needed (interactive mode for prompts the caller didn't pre-answer via
/// flags), then hands off to [`run_with_prompter`].
#[allow(clippy::too_many_arguments)]
pub fn run(
    path: &Path,
    pkg: Option<&str>,
    source_override: Option<&str>,
    brew: &[String],
    no_install_hint: bool,
    yes: bool,
    dry_run: bool,
    dirs: &ConfigFileDirs,
    args: &Args,
    output: &mut dyn Output,
) -> Result<(), Error> {
    if yes || dry_run {
        run_with_prompter(
            path,
            pkg,
            source_override,
            brew,
            no_install_hint,
            yes,
            dry_run,
            dirs,
            output,
            None,
        )
    } else if args.stdin_is_terminal {
        let mut prompter = RustylinePrompter::new().map_err(crate::prompt::PromptError::Read)?;
        run_with_prompter(
            path,
            pkg,
            source_override,
            brew,
            no_install_hint,
            yes,
            dry_run,
            dirs,
            output,
            Some(&mut prompter),
        )
    } else {
        Err(ImportError::NeedsTty.into())
    }
}

/// `run` minus the prompter construction so tests can drive interactive
/// paths against a [`crate::line_prompter::BufReadPrompter`]. `prompter` is
/// `None` under `--yes` / `--dry-run` (where prompts are pre-answered).
#[allow(clippy::too_many_arguments)]
pub fn run_with_prompter(
    path: &Path,
    pkg_override: Option<&str>,
    source_override: Option<&str>,
    brew: &[String],
    no_install_hint: bool,
    yes: bool,
    dry_run: bool,
    dirs: &ConfigFileDirs,
    output: &mut dyn Output,
    mut prompter: Option<&mut dyn LinePrompter>,
) -> Result<(), Error> {
    if brew.iter().any(|s| s.is_empty()) {
        return Err(ImportError::EmptyBrewPackage.into());
    }
    if let Some(over) = source_override {
        let parsed = Path::new(over);
        if parsed.is_absolute()
            || parsed
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(ImportError::SourceOverrideEscapesRepo(parsed.to_path_buf()).into());
        }
    }

    let cfg_path = dirs.zenops().join("config.toml");
    let cfg_text = match fs::read_to_string(&cfg_path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(ImportError::ZenopsRepoMissing(cfg_path).into());
        }
        Err(e) => return Err(ImportError::Io(cfg_path, e).into()),
    };
    let mut doc: DocumentMut = cfg_text
        .parse()
        .map_err(|e| Error::from(ImportError::ConfigParse(cfg_path.clone(), e)))?;

    let plan = build_plan(path, pkg_override, source_override, dirs, &doc)?;

    if matches!(
        plan.toml,
        TomlPlan::ExtendExisting { .. } | TomlPlan::Reconcile { .. }
    ) {
        let mut bad_flags: Vec<&'static str> = Vec::new();
        if pkg_override.is_some() {
            bad_flags.push("--pkg");
        }
        if source_override.is_some() {
            bad_flags.push("--source");
        }
        if !brew.is_empty() {
            bad_flags.push("--brew");
        }
        if no_install_hint {
            bad_flags.push("--no-install-hint");
        }
        if !bad_flags.is_empty() {
            return Err(ImportError::ExtendFlagsInvalid { flags: bad_flags }.into());
        }
    }

    let created_pkg = !pkg_block_exists(&doc, &plan.pkg_key);
    let brew_packages = resolve_brew(
        &plan.pkg_key,
        brew,
        no_install_hint,
        created_pkg,
        &mut prompter,
    )?;

    let is_noop = plan.files.is_empty()
        && plan.removed_files.is_empty()
        && plan.renamed_files.is_empty()
        && match &plan.toml {
            TomlPlan::Reconcile {
                added_symlinks,
                removed_symlinks,
                ..
            } => added_symlinks.is_empty() && removed_symlinks.is_empty(),
            _ => false,
        };

    output.push(Event::ImportPlan(plan_to_event(
        &plan,
        created_pkg,
        no_install_hint,
        &brew_packages,
    )))?;

    if dry_run {
        return Ok(());
    }

    // Nothing to do — skip confirm + apply + write so a no-op reconcile
    // doesn't reflow `config.toml` or pester the user for approval.
    if is_noop {
        output.push(Event::ImportApplied(ImportApplied {
            pkg: plan.pkg_key.clone(),
            is_noop: true,
        }))?;
        return Ok(());
    }

    if !yes
        && let Some(p) = prompter.as_mut()
        && !confirm(*p, "Apply this plan?")?
    {
        return Err(ImportError::Aborted.into());
    }

    apply_files(&plan, dirs, output)?;
    update_doc(
        &mut doc,
        &plan,
        created_pkg,
        no_install_hint,
        &brew_packages,
    )?;

    fs::write(&cfg_path, doc.to_string()).map_err(|e| Error::from(ImportError::Io(cfg_path, e)))?;

    output.push(Event::ImportApplied(ImportApplied {
        pkg: plan.pkg_key.clone(),
        is_noop: false,
    }))?;

    Ok(())
}

/// Assembled plan handed to the apply phase. Built up-front so every error
/// path bails before any filesystem state changes.
struct Plan {
    /// Pkg key the new `[[pkg.<key>.configs]]` lands under.
    pkg_key: SmolStr,
    /// Layout shape, derived from the canonical path (or the user's
    /// `--pkg`-induced override).
    r#type: ImportType,
    /// The canonical absolute path the user pointed at — used in the
    /// summary event.
    source: PathBuf,
    /// Directory we walk for source files. Equal to [`Self::source`] for
    /// directory imports; the parent of the file for single-file imports.
    /// Symlinks land at the same paths under here.
    source_root: PathBuf,
    /// Absolute repo-side destination root (`<zenops>/<repo_rel>`).
    repo_dest: PathBuf,
    /// Repo-side destination root, relative to `~/.config/zenops`.
    repo_rel: String,
    /// Per-file plan: each file's path relative to `source_root`. The
    /// symlink lands at `source_root.join(rel)`; the repo copy lands at
    /// `repo_dest.join(repo_rel)`.
    files: Vec<PlannedFile>,
    /// Entries skipped during the walk.
    skipped: Vec<SkippedEntry>,
    /// Repo-side files queued for deletion (reconcile mode only). Each
    /// `rel` resolves under `repo_dest`. Empty for non-reconcile plans.
    removed_files: Vec<RemovedFile>,
    /// Repo-side files queued for renaming (reconcile mode only). Each
    /// pair holds rels relative to `repo_dest`. Empty for non-reconcile
    /// plans.
    renamed_files: Vec<RenamedFile>,
    /// Layout-specific metadata used when rendering the TOML entry.
    toml: TomlPlan,
}

/// One entry the source walk decided not to touch (existing symlink,
/// non-regular file). Internal counterpart to [`ImportFileAction::Skip`];
/// we keep this internal type so changes to the wire variant don't ripple
/// through the planner.
#[derive(Debug, Clone)]
struct SkippedEntry {
    /// Path relative to the imported source root.
    path: PathBuf,
    /// Stable reason tag, lifted onto the wire variant verbatim.
    reason: SmolStr,
}

struct PlannedFile {
    /// Path relative to `Plan::source_root`. Source = `source_root.join(rel)`,
    /// symlink lands at `source_root.join(rel)`.
    rel: PathBuf,
    /// Relative path inside the in-repo destination tree. Same as `rel` in
    /// the typical case.
    repo_rel: PathBuf,
}

/// One repo-side file slated for removal in reconcile mode. The repo path
/// is `Plan::repo_dest.join(repo_rel)`.
struct RemovedFile {
    /// Path relative to `Plan::repo_dest`.
    repo_rel: PathBuf,
}

/// One repo-side file slated for renaming in reconcile mode (because the
/// user renamed the home-side symlink). Both rels resolve under
/// `Plan::repo_dest`.
struct RenamedFile {
    /// Old path (still listed in the entry's `symlinks` array).
    from: PathBuf,
    /// New path (where the home-side symlink now lives).
    to: PathBuf,
}

/// What the new TOML entry will carry beyond the shared `source` /
/// `symlinks` fields. Keeps layout-specific bookkeeping out of [`Plan`]'s
/// other fields.
enum TomlPlan {
    DotConfig {
        /// `name` field for the `[[pkg.<key>.configs]]` entry. `Some` when
        /// the pkg key differs from the dir under `~/.config`; `None`
        /// otherwise.
        name_override: Option<String>,
        /// Symlinks listed in the TOML entry, relative to the dir at
        /// `~/.config/<dir>/`.
        symlinks: Vec<String>,
    },
    Home {
        /// `dir` field of the `[[pkg.<key>.configs]]` entry. `""` when the
        /// imported source is a single file at the home root.
        dir: String,
        /// Symlinks listed in the TOML entry, relative to `~/<dir>/`.
        symlinks: Vec<String>,
    },
    /// Extend an existing `[[pkg.<key>.configs]]` entry by appending paths
    /// to its `symlinks` array. Used when the imported path lives inside
    /// an already-managed config dir.
    ExtendExisting {
        /// Index of the configs entry inside `[pkg.<key>].configs`.
        config_index: usize,
        /// Current contents of the entry's `symlinks` array — kept so the
        /// renderer can show the array as it will read after the append.
        existing_symlinks: Vec<String>,
        /// New paths to add (delta against `existing_symlinks`). Empty
        /// when the file rel was already listed; the array is then left
        /// untouched.
        added_symlinks: Vec<String>,
    },
    /// Reconcile an existing `[[pkg.<key>.configs]]` entry against its
    /// home-side directory. Triggered when the imported path *is* an
    /// already-managed on-disk root: walks the directory, proposes adds
    /// for new files, removes for paths whose home-side counterpart is
    /// gone, and renames for symlinks the user moved in place.
    Reconcile {
        /// Index of the configs entry inside `[pkg.<key>].configs`.
        config_index: usize,
        /// Current contents of the entry's `symlinks` array.
        existing_symlinks: Vec<String>,
        /// Paths to append to the array (new files found on disk plus
        /// the `to` side of any renames).
        added_symlinks: Vec<String>,
        /// Paths to drop from the array (no home-side counterpart, plus
        /// the `from` side of any renames).
        removed_symlinks: Vec<String>,
    },
}

/// Resolve and classify the source path; walk it; choose pkg key and
/// repo-side destination; flag any pre-existing destination files;
/// short-circuit any already-imported symlinks.
fn build_plan(
    raw_path: &Path,
    pkg_override: Option<&str>,
    source_override: Option<&str>,
    dirs: &ConfigFileDirs,
    doc: &DocumentMut,
) -> Result<Plan, Error> {
    let cwd = std::env::current_dir().map_err(|e| ImportError::Io(PathBuf::from("."), e))?;
    let joined = if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else {
        cwd.join(raw_path)
    };

    let probe = match joined.symlink_metadata() {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(ImportError::SourceMissing(joined).into());
        }
        Err(e) => return Err(ImportError::Io(joined, e).into()),
    };
    if probe.file_type().is_symlink() {
        return Err(ImportError::SourceIsSymlink(joined).into());
    }

    let canonical_source = joined
        .canonicalize()
        .map_err(|e| ImportError::Io(joined.clone(), e))?;
    let canonical_home = dirs
        .home()
        .canonicalize()
        .map_err(|e| ImportError::Io(dirs.home().to_path_buf(), e))?;

    // Guard against importing the zenops repo into itself: if the path
    // canonicalizes to (or under) `~/.config/zenops`, we'd otherwise walk
    // every file in the repo (including `.git`) into `configs/zenops/`.
    let canonical_zenops = dirs
        .zenops()
        .canonicalize()
        .map_err(|e| ImportError::Io(dirs.zenops().to_path_buf(), e))?;
    if canonical_source == canonical_zenops || canonical_source.starts_with(&canonical_zenops) {
        return Err(ImportError::CannotImportZenopsRepo(canonical_source).into());
    }

    let tail = canonical_source
        .strip_prefix(&canonical_home)
        .map_err(|_| ImportError::PathNotUnderHome(canonical_source.clone()))?;

    // Reconcile mode: imported path *is* an already-managed on-disk
    // root. Diff the directory against the entry's `symlinks` array.
    // Checked before the strict-descendant `find_matching_config` so
    // pointing at a subdir under a managed root still hits extend mode.
    if let Some(root_match) = find_managed_root(doc, tail)? {
        return build_reconcile_plan(root_match, canonical_source, &canonical_home, dirs);
    }

    // Extend mode: input lives inside an already-managed config dir.
    // Append to that config's `symlinks` array instead of creating a new
    // pkg. Only triggers for strict descendants of an existing on-disk
    // root.
    if let Some(matched) = find_matching_config(doc, tail)? {
        let is_dir = canonical_source.is_dir();
        if is_dir {
            return Err(ImportError::ExtendDirectoryNotSupported(canonical_source).into());
        }
        return build_extend_plan(matched, canonical_source, &canonical_home, dirs);
    }

    let layout = classify(tail)?;

    let is_dir = canonical_source.is_dir();
    let pkg_key = derive_pkg_key(&layout, pkg_override)?;

    // Refuse a brand-new pkg block when the key collides with an
    // already-populated pkg. (We're past `find_managed_root` /
    // `find_matching_config` already, so reaching this point with a
    // pkg block that already has configs entries means the user is
    // importing a different on-disk root under the same name — that's
    // almost always the wrong choice.)
    if pkg_has_configs_entries(doc, pkg_key.as_str()) {
        return Err(ImportError::PkgKeyTaken { pkg: pkg_key }.into());
    }

    let repo_rel = source_override
        .map(str::to_string)
        .unwrap_or_else(|| format!("configs/{pkg_key}"));
    let repo_dest = dirs.zenops().join(&repo_rel);

    let (source_root, toml_plan_kind) = match (&layout, is_dir) {
        (Layout::DotConfig { dir }, true) => {
            let root = canonical_home.join(".config").join(dir);
            let name_override = if dir == pkg_key.as_str() {
                None
            } else {
                Some(dir.clone())
            };
            (root, TomlPlanKind::DotConfig { name_override })
        }
        (Layout::DotConfig { .. }, false) => {
            return Err(ImportError::ExpectedDirectory(canonical_source).into());
        }
        (Layout::Home { name }, true) => {
            let root = canonical_home.join(name);
            (root, TomlPlanKind::Home { dir: name.clone() })
        }
        (Layout::Home { name: _ }, false) => {
            // Single dotfile at $HOME: walk the parent and let the file
            // carry its own leading-dot name in the symlinks entry.
            let root = canonical_source
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| canonical_home.clone());
            (root, TomlPlanKind::HomeSingleFile)
        }
    };

    let collected = if is_dir {
        let mut files = Vec::new();
        let mut skipped = Vec::new();
        walk_dir(&canonical_source, &mut files, &mut skipped, Path::new(""))?;
        (files, skipped)
    } else {
        let name = canonical_source.file_name().ok_or_else(|| {
            ImportError::Io(canonical_source.clone(), io::ErrorKind::InvalidInput.into())
        })?;
        (
            vec![CollectedEntry {
                rel: PathBuf::from(name),
                repo_rel: PathBuf::from(name),
            }],
            Vec::new(),
        )
    };
    let (collected, skipped) = collected;

    let mut files = Vec::with_capacity(collected.len());
    let mut already_imported = 0usize;
    for entry in collected {
        let symlink_path = source_root.join(&entry.rel);
        let dest_in_repo = repo_dest.join(&entry.repo_rel);

        // Idempotency: if the destination file already exists AND the
        // source is a symlink at the right path, skip it. Anything else
        // with an existing dest is a hard error.
        if dest_in_repo.try_exists().unwrap_or(false) {
            if symlink_path
                .symlink_metadata()
                .is_ok_and(|m| m.is_symlink())
                && std::fs::read_link(&symlink_path).is_ok_and(|t| t == dest_in_repo)
            {
                already_imported += 1;
                continue;
            }
            return Err(ImportError::DestExists(dest_in_repo).into());
        }

        files.push(PlannedFile {
            rel: entry.rel,
            repo_rel: entry.repo_rel,
        });
    }

    if files.is_empty() && already_imported == 0 {
        return Err(ImportError::SourceEmpty(canonical_source).into());
    }

    let toml = match toml_plan_kind {
        TomlPlanKind::DotConfig { name_override } => TomlPlan::DotConfig {
            name_override,
            symlinks: files
                .iter()
                .map(|f| path_to_forward_slash(&f.repo_rel))
                .collect(),
        },
        TomlPlanKind::Home { dir } => TomlPlan::Home {
            dir,
            symlinks: files
                .iter()
                .map(|f| path_to_forward_slash(&f.repo_rel))
                .collect(),
        },
        TomlPlanKind::HomeSingleFile => TomlPlan::Home {
            dir: String::new(),
            symlinks: files
                .iter()
                .map(|f| path_to_forward_slash(&f.rel))
                .collect(),
        },
    };

    let r#type = match &layout {
        Layout::DotConfig { .. } => ImportType::DotConfig,
        Layout::Home { .. } => ImportType::Home,
    };

    Ok(Plan {
        pkg_key,
        r#type,
        source: canonical_source,
        source_root,
        repo_dest,
        repo_rel,
        files,
        skipped,
        removed_files: Vec::new(),
        renamed_files: Vec::new(),
        toml,
    })
}

/// Classified layout shape of the input path's home-relative tail.
#[derive(Debug)]
enum Layout {
    /// `~/.config/<dir>` — a single component under `.config/`.
    DotConfig { dir: String },
    /// `~/.<name>` — a single dot-prefixed component at the home root.
    /// `name` keeps the leading dot.
    Home { name: String },
}

/// Build a [`Layout`] from the home-relative tail. Strict: rejects depths
/// other than 1 (Home) or 2 (DotConfig with `.config` first).
fn classify(tail: &Path) -> Result<Layout, ImportError> {
    let comps: Vec<&str> = tail
        .components()
        .map(|c| c.as_os_str().to_str().unwrap_or(""))
        .collect();
    match comps.as_slice() {
        [".config", dir] if !dir.is_empty() && SinglePathComponent::try_new(dir).is_ok() => {
            Ok(Layout::DotConfig {
                dir: (*dir).to_string(),
            })
        }
        // `.config` on its own is the config root, not an importable
        // dotfile — reject before the Home-shape arm matches it.
        [".config"] => Err(ImportError::UnsupportedLayout(".config".to_string())),
        [name]
            if name.starts_with('.')
                && name.len() > 1
                && SinglePathComponent::try_new(name).is_ok() =>
        {
            Ok(Layout::Home {
                name: (*name).to_string(),
            })
        }
        _ => Err(ImportError::UnsupportedLayout(
            tail.to_string_lossy().into_owned(),
        )),
    }
}

/// Pkg-key resolution: explicit `--pkg`, then derive from the layout. The
/// derived key strips the leading dot for Home entries.
fn derive_pkg_key(layout: &Layout, pkg_override: Option<&str>) -> Result<SmolStr, ImportError> {
    let candidate = match pkg_override {
        Some(s) => s.to_string(),
        None => match layout {
            Layout::DotConfig { dir } => dir.clone(),
            Layout::Home { name } => name
                .strip_prefix('.')
                .map(str::to_string)
                .unwrap_or_else(|| name.clone()),
        },
    };
    SinglePathComponent::try_new(&candidate)
        .map(|_| candidate.to_smolstr())
        .map_err(|_| ImportError::NoDerivablePkgKey(candidate))
}

/// `TomlPlan` mid-build state, before we know which file list to attach.
enum TomlPlanKind {
    DotConfig { name_override: Option<String> },
    Home { dir: String },
    HomeSingleFile,
}

/// One source file under the import root, pre-classification.
struct CollectedEntry {
    /// Path relative to the home-side root (e.g. `~/.config/foo/`).
    /// For a single-file import this matches `repo_rel`.
    rel: PathBuf,
    /// Path relative to the repo-side root (e.g. `<zenops>/configs/foo/`).
    repo_rel: PathBuf,
}

fn walk_dir(
    abs: &Path,
    files: &mut Vec<CollectedEntry>,
    skipped: &mut Vec<SkippedEntry>,
    rel_prefix: &Path,
) -> Result<(), Error> {
    let entries = fs::read_dir(abs).map_err(|e| ImportError::Io(abs.to_path_buf(), e))?;
    let mut sorted: Vec<_> = entries
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ImportError::Io(abs.to_path_buf(), e))?;
    sorted.sort_by_key(|e| e.file_name());

    for entry in sorted {
        let name = entry.file_name();
        let rel = rel_prefix.join(&name);
        let meta = entry
            .file_type()
            .map_err(|e| ImportError::Io(entry.path(), e))?;
        if meta.is_symlink() {
            skipped.push(SkippedEntry {
                path: rel,
                reason: SmolStr::new_static("symlink"),
            });
        } else if meta.is_file() {
            files.push(CollectedEntry {
                rel: rel.clone(),
                repo_rel: rel,
            });
        } else if meta.is_dir() {
            // VCS metadata dirs (a `.git` checkout in a config dir is a
            // common shape) get skipped wholesale rather than walked —
            // recursing them would symlink thousands of pack files into
            // the user's home tree.
            let name_str = name.to_str().unwrap_or("");
            if matches!(name_str, ".git" | ".hg" | ".svn") {
                skipped.push(SkippedEntry {
                    path: rel,
                    reason: SmolStr::new_static("vcs"),
                });
                continue;
            }
            walk_dir(&entry.path(), files, skipped, &rel)?;
        } else {
            skipped.push(SkippedEntry {
                path: rel,
                reason: SmolStr::new_static("other"),
            });
        }
    }
    Ok(())
}

fn pkg_block_exists(doc: &DocumentMut, key: &str) -> bool {
    doc.get("pkg")
        .and_then(|p| p.as_table())
        .and_then(|t| t.get(key))
        .is_some()
}

/// `true` if `[pkg.<key>]` has at least one `[[pkg.<key>.configs]]` entry
/// already. Distinct from [`pkg_block_exists`] because a user can pre-seed
/// `[pkg.<key>]` with metadata (description, install_hint) without any
/// configs — that case still allows new-pkg-style import to land the first
/// configs entry.
fn pkg_has_configs_entries(doc: &DocumentMut, key: &str) -> bool {
    doc.get("pkg")
        .and_then(|p| p.as_table())
        .and_then(|t| t.get(key))
        .and_then(|p| p.as_table())
        .and_then(|t| t.get("configs"))
        .and_then(Item::as_array_of_tables)
        .is_some_and(|aot| !aot.is_empty())
}

/// One existing `[[pkg.<key>.configs]]` entry whose on-disk root is a
/// strict ancestor of the imported path.
struct ConfigMatch {
    pkg_key: SmolStr,
    /// Index of the entry inside `[pkg.<key>].configs`.
    config_index: usize,
    r#type: ImportType,
    /// Home-relative on-disk root (e.g. `.config/helix` or `.ssh`). The
    /// imported path's tail is this prefix plus `file_rel`.
    on_disk_root: String,
    /// `source` field of the matched entry — repo-side dir relative to
    /// `~/.config/zenops` (e.g. `configs/helix`).
    source_rel: String,
    /// Imported file's path relative to `on_disk_root` — what we'd append
    /// to the entry's `symlinks` array.
    file_rel: PathBuf,
    /// Current `symlinks` array contents, used to compute the delta and
    /// to render an "after" preview.
    existing_symlinks: Vec<String>,
}

/// One existing `[[pkg.<key>.configs]]` entry whose on-disk root *equals*
/// the imported path. Returned by [`find_managed_root`] to drive
/// reconcile mode.
struct RootMatch {
    pkg_key: SmolStr,
    /// Index of the entry inside `[pkg.<key>].configs`.
    config_index: usize,
    r#type: ImportType,
    /// Home-relative on-disk root (e.g. `.config/helix` or `.ssh`).
    on_disk_root: String,
    /// `source` field of the matched entry — repo-side dir relative to
    /// `~/.config/zenops`.
    source_rel: String,
    /// Current `symlinks` array contents.
    existing_symlinks: Vec<String>,
}

/// Walk every `[[pkg.<x>.configs]]` entry in `doc` and pick the one whose
/// on-disk root is the longest strict ancestor of `home_tail`. Returns
/// `Ok(None)` if no entry claims the path. Errors with
/// [`ImportError::AmbiguousConfigMatch`] when two distinct entries tie for
/// longest prefix — that's a hand-edited config wart the user should
/// resolve first.
fn find_matching_config(
    doc: &DocumentMut,
    home_tail: &Path,
) -> Result<Option<ConfigMatch>, ImportError> {
    let pkg_table = match doc.get("pkg").and_then(Item::as_table) {
        Some(t) => t,
        None => return Ok(None),
    };

    let tail_str = path_to_forward_slash(home_tail);
    let mut matches: Vec<ConfigMatch> = Vec::new();

    for (pkg_key_raw, pkg_item) in pkg_table.iter() {
        let configs = match pkg_item
            .as_table()
            .and_then(|t| t.get("configs"))
            .and_then(Item::as_array_of_tables)
        {
            Some(c) => c,
            None => continue,
        };

        for (idx, entry) in configs.iter().enumerate() {
            let typ = match entry.get("type").and_then(|v| v.as_str()) {
                Some(t) => t,
                None => continue,
            };

            let (on_disk_root, import_type) = match typ {
                ".config" => {
                    let dir = entry
                        .get("name")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .unwrap_or(pkg_key_raw);
                    (format!(".config/{dir}"), ImportType::DotConfig)
                }
                "home" => {
                    let dir = match entry.get("dir").and_then(|v| v.as_str()) {
                        Some(d) if !d.is_empty() => d,
                        _ => continue,
                    };
                    (dir.to_string(), ImportType::Home)
                }
                _ => continue,
            };

            let prefix = format!("{on_disk_root}/");
            let file_rel_str = match tail_str.strip_prefix(&prefix) {
                Some(rest) if !rest.is_empty() => rest,
                _ => continue,
            };

            let source_rel = match entry.get("source").and_then(|v| v.as_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };

            let existing_symlinks = entry
                .get("symlinks")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();

            matches.push(ConfigMatch {
                pkg_key: SmolStr::new(pkg_key_raw),
                config_index: idx,
                r#type: import_type,
                on_disk_root,
                source_rel,
                file_rel: PathBuf::from(file_rel_str),
                existing_symlinks,
            });
        }
    }

    let max_len = match matches.iter().map(|m| m.on_disk_root.len()).max() {
        Some(n) => n,
        None => return Ok(None),
    };
    let best: Vec<ConfigMatch> = matches
        .into_iter()
        .filter(|m| m.on_disk_root.len() == max_len)
        .collect();

    if best.len() > 1 {
        return Err(ImportError::AmbiguousConfigMatch {
            path: PathBuf::from(tail_str),
            candidates: best.into_iter().map(|m| m.pkg_key).collect(),
        });
    }
    Ok(best.into_iter().next())
}

/// Find the unique `[[pkg.<x>.configs]]` entry whose on-disk root *equals*
/// `home_tail`. Returns `Ok(None)` when no entry matches; errors with
/// [`ImportError::AmbiguousConfigMatch`] when two distinct entries claim
/// the same root (a corrupt-config bug the user must resolve first).
fn find_managed_root(
    doc: &DocumentMut,
    home_tail: &Path,
) -> Result<Option<RootMatch>, ImportError> {
    let pkg_table = match doc.get("pkg").and_then(Item::as_table) {
        Some(t) => t,
        None => return Ok(None),
    };

    let tail_str = path_to_forward_slash(home_tail);
    let mut matches: Vec<RootMatch> = Vec::new();

    for (pkg_key_raw, pkg_item) in pkg_table.iter() {
        let configs = match pkg_item
            .as_table()
            .and_then(|t| t.get("configs"))
            .and_then(Item::as_array_of_tables)
        {
            Some(c) => c,
            None => continue,
        };

        for (idx, entry) in configs.iter().enumerate() {
            let typ = match entry.get("type").and_then(|v| v.as_str()) {
                Some(t) => t,
                None => continue,
            };

            let (on_disk_root, import_type) = match typ {
                ".config" => {
                    let dir = entry
                        .get("name")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .unwrap_or(pkg_key_raw);
                    (format!(".config/{dir}"), ImportType::DotConfig)
                }
                "home" => {
                    let dir = match entry.get("dir").and_then(|v| v.as_str()) {
                        Some(d) if !d.is_empty() => d,
                        _ => continue,
                    };
                    (dir.to_string(), ImportType::Home)
                }
                _ => continue,
            };

            if on_disk_root != tail_str {
                continue;
            }

            let source_rel = match entry.get("source").and_then(|v| v.as_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };

            let existing_symlinks = entry
                .get("symlinks")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();

            matches.push(RootMatch {
                pkg_key: SmolStr::new(pkg_key_raw),
                config_index: idx,
                r#type: import_type,
                on_disk_root,
                source_rel,
                existing_symlinks,
            });
        }
    }

    if matches.len() > 1 {
        return Err(ImportError::AmbiguousConfigMatch {
            path: PathBuf::from(tail_str),
            candidates: matches.into_iter().map(|m| m.pkg_key).collect(),
        });
    }
    Ok(matches.into_iter().next())
}

/// Build a [`Plan`] that extends an existing managed config by adding a
/// single file. The file is staged into the matched entry's repo dir and
/// its relative path is queued for append onto the entry's `symlinks`
/// array; both halves run through the same apply / TOML helpers as the
/// new-pkg path.
fn build_extend_plan(
    matched: ConfigMatch,
    canonical_source: PathBuf,
    canonical_home: &Path,
    dirs: &ConfigFileDirs,
) -> Result<Plan, Error> {
    let source_root = canonical_home.join(&matched.on_disk_root);
    let repo_dest = dirs.zenops().join(&matched.source_rel);
    let file_rel_str = path_to_forward_slash(&matched.file_rel);

    let symlink_path = source_root.join(&matched.file_rel);
    let dest_in_repo = repo_dest.join(&matched.file_rel);

    let mut files = Vec::new();
    if dest_in_repo.try_exists().unwrap_or(false) {
        let already_symlinked = symlink_path
            .symlink_metadata()
            .is_ok_and(|m| m.is_symlink())
            && fs::read_link(&symlink_path).is_ok_and(|t| t == dest_in_repo);
        if !already_symlinked {
            return Err(ImportError::DestExists(dest_in_repo).into());
        }
    } else {
        files.push(PlannedFile {
            rel: matched.file_rel.clone(),
            repo_rel: matched.file_rel.clone(),
        });
    }

    let added_symlinks = if matched.existing_symlinks.iter().any(|s| s == &file_rel_str) {
        Vec::new()
    } else {
        vec![file_rel_str]
    };

    let toml = TomlPlan::ExtendExisting {
        config_index: matched.config_index,
        existing_symlinks: matched.existing_symlinks,
        added_symlinks,
    };

    Ok(Plan {
        pkg_key: matched.pkg_key,
        r#type: matched.r#type,
        source: canonical_source,
        source_root,
        repo_dest,
        repo_rel: matched.source_rel,
        files,
        skipped: Vec::new(),
        removed_files: Vec::new(),
        renamed_files: Vec::new(),
        toml,
    })
}

/// Build a [`Plan`] that reconciles an existing `[[pkg.<key>.configs]]`
/// entry against the current state of its home-side directory. Walks
/// the directory and diffs each entry against the array's `symlinks`:
/// new files are queued for add, paths whose home-side counterpart is
/// missing entirely are queued for remove. Already-managed files (proper
/// symlinks at the right path) are no-ops.
fn build_reconcile_plan(
    matched: RootMatch,
    canonical_source: PathBuf,
    canonical_home: &Path,
    dirs: &ConfigFileDirs,
) -> Result<Plan, Error> {
    if !canonical_source.is_dir() {
        return Err(ImportError::UnsupportedLayout(matched.on_disk_root).into());
    }
    let source_root = canonical_home.join(&matched.on_disk_root);
    let repo_dest = dirs.zenops().join(&matched.source_rel);

    let mut walked = Vec::new();
    let mut skipped = Vec::new();
    walk_dir(&source_root, &mut walked, &mut skipped, Path::new(""))?;

    let mut files: Vec<PlannedFile> = Vec::new();
    let mut added_symlinks: Vec<String> = Vec::new();

    for entry in walked {
        let rel_str = path_to_forward_slash(&entry.rel);
        let symlink_path = source_root.join(&entry.rel);
        let dest_in_repo = repo_dest.join(&entry.repo_rel);
        let already_in_array = matched.existing_symlinks.iter().any(|s| s == &rel_str);

        let already_symlinked = symlink_path
            .symlink_metadata()
            .is_ok_and(|m| m.is_symlink())
            && fs::read_link(&symlink_path).is_ok_and(|t| t == dest_in_repo);

        if already_symlinked {
            // Existing zenops-managed symlink. If the array already lists
            // it, fully in sync. If it doesn't, the array drifted — record
            // the rel without re-copying (repo file is already in place).
            if !already_in_array {
                added_symlinks.push(rel_str);
            }
            continue;
        }

        if already_in_array {
            // Rel is in the array but home-side is a regular file (not the
            // expected symlink). Don't re-copy or unlink — `zenops apply`
            // is the right tool. Surface the situation as a skip.
            skipped.push(SkippedEntry {
                path: entry.rel,
                reason: SmolStr::new_static("present_but_not_linked"),
            });
            continue;
        }

        if dest_in_repo.try_exists().unwrap_or(false) {
            // Repo destination is occupied by something we don't recognize
            // (no matching symlink, not in the array). Refuse to clobber.
            return Err(ImportError::DestExists(dest_in_repo).into());
        }

        files.push(PlannedFile {
            rel: entry.rel.clone(),
            repo_rel: entry.repo_rel,
        });
        added_symlinks.push(rel_str);
    }

    // Walk skips every existing symlink under the source root. Three
    // sub-classifications fall out of inspecting the target:
    //   1. Target == repo_dest/<walk_rel>: managed and in sync — drop.
    //   2. Target == repo_dest/<R0> where R0 ∈ existing_symlinks AND
    //      walk_rel ∉ existing_symlinks: the user renamed the symlink
    //      in place. Queue a rename (R0 → walk_rel).
    //   3. Otherwise: surface as `symlink_elsewhere`.
    let mut classified_skipped: Vec<SkippedEntry> = Vec::with_capacity(skipped.len());
    let mut renamed_files: Vec<RenamedFile> = Vec::new();
    let mut rename_from_set: Vec<String> = Vec::new();
    for s in skipped {
        if s.reason != "symlink" {
            classified_skipped.push(s);
            continue;
        }
        let abs = source_root.join(&s.path);
        let target = fs::read_link(&abs).ok();
        let in_repo_at_walk = repo_dest.join(&s.path);
        if target.as_deref() == Some(in_repo_at_walk.as_path()) {
            continue;
        }

        let new_rel_str = path_to_forward_slash(&s.path);
        let new_in_array = matched.existing_symlinks.iter().any(|e| e == &new_rel_str);
        let detected_rename = if new_in_array {
            None
        } else if let Some(t) = target.as_deref()
            && let Ok(target_rel) = t.strip_prefix(&repo_dest)
        {
            let from_rel_str = path_to_forward_slash(target_rel);
            if from_rel_str != new_rel_str
                && matched.existing_symlinks.iter().any(|e| e == &from_rel_str)
                && !rename_from_set.iter().any(|s| s == &from_rel_str)
            {
                Some((PathBuf::from(target_rel), from_rel_str))
            } else {
                None
            }
        } else {
            None
        };

        match detected_rename {
            Some((from_path, from_rel_str)) => {
                rename_from_set.push(from_rel_str);
                renamed_files.push(RenamedFile {
                    from: from_path,
                    to: s.path,
                });
            }
            None => classified_skipped.push(SkippedEntry {
                path: s.path,
                reason: SmolStr::new_static("symlink_elsewhere"),
            }),
        }
    }

    // Compute removals: paths in `existing_symlinks` whose home-side
    // counterpart is gone (neither a symlink nor a regular file). Skip
    // rels that participated in a rename — those are handled by the
    // rename pass instead of an outright delete.
    let mut removed_symlinks: Vec<String> = Vec::new();
    let mut removed_files: Vec<RemovedFile> = Vec::new();
    for rel in &matched.existing_symlinks {
        if rename_from_set.iter().any(|f| f == rel) {
            continue;
        }
        let home_path = source_root.join(rel);
        let exists = home_path.symlink_metadata().is_ok();
        if !exists {
            removed_symlinks.push(rel.clone());
            removed_files.push(RemovedFile {
                repo_rel: PathBuf::from(rel),
            });
        }
    }

    // Fold renames into the array delta. The post-edit array reads as
    // `existing − removed_symlinks − rename_from + added_symlinks +
    // rename_to`, which the merged Trim/Append events render in their
    // shared after-preview.
    for r in &renamed_files {
        added_symlinks.push(path_to_forward_slash(&r.to));
        removed_symlinks.push(path_to_forward_slash(&r.from));
    }

    let toml = TomlPlan::Reconcile {
        config_index: matched.config_index,
        existing_symlinks: matched.existing_symlinks,
        added_symlinks,
        removed_symlinks,
    };

    Ok(Plan {
        pkg_key: matched.pkg_key,
        r#type: matched.r#type,
        source: canonical_source,
        source_root,
        repo_dest,
        repo_rel: matched.source_rel,
        files,
        skipped: classified_skipped,
        removed_files,
        renamed_files,
        toml,
    })
}

/// Resolve the install_hint brew package list. If the user passed `--brew`,
/// use those. If they passed `--no-install-hint`, return an empty list. For
/// a new pkg with neither, prompt with the pkg key as the default; in
/// non-interactive mode, error out.
fn resolve_brew(
    pkg_key: &SmolStr,
    brew: &[String],
    no_install_hint: bool,
    created_pkg: bool,
    prompter: &mut Option<&mut dyn LinePrompter>,
) -> Result<Vec<String>, Error> {
    if !brew.is_empty() {
        return Ok(brew.to_vec());
    }
    if no_install_hint || !created_pkg {
        return Ok(Vec::new());
    }
    match prompter.as_deref_mut() {
        Some(p) => {
            match read_trimmed(p, &format!("Brew package(s) for `{pkg_key}` [{pkg_key}]: "))? {
                Some(line) => Ok(line.split_whitespace().map(str::to_string).collect()),
                None => Ok(vec![pkg_key.to_string()]),
            }
        }
        None => Err(ImportError::MissingInstallHint(pkg_key.to_string()).into()),
    }
}

fn read_trimmed(p: &mut dyn LinePrompter, prompt: &str) -> Result<Option<String>, Error> {
    match p
        .read_line(prompt)
        .map_err(crate::prompt::PromptError::Read)?
    {
        LineOutcome::Eof => Ok(None),
        LineOutcome::Interrupted => Err(crate::prompt::PromptError::Interrupted.into()),
        LineOutcome::Line(line) => {
            let trimmed = line.trim().to_string();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed))
            }
        }
    }
}

fn confirm(p: &mut dyn LinePrompter, prompt: &str) -> Result<bool, Error> {
    loop {
        let answer = read_trimmed(p, &format!("{prompt} [y/N]: "))?;
        match answer.as_deref().map(str::to_ascii_lowercase).as_deref() {
            None | Some("n" | "no") => return Ok(false),
            Some("y" | "yes") => return Ok(true),
            _ => p
                .writeln("Please answer y or n.")
                .map_err(crate::prompt::PromptError::Read)?,
        }
    }
}

/// Filesystem half of the apply phase: copy each source file into the
/// repo, then walk the same list a second time to remove the originals
/// and replace them with symlinks. Two passes so a copy failure leaves
/// the originals untouched. A third pass (reconcile mode only) deletes
/// repo-side copies whose home-side counterpart is already gone.
///
/// Pass 3 is not rolled back on partial failure: the TOML write happens
/// after this function, so a partial-Pass-3 / no-TOML-write state is
/// still consistent with the on-disk array (entries we couldn't delete
/// stay listed) and is recoverable by re-running `zenops import` on the
/// same root.
fn apply_files(plan: &Plan, dirs: &ConfigFileDirs, output: &mut dyn Output) -> Result<(), Error> {
    let mut undo = UndoLog::default();

    if !plan.repo_dest.try_exists().unwrap_or(false) {
        fs::create_dir_all(&plan.repo_dest)
            .map_err(|e| ImportError::Io(plan.repo_dest.clone(), e))?;
        undo.dirs_created.push(plan.repo_dest.clone());
        push_dir_event(output, dirs, &plan.repo_rel)?;
    }

    // Pass 1: copy source -> repo (creating intermediate dirs).
    for f in &plan.files {
        let src = plan.source_root.join(&f.rel);
        let dst = plan.repo_dest.join(&f.repo_rel);
        if let Some(parent) = dst.parent()
            && !parent.try_exists().unwrap_or(false)
        {
            if let Err(e) = fs::create_dir_all(parent) {
                rollback(&undo);
                return Err(ImportError::Io(parent.to_path_buf(), e).into());
            }
            undo.dirs_created.push(parent.to_path_buf());
            // Nested repo subdirs aren't surfaced via AppliedAction — the
            // per-file CreatedFile event covers user-visible progress.
        }
        if let Err(e) = fs::copy(&src, &dst) {
            rollback(&undo);
            return Err(ImportError::Copy {
                src,
                dst,
                source: e,
            }
            .into());
        }
        undo.files_copied.push(dst.clone());
        push_file_event(output, dirs, &plan.repo_rel, &f.repo_rel)?;
    }

    // Pass 2: remove original, create symlink.
    for f in &plan.files {
        let symlink_path = plan.source_root.join(&f.rel);
        let target = plan.repo_dest.join(&f.repo_rel);
        if let Err(e) = fs::remove_file(&symlink_path) {
            rollback(&undo);
            return Err(ImportError::RemoveOriginal(symlink_path, e).into());
        }
        if let Err(e) = std::os::unix::fs::symlink(&target, &symlink_path) {
            rollback(&undo);
            return Err(ImportError::Symlink {
                real: target,
                symlink: symlink_path,
                source: e,
            }
            .into());
        }
        undo.symlinks_created
            .push((symlink_path.clone(), target.clone()));
        push_symlink_event(
            output,
            dirs,
            &plan.repo_rel,
            &f.repo_rel,
            &plan.source_root,
            &f.rel,
        )?;
    }

    // Pass 3 (reconcile only): rename repo-side copies whose home-side
    // symlink the user moved in place. Each rename moves the repo file
    // from the old rel to the new one, then re-creates the home symlink
    // with the right target so it no longer dangles.
    for r in &plan.renamed_files {
        let from_abs = plan.repo_dest.join(&r.from);
        let to_abs = plan.repo_dest.join(&r.to);
        if let Some(parent) = to_abs.parent()
            && !parent.try_exists().unwrap_or(false)
        {
            fs::create_dir_all(parent).map_err(|e| ImportError::Io(parent.to_path_buf(), e))?;
        }
        fs::rename(&from_abs, &to_abs).map_err(|e| ImportError::Io(from_abs.clone(), e))?;
        push_renamed_file_event(output, dirs, &plan.repo_rel, &r.from, &r.to)?;

        let home_link = plan.source_root.join(&r.to);
        if let Err(e) = fs::remove_file(&home_link) {
            return Err(ImportError::RemoveOriginal(home_link, e).into());
        }
        if let Err(e) = std::os::unix::fs::symlink(&to_abs, &home_link) {
            return Err(ImportError::Symlink {
                real: to_abs.clone(),
                symlink: home_link,
                source: e,
            }
            .into());
        }
        push_symlink_event(
            output,
            dirs,
            &plan.repo_rel,
            &r.to,
            &plan.source_root,
            &r.to,
        )?;

        // Trim the now-empty parent dirs of the old repo path.
        let mut parent = from_abs.parent().map(Path::to_path_buf);
        while let Some(p) = parent {
            if p == plan.repo_dest || !p.starts_with(&plan.repo_dest) {
                break;
            }
            match fs::read_dir(&p) {
                Ok(mut iter) => {
                    if iter.next().is_some() {
                        break;
                    }
                }
                Err(_) => break,
            }
            if fs::remove_dir(&p).is_err() {
                break;
            }
            push_removed_dir_event_abs(output, dirs, &p)?;
            parent = p.parent().map(Path::to_path_buf);
        }
    }

    // Pass 4 (reconcile only): delete repo-side copies the user dropped
    // from the home-side directory, then trim now-empty parent dirs back
    // up to (but not including) `repo_dest`.
    for r in &plan.removed_files {
        let target = plan.repo_dest.join(&r.repo_rel);
        if target.try_exists().unwrap_or(false) {
            fs::remove_file(&target).map_err(|e| ImportError::Io(target.clone(), e))?;
        }
        push_removed_file_event(output, dirs, &plan.repo_rel, &r.repo_rel)?;

        let mut parent = target.parent().map(Path::to_path_buf);
        while let Some(p) = parent {
            if p == plan.repo_dest || !p.starts_with(&plan.repo_dest) {
                break;
            }
            match fs::read_dir(&p) {
                Ok(mut iter) => {
                    if iter.next().is_some() {
                        break;
                    }
                }
                Err(_) => break,
            }
            if fs::remove_dir(&p).is_err() {
                break;
            }
            push_removed_dir_event_abs(output, dirs, &p)?;
            parent = p.parent().map(Path::to_path_buf);
        }
    }

    Ok(())
}

#[derive(Default)]
struct UndoLog {
    dirs_created: Vec<PathBuf>,
    files_copied: Vec<PathBuf>,
    symlinks_created: Vec<(PathBuf, PathBuf)>,
}

/// Best-effort cleanup. Walks each step backward; rollback failures are
/// not surfaced — the primary error has already fired, and any surviving
/// partial state is visible in the user's working tree.
fn rollback(undo: &UndoLog) {
    for (symlink, target) in undo.symlinks_created.iter().rev() {
        let _ = fs::remove_file(symlink);
        if target.try_exists().unwrap_or(false) {
            let _ = fs::copy(target, symlink);
        }
    }
    for f in undo.files_copied.iter().rev() {
        let _ = fs::remove_file(f);
    }
    for d in undo.dirs_created.iter().rev() {
        let _ = fs::remove_dir(d);
    }
}

fn push_dir_event(
    output: &mut dyn Output,
    dirs: &ConfigFileDirs,
    repo_rel: &str,
) -> Result<(), Error> {
    let path = ConfigFilePath::Zenops(
        zenops_safe_relative_path::SafeRelativePath::from_relative_path(repo_rel)?.into(),
    );
    let resolved = crate::output::ResolvedConfigFilePath {
        full: std::sync::Arc::from(path.resolved(dirs)),
        path,
    };
    output
        .push(Event::AppliedAction(AppliedAction::CreatedDir(resolved)))
        .map_err(Into::into)
}

fn push_file_event(
    output: &mut dyn Output,
    dirs: &ConfigFileDirs,
    repo_rel: &str,
    file_rel: &Path,
) -> Result<(), Error> {
    let joined = format!("{repo_rel}/{}", path_to_forward_slash(file_rel));
    let path = ConfigFilePath::Zenops(
        zenops_safe_relative_path::SafeRelativePath::from_relative_path(&joined)?.into(),
    );
    let resolved = crate::output::ResolvedConfigFilePath {
        full: std::sync::Arc::from(path.resolved(dirs)),
        path,
    };
    output
        .push(Event::AppliedAction(AppliedAction::CreatedFile(resolved)))
        .map_err(Into::into)
}

fn push_symlink_event(
    output: &mut dyn Output,
    dirs: &ConfigFileDirs,
    repo_rel: &str,
    file_rel: &Path,
    home_root: &Path,
    home_file_rel: &Path,
) -> Result<(), Error> {
    let real_joined = format!("{repo_rel}/{}", path_to_forward_slash(file_rel));
    let real_path = ConfigFilePath::Zenops(
        zenops_safe_relative_path::SafeRelativePath::from_relative_path(&real_joined)?.into(),
    );
    let real = crate::output::ResolvedConfigFilePath {
        full: std::sync::Arc::from(real_path.resolved(dirs)),
        path: real_path,
    };

    // Symlink path: home_root.join(home_file_rel) is absolute; we need it
    // expressed as ConfigFilePath::Home with a $HOME-relative tail.
    let symlink_full = home_root.join(home_file_rel);
    let home_rel = symlink_full
        .strip_prefix(dirs.home())
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| symlink_full.clone());
    let home_rel_str = path_to_forward_slash(&home_rel);
    let symlink_path = ConfigFilePath::Home(
        zenops_safe_relative_path::SafeRelativePath::from_relative_path(&home_rel_str)?.into(),
    );
    let symlink = crate::output::ResolvedConfigFilePath {
        full: std::sync::Arc::from(symlink_full.as_path()),
        path: symlink_path,
    };

    output
        .push(Event::AppliedAction(AppliedAction::CreatedSymlink {
            real,
            symlink,
        }))
        .map_err(Into::into)
}

fn push_removed_file_event(
    output: &mut dyn Output,
    dirs: &ConfigFileDirs,
    repo_rel: &str,
    file_rel: &Path,
) -> Result<(), Error> {
    let joined = format!("{repo_rel}/{}", path_to_forward_slash(file_rel));
    let path = ConfigFilePath::Zenops(
        zenops_safe_relative_path::SafeRelativePath::from_relative_path(&joined)?.into(),
    );
    let resolved = crate::output::ResolvedConfigFilePath {
        full: std::sync::Arc::from(path.resolved(dirs)),
        path,
    };
    output
        .push(Event::AppliedAction(AppliedAction::RemovedFile(resolved)))
        .map_err(Into::into)
}

fn push_removed_dir_event_abs(
    output: &mut dyn Output,
    dirs: &ConfigFileDirs,
    abs: &Path,
) -> Result<(), Error> {
    let zenops_root = dirs.zenops();
    let rel = match abs.strip_prefix(zenops_root) {
        Ok(r) => path_to_forward_slash(r),
        Err(_) => return Ok(()),
    };
    if rel.is_empty() {
        return Ok(());
    }
    let path = ConfigFilePath::Zenops(
        zenops_safe_relative_path::SafeRelativePath::from_relative_path(&rel)?.into(),
    );
    let resolved = crate::output::ResolvedConfigFilePath {
        full: std::sync::Arc::from(path.resolved(dirs)),
        path,
    };
    output
        .push(Event::AppliedAction(AppliedAction::RemovedDir(resolved)))
        .map_err(Into::into)
}

fn push_renamed_file_event(
    output: &mut dyn Output,
    dirs: &ConfigFileDirs,
    repo_rel: &str,
    from_rel: &Path,
    to_rel: &Path,
) -> Result<(), Error> {
    let from_joined = format!("{repo_rel}/{}", path_to_forward_slash(from_rel));
    let to_joined = format!("{repo_rel}/{}", path_to_forward_slash(to_rel));
    let from_path = ConfigFilePath::Zenops(
        zenops_safe_relative_path::SafeRelativePath::from_relative_path(&from_joined)?.into(),
    );
    let to_path = ConfigFilePath::Zenops(
        zenops_safe_relative_path::SafeRelativePath::from_relative_path(&to_joined)?.into(),
    );
    let from = crate::output::ResolvedConfigFilePath {
        full: std::sync::Arc::from(from_path.resolved(dirs)),
        path: from_path,
    };
    let to = crate::output::ResolvedConfigFilePath {
        full: std::sync::Arc::from(to_path.resolved(dirs)),
        path: to_path,
    };
    output
        .push(Event::AppliedAction(AppliedAction::RenamedFile {
            from,
            to,
        }))
        .map_err(Into::into)
}

fn path_to_forward_slash(p: &Path) -> String {
    let mut out = String::new();
    for (i, c) in p.components().enumerate() {
        if i > 0 {
            out.push('/');
        }
        out.push_str(&c.as_os_str().to_string_lossy());
    }
    out
}

/// Splice the new `[[pkg.<key>.configs]]` entry (and the surrounding
/// `[pkg.<key>]` block, when this is a brand-new pkg) into the document.
/// In extend mode, append the new paths to the matched entry's `symlinks`
/// array. In reconcile mode, both append new paths and trim removed
/// ones from the matched entry's array.
fn update_doc(
    doc: &mut DocumentMut,
    plan: &Plan,
    created_pkg: bool,
    no_install_hint: bool,
    brew: &[String],
) -> Result<(), Error> {
    if let TomlPlan::ExtendExisting {
        config_index,
        added_symlinks,
        ..
    } = &plan.toml
    {
        append_symlinks_to_configs_entry(doc, &plan.pkg_key, *config_index, added_symlinks);
        return Ok(());
    }

    if let TomlPlan::Reconcile {
        config_index,
        added_symlinks,
        removed_symlinks,
        ..
    } = &plan.toml
    {
        append_symlinks_to_configs_entry(doc, &plan.pkg_key, *config_index, added_symlinks);
        remove_symlinks_from_configs_entry(doc, &plan.pkg_key, *config_index, removed_symlinks);
        return Ok(());
    }

    let pkg_root = doc
        .entry("pkg")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .expect("pkg should be a table");
    pkg_root.set_implicit(true);

    let key_table = pkg_root
        .entry(&plan.pkg_key)
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .expect("pkg.<key> should be a table");

    if created_pkg {
        populate_install_hint(key_table, brew, no_install_hint);
    }

    let configs_item = key_table
        .entry("configs")
        .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()));
    let configs = configs_item
        .as_array_of_tables_mut()
        .expect("configs should be an array of tables");

    configs.push(build_configs_entry_table(plan));

    Ok(())
}

/// Push `new_paths` onto the `symlinks` array of `pkg.<pkg_key>.configs[config_index]`.
/// No-op when `new_paths` is empty (the rel was already listed). Creates
/// the array if the entry didn't have one. Caller is expected to have
/// validated `(pkg_key, config_index)` against the same document.
fn append_symlinks_to_configs_entry(
    doc: &mut DocumentMut,
    pkg_key: &str,
    config_index: usize,
    new_paths: &[String],
) {
    if new_paths.is_empty() {
        return;
    }
    let entry = doc
        .get_mut("pkg")
        .and_then(|p| p.as_table_mut())
        .and_then(|t| t.get_mut(pkg_key))
        .and_then(|p| p.as_table_mut())
        .and_then(|t| t.get_mut("configs"))
        .and_then(Item::as_array_of_tables_mut)
        .and_then(|aot| aot.get_mut(config_index))
        .expect("matched configs entry should still exist");

    if entry.get("symlinks").is_none() {
        entry["symlinks"] = value(Array::new());
    }
    let arr = entry["symlinks"]
        .as_array_mut()
        .expect("symlinks should be an array");
    for path in new_paths {
        arr.push(path.as_str());
    }
}

/// Drop `paths` from the `symlinks` array of `pkg.<pkg_key>.configs[config_index]`.
/// No-op when `paths` is empty. Caller is expected to have validated
/// `(pkg_key, config_index)` against the same document.
fn remove_symlinks_from_configs_entry(
    doc: &mut DocumentMut,
    pkg_key: &str,
    config_index: usize,
    paths: &[String],
) {
    if paths.is_empty() {
        return;
    }
    let entry = doc
        .get_mut("pkg")
        .and_then(|p| p.as_table_mut())
        .and_then(|t| t.get_mut(pkg_key))
        .and_then(|p| p.as_table_mut())
        .and_then(|t| t.get_mut("configs"))
        .and_then(Item::as_array_of_tables_mut)
        .and_then(|aot| aot.get_mut(config_index))
        .expect("matched configs entry should still exist");

    let Some(arr) = entry.get_mut("symlinks").and_then(|v| v.as_array_mut()) else {
        return;
    };
    let mut idx = 0;
    while idx < arr.len() {
        let drop = arr
            .get(idx)
            .and_then(|v| v.as_str())
            .is_some_and(|s| paths.iter().any(|p| p == s));
        if drop {
            arr.remove(idx);
        } else {
            idx += 1;
        }
    }
}

/// Populate `key_table` with the `install_hint` sub-table for a freshly-
/// created `[pkg.<key>]` block. Always writes a `packages` array so the
/// deserializer's required-field invariant holds; the array is empty under
/// `--no-install-hint`. Pure helper shared by [`update_doc`] and the
/// TOML-preview path so the displayed plan matches the eventual write.
fn populate_install_hint(key_table: &mut Table, brew: &[String], no_install_hint: bool) {
    let install_hint = key_table
        .entry("install_hint")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .expect("install_hint should be a table");
    let brew_t = install_hint
        .entry("brew")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .expect("brew should be a table");
    let mut arr = Array::new();
    if !no_install_hint {
        for pkg in brew {
            arr.push(pkg.as_str());
        }
    }
    brew_t["packages"] = value(arr);
}

/// Build the `[[pkg.<key>.configs]]` entry table. Pure helper used by
/// both [`update_doc`] (writes into the live document) and the
/// TOML-preview path (renders the entry body for the plan event).
/// Only called for new-pkg / new-entry imports — extend mode appends
/// onto an existing entry via [`append_symlinks_to_configs_entry`] and
/// never reaches this builder.
fn build_configs_entry_table(plan: &Plan) -> Table {
    let mut entry = Table::new();
    match &plan.toml {
        TomlPlan::DotConfig {
            name_override,
            symlinks,
        } => {
            entry["type"] = value(".config");
            if let Some(name) = name_override {
                entry["name"] = value(name.as_str());
            }
            entry["source"] = value(plan.repo_rel.as_str());
            let mut arr = Array::new();
            for s in symlinks {
                arr.push(s.as_str());
            }
            entry["symlinks"] = value(arr);
        }
        TomlPlan::Home { dir, symlinks } => {
            entry["type"] = value("home");
            entry["dir"] = value(dir.as_str());
            entry["source"] = value(plan.repo_rel.as_str());
            let mut arr = Array::new();
            for s in symlinks {
                arr.push(s.as_str());
            }
            entry["symlinks"] = value(arr);
        }
        TomlPlan::ExtendExisting { .. } | TomlPlan::Reconcile { .. } => {
            unreachable!(
                "extend / reconcile modes emit AppendSymlinks / TrimSymlinks, not a new configs entry"
            )
        }
    }
    entry
}

/// Translate the internal [`Plan`] into the wire-level [`ImportPlan`]
/// event broadcast through [`Output`]. The two enums on
/// [`ImportPlan`]'s body — [`ImportFileAction`] and
/// [`ImportTomlChange`] — are extension points: as `import` grows new
/// shapes (per-file include/exclude, reconcile-against-existing, …)
/// new variants land here and in the renderer without touching the
/// surrounding flow.
fn plan_to_event(
    plan: &Plan,
    created_pkg: bool,
    no_install_hint: bool,
    brew_packages: &[String],
) -> ImportPlan {
    let mut file_actions = Vec::with_capacity(
        plan.files.len() + plan.skipped.len() + plan.removed_files.len() + plan.renamed_files.len(),
    );
    for f in &plan.files {
        file_actions.push(ImportFileAction::MoveAndSymlink { rel: f.rel.clone() });
    }
    for r in &plan.renamed_files {
        file_actions.push(ImportFileAction::RenameInRepo {
            from: r.from.clone(),
            to: r.to.clone(),
        });
    }
    for r in &plan.removed_files {
        file_actions.push(ImportFileAction::RemoveFromRepo {
            rel: r.repo_rel.clone(),
        });
    }
    for s in &plan.skipped {
        file_actions.push(ImportFileAction::Skip {
            path: s.path.clone(),
            reason: s.reason.clone(),
        });
    }

    let mut toml_changes = Vec::new();
    match &plan.toml {
        TomlPlan::ExtendExisting {
            config_index,
            existing_symlinks,
            added_symlinks,
        } => {
            toml_changes.push(ImportTomlChange::AppendSymlinks {
                pkg: plan.pkg_key.clone(),
                config_index: *config_index,
                paths: added_symlinks.clone(),
                array_after_preview: render_symlinks_array_preview(
                    existing_symlinks,
                    added_symlinks,
                    &[],
                ),
            });
        }
        TomlPlan::Reconcile {
            config_index,
            existing_symlinks,
            added_symlinks,
            removed_symlinks,
        } => {
            let after_preview =
                render_symlinks_array_preview(existing_symlinks, added_symlinks, removed_symlinks);
            if !added_symlinks.is_empty() {
                toml_changes.push(ImportTomlChange::AppendSymlinks {
                    pkg: plan.pkg_key.clone(),
                    config_index: *config_index,
                    paths: added_symlinks.clone(),
                    array_after_preview: after_preview.clone(),
                });
            }
            if !removed_symlinks.is_empty() {
                toml_changes.push(ImportTomlChange::TrimSymlinks {
                    pkg: plan.pkg_key.clone(),
                    config_index: *config_index,
                    paths: removed_symlinks.clone(),
                    array_after_preview: after_preview,
                });
            }
        }
        TomlPlan::DotConfig { .. } | TomlPlan::Home { .. } => {
            if created_pkg {
                toml_changes.push(ImportTomlChange::CreatePkg {
                    pkg: plan.pkg_key.clone(),
                    brew_packages: brew_packages.to_vec(),
                    block_preview: render_pkg_block_preview(
                        &plan.pkg_key,
                        brew_packages,
                        no_install_hint,
                    ),
                });
            }
            toml_changes.push(ImportTomlChange::AppendConfigsEntry {
                pkg: plan.pkg_key.clone(),
                entry_preview: render_configs_entry_preview(plan),
            });
        }
    }

    let mode = match &plan.toml {
        TomlPlan::Reconcile { .. } => ImportMode::Reconcile,
        TomlPlan::ExtendExisting { .. } => ImportMode::Extend,
        TomlPlan::DotConfig { .. } | TomlPlan::Home { .. } => {
            if created_pkg {
                ImportMode::NewPkg
            } else {
                ImportMode::Extend
            }
        }
    };

    ImportPlan {
        pkg: plan.pkg_key.clone(),
        created_pkg,
        mode,
        r#type: plan.r#type,
        source: plan.source.clone(),
        repo_dest: plan.repo_dest.clone(),
        file_actions,
        toml_changes,
    }
}

/// Render the `symlinks` array as it will read after appending `added`
/// and dropping `removed` from `existing` — used in the AppendSymlinks /
/// TrimSymlinks plan previews so the user sees the array in its
/// post-edit form. Pass an empty `removed` for non-reconcile callers.
fn render_symlinks_array_preview(
    existing: &[String],
    added: &[String],
    removed: &[String],
) -> String {
    let mut arr = Array::new();
    for s in existing {
        if removed.iter().any(|r| r == s) {
            continue;
        }
        arr.push(s.as_str());
    }
    for s in added {
        arr.push(s.as_str());
    }
    format!("symlinks = {arr}")
}

/// Copy-paste-ready snippet showing the new `[pkg.<key>]` block. The
/// preview uses the same `populate_install_hint` helper that
/// [`update_doc`] runs, so the displayed body is byte-equivalent to
/// what eventually lands. Intermediate `install_hint` /
/// `install_hint.brew` sub-tables are marked implicit so they collapse
/// into a single dotted-key line, which matches the spirit of how the
/// merged document reads even if the live `update_doc` write keeps the
/// sub-table headers (those exist anyway in the on-disk file).
fn render_pkg_block_preview(pkg_key: &str, brew: &[String], no_install_hint: bool) -> String {
    let mut doc = DocumentMut::new();
    let pkg_root = doc
        .entry("pkg")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .expect("pkg should be a table");
    pkg_root.set_implicit(true);
    let key_table = pkg_root
        .entry(pkg_key)
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .expect("pkg.<key> should be a table");
    populate_install_hint(key_table, brew, no_install_hint);
    if let Some(ih) = key_table
        .get_mut("install_hint")
        .and_then(Item::as_table_mut)
    {
        ih.set_implicit(true);
        if let Some(brew_t) = ih.get_mut("brew").and_then(Item::as_table_mut) {
            brew_t.set_implicit(true);
        }
    }
    doc.to_string().trim_end_matches('\n').to_string()
}

/// Body of the new `[[pkg.<key>.configs]]` entry, exactly as
/// [`update_doc`] would emit it. The header line itself is supplied by
/// the renderer so the snippet can be indented under a "configs entry"
/// heading without re-formatting.
fn render_configs_entry_preview(plan: &Plan) -> String {
    let entry = build_configs_entry_table(plan);
    entry.to_string().trim_end_matches('\n').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn classify_dot_config() {
        let layout = classify(Path::new(".config/helix")).unwrap();
        assert!(matches!(&layout, Layout::DotConfig { dir } if dir == "helix"));
    }

    #[test]
    fn classify_home_dotfile() {
        let layout = classify(Path::new(".zshrc")).unwrap();
        assert!(matches!(&layout, Layout::Home { name } if name == ".zshrc"));
    }

    #[test]
    fn classify_home_dotdir() {
        let layout = classify(Path::new(".ssh")).unwrap();
        assert!(matches!(&layout, Layout::Home { name } if name == ".ssh"));
    }

    #[test]
    fn classify_rejects_dot_config_subdir() {
        let err = classify(Path::new(".config/helix/themes")).unwrap_err();
        assert!(matches!(err, ImportError::UnsupportedLayout(_)));
    }

    #[test]
    fn classify_rejects_non_dot_home() {
        let err = classify(Path::new("dotfiles")).unwrap_err();
        assert!(matches!(err, ImportError::UnsupportedLayout(_)));
    }

    #[test]
    fn classify_rejects_nested_home_dotdir() {
        let err = classify(Path::new(".ssh/config")).unwrap_err();
        assert!(matches!(err, ImportError::UnsupportedLayout(_)));
    }

    #[test]
    fn derive_pkg_key_strips_leading_dot_for_home() {
        let layout = Layout::Home {
            name: ".zshrc".into(),
        };
        let key = derive_pkg_key(&layout, None).unwrap();
        assert_eq!(key.as_str(), "zshrc");
    }

    #[test]
    fn derive_pkg_key_uses_dir_for_dotconfig() {
        let layout = Layout::DotConfig {
            dir: "helix".into(),
        };
        let key = derive_pkg_key(&layout, None).unwrap();
        assert_eq!(key.as_str(), "helix");
    }

    #[test]
    fn derive_pkg_key_honors_override() {
        let layout = Layout::DotConfig { dir: "nvim".into() };
        let key = derive_pkg_key(&layout, Some("neovim")).unwrap();
        assert_eq!(key.as_str(), "neovim");
    }

    use crate::line_prompter::BufReadPrompter;

    #[test]
    fn resolve_brew_uses_explicit_flag() {
        let mut prompter: Option<&mut dyn LinePrompter> = None;
        let pkg = SmolStr::new_static("foo");
        let got =
            resolve_brew(&pkg, &["a".into(), "b".into()], false, true, &mut prompter).unwrap();
        assert_eq!(got, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn resolve_brew_no_install_hint_returns_empty() {
        let mut prompter: Option<&mut dyn LinePrompter> = None;
        let pkg = SmolStr::new_static("foo");
        let got = resolve_brew(&pkg, &[], true, true, &mut prompter).unwrap();
        assert!(got.is_empty());
    }

    #[test]
    fn resolve_brew_existing_pkg_returns_empty() {
        let mut prompter: Option<&mut dyn LinePrompter> = None;
        let pkg = SmolStr::new_static("foo");
        let got = resolve_brew(&pkg, &[], false, false, &mut prompter).unwrap();
        assert!(got.is_empty());
    }

    #[test]
    fn resolve_brew_new_pkg_no_prompter_errors() {
        let mut prompter: Option<&mut dyn LinePrompter> = None;
        let pkg = SmolStr::new_static("foo");
        let err = resolve_brew(&pkg, &[], false, true, &mut prompter).unwrap_err();
        match err {
            Error::Import(ImportError::MissingInstallHint(k)) => assert_eq!(k, "foo"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn resolve_brew_prompts_uses_default_on_blank() {
        let input = b"\n";
        let mut p = BufReadPrompter::new(&input[..], Vec::<u8>::new());
        let mut handle: Option<&mut dyn LinePrompter> = Some(&mut p);
        let pkg = SmolStr::new_static("foo");
        let got = resolve_brew(&pkg, &[], false, true, &mut handle).unwrap();
        assert_eq!(got, vec!["foo".to_string()]);
    }

    #[test]
    fn resolve_brew_prompts_splits_whitespace() {
        let input = b"a b  c\n";
        let mut p = BufReadPrompter::new(&input[..], Vec::<u8>::new());
        let mut handle: Option<&mut dyn LinePrompter> = Some(&mut p);
        let pkg = SmolStr::new_static("foo");
        let got = resolve_brew(&pkg, &[], false, true, &mut handle).unwrap();
        assert_eq!(got, vec!["a".to_string(), "b".to_string(), "c".to_string()],);
    }

    #[test]
    fn confirm_yes_then_no() {
        let mut p = BufReadPrompter::new(&b"y\n"[..], Vec::<u8>::new());
        assert!(confirm(&mut p, "Apply?").unwrap());
        let mut p = BufReadPrompter::new(&b"n\n"[..], Vec::<u8>::new());
        assert!(!confirm(&mut p, "Apply?").unwrap());
        let mut p = BufReadPrompter::new(&b"\n"[..], Vec::<u8>::new());
        assert!(!confirm(&mut p, "Apply?").unwrap());
    }
}
