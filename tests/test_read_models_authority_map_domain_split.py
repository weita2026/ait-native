from __future__ import annotations

from pathlib import Path

from ait_server import read_models
from ait_server.read_models_domains import authority_map as authority_map_domain

WORKSPACE_ROOT = Path(__file__).resolve().parents[1]


def test_read_models_authority_map_facade_reexports_domain_module() -> None:
    assert read_models.authority_map is authority_map_domain.authority_map


def test_read_models_imports_authority_map_domain_module() -> None:
    content = (WORKSPACE_ROOT / "src/ait_server/read_models.py").read_text(encoding="utf-8")
    assert "from .read_models_domains.authority_map import authority_map" in content
