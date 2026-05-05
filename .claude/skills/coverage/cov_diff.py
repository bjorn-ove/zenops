#!/usr/bin/env python3
"""Diff two cargo-llvm-cov summary JSONs and print the delta.

Usage:
  cov_diff.py <before.json> <after.json> [--top N] [--min-delta D]
              [--format text|json]

Reports:
  - Workspace total before → after (lines, regions, functions).
  - Per-file deltas, sorted by absolute change in line %.
  - New files (present only in after) and dropped files (only in before).

Stdlib-only.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path


def load(path: Path) -> tuple[dict, dict[str, dict]]:
    data = json.loads(path.read_text())
    entry = data["data"][0]
    by_name = {f["filename"]: f["summary"] for f in entry["files"]}
    return entry["totals"], by_name


def fmt_totals_line(label: str, before: dict, after: dict) -> str:
    b = before["lines"]["percent"]
    a = after["lines"]["percent"]
    return f"  {label:18s} {b:6.2f}% -> {a:6.2f}%  ({a - b:+5.2f} pts)"


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("before", type=Path)
    p.add_argument("after", type=Path)
    p.add_argument("--top", type=int, default=15)
    p.add_argument("--min-delta", type=float, default=0.5,
                   help="hide per-file rows with |delta| < this many points")
    p.add_argument("--format", choices=["text", "json"], default="text")
    args = p.parse_args()

    b_totals, b_files = load(args.before)
    a_totals, a_files = load(args.after)

    rows: list[tuple[float, str, dict, dict]] = []
    for fname, a_sum in a_files.items():
        b_sum = b_files.get(fname)
        if b_sum is None:
            rows.append((a_sum["lines"]["percent"], fname, {"lines": {"percent": 0.0, "count": 0, "covered": 0}}, a_sum))
            continue
        delta = a_sum["lines"]["percent"] - b_sum["lines"]["percent"]
        if abs(delta) >= args.min_delta:
            rows.append((delta, fname, b_sum, a_sum))
    rows.sort(key=lambda t: t[0], reverse=True)

    dropped = [fn for fn in b_files if fn not in a_files]

    if args.format == "json":
        out = {
            "totals": {"before": b_totals, "after": a_totals},
            "deltas": [
                {
                    "file": fn,
                    "before_pct": b["lines"]["percent"],
                    "after_pct": a["lines"]["percent"],
                    "delta_pts": a["lines"]["percent"] - b["lines"]["percent"],
                    "lines_count": a["lines"]["count"],
                    "lines_covered_before": b["lines"].get("covered", 0),
                    "lines_covered_after": a["lines"]["covered"],
                }
                for _d, fn, b, a in rows
            ],
            "dropped_files": dropped,
        }
        print(json.dumps(out, indent=2))
        return 0

    cwd = os.getcwd() + "/"
    print("Coverage delta:")
    print(fmt_totals_line("Workspace lines:", b_totals, a_totals))
    bR = b_totals["regions"]
    aR = a_totals["regions"]
    bF = b_totals["functions"]
    aF = a_totals["functions"]
    print(f"  {'Workspace regions:':18s} "
          f"{bR['percent']:6.2f}% -> {aR['percent']:6.2f}%  "
          f"({aR['percent'] - bR['percent']:+5.2f} pts)")
    print(f"  {'Workspace funcs:':18s} "
          f"{bF['percent']:6.2f}% -> {aF['percent']:6.2f}%  "
          f"({aF['percent'] - bF['percent']:+5.2f} pts)")

    if rows:
        print()
        print(f"Per-file changes (|delta| >= {args.min_delta} pts):")
        for delta, fn, b, a in rows[: args.top]:
            short = fn.removeprefix(cwd)
            b_pct = b["lines"]["percent"]
            a_pct = a["lines"]["percent"]
            newly_covered = a["lines"]["covered"] - b["lines"].get("covered", 0)
            print(f"  {short:50s}  {b_pct:6.2f}% -> {a_pct:6.2f}%  "
                  f"({delta:+6.2f} pts, {newly_covered:+d} lines)")

    if dropped:
        print()
        print("Files present in before but missing in after:")
        for fn in dropped:
            print(f"  {fn.removeprefix(cwd)}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
