from __future__ import annotations

from pathlib import Path

from ait.repo_paths import RepoContext

WORKSPACE_ROOT = Path(__file__).resolve().parents[1]
ROOT = RepoContext.discover(WORKSPACE_ROOT).repo_root
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


def iter_markdown_files() -> list[Path]:
    files: list[Path] = []
    for path in ROOT.rglob("*.md"):
        relative_parts = path.relative_to(ROOT).parts
        if any(part in IGNORED_DIRS for part in relative_parts):
            continue
        if relative_parts[:2] == ("docs", "benchmarks"):
            continue
        if relative_parts == ("site", "README.md"):
            continue
        files.append(path)
    return sorted(files)


def test_markdown_files_declare_authority_status_and_scope() -> None:
    missing: list[str] = []
    for path in iter_markdown_files():
        header = "\n".join(path.read_text(encoding="utf-8").splitlines()[:20])
        for field in ("Authority:", "Status:", "Scope:"):
            if field not in header:
                missing.append(f"{path.relative_to(ROOT)} missing {field}")

    assert missing == []
