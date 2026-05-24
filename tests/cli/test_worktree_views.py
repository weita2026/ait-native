from __future__ import annotations

import importlib
from pathlib import Path

from ait.cli import worktree_views

cli_app_module = importlib.import_module("ait.cli.app")


def test_cli_app_reexports_extracted_worktree_view_helpers() -> None:
    helper_names = [
        "_worktree_runtime_paths",
        "_worktree_runtime_env",
        "_worktree_shell_command",
        "_render_workspace_status",
        "_render_worktrees",
        "_render_worktree_doctor",
        "_render_repo_status",
    ]

    for name in helper_names:
        assert getattr(cli_app_module, name) is getattr(worktree_views, name)


def test_worktree_runtime_helpers_preserve_shell_and_env_contract(tmp_path: Path) -> None:
    worktree_root = tmp_path / "rt-1146"
    (worktree_root / "src").mkdir(parents=True)
    (worktree_root / ".venv" / "bin").mkdir(parents=True)
    payload = {"name": "rt-1146", "current_line": "feature/rt-1146"}

    paths = worktree_views._worktree_runtime_paths(str(worktree_root))
    assert paths["src_path"] == str((worktree_root / "src").resolve())
    assert paths["venv_path"] == str((worktree_root / ".venv").resolve())
    assert paths["venv_bin_path"] == str((worktree_root / ".venv" / "bin").resolve())

    env = worktree_views._worktree_runtime_env(
        str(worktree_root),
        payload,
        base_env={"PATH": "/usr/bin", "PYTHONPATH": "existing"},
    )
    assert env["AIT_WORKTREE_NAME"] == "rt-1146"
    assert env["AIT_WORKTREE_PATH"] == str(worktree_root)
    assert env["AIT_WORKTREE_LINE"] == "feature/rt-1146"
    assert env["PYTHONPATH"] == f"{paths['src_path']}:existing"
    assert env["PATH"] == f"{paths['venv_bin_path']}:/usr/bin"

    shell_command = worktree_views._worktree_shell_command(str(worktree_root), payload)
    assert f"cd {str(worktree_root)}" in shell_command
    assert "export AIT_WORKTREE_NAME=rt-1146" in shell_command
    assert "export AIT_WORKTREE_LINE=feature/rt-1146" in shell_command
    assert f"export PYTHONPATH={paths['src_path']}" in shell_command
    assert f"export PATH={paths['venv_bin_path']}:$PATH" in shell_command


def test_workspace_baseline_label_and_render_status_smoke(capsys) -> None:
    assert worktree_views._workspace_baseline_label(
        {"baseline_source": "snapshot", "baseline_snapshot_id": "SNP-1"}
    ) == "snapshot SNP-1"
    assert worktree_views._workspace_baseline_label(
        {
            "baseline_source": "current_line_head",
            "baseline_snapshot_id": "SNP-2",
            "baseline_line_name": "main",
        }
    ) == "current line head main (SNP-2)"

    worktree_views._render_workspace_status(
        {
            "clean": True,
            "repo_name": "ait",
            "workspace_root": "/tmp/repo",
            "current_line": "feature/rt-1146",
            "baseline_source": "snapshot",
            "baseline_snapshot_id": "SNP-1",
            "changed_count": 0,
        }
    )
    captured = capsys.readouterr().out
    assert "ait workspace status" in captured
    assert "Workspace is clean" in captured
