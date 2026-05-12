#!/usr/bin/env python3
"""Step 01: install zenops via ``cargo install``.

Two modes (selected by ``--install``):

* ``local``    — ``cargo install --path /src --locked``
* ``registry`` — ``cargo install zenops --locked``

After install, sanity-checks ``zenops --version``.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from common import parse_args, run, run_zenops, log


def main() -> int:
    args = parse_args()

    if args.install == "local":
        log(f"installing zenops from {args.source_path}")
        run("cargo", "install", "--path", args.source_path, "--locked")
    else:
        log("installing zenops from crates.io")
        run("cargo", "install", "zenops", "--locked")

    log("checking zenops --version")
    result = run_zenops("--version", capture=True)
    version_line = (result.stdout or "").strip()
    if not version_line:
        print("expected non-empty --version output", file=sys.stderr)
        return 1
    log(f"installed: {version_line}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
