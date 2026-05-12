"""Cross-platform test matrix.

Each tuple is ``(distro, shell, package_manager)``. Linux only — macOS is out
of scope for now. The package_manager slot is implied by ``distro`` today and
is forward-looking: once zenops can install via apt/dnf/pacman, the matrix
already discriminates.
"""

MATRIX = [
    ("ubuntu",    "bash", "apt"),
    ("ubuntu",    "zsh",  "apt"),
    ("fedora",    "bash", "dnf"),
    ("fedora",    "zsh",  "dnf"),
    ("archlinux", "bash", "pacman"),
    ("archlinux", "zsh",  "pacman"),
]

DISTROS = sorted({d for d, _, _ in MATRIX})
SHELLS = sorted({s for _, s, _ in MATRIX})
