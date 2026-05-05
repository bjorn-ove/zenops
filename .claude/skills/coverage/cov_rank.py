#!/usr/bin/env python3
"""Rank weak files from a cargo-llvm-cov JSON report.

Usage:
  cov_rank.py <summary.json> [--top N] [--min-lines N] [--max-pct P]
              [--full <full.json>] [--src-root PATH]
              [--unwraps] [--format text|tsv|json]

Default mode prints the workspace totals, the lowest-coverage files
(filtered by --min-lines, default 50; --max-pct, default 95), and a
"templates" section that surfaces:

  - Lowest-coverage files with enough lines to matter (template 1).
  - Modules with very low function coverage (template 4 candidates).

When --unwraps is passed and --full points at a non-summary JSON, the
tool also intersects unwrap()/expect()/panic!() call sites with
uncovered regions and surfaces them as template 3 candidates.

The output is plain text by default so it pastes cleanly into a chat;
--format tsv emits rank rows on stdout for further scripting.

This script is intentionally stdlib-only — no pip install needed.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from dataclasses import dataclass
from pathlib import Path


@dataclass
class FileRow:
    filename: str
    lines_count: int
    lines_covered: int
    lines_pct: float
    funcs_count: int
    funcs_covered: int
    funcs_pct: float
    regions_count: int
    regions_covered: int
    regions_pct: float

    @property
    def uncov_lines(self) -> int:
        return self.lines_count - self.lines_covered

    @property
    def short(self) -> str:
        cwd = os.getcwd() + "/"
        return self.filename.removeprefix(cwd)


def load_files(path: Path) -> tuple[dict, list[FileRow]]:
    data = json.loads(path.read_text())
    entry = data["data"][0]
    rows: list[FileRow] = []
    for f in entry["files"]:
        s = f["summary"]
        rows.append(
            FileRow(
                filename=f["filename"],
                lines_count=s["lines"]["count"],
                lines_covered=s["lines"]["covered"],
                lines_pct=s["lines"]["percent"],
                funcs_count=s["functions"]["count"],
                funcs_covered=s["functions"]["covered"],
                funcs_pct=s["functions"]["percent"],
                regions_count=s["regions"]["count"],
                regions_covered=s["regions"]["covered"],
                regions_pct=s["regions"]["percent"],
            )
        )
    return entry["totals"], rows


def uncovered_line_ranges(file_entry: dict) -> list[tuple[int, int]]:
    """Walk per-file `segments` and return [(start_line, end_line), ...]
    for runs of zero-execution-count lines.

    The segment shape is `[line, col, count, has_count, is_region_entry,
    is_gap_region]`. A line is uncovered when the most recent segment
    that started a region has count == 0 and `has_count == True`.
    """
    segments = file_entry.get("segments") or []
    if not segments:
        return []
    uncov_lines: set[int] = set()
    current_count = None
    current_has = False
    for seg in segments:
        line, _col, count, has_count, is_entry, is_gap = (
            seg[0],
            seg[1],
            seg[2],
            seg[3],
            seg[4],
            seg[5],
        )
        if is_entry and has_count:
            current_count = count
            current_has = True
        if is_gap:
            current_has = False
        if current_has and current_count == 0 and not is_gap:
            uncov_lines.add(line)
    if not uncov_lines:
        return []
    sorted_lines = sorted(uncov_lines)
    ranges: list[tuple[int, int]] = []
    start = prev = sorted_lines[0]
    for n in sorted_lines[1:]:
        if n == prev + 1:
            prev = n
        else:
            ranges.append((start, prev))
            start = prev = n
    ranges.append((start, prev))
    return ranges


def find_panicky_calls(src_root: Path) -> dict[str, set[int]]:
    """Map filename → set of line numbers containing unwrap/expect/panic.

    Walks `src/` and `crates/*/src/`, skipping `tests/` and any file
    named `test_env.rs`. Pure regex — good enough for ranking.
    """
    pattern = re.compile(r"\.unwrap\(|\.expect\(|panic!\(|todo!\(|unimplemented!\(")
    sites: dict[str, set[int]] = {}
    roots = [src_root / "src"]
    crates_dir = src_root / "crates"
    if crates_dir.exists():
        for crate in crates_dir.iterdir():
            csrc = crate / "src"
            if csrc.exists():
                roots.append(csrc)
    for root in roots:
        for path in root.rglob("*.rs"):
            if path.name == "test_env.rs":
                continue
            if "/tests/" in str(path):
                continue
            try:
                lines = path.read_text().splitlines()
            except (OSError, UnicodeDecodeError):
                continue
            hits = {i + 1 for i, line in enumerate(lines) if pattern.search(line)}
            if hits:
                sites[str(path.resolve())] = hits
    return sites


def fmt_totals(totals: dict) -> str:
    L = totals["lines"]
    R = totals["regions"]
    F = totals["functions"]
    return (
        f"Workspace coverage: "
        f"{L['percent']:.2f}% lines ({L['covered']}/{L['count']}), "
        f"{R['percent']:.2f}% regions, {F['percent']:.2f}% functions"
    )


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("summary", type=Path, help="path to summary JSON")
    p.add_argument("--full", type=Path, help="path to non-summary JSON for region detail")
    p.add_argument("--top", type=int, default=10)
    p.add_argument("--min-lines", type=int, default=50)
    p.add_argument("--max-pct", type=float, default=95.0)
    p.add_argument("--src-root", type=Path, default=Path.cwd())
    p.add_argument("--unwraps", action="store_true",
                   help="intersect unwrap/expect/panic sites with uncovered regions")
    p.add_argument("--format", choices=["text", "tsv", "json"], default="text")
    args = p.parse_args()

    totals, rows = load_files(args.summary)

    weak = [
        r for r in rows
        if r.lines_count >= args.min_lines and r.lines_pct < args.max_pct
    ]
    weak.sort(key=lambda r: r.lines_pct)

    low_func = [r for r in rows if r.funcs_count >= 5 and r.funcs_pct < 50.0]
    low_func.sort(key=lambda r: r.funcs_pct)

    if args.format == "json":
        out = {
            "totals": totals,
            "weak": [r.__dict__ for r in weak[: args.top]],
            "low_function_coverage": [r.__dict__ for r in low_func[: args.top]],
        }
        print(json.dumps(out, indent=2))
        return 0

    if args.format == "tsv":
        print("file\tlines_pct\tuncov\ttotal\tfuncs_pct")
        for r in weak[: args.top]:
            print(f"{r.short}\t{r.lines_pct:.2f}\t{r.uncov_lines}\t{r.lines_count}\t{r.funcs_pct:.2f}")
        return 0

    print(fmt_totals(totals))
    print()
    print(f"Weakest files (lines.count >= {args.min_lines}, lines% < {args.max_pct}):")
    for r in weak[: args.top]:
        print(f"  {r.lines_pct:6.2f}%  uncov={r.uncov_lines:3d}/{r.lines_count:<4d}"
              f"  funcs={r.funcs_pct:6.2f}%  {r.short}")

    if low_func:
        print()
        print("Modules with low function coverage (funcs% < 50, count >= 5):")
        for r in low_func[: args.top]:
            print(f"  funcs={r.funcs_pct:6.2f}% ({r.funcs_covered}/{r.funcs_count})"
                  f"  lines={r.lines_pct:6.2f}%  {r.short}")

    if args.unwraps:
        if not args.full:
            print("\n[--unwraps requires --full <full.json>]", file=sys.stderr)
            return 2
        full_data = json.loads(args.full.read_text())
        full_files = {f["filename"]: f for f in full_data["data"][0]["files"]}
        sites = find_panicky_calls(args.src_root)
        rows_panic: list[tuple[int, str, list[int]]] = []
        for fname, lines in sites.items():
            entry = full_files.get(fname)
            if entry is None:
                continue
            uncov = set()
            for start, end in uncovered_line_ranges(entry):
                for n in range(start, end + 1):
                    uncov.add(n)
            hit_lines = sorted(lines & uncov)
            if hit_lines:
                short = fname.removeprefix(os.getcwd() + "/")
                rows_panic.append((len(hit_lines), short, hit_lines))
        rows_panic.sort(reverse=True)
        if rows_panic:
            print()
            print("Unwrap/expect/panic on uncovered branches:")
            for count, short, hit_lines in rows_panic[: args.top]:
                preview = ",".join(str(n) for n in hit_lines[:6])
                more = f" (+{len(hit_lines) - 6})" if len(hit_lines) > 6 else ""
                print(f"  {count:3d} sites  {short}  lines: {preview}{more}")
        else:
            print()
            print("Unwrap/expect/panic on uncovered branches: none found.")

    return 0


if __name__ == "__main__":
    sys.exit(main())
