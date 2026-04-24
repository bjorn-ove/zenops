#!/usr/bin/env bash
# Helper subcommands used by the bump-version skill
# (.claude/skills/bump-version/SKILL.md).
#
# Each subcommand wraps a mechanical step — workspace discovery, last-tag
# resolution with fallback to the pre-split `v<version>` anchor, focused
# `git log`/`git diff` for a crate, and GitHub release-URL construction —
# so the skill prompt does not have to re-invent inline pipelines.
#
# Safe to run directly from a clean tree. All operations are read-only.

set -euo pipefail

cd "$(dirname "$0")/.."

if ! command -v jq >/dev/null 2>&1; then
    printf 'bump-helper: jq is required (install via `brew install jq`).\n' >&2
    exit 1
fi

usage() {
    cat >&2 <<'EOF'
Usage: scripts/bump-helper.sh <subcommand> [args]

Subcommands:
  list                         Print one compact JSON object per workspace
                               crate. Fields: name, version, manifest_path,
                               crate_dir, internal_deps.
  candidates                   Print per-crate status as TSV columns:
                               name, version, last_tag, tag_source,
                               commits_since.
                               tag_source is one of: own, anchor, none.
                               commits_since is "-" when tag_source is none.
  commits <crate>              Print `git log --oneline` for <crate> since
                               its last tag, using the same fallback and
                               path filter as `candidates`.
  diff <crate>                 Print the focused diff used for SemVer
                               classification (Rust sources + Cargo.toml in
                               the crate directory, since last tag).
  release-url <crate> <ver>    Print a GitHub releases/new URL pre-filled
                               with tag and title for <crate>-v<ver>.
                               Exits 1 if the origin is not a GitHub URL.
EOF
}

# --- crate metadata --------------------------------------------------------

list_crates() {
    cargo metadata --format-version=1 --no-deps | jq -c '
      . as $meta
      | ($meta.packages | map(.name)) as $ws
      | $meta.workspace_root as $root
      | $meta.packages[]
      | {
          name,
          version,
          manifest_path: (.manifest_path | ltrimstr($root + "/")),
          crate_dir: (
            .manifest_path
            | ltrimstr($root + "/")
            | sub("/?Cargo\\.toml$"; "")
            | if . == "" then "." else . end
          ),
          internal_deps: [.dependencies[].name | select(. as $d | $ws | index($d))]
        }
    '
}

crate_json() {
    local name="$1" info
    info=$(list_crates | jq -c --arg n "$name" 'select(.name == $n)')
    if [[ -z "$info" ]]; then
        printf 'bump-helper: unknown crate %q\n' "$name" >&2
        exit 1
    fi
    printf '%s' "$info"
}

# --- tag resolution --------------------------------------------------------

# Prints "<tag>\t<source>" where source is own | anchor | none.
resolve_tag() {
    local name="$1" version="$2"
    local own_tag="${name}-v${version}"
    if git rev-parse -q --verify "refs/tags/${own_tag}" >/dev/null; then
        printf '%s\town\n' "$own_tag"
        return 0
    fi
    local anchor="v${version}"
    if git rev-parse -q --verify "refs/tags/${anchor}" >/dev/null; then
        printf '%s\tanchor\n' "$anchor"
        return 0
    fi
    printf -- '-\tnone\n'
}

# --- git path filters ------------------------------------------------------
#
# The root crate (crate_dir == ".") lives at the repo root and shares it
# with the workspace manifest + Cargo.lock + subcrates. Its path filter is
# the whole repo minus crates/, so subcrate churn doesn't bleed in. Every
# other crate is a self-contained directory, plus the root Cargo.toml and
# Cargo.lock (workspace pins live there and affect every member).

# Populates the global `paths` array for use as `git log`/`git diff` --
# arguments. Caller must declare `local paths=()` first.
set_paths_for_crate() {
    local crate_dir="$1"
    if [[ "$crate_dir" == "." ]]; then
        paths=("." ":(exclude)crates/")
    else
        paths=("${crate_dir}/" "Cargo.toml" "Cargo.lock")
    fi
}

# --- subcommands -----------------------------------------------------------

cmd_list() {
    list_crates
}

