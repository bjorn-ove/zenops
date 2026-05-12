#!/usr/bin/env python3
"""Step 03: add a managed config to the zenops repo and apply it.

Appends a ``[pkg.demo]`` block to config.toml, writes the corresponding
source file under ``configs/demo/``, commits in the zenops repo so the apply
sees a clean tree, then runs ``zenops apply --yes``. Verifies the symlink
landed under ``~/.config/demo/`` and that the shell's generated files exist.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from common import (
    CONFIGS_DIR,
    CONFIG_TOML,
    EXPECTED_FILES,
    HOME,
    ZENOPS_DIR,
    assert_file_exists,
    assert_symlink,
    assert_true,
    log,
    parse_args,
    run,
    run_zenops,
)

PKG_BLOCK = """

[pkg.demo]
enable = "on"
[pkg.demo.install_hint.brew]
packages = []
[[pkg.demo.configs]]
type = ".config"
source = "configs/demo"
symlinks = ["data.txt"]
"""

DEMO_CONTENT = "hello from zenops cross-platform test\n"


def main() -> int:
    args = parse_args()

    log("appending [pkg.demo] block to config.toml")
    with CONFIG_TOML.open("a") as f:
        f.write(PKG_BLOCK)

    log("writing configs/demo/data.txt under zenops repo")
    repo_copy = CONFIGS_DIR / "demo" / "data.txt"
    repo_copy.parent.mkdir(parents=True, exist_ok=True)
    repo_copy.write_text(DEMO_CONTENT)

    log("committing in zenops repo")
    run("git", "-C", str(ZENOPS_DIR), "add", "-A")
    run("git", "-C", str(ZENOPS_DIR), "commit", "-m", "Add demo pkg")

    log("running zenops apply --yes --output json")
    run_zenops("apply", "--yes", "--output", "json")

    log("verifying ~/.config/demo/data.txt is a symlink to the repo copy")
    expected_link = HOME / ".config" / "demo" / "data.txt"
    assert_symlink(expected_link, points_to=repo_copy)
    assert_true(
        expected_link.read_text() == DEMO_CONTENT,
        f"symlink content unexpected: {expected_link.read_text()!r}",
    )

    log(f"verifying generated {args.shell} files exist")
    for path in EXPECTED_FILES[args.shell]:
        assert_file_exists(path)

    return 0


if __name__ == "__main__":
    sys.exit(main())
