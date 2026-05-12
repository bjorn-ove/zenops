"""Basic happy-path scenario: install -> bootstrap -> add+apply -> import.

Steps run in declared order; the runner halts at the first failure. To
exercise an edge case, copy this file and swap one of the step names —
files in ``steps/`` are independent, so you only touch what you mean to.
"""

STEPS = [
    "01_install.py",
    "02_bootstrap.py",
    "03_add_apply.py",
    "04_import.py",
]
