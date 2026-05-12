# Cross-platform integration tests

Docker-driven integration tests for the `zenops` binary. Each combination of
`(distro, shell, package_manager)` runs in its own container with a real
shell, a real package manager, and a real `cargo install` — no env-var
fakery, no mocked filesystem.

## Requirements

- Docker (any recent version).
- ~2 GB free disk for the cached images (one per distro).
- Internet access (rustup, distro packages, crates.io).
- Python 3.10+ on the host. Stdlib only — no `pip install` needed.

## Quick start

From the repo root:

```bash
python tests/cross-platform/run.py --only ubuntu --shell bash
```

Full matrix:

```bash
python tests/cross-platform/run.py
```

Install from crates.io instead of the local source tree:

```bash
python tests/cross-platform/run.py --install registry
```

Other useful flags:

```bash
--scenario <name>   # which scenario to run (default: basic)
--keep              # leave the container around for `docker exec` debugging
--pull              # docker build --pull (refresh base images)
--no-build          # assume images already exist; skip build step
```

## What gets tested (scenario `basic`)

A fresh container, four steps in order. The runner stops at the first
failing step.

1. **`01_install`** — `cargo install --path /src --locked` (or `cargo install
   zenops` from crates.io).
2. **`02_bootstrap`** — drive `zenops init` over a pseudo-TTY, answer the
   rustyline prompts (shell / name / email), verify `config.toml` and the
   initial commit.
3. **`03_add_apply`** — append a `[pkg.demo]` block to `config.toml`, commit
   it in the zenops repo, then `zenops apply --yes`. Verify the symlink
   landed under `~/.config/demo/` and the shell's generated rc files exist.
4. **`04_import`** — create a real config at `~/.config/widget/`, run
   `zenops import --yes`, verify the original is now a symlink into the
   zenops repo and `config.toml` has gained a `[pkg.widget]` block.

## Layout

```
tests/cross-platform/
  run.py                          host driver
  matrix.py                       (distro, shell, pm) tuples
  docker/
    ubuntu.Dockerfile
    fedora.Dockerfile
    archlinux.Dockerfile
  container/
    runner.py                     in-container entrypoint
    common.py                     shared helpers (paths, pexpect, asserts)
    scenarios/
      basic.py                    STEPS = [...]
    steps/
      01_install.py
      02_bootstrap.py
      03_add_apply.py
      04_import.py
```

## Extending

**Adding a shell.** Append to `MATRIX` in `matrix.py` and add an entry to
`EXPECTED_FILES` in `container/common.py`. If the bootstrap prompt response
isn't the shell's plain name, override that in `02_bootstrap.py`.

**Adding a distro.** Drop a Dockerfile under `docker/<distro>.Dockerfile`
that installs `git`, `bash`, `zsh`, `curl`, build tools, Python 3 with
`pexpect`, a non-root `tester` user, and rustup. Then add matrix entries.
No test code changes.

**Adding a scenario.** Copy `container/scenarios/basic.py` and edit the
`STEPS` list. New step scripts go under `container/steps/` and are loaded by
file path, so the `NN_<name>.py` numbering controls order. Each step is a
standalone script (runnable directly during debugging) that uses helpers
from `container/common.py`.

## Debugging

`ZENOPS_TEST_DEBUG=1` in the container's environment makes the pexpect
wrapper echo the raw PTY stream — useful when a prompt match fails.

Inspect a failed run's state:

```bash
python tests/cross-platform/run.py --only ubuntu --shell bash --keep
# ... (failure) ...
docker exec -it zenops-test-ubuntu-bash bash
```

Remove cached images when something's stuck:

```bash
docker image rm zenops-test:ubuntu zenops-test:fedora zenops-test:archlinux
```
