#!/usr/bin/env python3
"""Render `cargo llvm-cov report --json --summary-only` output as Markdown.

Used by `.github/workflows/ci.yaml` to build the coverage comment posted on
pull requests. Kept as a standalone script (rather than inline `run:` shell)
so it can be run locally against a report produced by:

    cargo llvm-cov report --json --summary-only --output-path coverage.json
    python3 .github/scripts/coverage_report.py --summary coverage.json

Only depends on the standard library: the CI runner has a system Python but
no project-specific virtualenv, and pulling one in for a formatting script
would cost more than it buys.
"""

from __future__ import annotations

import argparse
import json
import os
import sys

# Coverage bands used for the per-file status dot. These are presentation
# only -- nothing here fails the build, because a hard gate on a young
# codebase mostly teaches people to write assertion-free tests.
GOOD = 80.0
OKAY = 60.0

# Anchors the comment so CI can find and update its own previous comment
# instead of appending a new one to every push.
MARKER = "<!-- qbrs-coverage-report -->"

# Below this many percentage points a delta is noise (renaming a function can
# move a total by a hair), so it renders as "no change" rather than a
# red/green arrow.
DELTA_EPSILON = 0.05


class Metric:
    """One `count`/`covered`/`percent` triple out of the llvm-cov summary."""

    def __init__(self, raw: dict) -> None:
        self.count = int(raw.get("count", 0))
        self.covered = int(raw.get("covered", 0))
        # llvm-cov reports percent 0 for an empty file; treat "nothing to
        # cover" as fully covered so type-level-only modules (of which this
        # workspace has several) don't drag a per-file table down.
        self.percent = float(raw.get("percent", 0.0)) if self.count else 100.0


def load(path: str, root: str) -> tuple[dict[str, dict[str, Metric]], dict[str, Metric]]:
    """Read an llvm-cov JSON export into {relative path: {kind: Metric}} plus totals."""
    with open(path, encoding="utf-8") as handle:
        report = json.load(handle)

    export = report["data"][0]
    kinds = ("lines", "regions", "functions")

    files: dict[str, dict[str, Metric]] = {}
    for entry in export["files"]:
        name = os.path.relpath(entry["filename"], root)
        files[name] = {kind: Metric(entry["summary"][kind]) for kind in kinds}

    totals = {kind: Metric(export["totals"][kind]) for kind in kinds}
    return files, totals


def dot(percent: float) -> str:
    if percent >= GOOD:
        return "🟢"
    if percent >= OKAY:
        return "🟡"
    return "🔴"


def delta(current: float, baseline: float | None) -> str:
    if baseline is None:
        return "—"
    diff = current - baseline
    if abs(diff) < DELTA_EPSILON:
        return "±0.00%"
    return f"{'🔺' if diff > 0 else '🔻'} {diff:+.2f}%"


def file_row(name: str, metrics: dict[str, Metric], baseline: dict[str, Metric] | None) -> str:
    lines = metrics["lines"]
    base = baseline["lines"].percent if baseline else None
    return (
        f"| `{name}` | {dot(lines.percent)} {lines.percent:.2f}% "
        f"| {lines.covered}/{lines.count} "
        f"| {metrics['regions'].percent:.2f}% "
        f"| {metrics['functions'].percent:.2f}% "
        f"| {delta(lines.percent, base)} |"
    )


FILE_HEADER = (
    "| File | Lines | Covered | Regions | Functions | Δ |\n"
    "| --- | ---: | ---: | ---: | ---: | ---: |"
)


def render(
    files: dict[str, dict[str, Metric]],
    totals: dict[str, Metric],
    base_files: dict[str, dict[str, Metric]],
    base_totals: dict[str, Metric] | None,
    changed: list[str],
) -> str:
    out = [MARKER, "## 🧪 Test coverage", ""]

    out.append("| Metric | Coverage | Covered / Total | Δ vs base |")
    out.append("| --- | ---: | ---: | ---: |")
    for kind in ("lines", "regions", "functions"):
        metric = totals[kind]
        base = base_totals[kind].percent if base_totals else None
        out.append(
            f"| {kind.capitalize()} | {dot(metric.percent)} **{metric.percent:.2f}%** "
            f"| {metric.covered} / {metric.count} | {delta(metric.percent, base)} |"
        )
    out.append("")

    # The files this PR actually touched are the part a reviewer can act on,
    # so they go first and uncollapsed; everything else is reference material.
    touched = sorted(name for name in changed if name in files)
    if touched:
        out.append(f"### Files changed in this PR ({len(touched)})")
        out.append("")
        out.append(FILE_HEADER)
        out.extend(file_row(name, files[name], base_files.get(name)) for name in touched)
        out.append("")
        untracked = sorted(set(changed) - set(files))
        if untracked:
            out.append(
                f"<sub>{len(untracked)} changed Rust file(s) have no instrumented code "
                "(type-level-only or not built by the covered targets).</sub>"
            )
            out.append("")

    out.append(f"<details><summary>All instrumented files ({len(files)})</summary>")
    out.append("")
    out.append(FILE_HEADER)
    out.extend(file_row(name, files[name], base_files.get(name)) for name in sorted(files))
    out.append("")
    out.append("</details>")
    out.append("")
    out.append(
        "<sub>Measured with "
        "[`cargo-llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov) over "
        "`cargo test --workspace`. The `qbrs-examples` and `compile-bench` crates are "
        "excluded: they exist to be compiled, not run. Δ compares against the latest "
        "successful run on the base branch, and is `—` when no such run is "
        "available.</sub>"
    )
    return "\n".join(out) + "\n"


def read_changed(path: str | None) -> list[str]:
    if not path or not os.path.exists(path):
        return []
    with open(path, encoding="utf-8") as handle:
        return [line.strip() for line in handle if line.strip().endswith(".rs")]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--summary", required=True, help="llvm-cov JSON for this run")
    parser.add_argument("--baseline", help="llvm-cov JSON from the base branch, if any")
    parser.add_argument("--changed-files", help="file listing one changed path per line")
    parser.add_argument("--root", default=os.getcwd(), help="repo root, for relative paths")
    parser.add_argument("--output", help="write Markdown here instead of stdout")
    args = parser.parse_args()

    files, totals = load(args.summary, args.root)

    base_files: dict[str, dict[str, Metric]] = {}
    base_totals: dict[str, Metric] | None = None
    if args.baseline and os.path.exists(args.baseline):
        try:
            base_files, base_totals = load(args.baseline, args.root)
        except (OSError, ValueError, KeyError, IndexError) as err:
            # A stale or truncated baseline artifact must not fail the job;
            # the report is still useful without the delta column.
            print(f"ignoring unreadable baseline {args.baseline}: {err}", file=sys.stderr)

    markdown = render(files, totals, base_files, base_totals, read_changed(args.changed_files))

    if args.output:
        with open(args.output, "w", encoding="utf-8") as handle:
            handle.write(markdown)
    else:
        sys.stdout.write(markdown)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
