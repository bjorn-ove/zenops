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
        AppliedAction, Event, ImportApplied, ImportFileAction, ImportPlan, ImportTomlChange,
        ImportType, Output,
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
    let plan = build_plan(path, pkg_override, source_override, dirs)?;

    let cfg_path = dirs.zenops().join("config.toml");
    let cfg_text = fs::read_to_string(&cfg_path)
        .map_err(|e| Error::from(ImportError::Io(cfg_path.clone(), e)))?;
    let mut doc: DocumentMut = cfg_text
        .parse()
        .map_err(|e| Error::from(ImportError::ConfigParse(cfg_path.clone(), e)))?;

    let created_pkg = !pkg_block_exists(&doc, &plan.pkg_key);
    let brew_packages = resolve_brew(
        &plan.pkg_key,
        brew,
        no_install_hint,
        created_pkg,
        &mut prompter,
    )?;

    output.push(Event::ImportPlan(plan_to_event(
        &plan,
        created_pkg,
        no_install_hint,
        &brew_packages,
    )))?;

    if dry_run {
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
}

/// Resolve and classify the source path; walk it; choose pkg key and
/// repo-side destination; flag any pre-existing destination files;
/// short-circuit any already-imported symlinks.
fn build_plan(
    raw_path: &Path,
    pkg_override: Option<&str>,
    source_override: Option<&str>,
    dirs: &ConfigFileDirs,
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

    let tail = canonical_source
        .strip_prefix(&canonical_home)
        .map_err(|_| ImportError::PathNotUnderHome(canonical_source.clone()))?;
    let layout = classify(tail)?;

    let is_dir = canonical_source.is_dir();
    let pkg_key = derive_pkg_key(&layout, pkg_override)?;

    let repo_rel = source_override
        .map(str::to_string)
        .unwrap_or_else(|| format!("configs/{pkg_key}"));
    let repo_dest = dirs.zenops().join(&repo_rel);

    let (source_root, toml_plan_kind) = match (&layout, is_dir) {
        (Layout::DotConfig { dir }, _) => {
            let root = canonical_home.join(".config").join(dir);
            let name_override = if dir == pkg_key.as_str() {
                None
            } else {
                Some(dir.clone())
            };
            (root, TomlPlanKind::DotConfig { name_override })
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
/// the originals untouched.
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
fn update_doc(
    doc: &mut DocumentMut,
    plan: &Plan,
    created_pkg: bool,
    no_install_hint: bool,
    brew: &[String],
) -> Result<(), Error> {
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
    let mut file_actions = Vec::with_capacity(plan.files.len() + plan.skipped.len());
    for f in &plan.files {
        file_actions.push(ImportFileAction::MoveAndSymlink { rel: f.rel.clone() });
    }
    for s in &plan.skipped {
        file_actions.push(ImportFileAction::Skip {
            path: s.path.clone(),
            reason: s.reason.clone(),
        });
    }

    let mut toml_changes = Vec::new();
    if created_pkg {
        toml_changes.push(ImportTomlChange::CreatePkg {
            pkg: plan.pkg_key.clone(),
            brew_packages: brew_packages.to_vec(),
            block_preview: render_pkg_block_preview(&plan.pkg_key, brew_packages, no_install_hint),
        });
    }
    toml_changes.push(ImportTomlChange::AppendConfigsEntry {
        pkg: plan.pkg_key.clone(),
        entry_preview: render_configs_entry_preview(plan),
    });

    ImportPlan {
        pkg: plan.pkg_key.clone(),
        created_pkg,
        r#type: plan.r#type,
        source: plan.source.clone(),
        repo_dest: plan.repo_dest.clone(),
        file_actions,
        toml_changes,
    }
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
