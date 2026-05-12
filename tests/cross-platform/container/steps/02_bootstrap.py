#!/usr/bin/env python3
"""Step 02: bootstrap a fresh zenops repo via interactive ``zenops init``.

Drives the three rustyline prompts (shell / name / email) over a PTY, then
verifies the on-disk state: config.toml has the expected shape and the
zenops dir is a git repo with a single ``Initial zenops config`` commit.

The Dockerfile pre-sets git config user.name / user.email to known values,
so the prompts show those as defaults. We send explicit values anyway, both
to exercise the input path and to keep the test deterministic.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from common import (
    CONFIG_TOML,
    ZENOPS_DIR,
    assert_eq,
    assert_true,
    expect_zenops,
    fail,
    log,
    parse_args,
    run,
)


def main() -> int:
    args = parse_args()
    shell = args.shell

    log(f"running `zenops init` and answering prompts (shell={shell})")
    child = expect_zenops(
        "init",
        prompts=[
            ("Shell (bash/zsh/none)", shell),
            ("Name",                  "Test User"),
            ("Email",                 "test@example.com"),
        ],
    )
    if child.exitstatus != 0:
        fail(f"zenops init exited with status {child.exitstatus}")

    log("verifying config.toml")
    assert_true(CONFIG_TOML.exists(), f"{CONFIG_TOML} missing")
    body = CONFIG_TOML.read_text()
    assert_true(f'type = "{shell}"' in body, f"config.toml missing shell={shell}")
    assert_true('name = "Test User"' in body, "config.toml missing name")
    assert_true('email = "test@example.com"' in body, "config.toml missing email")

    log("verifying single 'Initial zenops config' commit")
    assert_true((ZENOPS_DIR / ".git").exists(), "no .git directory")
    log_out = run(
        "git", "-C", str(ZENOPS_DIR), "log", "--oneline",
        capture=True,
    ).stdout.strip()
    lines = log_out.splitlines()
    assert_eq(len(lines), 1, "expected exactly one commit")
    assert_true("Initial zenops config" in lines[0], f"unexpected commit subject: {lines[0]}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
