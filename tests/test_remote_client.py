from __future__ import annotations

import pytest

from ait import remote_client


def test_request_wraps_raw_timeout_error(monkeypatch) -> None:
    monkeypatch.setattr(remote_client, "_auth_headers", lambda: {})

    def fake_urlopen(_req, timeout=30):
        raise TimeoutError("timed out")

    monkeypatch.setattr(remote_client.urllib.request, "urlopen", fake_urlopen)

    with pytest.raises(remote_client.RemoteError, match="timed out"):
        remote_client._request("GET", "http://example.test/v1/native/sessions")


class _FakeResponse:
    def __init__(self, payload: str) -> None:
        self._payload = payload

    def __enter__(self) -> "_FakeResponse":
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        return None

    def read(self) -> bytes:
        return self._payload.encode("utf-8")


def test_run_repo_ci_uses_extended_timeout(monkeypatch) -> None:
    monkeypatch.setattr(remote_client, "_auth_headers", lambda: {})
    captured: dict[str, float] = {}

    def fake_urlopen(_req, timeout=30):
        captured["timeout"] = timeout
        return _FakeResponse('{"status":"pass"}')

    monkeypatch.setattr(remote_client.urllib.request, "urlopen", fake_urlopen)

    result = remote_client.run_repo_ci("http://example.test", "housekeeper", plane="nightly")

    assert result == {"status": "pass"}
    assert captured["timeout"] == remote_client._LONG_RUNNING_CI_REQUEST_TIMEOUT_SECONDS


def test_run_patchset_ci_uses_extended_timeout(monkeypatch) -> None:
    monkeypatch.setattr(remote_client, "_auth_headers", lambda: {})
    captured: dict[str, float] = {}

    def fake_urlopen(_req, timeout=30):
        captured["timeout"] = timeout
        return _FakeResponse('{"status":"pass"}')

    monkeypatch.setattr(remote_client.urllib.request, "urlopen", fake_urlopen)

    result = remote_client.run_patchset_ci("http://example.test", "P-1")

    assert result == {"status": "pass"}
    assert captured["timeout"] == remote_client._LONG_RUNNING_CI_REQUEST_TIMEOUT_SECONDS


def test_repo_scoped_change_helpers_skip_canonical_lookup_for_exact_ids(monkeypatch) -> None:
    monkeypatch.setattr(remote_client, "_auth_headers", lambda: {})
    captured_urls: list[str] = []

    def fake_urlopen(req, timeout=30):
        captured_urls.append(req.full_url)
        return _FakeResponse('{"ok":true}')

    def unexpected_get_change(*_args, **_kwargs):
        raise AssertionError("unexpected get_change")

    monkeypatch.setattr(remote_client, "get_change", unexpected_get_change)
    monkeypatch.setattr(remote_client.urllib.request, "urlopen", fake_urlopen)

    remote_client.publish_patchset(
        "http://example.test",
        "RC-123",
        "SNP-BASE",
        "SNP-REV",
        "summary",
        "ai_with_human_review",
        repo_name="housekeeper",
        exact_id=True,
    )
    remote_client.request_review(
        "http://example.test",
        "RC-123",
        "RP-123-1",
        ["maintainers"],
        repo_name="housekeeper",
        exact_id=True,
    )
    remote_client.list_reviews(
        "http://example.test",
        "RC-123",
        repo_name="housekeeper",
        exact_id=True,
    )
    remote_client.record_review(
        "http://example.test",
        "RC-123",
        "RP-123-1",
        "reviewer@example.com",
        "approve",
        repo_name="housekeeper",
        exact_id=True,
    )

    assert captured_urls == [
        "http://example.test/v1/native/changes/RC-123/patchsets",
        "http://example.test/v1/native/changes/RC-123:requestReview",
        "http://example.test/v1/native/changes/RC-123/reviews",
        "http://example.test/v1/native/changes/RC-123/reviews",
    ]


def test_repo_scoped_patchset_helpers_skip_canonical_lookup_for_exact_ids(monkeypatch) -> None:
    monkeypatch.setattr(remote_client, "_auth_headers", lambda: {})
    captured_urls: list[str] = []
    captured_timeouts: list[float] = []

    def fake_urlopen(req, timeout=30):
        captured_urls.append(req.full_url)
        captured_timeouts.append(timeout)
        return _FakeResponse('{"ok":true}')

    def unexpected_get_patchset(*_args, **_kwargs):
        raise AssertionError("unexpected get_patchset")

    monkeypatch.setattr(remote_client, "get_patchset", unexpected_get_patchset)
    monkeypatch.setattr(remote_client.urllib.request, "urlopen", fake_urlopen)

    remote_client.run_patchset_ci(
        "http://example.test",
        "RP-123-1",
        repo_name="housekeeper",
        exact_id=True,
    )
    remote_client.put_attestation(
        "http://example.test",
        "RP-123-1",
        "ai_with_human_review",
        {"tests": "pass"},
        repo_name="housekeeper",
        exact_id=True,
    )
    remote_client.get_attestation(
        "http://example.test",
        "RP-123-1",
        repo_name="housekeeper",
        exact_id=True,
    )
    remote_client.evaluate_policy(
        "http://example.test",
        "RP-123-1",
        repo_name="housekeeper",
        exact_id=True,
    )
    remote_client.get_policy(
        "http://example.test",
        "RP-123-1",
        repo_name="housekeeper",
        exact_id=True,
    )
    remote_client.create_waiver(
        "http://example.test",
        "RP-123-1",
        "tests",
        "reason",
        repo_name="housekeeper",
        exact_id=True,
    )

    assert captured_urls == [
        "http://example.test/v1/native/patchsets/RP-123-1:runCi",
        "http://example.test/v1/native/patchsets/RP-123-1/attestation",
        "http://example.test/v1/native/patchsets/RP-123-1/attestation",
        "http://example.test/v1/native/patchsets/RP-123-1:evaluatePolicy",
        "http://example.test/v1/native/patchsets/RP-123-1/policy",
        "http://example.test/v1/native/patchsets/RP-123-1/waivers",
    ]
    assert captured_timeouts[0] == remote_client._LONG_RUNNING_CI_REQUEST_TIMEOUT_SECONDS


def test_read_queue_summary_bundle_builds_expected_query(monkeypatch) -> None:
    monkeypatch.setattr(remote_client, "_auth_headers", lambda: {})
    captured: dict[str, str | float] = {}

    def fake_urlopen(req, timeout=30):
        captured["url"] = req.full_url
        captured["timeout"] = timeout
        return _FakeResponse('{"task_queue":{"count":1},"reviewer_inbox":{"count":0}}')

    monkeypatch.setattr(remote_client.urllib.request, "urlopen", fake_urlopen)

    result = remote_client.read_queue_summary_bundle("http://example.test", "housekeeper", status="completed")

    assert result == {"task_queue": {"count": 1}, "reviewer_inbox": {"count": 0}}
    assert captured["url"] == "http://example.test/v1/native/read/queue-summary?repo_name=housekeeper&status=completed"
    assert captured["timeout"] == remote_client._DEFAULT_REQUEST_TIMEOUT_SECONDS
