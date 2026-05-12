"""Shared helpers for the in-container step scripts.

Steps run as separate subprocess invocations so each is debuggable on its
own; this module provides the bits they all need: well-known paths, a shared
arg parser, a thin subprocess wrapper around ``zenops``, and a ``pexpect``
wrapper for interactive flows.
"""
import argparse
import os
import subprocess
import sys
from pathlib import Path

HOME = Path(os.environ.get("HOME", "/home/tester"))
ZENOPS_DIR = HOME / ".config" / "zenops"
CONFIG_TOML = ZENOPS_DIR / "config.toml"
CONFIGS_DIR = ZENOPS_DIR / "configs"

# Files zenops generates per shell. Used by step 03 to assert that apply
# produced something. zsh's .zprofile is conditional on having login-init
# actions, so we don't assert on it here.
EXPECTED_FILES = {
    "bash": [HOME / ".zenops_bash_profile"],
    "zsh":  [HOME / ".zshenv", HOME / ".zshrc"],
}


def parse_args() -> argparse.Namespace:
    """Parse the arg set the runner forwards to every step.

    Steps that don't need a given arg simply ignore it; keeping the parser
    shared means the runner can pass one fixed set of flags.
    """
    p = argparse.ArgumentParser()
    p.add_argument("--shell", choices=["bash", "zsh"], required=True)
    p.add_argument("--install", choices=["local", "registry"], default="local")
    p.add_argument("--source-path", default="/src")
    return p.parse_args()


def log(msg: str) -> None:
    print(f"  . {msg}", flush=True)


def fail(msg: str):
    print(f"  ! {msg}", flush=True)
    sys.exit(1)


def run(*argv, check: bool = True, capture: bool = False, **kw):
    """Subprocess wrapper that echoes the command line before running."""
    cmd = [str(a) for a in argv]
    print(f"  $ {' '.join(cmd)}", flush=True)
    return subprocess.run(
        cmd,
        check=check,
        text=True,
        capture_output=capture,
        **kw,
    )


def run_zenops(*args, **kw):
    return run("zenops", *args, **kw)


def expect_zenops(*args, prompts, timeout: int = 30):
    """Drive ``zenops <args>`` over a PTY, answering rustyline prompts.

    ``prompts`` is a list of ``(expected_substring, reply)`` tuples. The
    expected substring is matched literally (no regex) so colour codes in the
    prompt don't break the test. The reply is sent with a trailing newline.

    Blocks until the process exits, then returns the spawn object. Caller
    asserts on ``child.exitstatus``.
    """
    import pexpect

    print(f"  $ zenops {' '.join(args)}  (pty)", flush=True)
    child = pexpect.spawn(
        "zenops",
        list(args),
        encoding="utf-8",
        codec_errors="replace",
        dimensions=(24, 200),
        timeout=timeout,
    )
    if os.environ.get("ZENOPS_TEST_DEBUG"):
        child.logfile_read = sys.stdout
    for expected, reply in prompts:
        child.expect_exact(expected)
        child.sendline(reply)
    child.expect(pexpect.EOF)
    child.wait()
    return child


def assert_true(cond, msg: str = ""):
    if not cond:
        fail(f"assertion failed{': ' + msg if msg else ''}")


def assert_eq(actual, expected, msg: str = ""):
    if actual != expected:
        fail(
            f"assertion failed{': ' + msg if msg else ''}: "
            f"got {actual!r}, expected {expected!r}"
        )


def assert_file_exists(path: Path):
    if not path.exists():
        fail(f"expected file to exist: {path}")


def assert_symlink(path: Path, points_to: Path | None = None):
    if not path.is_symlink():
        fail(f"expected symlink at {path}, not found or not a symlink")
    if points_to is not None:
        target = Path(os.readlink(path))
        if target != points_to:
            fail(f"symlink {path} points to {target}, expected {points_to}")
