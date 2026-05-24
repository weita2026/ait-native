from __future__ import annotations

from pathlib import Path

from ait_server import server_store
from ait_server.store import reviews as review_store


def test_server_store_review_lane_facade_reexports_review_module() -> None:
    assert server_store.request_review is review_store.request_review
    assert server_store.record_review is review_store.record_review
    assert server_store.list_reviews is review_store.list_reviews
    assert server_store._review_summary is review_store._review_summary
    assert server_store._required_approvals is review_store._required_approvals


def test_server_store_imports_review_lane_module() -> None:
    content = Path("src/ait_server/server_store.py").read_text(encoding="utf-8")
    assert "from .store.reviews import (" in content
    assert "_required_approvals" in content
    assert "_review_summary" in content
    assert "request_review" in content
    assert "record_review" in content
    assert "list_reviews" in content
