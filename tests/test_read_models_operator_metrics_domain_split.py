from __future__ import annotations

from pathlib import Path

from ait_server import read_models
from ait_server.read_models_domains import operator_metrics as operator_metrics_domain


def test_read_models_operator_metrics_facade_reexports_domain_module() -> None:
    assert read_models.server_metrics is operator_metrics_domain.server_metrics
    assert read_models.server_readiness is operator_metrics_domain.server_readiness
    assert read_models.operator_pressure_cache_ttl_seconds is operator_metrics_domain.operator_pressure_cache_ttl_seconds


def test_read_models_imports_operator_metrics_domain_module() -> None:
    text = Path(read_models.__file__).read_text(encoding="utf-8")

    assert "from .read_models_domains.operator_metrics import (" in text
