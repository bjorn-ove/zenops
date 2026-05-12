#!/usr/bin/env python3
"""Step 04: import a real on-disk config into the zenops repo.

Creates ``~/.config/widget/settings.toml`` with known content, runs
``zenops import --yes --brew widget``, then verifies the original is now a
symlink into the zenops repo and config.toml has gained a ``[pkg.widget]``
block.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from common import (
    CONFIGS_DIR,
    CONFIG_TOML,
    HOME,
    assert_symlink,
    assert_true,
    log,
    parse_args,
    run_zenops,
)

WIDGET_CONTENT = 'color = "orange"\nopacity = 0.8\n'


def main() -> int:
    parse_args()  # ignore: this step doesn't branch on shell/install

    log("creating ~/.config/widget/settings.toml")
    widget_dir = HOME / ".config" / "widget"
    widget_dir.mkdir(parents=True, exist_ok=True)
    widget_file = widget_dir / "settings.toml"
    widget_file.write_text(WIDGET_CONTENT)

    log("running zenops import ~/.config/widget --yes --brew widget")
    run_zenops(
        "import", str(widget_dir),
        "--yes",
        "--brew", "widget",
    )

    log("verifying ~/.config/widget/settings.toml is now a symlink into the repo")
    expected_target = CONFIGS_DIR / "widget" / "settings.toml"
    assert_symlink(widget_file, points_to=expected_target)
    assert_true(
        expected_target.read_text() == WIDGET_CONTENT,
        f"repo copy content unexpected: {expected_target.read_text()!r}",
    )

    log("verifying [pkg.widget] block in config.toml")
    body = CONFIG_TOML.read_text()
    assert_true("[pkg.widget]" in body, "config.toml missing [pkg.widget] block")
    assert_true("configs/widget" in body, "config.toml missing widget source path")

    return 0


if __name__ == "__main__":
    sys.exit(main())
