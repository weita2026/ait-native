from __future__ import annotations

from pathlib import Path


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]


def test_release_ops_build_helpers_are_extracted_into_builder_module() -> None:
    release_ops_text = (WORKSPACE_ROOT / "src/ait/release_ops.py").read_text(encoding="utf-8")
    builder_text = (WORKSPACE_ROOT / "src/ait/release_artifact_builder.py").read_text(encoding="utf-8")

    assert "def _build_environment(" not in release_ops_text
    assert "def _build_sdist(" not in release_ops_text
    assert "def _build_wheel(" not in release_ops_text
    assert "from .release_artifact_builder import build_release_candidate as _impl" in release_ops_text
    assert "from .release_artifact_builder import generate_release_formula as _impl" in release_ops_text

    assert "def _build_environment(" in builder_text
    assert "def _build_sdist(" in builder_text
    assert "def _build_wheel(" in builder_text
    assert "def build_release_candidate(" in builder_text
    assert "def generate_release_formula(" in builder_text


def test_release_cli_uses_release_artifact_builder_narrow_seam() -> None:
    cli_text = (WORKSPACE_ROOT / "src/ait/cli/commands/release.py").read_text(encoding="utf-8")

    assert "from ...release_artifact_builder import (" in cli_text
    assert "build_release_candidate," in cli_text
    assert "generate_release_formula," in cli_text
    assert "from ...release_ops import (" in cli_text
    assert "create_release_candidate," in cli_text
    assert "get_release_candidate," in cli_text
