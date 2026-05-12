#!/usr/bin/env python3
"""In-container entrypoint. Loads a scenario, runs its steps in order.

A scenario is a Python module under ``scenarios/`` that defines ``STEPS`` —
an ordered list of step script filenames (relative to ``steps/``). Each step
runs as its own subprocess so steps stay debuggable individually; the runner
stops at the first failing step.
"""
import argparse
import importlib.util
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCENARIOS = HERE / "scenarios"
STEPS = HERE / "steps"


def load_scenario(name: str) -> list[str]:
    path = SCENARIOS / f"{name}.py"
    if not path.exists():
        sys.exit(f"no such scenario: {path}")
    spec = importlib.util.spec_from_file_location(f"scenarios.{name}", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.STEPS


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--scenario", required=True)
    p.add_argument("--shell", choices=["bash", "zsh"], required=True)
    p.add_argument("--install", choices=["local", "registry"], default="local")
    p.add_argument("--source-path", default="/src")
    args = p.parse_args()

    steps = load_scenario(args.scenario)
    forwarded = [
        "--shell", args.shell,
        "--install", args.install,
        "--source-path", args.source_path,
    ]

    for step in steps:
        path = STEPS / step
        print(f"\n=== step {step} ===", flush=True)
        result = subprocess.run([sys.executable, str(path), *forwarded])
        if result.returncode != 0:
            print(f"\n!!! step {step} FAILED (exit {result.returncode})", flush=True)
            return result.returncode

    print("\n=== all steps passed ===", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
