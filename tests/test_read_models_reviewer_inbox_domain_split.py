from __future__ import annotations

from pathlib import Path

from ait_server import read_models
from ait_server.read_models_domains import reviewer_inbox as reviewer_inbox_domain


def test_read_models_reviewer_inbox_facade_reexports_domain_module() -> None:
    assert read_models.reviewer_inbox is reviewer_inbox_domain.reviewer_inbox


def test_read_models_imports_reviewer_inbox_domain_module() -> None:
    text = Path(read_models.__file__).read_text(encoding="utf-8")

    assert "from .read_models_domains.reviewer_inbox import reviewer_inbox" in text
