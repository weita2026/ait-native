from __future__ import annotations

from pathlib import Path

from ait_server import read_models
from ait_server.read_models_domains import task_queue as task_queue_domain


def test_read_models_task_queue_facade_reexports_domain_module() -> None:
    assert read_models.task_queue is task_queue_domain.task_queue
    assert read_models.task_audit is task_queue_domain.task_audit


def test_read_models_imports_task_queue_domain_module() -> None:
    text = Path(read_models.__file__).read_text(encoding="utf-8")

    assert "from .read_models_domains.task_queue import (" in text
