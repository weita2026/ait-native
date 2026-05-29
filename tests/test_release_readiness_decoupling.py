from __future__ import annotations

from pathlib import Path


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]


def test_release_ops_readiness_helpers_are_extracted_into_readiness_module() -> None:
    release_ops_text = (WORKSPACE_ROOT / "src/ait/release_ops.py").read_text(encoding="utf-8")
    readiness_text = (WORKSPACE_ROOT / "src/ait/release_readiness.py").read_text(encoding="utf-8")

    assert "def _workspace_path_exists(" not in release_ops_text
    assert "def _markdown_link_audit(" not in release_ops_text
    assert "def _scan_private_surface(" not in release_ops_text
    assert "def _check_result(" not in release_ops_text
    assert "def _run_command(" not in release_ops_text
    assert "from .release_readiness import run_release_checks as _impl" in release_ops_text

    assert "def _workspace_path_exists(" in readiness_text
    assert "def _markdown_link_audit(" in readiness_text
    assert "def _scan_private_surface(" in readiness_text
    assert "def _check_result(" in readiness_text
    assert "def _run_command(" in readiness_text
    assert "def run_release_checks(" in readiness_text


def test_release_cli_uses_release_readiness_narrow_seam() -> None:
    cli_text = (WORKSPACE_ROOT / "src/ait/cli/commands/release.py").read_text(encoding="utf-8")

    assert "from ...release_readiness import (" in cli_text
    assert "run_release_checks," in cli_text
    assert "from ...release_ops import (" in cli_text
    assert "create_release_candidate," in cli_text
    assert "get_release_candidate," in cli_text