cmd_candidates() {
    list_crates | while IFS= read -r row; do
        local name version crate_dir resolved tag tag_source count paths
        name=$(printf '%s' "$row" | jq -r .name)
        version=$(printf '%s' "$row" | jq -r .version)
        crate_dir=$(printf '%s' "$row" | jq -r .crate_dir)
        resolved=$(resolve_tag "$name" "$version")
        tag=$(printf '%s' "$resolved" | cut -f1)
        tag_source=$(printf '%s' "$resolved" | cut -f2)
        if [[ "$tag_source" == "none" ]]; then
            count="-"
        else
            paths=()
            set_paths_for_crate "$crate_dir"
            count=$(git log --oneline "${tag}..HEAD" -- "${paths[@]}" | wc -l | tr -d ' ')
        fi
        printf '%s\t%s\t%s\t%s\t%s\n' "$name" "$version" "$tag" "$tag_source" "$count"
    done
}

require_tag_for() {
    # Sets globals: info, version, crate_dir, tag.
    local name="$1"
    info=$(crate_json "$name")
    version=$(printf '%s' "$info" | jq -r .version)
    crate_dir=$(printf '%s' "$info" | jq -r .crate_dir)
    local resolved tag_source
    resolved=$(resolve_tag "$name" "$version")
    tag=$(printf '%s' "$resolved" | cut -f1)
    tag_source=$(printf '%s' "$resolved" | cut -f2)
    if [[ "$tag_source" == "none" ]]; then
        printf 'bump-helper: no release tag and no pre-split anchor for %s v%s\n' "$name" "$version" >&2
        exit 1
    fi
}

cmd_commits() {
    local name="${1:-}"
    [[ -n "$name" ]] || { printf 'bump-helper: `commits` requires <crate>\n' >&2; exit 1; }
    local info version crate_dir tag paths=()
    require_tag_for "$name"
    set_paths_for_crate "$crate_dir"
    git log --oneline "${tag}..HEAD" -- "${paths[@]}"
}

cmd_diff() {
    local name="${1:-}"
    [[ -n "$name" ]] || { printf 'bump-helper: `diff` requires <crate>\n' >&2; exit 1; }
    local info version crate_dir tag
    require_tag_for "$name"
    if [[ "$crate_dir" == "." ]]; then
        git diff "${tag}..HEAD" -- '*.rs' 'Cargo.toml' ':(exclude)crates/'
    else
        git diff "${tag}..HEAD" -- "${crate_dir}/*.rs" "${crate_dir}/Cargo.toml"
    fi
}

cmd_release_url() {
    local name="${1:-}" version="${2:-}"
    if [[ -z "$name" || -z "$version" ]]; then
        printf 'bump-helper: `release-url` requires <crate> <version>\n' >&2
        exit 1
    fi
    local url path
    url=$(git remote get-url origin)
    if [[ "$url" =~ ^git@github\.com:(.+)$ ]]; then
        path="${BASH_REMATCH[1]%.git}"
    elif [[ "$url" =~ ^https://github\.com/(.+)$ ]]; then
        path="${BASH_REMATCH[1]%.git}"
    else
        printf 'bump-helper: origin %q is not a GitHub URL; use the raw tag name %s-v%s instead\n' "$url" "$name" "$version" >&2
        exit 1
    fi
    # GitHub's releases/new form decodes `+` in &title= as a literal plus
    # sign, so encode spaces as %20. Cargo crate names don't contain chars
    # that need encoding in practice.
    printf 'https://github.com/%s/releases/new?tag=%s-v%s&title=%s%%20v%s\n' \
        "$path" "$name" "$version" "$name" "$version"
}

# --- dispatch --------------------------------------------------------------

cmd="${1:-}"
shift || true
case "$cmd" in
    list)           cmd_list "$@" ;;
    candidates)     cmd_candidates "$@" ;;
    commits)        cmd_commits "$@" ;;
    diff)           cmd_diff "$@" ;;
    release-url)    cmd_release_url "$@" ;;
    -h|--help|help) usage ;;
    "")             usage; exit 1 ;;
    *)              printf 'bump-helper: unknown subcommand %q\n' "$cmd" >&2; usage; exit 1 ;;
esac
