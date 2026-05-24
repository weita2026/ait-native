from __future__ import annotations

import threading

import pytest

from ait_server.live_turns import (
    LiveTurnRegistry,
    export_live_turns_snapshot,
    finish_live_turn,
    get_live_turn_registry,
    reset_live_turns_for_tests,
    snapshot_live_turn_metrics,
    start_live_turn,
)


class FakeClock:
    def __init__(self, initial: float = 0.0) -> None:
        self.now = float(initial)

    def __call__(self) -> float:
        return self.now

    def advance(self, seconds: float) -> None:
        self.now += float(seconds)


def test_live_turn_registry_snapshot_tracks_active_turn_counts_and_repo_breakdown():
    clock = FakeClock(100.0)
    registry = LiveTurnRegistry(time_fn=clock, recent_completed_limit=5)

    first = registry.start(repo_name="ait", session_id="S-1", surface="telegram")
    clock.advance(5.0)
    second = registry.start(repo_name="ACC", session_id="S-2", surface="editor")
    clock.advance(2.0)
    third = registry.start(repo_name="ait", session_id="S-3")
    snapshot = registry.snapshot()

    assert first != second
    assert second != third
    assert snapshot == {
        "active_turns": 3,
        "active_repositories": {"ACC": 1, "ait": 2},
        "oldest_active_turn_started_at": 100.0,
        "oldest_active_turn_age_seconds": 7.0,
        "recent_completed_turns": [],
        "recent_failed_turns": [],
        "recent_completed_p95_seconds": None,
        "snapshot_at_epoch_seconds": 107.0,
        "active_turn_count": 3,
        "oldest_active_turn_started_at_epoch_seconds": 100.0,
        "active_turns_by_repo": {"ACC": 1, "ait": 2},
        "recent_completed_turn_count": 0,
        "recent_failed_turn_count": 0,
    }


def test_live_turn_registry_finish_moves_turn_into_recent_completed_summary():
    clock = FakeClock(10.0)
    registry = LiveTurnRegistry(time_fn=clock, recent_completed_limit=5)

    token = registry.start(
        repo_name="ait",
        session_id="session-1",
        surface="telegram",
        metadata={"chat_id": "42"},
    )
    clock.advance(3.25)
    finished = registry.finish(token, outcome="ok", response_id="resp-1")
    snapshot = registry.snapshot()

    assert finished == {
        "turn_token": token,
        "repo_name": "ait",
        "session_id": "session-1",
        "surface": "telegram",
        "started_at_epoch_seconds": 10.0,
        "finished_at_epoch_seconds": 13.25,
        "duration_seconds": 3.25,
        "outcome": "ok",
        "failed": False,
        "metadata": {"chat_id": "42"},
        "completion_metadata": {"response_id": "resp-1"},
    }
    assert snapshot["active_turns"] == 0
    assert snapshot["active_turn_count"] == 0
    assert snapshot["oldest_active_turn_started_at"] is None
    assert snapshot["oldest_active_turn_started_at_epoch_seconds"] is None
    assert snapshot["oldest_active_turn_age_seconds"] is None
    assert snapshot["recent_completed_turn_count"] == 1
    assert snapshot["recent_completed_turns"] == [finished]
    assert snapshot["recent_failed_turns"] == []
    assert snapshot["recent_completed_p95_seconds"] == 3.25


def test_live_turn_registry_recent_completion_history_tracks_failures_and_p95():
    clock = FakeClock(200.0)
    registry = LiveTurnRegistry(time_fn=clock, recent_completed_limit=3)

    token1 = registry.start(repo_name="repo-1", session_id="S-1")
    clock.advance(1.0)
    completed1 = registry.finish(token1, outcome="done-1")

    token2 = registry.start(repo_name="repo-2", session_id="S-2")
    clock.advance(4.0)
    completed2 = registry.finish(token2, outcome="done-2")

    token3 = registry.start(repo_name="repo-3", session_id="S-3")
    clock.advance(2.0)
    failed = registry.finish(token3, error="timeout")

    snapshot = registry.snapshot()

    assert snapshot["recent_completed_turn_count"] == 2
    assert snapshot["recent_failed_turn_count"] == 1
    assert snapshot["recent_completed_turns"] == [completed2, completed1]
    assert snapshot["recent_failed_turns"] == [failed]
    assert snapshot["recent_completed_p95_seconds"] == 4.0


def test_live_turn_registry_finish_unknown_token_is_a_noop():
    registry = LiveTurnRegistry(time_fn=FakeClock(50.0))
    token = registry.start(repo_name="ait")

    assert registry.finish("missing-token") == {}

    finished = registry.finish(token)
    assert finished
    assert registry.finish(token) == {}


def test_live_turn_registry_rejects_invalid_arguments():
    with pytest.raises(ValueError, match="recent_completed_limit"):
        LiveTurnRegistry(recent_completed_limit=0)

    registry = LiveTurnRegistry()
    with pytest.raises(ValueError, match="repo_name"):
        registry.start(repo_name=" ")
    with pytest.raises(ValueError, match="recent_completed_limit"):
        registry.snapshot(recent_completed_limit=-1)


def test_live_turn_registry_handles_parallel_start_and_finish():
    registry = LiveTurnRegistry(time_fn=FakeClock(100.0), recent_completed_limit=16)
    barrier = threading.Barrier(5)
    errors: list[BaseException] = []
    completed_tokens: list[str] = []
    completed_tokens_lock = threading.Lock()

    def worker(idx: int) -> None:
        try:
            barrier.wait(timeout=5)
            token = registry.start(repo_name=f"repo-{idx % 2}", session_id=f"S-{idx}")
            barrier.wait(timeout=5)
            finished = registry.finish(token, outcome="ok")
            assert finished is not None
            with completed_tokens_lock:
                completed_tokens.append(token)
        except BaseException as exc:  # pragma: no cover - assertion forwarded after join
            errors.append(exc)

    threads = [threading.Thread(target=worker, args=(idx,)) for idx in range(5)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(timeout=5)

    assert not errors
    assert len(completed_tokens) == 5
    assert len(set(completed_tokens)) == 5

    snapshot = registry.snapshot()
    assert snapshot["active_turn_count"] == 0
    assert snapshot["recent_completed_turn_count"] == 5
    assert snapshot["recent_failed_turn_count"] == 0


def test_module_level_helpers_share_the_default_registry_and_support_reset():
    reset_live_turns_for_tests()
    try:
        assert get_live_turn_registry() is get_live_turn_registry()
        token = start_live_turn(repo_name="ait", session_id="session-1", surface="editor")
        active_snapshot = snapshot_live_turn_metrics()
        assert active_snapshot["active_turns"] == 1
        assert active_snapshot["active_repositories"] == {"ait": 1}

        finished = finish_live_turn(token, outcome="ok")
        assert finished
        final_snapshot = export_live_turns_snapshot()
        assert final_snapshot["active_turn_count"] == 0
        assert final_snapshot["recent_completed_turn_count"] == 1
    finally:
        reset_live_turns_for_tests()
        assert export_live_turns_snapshot()["active_turn_count"] == 0
