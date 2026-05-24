from __future__ import annotations

import os
import re
import subprocess
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]


def test_plan_cli_contract_script_passes():
    script = REPO_ROOT / "scripts" / "check_plan_cli_contracts.py"
    completed = subprocess.run(
        [sys.executable, str(script)],
        cwd=REPO_ROOT,
        check=False,
        capture_output=True,
        text=True,
        env={**os.environ, "PYTHONPATH": str(REPO_ROOT / "src")},
    )
    assert completed.returncode == 0, completed.stdout + completed.stderr


def test_plan_cli_contract_script_guards_legacy_line_alignment_surface():
    script = REPO_ROOT / "scripts" / "check_plan_cli_contracts.py"
    text = script.read_text(encoding="utf-8")

    for required in ("line_sync", "root_main_sync", "remote_main_sync", "--default-line"):
        assert required in text


def test_plan_source_files_omit_legacy_line_alignment_contract():
    command_module = (REPO_ROOT / "src" / "ait" / "cli" / "commands" / "plan.py").read_text(encoding="utf-8")
    for forbidden in ("line_sync", "root_main_sync", "remote_main_sync", "--default-line"):
        assert forbidden not in command_module

    app_module = (REPO_ROOT / "src" / "ait" / "cli" / "app.py").read_text(encoding="utf-8")
    for forbidden in ("line_sync", "root_main_sync", "remote_main_sync"):
        assert forbidden not in app_module
    assert not re.search(r"(?is)plan sync.{0,400}--default-line", app_module)
    assert not re.search(r"(?is)--default-line.{0,400}plan sync", app_module)


def test_plan_cli_contract_script_scans_plan_implementation_sources():
    script = REPO_ROOT / "scripts" / "check_plan_cli_contracts.py"
    text = script.read_text(encoding="utf-8")

    assert "src\" / \"ait\" / \"cli\" / \"commands\" / \"plan.py" in text
    assert "src\" / \"ait\" / \"cli\" / \"app.py" in text
    assert "_assert_plan_source_files_omit_legacy_line_alignment_contract" in text


def test_plan_cli_contract_script_guards_plan_sync_root_worktree_bypass():
    script = REPO_ROOT / "scripts" / "check_plan_cli_contracts.py"
    text = script.read_text(encoding="utf-8")

    assert "_assert_plan_sync_bypasses_root_worktree_guard" in text
    assert "Repo root is pinned to bound worktree" in text
    assert "[\"plan\", \"sync\"" in text
