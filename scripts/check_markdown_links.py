#!/usr/bin/env python3
"""Check local Markdown links for missing targets."""

from __future__ import annotations

import argparse
import re
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import unquote, urlsplit


IGNORED_DIRS = {
    ".ait",
    ".ait-server",
    ".git",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".venv",
    "__pycache__",
    "build",
    "dist",
    "node_modules",
    "venv",
}


def should_ignore_part(part: str) -> bool:
    return part in IGNORED_DIRS or part.startswith(".tmp-")

INLINE_LINK_RE = re.compile(r"!?\[[^\]\n]*\]\(([^)\n]+)\)")
REFERENCE_DEF_RE = re.compile(r"^\s{0,3}\[[^\]]+\]:\s*(\S+)")
FENCE_RE = re.compile(r"^\s{0,3}(```+|~~~+)")
SCHEME_RE = re.compile(r"^[a-zA-Z][a-zA-Z0-9+.-]*:")


@dataclass(frozen=True)
class LinkIssue:
    path: Path
    line_number: int
    target: str
    resolved_path: Path


def iter_markdown_files(root: Path) -> list[Path]:
    files: list[Path] = []
    for path in root.rglob("*.md"):
        if any(should_ignore_part(part) for part in path.relative_to(root).parts):
            continue
        files.append(path)
    return sorted(files)


def iter_scan_lines(path: Path) -> list[tuple[int, str]]:
    lines: list[tuple[int, str]] = []
    in_fence = False
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        if FENCE_RE.match(line):
            in_fence = not in_fence
            continue
        if not in_fence:
            lines.append((line_number, line))
    return lines


def extract_destination(raw: str) -> str:
    text = raw.strip()
    if text.startswith("<") and ">" in text:
        return text[1 : text.index(">")].strip()
    return text.split()[0].strip()


def is_external_or_anchor(destination: str) -> bool:
    return (
        not destination
        or destination.startswith("#")
        or destination.startswith("//")
        or SCHEME_RE.match(destination) is not None
    )


def normalize_local_target(destination: str) -> str | None:
    if is_external_or_anchor(destination):
        return None
    split = urlsplit(destination)
    target = split.path
    if not target:
        return None
    return unquote(target)


def resolve_target(root: Path, source: Path, target: str) -> Path:
    if target.startswith("/"):
        return (root / target.lstrip("/")).resolve()
    return (source.parent / target).resolve()


def target_exists(resolved_path: Path) -> bool:
    return resolved_path.exists()


def should_skip_missing_target(root: Path, source: Path, resolved_path: Path) -> bool:
    try:
        target_rel = resolved_path.relative_to(root)
    except ValueError:
        return False
    if not (root / "docs" / "sprints").exists() and target_rel.parts[:2] == ("docs", "sprints"):
        return True
    try:
        source_rel = source.relative_to(root)
    except ValueError:
        return False
    if source_rel.parts[:2] != ("docs", "sprints"):
        return False
    if not (root / "docs" / "plan.md").exists():
        return True
    if resolved_path.suffix.lower() != ".md":
        return False
    return target_rel.parts[:2] != ("docs", "sprints")


def find_broken_links(root: Path) -> list[LinkIssue]:
    normalized_root = root.resolve()
    issues: list[LinkIssue] = []
    for path in iter_markdown_files(normalized_root):
        for line_number, line in iter_scan_lines(path):
            destinations = [extract_destination(match.group(1)) for match in INLINE_LINK_RE.finditer(line)]
            destinations.extend(
                extract_destination(match.group(1)) for match in REFERENCE_DEF_RE.finditer(line)
            )
            for destination in destinations:
                target = normalize_local_target(destination)
                if target is None:
                    continue
                resolved_path = resolve_target(normalized_root, path, target)
                if not target_exists(resolved_path):
                    if should_skip_missing_target(normalized_root, path, resolved_path):
                        continue
                    issues.append(
                        LinkIssue(
                            path=path.relative_to(normalized_root),
                            line_number=line_number,
                            target=destination,
                            resolved_path=resolved_path,
                        )
                    )
    return issues


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", nargs="?", default=".", help="Repository root to scan.")
    args = parser.parse_args(argv)

    root = Path(args.root).resolve()
    issues = find_broken_links(root)
    if issues:
        print("Broken local Markdown links:")
        for issue in issues:
            print(
                f"{issue.path}:{issue.line_number}: "
                f"{issue.target!r} -> {issue.resolved_path}"
            )
        return 1

    print(f"Checked {len(iter_markdown_files(root))} Markdown files; no broken local links found.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
