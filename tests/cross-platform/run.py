#!/usr/bin/env python3
"""Host driver for the cross-platform integration test harness.

Builds a Docker image per distro (cached across runs) and runs the
in-container scenario for each filtered matrix entry. Stdlib only — no pip
install needed on the host.

Examples:
    python tests/cross-platform/run.py
    python tests/cross-platform/run.py --only ubuntu
    python tests/cross-platform/run.py --only ubuntu --shell bash
    python tests/cross-platform/run.py --install registry
    python tests/cross-platform/run.py --scenario basic
"""
import argparse
import shlex
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parent.parent
DOCKER_DIR = HERE / "docker"
CONTAINER_DIR = HERE / "container"

sys.path.insert(0, str(HERE))
from matrix import DISTROS, MATRIX, SHELLS  # noqa: E402


def build_image(distro: str, *, pull: bool) -> str:
    tag = f"zenops-test:{distro}"
    dockerfile = DOCKER_DIR / f"{distro}.Dockerfile"
    if not dockerfile.exists():
        sys.exit(f"no Dockerfile for distro={distro}: {dockerfile}")
    cmd = ["docker", "build", "-f", str(dockerfile), "-t", tag]
    if pull:
        cmd.append("--pull")
    cmd.append(str(DOCKER_DIR))
    print(f"\n>>> building {tag}")
    print(f"    $ {' '.join(shlex.quote(c) for c in cmd)}")
    subprocess.run(cmd, check=True)
    return tag


def run_combo(
    tag: str,
    distro: str,
    shell: str,
    *,
    install: str,
    scenario: str,
    keep: bool,
) -> bool:
    prefix = f"[{distro}/{shell}]"
    print(f"\n>>> {prefix} scenario={scenario} install={install}")

    docker_args = ["docker", "run"]
    if keep:
        docker_args += ["--name", f"zenops-test-{distro}-{shell}"]
    else:
        docker_args.append("--rm")
    docker_args += [
        # cargo install needs to fetch crates.io regardless of mode; bridge
        # is required for both local (deps) and registry (zenops itself).
        "--network", "bridge",
        "-v", f"{REPO_ROOT}:/src:ro",
        "-v", f"{CONTAINER_DIR}:/test:ro",
        tag,
        "python3", "/test/runner.py",
        "--scenario", scenario,
        "--shell", shell,
        "--install", install,
        "--source-path", "/src",
    ]

    print(f"    $ {' '.join(shlex.quote(c) for c in docker_args)}")
    proc = subprocess.Popen(
        docker_args,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    assert proc.stdout is not None
    for line in proc.stdout:
        print(f"{prefix} {line.rstrip()}")
    proc.wait()
    ok = proc.returncode == 0
    print(f"{prefix} {'PASS' if ok else 'FAIL'} (exit {proc.returncode})")
    return ok


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--only", choices=DISTROS, help="limit to one distro")
    p.add_argument("--shell", choices=SHELLS, help="limit to one shell")
    p.add_argument("--install", choices=["local", "registry"], default="local")
    p.add_argument("--scenario", default="basic")
    p.add_argument("--keep", action="store_true",
                   help="don't `docker rm` (use `docker exec` to inspect)")
    p.add_argument("--pull", action="store_true",
                   help="docker build --pull (refresh base images)")
    p.add_argument("--no-build", action="store_true",
                   help="skip docker build (assume images already exist)")
    args = p.parse_args()

    if not shutil.which("docker"):
        sys.exit("docker not on PATH")

    selected = [
        entry for entry in MATRIX
        if (args.only is None or entry[0] == args.only)
        and (args.shell is None or entry[1] == args.shell)
    ]
    if not selected:
        sys.exit("no matrix entries selected; check --only/--shell")

    print(f"matrix: {len(selected)} combination(s)")
    for d, s, pm in selected:
        print(f"  - {d} / {s} / {pm}")

    distros_needed = sorted({d for d, _, _ in selected})
    images = {}
    for distro in distros_needed:
        if args.no_build:
            images[distro] = f"zenops-test:{distro}"
        else:
            images[distro] = build_image(distro, pull=args.pull)

    results = []
    for distro, shell, _ in selected:
        ok = run_combo(
            images[distro], distro, shell,
            install=args.install,
            scenario=args.scenario,
            keep=args.keep,
        )
        results.append((distro, shell, ok))

    print("\n=== summary ===")
    for distro, shell, ok in results:
        print(f"  {distro}/{shell}: {'PASS' if ok else 'FAIL'}")
    failures = [r for r in results if not r[2]]
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
