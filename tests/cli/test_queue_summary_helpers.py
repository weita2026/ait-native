from __future__ import annotations

import importlib
import threading
from pathlib import Path
from types import SimpleNamespace

from ait.cli import queue_summary_helpers
from ait.remote_client import RemoteError

cli_app_module = importlib.import_module("ait.cli.app")


def test_queue_remote_section_prefers_bundled_read_endpoint(monkeypatch) -> None:
    ctx = SimpleNamespace(root=Path("/tmp/ait"))
    task_queue_payload = {
        "count": 1,
        "items": [{"focus_change": {"change_id": "RC-1", "reason": "queue item"}}],
        "summary": {"attention_required": 1},
    }
    reviewer_inbox_payload = {
        "count": 1,
        "items": [{"change_id": "RC-1", "review_state": {"blocking": 0}}],
    }

    monkeypatch.setattr(cli_app_module, "load_config", lambda _ctx: {"repo_name": "ait", "default_remote": "origin"})
    monkeypatch.setattr(cli_app_module, "list_remotes", lambda _ctx: [{"name": "origin"}])
    monkeypatch.setattr(
        cli_app_module,
        "_remote_tuple",
        lambda _ctx, _remote_name: ({"name": "origin", "repo_name": "ait", "url": "http://example.test"}, "ait"),
    )
    monkeypatch.setattr(
        cli_app_module,
        "remote_read_queue_summary_bundle",
        lambda url, repo_name, *, status: {
            "task_queue": task_queue_payload,
            "reviewer_inbox": reviewer_inbox_payload,
        },
    )
    monkeypatch.setattr(
        cli_app_module,
        "remote_read_task_queue",
        lambda *args, **kwargs: (_ for _ in ()).throw(AssertionError("should not use fallback task queue read")),
    )
    monkeypatch.setattr(
        cli_app_module,
        "remote_read_reviewer_inbox",
        lambda *args, **kwargs: (_ for _ in ()).throw(AssertionError("should not use fallback reviewer inbox read")),
    )
    monkeypatch.setattr(
        cli_app_module,
        "remote_list_changes",
        lambda url, repo_name: [{"change_id": "RC-1", "title": "Remote queue change"}],
    )
    monkeypatch.setattr(
        cli_app_module,
        "_queue_change_inventory",
        lambda change_rows, task_items, review_items: [
            {
                "change_id": change_rows[0]["change_id"],
                "task_reason": task_items[0]["focus_change"]["reason"],
                "review_blocking": review_items[0]["review_state"]["blocking"],
            }
        ],
    )

    payload = queue_summary_helpers._queue_remote_section(ctx, None, "active", True)

    assert payload["error"] is None
    assert payload["task_queue"] == task_queue_payload
    assert payload["reviewer_inbox"] == reviewer_inbox_payload
    assert payload["changes"] == [
        {
            "change_id": "RC-1",
            "task_reason": "queue item",
            "review_blocking": 0,
        }
    ]


def test_queue_remote_section_falls_back_to_parallel_reads_when_bundle_endpoint_is_missing(monkeypatch) -> None:
    ctx = SimpleNamespace(root=Path("/tmp/ait"))
    barrier = threading.Barrier(2, timeout=1.0)
    task_queue_payload = {
        "count": 1,
        "items": [{"focus_change": {"change_id": "RC-1", "reason": "queue item"}}],
        "summary": {"attention_required": 1},
    }
    reviewer_inbox_payload = {
        "count": 1,
        "items": [{"change_id": "RC-1", "review_state": {"blocking": 0}}],
    }

    monkeypatch.setattr(cli_app_module, "load_config", lambda _ctx: {"repo_name": "ait", "default_remote": "origin"})
    monkeypatch.setattr(cli_app_module, "list_remotes", lambda _ctx: [{"name": "origin"}])
    monkeypatch.setattr(
        cli_app_module,
        "_remote_tuple",
        lambda _ctx, _remote_name: ({"name": "origin", "repo_name": "ait", "url": "http://example.test"}, "ait"),
    )
    monkeypatch.setattr(
        cli_app_module,
        "remote_read_queue_summary_bundle",
        lambda url, repo_name, *, status: (_ for _ in ()).throw(
            RemoteError("GET http://example.test/v1/native/read/queue-summary?repo_name=ait&status=active failed: 404 Not Found")
        ),
    )

    def fake_task_queue(url: str, repo_name: str, *, status: str):
        assert url == "http://example.test"
        assert repo_name == "ait"
        assert status == "active"
        barrier.wait()
        return task_queue_payload

    def fake_reviewer_inbox(url: str, repo_name: str):
        assert url == "http://example.test"
        assert repo_name == "ait"
        barrier.wait()
        return reviewer_inbox_payload

    monkeypatch.setattr(cli_app_module, "remote_read_task_queue", fake_task_queue)
    monkeypatch.setattr(cli_app_module, "remote_read_reviewer_inbox", fake_reviewer_inbox)
    monkeypatch.setattr(
        cli_app_module,
        "remote_list_changes",
        lambda url, repo_name: [{"change_id": "RC-1", "title": "Remote queue change"}],
    )
    monkeypatch.setattr(
        cli_app_module,
        "_queue_change_inventory",
        lambda change_rows, task_items, review_items: [
            {
                "change_id": change_rows[0]["change_id"],
                "task_reason": task_items[0]["focus_change"]["reason"],
                "review_blocking": review_items[0]["review_state"]["blocking"],
            }
        ],
    )

    payload = queue_summary_helpers._queue_remote_section(ctx, None, "active", True)

    assert payload["error"] is None
    assert payload["task_queue"] == task_queue_payload
    assert payload["reviewer_inbox"] == reviewer_inbox_payload
    assert payload["changes"] == [
        {
            "change_id": "RC-1",
            "task_reason": "queue item",
            "review_blocking": 0,
        }
    ]


def test_queue_remote_section_reports_remote_error_from_parallel_reads(monkeypatch) -> None:
    ctx = SimpleNamespace(root=Path("/tmp/ait"))

    monkeypatch.setattr(cli_app_module, "load_config", lambda _ctx: {"repo_name": "ait", "default_remote": "origin"})
    monkeypatch.setattr(cli_app_module, "list_remotes", lambda _ctx: [{"name": "origin"}])
    monkeypatch.setattr(
        cli_app_module,
        "_remote_tuple",
        lambda _ctx, _remote_name: ({"name": "origin", "repo_name": "ait", "url": "http://example.test"}, "ait"),
    )
    monkeypatch.setattr(
        cli_app_module,
        "remote_read_queue_summary_bundle",
        lambda url, repo_name, *, status: (_ for _ in ()).throw(
            RemoteError("GET http://example.test/v1/native/read/queue-summary?repo_name=ait&status=active failed: 404 Not Found")
        ),
    )
    monkeypatch.setattr(
        cli_app_module,
        "remote_read_task_queue",
        lambda url, repo_name, *, status: (_ for _ in ()).throw(RemoteError("task queue unavailable")),
    )
    monkeypatch.setattr(
        cli_app_module,
        "remote_read_reviewer_inbox",
        lambda url, repo_name: {"count": 0, "items": []},
    )

    payload = queue_summary_helpers._queue_remote_section(ctx, None, "active", False)

    assert payload["task_queue"] is None
    assert payload["reviewer_inbox"] is None
    assert payload["changes"] is None
    assert payload["error"] == "task queue unavailable"
