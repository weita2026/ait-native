from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import sys
import textwrap
from uuid import uuid4

import pytest
from typer.testing import CliRunner

from ait.cli.app import app

from tests._ram_root import detect_host_ram_root, managed_host_ram_root, remove_host_ram_root_children


runner = CliRunner()
ROOT = Path(__file__).resolve().parents[1]


def _host_ram_root_children(root: Path) -> set[str]:
    auto_root = root / ".ait-repos"
    if not auto_root.exists():
        return set()
    return {child.name for child in auto_root.iterdir() if child.is_dir()}


def _run_subprocess_pytest(nodeid: str, *, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    merged_env = os.environ.copy()
    if env:
        merged_env.update(env)
    return subprocess.run(
        [sys.executable, "-m", "pytest", "-q", nodeid],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
        env=merged_env,
    )


def _assert_subprocess_test_keeps_host_ram_root_clean(nodeid: str) -> None:
    root = detect_host_ram_root()
    if root is None:
        pytest.skip("No host memory-backed root is available on this machine.")

    before_children = _host_ram_root_children(root)
    result = _run_subprocess_pytest(
        nodeid,
        env={"AIT_TEST_DISABLE_GLOBAL_HOST_RAM_ROOT_CLEANUP": "1"},
    )
    after_children = _host_ram_root_children(root)
    leaked = sorted(after_children - before_children)
    try:
        assert result.returncode == 0, f"{result.stdout}\n{result.stderr}"
        assert leaked == [], leaked
    finally:
        if leaked:
            remove_host_ram_root_children(root, leaked)


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
        "memory_root": {"value": None, "source": "built_in"},
        "main_seed_ram_max_bytes": {"value": None, "source": "built_in"},
    }


def test_managed_host_ram_root_removes_new_repo_hash_dirs(disable_host_ram_root_cleanup):
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


def test_managed_host_ram_root_supports_nested_guards(disable_host_ram_root_cleanup):
    root = detect_host_ram_root()
    if root is None:
        pytest.skip("No host memory-backed root is available on this machine.")

    probe_hash_dir = root / ".ait-repos" / f"pytest-host-ram-root-nested-{uuid4().hex}"
    probe_repo_dir = probe_hash_dir / "housekeeper-probe"
    assert not probe_hash_dir.exists()

    with managed_host_ram_root(root):
        with managed_host_ram_root(root):
            probe_repo_dir.mkdir(parents=True)
            (probe_repo_dir / "README.md").write_text("probe\n", encoding="utf-8")
            assert probe_repo_dir.exists()
        assert not probe_hash_dir.exists()

    assert not probe_hash_dir.exists()


def test_pytest_harness_autocleans_host_ram_root_after_each_test(disable_host_ram_root_cleanup):
    root = detect_host_ram_root()
    if root is None:
        pytest.skip("No host memory-backed root is available on this machine.")

    probe_name = f"pytest-autouse-host-ram-{uuid4().hex}"
    probe_hash_dir = root / ".ait-repos" / probe_name
    probe_test = ROOT / "tests" / f"_tmp_host_ram_cleanup_probe_{uuid4().hex}.py"
    try:
        probe_test.write_text(
            textwrap.dedent(
                f"""
                from pathlib import Path

                def test_probe_host_ram_cleanup():
                    probe_repo_dir = Path({str(probe_hash_dir)!r}) / "housekeeper-probe"
                    probe_repo_dir.mkdir(parents=True)
                    (probe_repo_dir / "README.md").write_text("probe\\n", encoding="utf-8")
                    assert probe_repo_dir.exists()
                """
            ),
            encoding="utf-8",
        )

        result = subprocess.run(
            [sys.executable, "-m", "pytest", "-q", str(probe_test)],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
    finally:
        probe_test.unlink(missing_ok=True)

    assert result.returncode == 0, f"{result.stdout}\n{result.stderr}"
    assert not probe_hash_dir.exists()


def test_demo_suite_explicit_host_ram_cleanup_survives_without_ambient_global_guard(disable_host_ram_root_cleanup):
    _assert_subprocess_test_keeps_host_ram_root_clean(
        "tests/test_task_dag_cli.py::test_demo_suite_explicit_host_ram_cleanup_contract_creates_probe_root"
    )


def test_batch_suite_explicit_host_ram_cleanup_survives_without_ambient_global_guard(disable_host_ram_root_cleanup):
    _assert_subprocess_test_keeps_host_ram_root_clean(
        "tests/cli/test_workflow_land_batch.py::test_batch_suite_explicit_host_ram_cleanup_contract_creates_probe_root"
    )
