from __future__ import annotations

import json
from pathlib import Path
from uuid import uuid4

import pytest
from typer.testing import CliRunner

from ait.cli.app import app

from tests._ram_root import detect_host_ram_root, managed_host_ram_root


runner = CliRunner()


def test_pytest_harness_disables_ambient_memory_root_detection(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-harness-default"
    repo.mkdir()
    monkeypatch.chdir(repo)

    result = runner.invoke(app, ["init", "--name", "housekeeper", "--json"], catch_exceptions=False)

    assert result.exit_code == 0, result.stdout
    payload = json.loads(result.stdout)
    assert payload["task_worktree"] == {
        "ephemeral_root": {"value": None, "source": "built_in"},
        "alias_root": {"value": ".ait/worktree-links", "source": "built_in"},
    }


def test_managed_host_ram_root_removes_new_repo_hash_dirs():
    root = detect_host_ram_root()
    if root is None:
        pytest.skip("No host memory-backed root is available on this machine.")

    probe_hash_dir = root / ".ait-repos" / f"pytest-host-ram-root-cleanup-{uuid4().hex}"
    probe_repo_dir = probe_hash_dir / "housekeeper-probe"
    assert not probe_hash_dir.exists()

    with managed_host_ram_root(root):
        probe_repo_dir.mkdir(parents=True)
        (probe_repo_dir / "README.md").write_text("probe\n", encoding="utf-8")
        assert probe_repo_dir.exists()

    assert not probe_hash_dir.exists()
