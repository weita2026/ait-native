from __future__ import annotations

from pathlib import Path

from ait_server import read_models
from ait_server.read_models_domains import repository_overview as repository_overview_domain


def test_read_models_repository_overview_facades_reexport_domain_module() -> None:
    assert read_models.repository_index is repository_overview_domain.repository_index
    assert read_models.repository_detail is repository_overview_domain.repository_detail
    assert read_models.repository_worker_status is repository_overview_domain.repository_worker_status


def test_read_models_imports_repository_overview_domain_module() -> None:
    content = Path("src/ait_server/read_models.py").read_text(encoding="utf-8")
    assert "from .read_models_domains.repository_overview import (" in content
