from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

import pytest


REPO_ROOT = Path(__file__).resolve().parents[1]
REPO_IGNORE = REPO_ROOT / ".ignore"
REQUIRED_PATTERNS = {
    ".ait/",
    ".ait-server/",
    "build/",
    "dist/",
    "htmlcov/",
    ".coverage*",
    ".mypy_cache/",
    ".nox/",
    ".pytest_cache/",
    ".ruff_cache/",
    ".tox/",
    ".venv/",
    "__pycache__/",
    "node_modules/",
    "venv/",
    "*.egg-info/",
}


def _ignore_lines(path: Path) -> list[str]:
    return [
        line.strip()
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    ]


def test_repo_ignore_covers_runtime_roots_and_tool_noise() -> None:
    ignore_lines = set(_ignore_lines(REPO_IGNORE))
    missing = REQUIRED_PATTERNS - ignore_lines
    assert not missing, f"Missing .ignore patterns: {sorted(missing)}"


def test_ripgrep_hidden_file_listing_honors_repo_ignore(tmp_path: Path) -> None:
    rg = shutil.which("rg")
    if rg is None:
        pytest.skip("ripgrep is not installed in this test environment")

    (tmp_path / ".ignore").write_text(
        REPO_IGNORE.read_text(encoding="utf-8"),
        encoding="utf-8",
    )

    hidden_runtime = tmp_path / ".ait"
    hidden_runtime.mkdir()
    (hidden_runtime / "state.txt").write_text("runtime", encoding="utf-8")

    hidden_server = tmp_path / ".ait-server"
    hidden_server.mkdir()
    (hidden_server / "pid.txt").write_text("server", encoding="utf-8")

    build_dir = tmp_path / "build"
    build_dir.mkdir()
    (build_dir / "artifact.txt").write_text("artifact", encoding="utf-8")

    src_dir = tmp_path / "src"
    src_dir.mkdir()
    (src_dir / "visible.txt").write_text("visible", encoding="utf-8")

    result = subprocess.run(
        [rg, "--files", "--hidden"],
        cwd=tmp_path,
        check=True,
        capture_output=True,
        text=True,
    )

    files = set(result.stdout.splitlines())
    assert "src/visible.txt" in files
    assert ".ait/state.txt" not in files
    assert ".ait-server/pid.txt" not in files
    assert "build/artifact.txt" not in files
