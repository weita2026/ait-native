from __future__ import annotations

from importlib import import_module
from pathlib import Path

cli_app_module = import_module("ait.cli.app")
queue_module = import_module("ait.cli.commands.queue")
land_module = import_module("ait.cli.commands.queue_workflow_land")
state_module = import_module("ait.cli.workflow_land_state")
publish_module = import_module("ait.cli.workflow_land_publish")
review_submission_helpers = import_module("ait.cli.review_submission_helpers")


def _build_ctx() -> object:
    class _Ctx:
        is_worktree = False
        root = Path("__test_repo_root__")

    return _Ctx()


def test_workflow_land_payload_forwards_repo_name_to_repo_scoped_helpers(monkeypatch):
    calls: list[tuple] = []

    def fake_remote_tuple(ctx, remote_name):
        calls.append(("remote_tuple", remote_name))
        return {"url": "http://example.test"}, "repo-alpha"

    def fake_remote_get_change(base_url, change_id, *, repo_name=None):
        calls.append(("get_change", base_url, change_id, repo_name))
        return {
            "change_id": change_id,
            "task_id": "T-1",
            "base_line": "main",
            "selected_patchset_id": "P-1",
            "current_patchset_id": "P-1",
        }

    def fake_remote_get_task(base_url, task_id, *, repo_name=None):
        calls.append(("get_task", base_url, task_id, repo_name))
        return {
            "task_id": task_id,
            "status": "active",
            "repo_name": "repo-alpha",
            "title": "task-title",
            "risk_tier": "medium",
        }

    def fake_remote_get_patchset(base_url, patchset_id, *, repo_name=None):
        calls.append(("get_patchset", base_url, patchset_id, repo_name))
        return {
            "patchset_id": patchset_id,
            "change_id": "C-1",
            "base_snapshot_id": "SNP-BASE",
            "revision_snapshot_id": "SNP-REV",
        }

    def fake_remote_list_reviews(base_url, change_id, *, repo_name=None, exact_id=False):
        calls.append(("list_reviews", base_url, change_id, repo_name, exact_id))
        return {"reviews": []}

    def fake_remote_get_attestation(base_url, patchset_id, *, repo_name=None, exact_id=False):
        calls.append(("get_attestation", base_url, patchset_id, repo_name, exact_id))
        return None

    def fake_remote_get_policy(base_url, patchset_id, *, repo_name=None, exact_id=False):
        calls.append(("get_policy", base_url, patchset_id, repo_name, exact_id))
        return {"decision": "pass", "patchset_id": patchset_id}

    monkeypatch.setattr(state_module, "_remote_tuple", fake_remote_tuple)
    monkeypatch.setattr(state_module, "remote_get_change", fake_remote_get_change)
    monkeypatch.setattr(state_module, "remote_get_task", fake_remote_get_task)
    monkeypatch.setattr(state_module, "remote_get_patchset", fake_remote_get_patchset)
    monkeypatch.setattr(state_module, "remote_list_reviews", fake_remote_list_reviews)
    monkeypatch.setattr(state_module, "remote_get_policy", fake_remote_get_policy)
    monkeypatch.setattr(state_module, "remote_get_attestation", fake_remote_get_attestation)
    monkeypatch.setattr(state_module, "_current_worktree_retarget_state", lambda ctx, change_id: None)
    monkeypatch.setattr(state_module, "local_workspace_status", lambda ctx: {"current_line": "main", "baseline_snapshot_id": "SNP-BASE"})
    monkeypatch.setattr(state_module, "current_line", lambda ctx: "main")
    monkeypatch.setattr(state_module, "get_line", lambda ctx, line_name: {"head_snapshot_id": "SNP-REV"})
    monkeypatch.setattr(state_module, "get_remote_line", lambda base_url, repo_name, line_name: {"head_snapshot_id": "SNP-BASE"})

    queue_module._workflow_land_payload(_build_ctx(), change_id="C-1", patchset_id=None, remote_name="origin")

    assert ("get_change", "http://example.test", "C-1", "repo-alpha") in calls
    assert ("get_task", "http://example.test", "T-1", "repo-alpha") in calls
    assert ("get_patchset", "http://example.test", "P-1", "repo-alpha") in calls
    assert ("list_reviews", "http://example.test", "C-1", "repo-alpha", True) in calls
    assert ("get_attestation", "http://example.test", "P-1", "repo-alpha", True) in calls
    assert ("get_policy", "http://example.test", "P-1", "repo-alpha", True) in calls


def test_workflow_land_payload_short_circuits_remote_hydration_for_landed_changes(monkeypatch):
    calls: list[tuple] = []

    def fake_remote_tuple(ctx, remote_name):
        calls.append(("remote_tuple", remote_name))
        return {"url": "http://example.test"}, "repo-alpha"

    def fake_remote_get_change(base_url, change_id, *, repo_name=None):
        calls.append(("get_change", base_url, change_id, repo_name))
        return {
            "change_id": change_id,
            "task_id": "T-1",
            "status": "landed",
            "base_line": "main",
            "selected_patchset_id": "P-1",
            "current_patchset_id": "P-1",
        }

    def fake_remote_get_task(base_url, task_id, *, repo_name=None):
        calls.append(("get_task", base_url, task_id, repo_name))
        return {
            "task_id": task_id,
            "status": "active",
            "repo_name": "repo-alpha",
            "title": "task-title",
            "risk_tier": "medium",
        }

    def _unexpected(*args, **kwargs):
        raise AssertionError("landed fast path should not hydrate patchset/review/policy state")

    monkeypatch.setattr(state_module, "_remote_tuple", fake_remote_tuple)
    monkeypatch.setattr(state_module, "remote_get_change", fake_remote_get_change)
    monkeypatch.setattr(state_module, "remote_get_task", fake_remote_get_task)
    monkeypatch.setattr(state_module, "remote_get_patchset", _unexpected)
    monkeypatch.setattr(state_module, "remote_list_reviews", _unexpected)
    monkeypatch.setattr(state_module, "remote_get_policy", _unexpected)
    monkeypatch.setattr(state_module, "remote_get_attestation", _unexpected)
    monkeypatch.setattr(state_module, "get_remote_line", _unexpected)
    monkeypatch.setattr(state_module, "_current_worktree_retarget_state", _unexpected)
    monkeypatch.setattr(
        state_module,
        "local_workspace_status",
        lambda ctx: {"clean": False, "changed_count": 2, "current_line": "main", "baseline_snapshot_id": "SNP-REV"},
    )
    monkeypatch.setattr(state_module, "current_line", lambda ctx: "main")
    monkeypatch.setattr(state_module, "get_line", lambda ctx, line_name: {"head_snapshot_id": "SNP-REV"})

    payload = queue_module._workflow_land_payload(_build_ctx(), change_id="C-1", patchset_id=None, remote_name="origin")

    assert payload["change"]["status"] == "landed"
    assert payload["patchset"]["patchset_id"] == "P-1"
    assert payload["policy"]["decision"] == "pass"
    assert payload["next_action"]["code"] == "complete_task"
    assert payload["suggested_commands"] == ["ait task complete T-1"]
    assert payload["steps"][-1]["status"] == "done"
    assert ("get_change", "http://example.test", "C-1", "repo-alpha") in calls
    assert ("get_task", "http://example.test", "T-1", "repo-alpha") in calls


def test_workflow_publish_payload_forwards_repo_name_to_repo_scoped_helpers(monkeypatch):
    calls: list[tuple] = []

    def fake_remote_tuple(ctx, remote_name):
        calls.append(("remote_tuple", remote_name))
        return {"url": "http://example.test"}, "repo-alpha"

    def fake_remote_get_task(base_url, task_id, *, repo_name=None):
        calls.append(("get_task", base_url, task_id, repo_name))
        return {
            "task_id": task_id,
            "status": "active",
            "repo_name": "repo-alpha",
            "title": "task-title",
            "risk_tier": "medium",
        }

    def fake_publish_patchset(
        base_url,
        change_id,
        base_snapshot_id,
        revision_snapshot_id,
        summary,
        author_mode,
        *,
        repo_name=None,
        exact_id=False,
    ):
        calls.append(
            (
                "publish_patchset",
                base_url,
                change_id,
                base_snapshot_id,
                revision_snapshot_id,
                summary,
                author_mode,
                repo_name,
                exact_id,
            )
        )
        return {"patchset_id": "PS-1", "revision_snapshot_id": revision_snapshot_id}

    monkeypatch.setattr(publish_module, "_remote_tuple", fake_remote_tuple)
    monkeypatch.setattr(publish_module, "load_config", lambda ctx: {"default_line": "main"})
    monkeypatch.setattr(publish_module, "remote_get_task", fake_remote_get_task)
    monkeypatch.setattr(publish_module, "local_workspace_status", lambda ctx: {"clean": True, "baseline_snapshot_id": "SNP-BASE"})
    monkeypatch.setattr(publish_module, "current_line", lambda ctx: "main")
    monkeypatch.setattr(publish_module, "get_line", lambda ctx, line_name: {"head_snapshot_id": "SNP-REV"})
    monkeypatch.setattr(publish_module, "get_remote_line", lambda base_url, repo_name, line_name: {"head_snapshot_id": "SNP-BASE"})
    monkeypatch.setattr(publish_module, "get_snapshot", lambda ctx, snapshot_id: {"parent_snapshot_id": "SNP-PARENT"})
    monkeypatch.setattr(publish_module, "_guard_patchset_revision_scope", lambda *args, **kwargs: None)
    monkeypatch.setattr(publish_module, "_ensure_patchset_not_empty", lambda *args, **kwargs: None)
    monkeypatch.setattr(publish_module, "_ensure_local_line_at_snapshot", lambda ctx, line_name, snapshot_id: {"line": line_name})
    monkeypatch.setattr(publish_module, "_push_line", lambda ctx, remote_name, line_name: {"line": line_name, "line_updated": True})
    monkeypatch.setattr(
        publish_module,
        "_sync_patchset_revision_snapshot",
        lambda *args, **kwargs: {"line": kwargs.get("line_name"), "line_updated": True},
    )
    monkeypatch.setattr(publish_module, "_effective_author_mode", lambda ctx, author_mode=None: "human")
    monkeypatch.setattr(
        publish_module,
        "remote_create_change",
        lambda base_url, repo_name, task_id, title, base_line, risk_tier, **kwargs: {
            "change_id": "C-NEW",
            "base_line": base_line,
        },
    )
    monkeypatch.setattr(publish_module, "remote_publish_patchset", fake_publish_patchset)

    queue_module._workflow_publish_payload(
        _build_ctx(),
        task_id="T-1",
        summary="summary",
        remote_name="origin",
        base_snapshot_id=None,
        base_line_name=None,
        target_line=None,
        change_title=None,
        snapshot_message=None,
        author_mode=None,
    )

    assert ("get_task", "http://example.test", "T-1", "repo-alpha") in calls
    assert (
        "publish_patchset",
        "http://example.test",
        "C-NEW",
        "SNP-BASE",
        "SNP-REV",
        "summary",
        "human",
        "repo-alpha",
        True,
    ) in calls


def test_workflow_publish_payload_defaults_to_repo_default_target_line(monkeypatch):
    calls: list[tuple] = []

    def fake_remote_tuple(ctx, remote_name):
        return {"url": "http://example.test"}, "repo-alpha"

    def fake_remote_get_task(base_url, task_id, *, repo_name=None):
        return {
            "task_id": task_id,
            "status": "active",
            "repo_name": "repo-alpha",
            "title": "task-title",
            "risk_tier": "medium",
        }

    def fake_create_change(base_url, repo_name, task_id, title, base_line, risk_tier, **kwargs):
        calls.append(("create_change", base_line, kwargs.get("fork_snapshot_id"), kwargs.get("forked_from_line")))
        return {"change_id": "C-DEFAULT", "base_line": base_line}

    monkeypatch.setattr(publish_module, "_remote_tuple", fake_remote_tuple)
    monkeypatch.setattr(publish_module, "load_config", lambda ctx: {"default_line": "main"})
    monkeypatch.setattr(publish_module, "remote_get_task", fake_remote_get_task)
    monkeypatch.setattr(publish_module, "local_workspace_status", lambda ctx: {"clean": True, "baseline_snapshot_id": "SNP-OLD"})
    monkeypatch.setattr(publish_module, "current_line", lambda ctx: "feature/t-1")
    monkeypatch.setattr(publish_module, "get_line", lambda ctx, line_name: {"head_snapshot_id": "SNP-REV"})
    monkeypatch.setattr(publish_module, "get_remote_line", lambda base_url, repo_name, line_name: {"head_snapshot_id": "SNP-MAIN"})
    monkeypatch.setattr(publish_module, "_guard_patchset_revision_scope", lambda *args, **kwargs: None)
    monkeypatch.setattr(publish_module, "_ensure_patchset_not_empty", lambda *args, **kwargs: None)
    monkeypatch.setattr(publish_module, "_workflow_publish_auto_rebase_if_needed", lambda ctx, target_line: {"rebase": {"status": "applied"}})
    monkeypatch.setattr(
        publish_module,
        "_sync_patchset_revision_snapshot",
        lambda *args, **kwargs: {"line": kwargs.get("line_name"), "line_updated": True},
    )
    monkeypatch.setattr(publish_module, "_effective_author_mode", lambda ctx, author_mode=None: "human")
    monkeypatch.setattr(publish_module, "remote_create_change", fake_create_change)
    monkeypatch.setattr(
        publish_module,
        "remote_publish_patchset",
        lambda *args, **kwargs: {"patchset_id": "P-DEFAULT", "revision_snapshot_id": "SNP-REV"},
    )

    payload = queue_module._workflow_publish_payload(
        _build_ctx(),
        task_id="T-1",
        summary="summary",
        remote_name="origin",
        base_snapshot_id=None,
        base_line_name=None,
        target_line=None,
        change_title=None,
        snapshot_message=None,
        author_mode=None,
    )

    assert payload["promotion_mode"] == "local_first_final_remote_land"
    assert payload["change"]["base_line"] == "main"
    assert payload["base_line"]["line_name"] == "main"
    assert ("create_change", "main", "SNP-MAIN", "main") in calls


def test_workflow_publish_payload_target_line_uses_remote_target_head(monkeypatch):
    calls: list[tuple] = []

    def fake_remote_tuple(ctx, remote_name):
        return {"url": "http://example.test"}, "repo-alpha"

    def fake_remote_get_task(base_url, task_id, *, repo_name=None):
        return {
            "task_id": task_id,
            "status": "active",
            "repo_name": "repo-alpha",
            "title": "task-title",
            "risk_tier": "medium",
        }

    def fake_create_change(base_url, repo_name, task_id, title, base_line, risk_tier, **kwargs):
        calls.append(("create_change", base_line, kwargs.get("fork_snapshot_id"), kwargs.get("forked_from_line")))
        return {"change_id": "C-FINAL"}

    def fake_publish_patchset(
        base_url,
        change_id,
        base_snapshot_id,
        revision_snapshot_id,
        summary,
        author_mode,
        *,
        repo_name=None,
        exact_id=False,
    ):
        calls.append(("publish_patchset", change_id, base_snapshot_id, revision_snapshot_id, repo_name, exact_id))
        return {"patchset_id": "P-FINAL", "revision_snapshot_id": revision_snapshot_id}

    monkeypatch.setattr(publish_module, "_remote_tuple", fake_remote_tuple)
    monkeypatch.setattr(publish_module, "load_config", lambda ctx: {"default_line": "main"})
    monkeypatch.setattr(publish_module, "remote_get_task", fake_remote_get_task)
    monkeypatch.setattr(publish_module, "local_workspace_status", lambda ctx: {"clean": True, "baseline_snapshot_id": "SNP-OLD"})
    monkeypatch.setattr(publish_module, "current_line", lambda ctx: "feature/t-1")
    monkeypatch.setattr(publish_module, "get_line", lambda ctx, line_name: {"head_snapshot_id": "SNP-REV"})
    monkeypatch.setattr(publish_module, "get_remote_line", lambda base_url, repo_name, line_name: {"head_snapshot_id": "SNP-MAIN"})
    monkeypatch.setattr(publish_module, "_guard_patchset_revision_scope", lambda *args, **kwargs: None)
    monkeypatch.setattr(publish_module, "_ensure_patchset_not_empty", lambda *args, **kwargs: None)
    monkeypatch.setattr(publish_module, "_workflow_publish_auto_rebase_if_needed", lambda ctx, target_line: {"rebase": {"status": "applied"}})
    monkeypatch.setattr(
        publish_module,
        "_sync_patchset_revision_snapshot",
        lambda *args, **kwargs: {"line": kwargs.get("line_name"), "line_updated": True},
    )
    monkeypatch.setattr(publish_module, "_effective_author_mode", lambda ctx, author_mode=None: "human")
    monkeypatch.setattr(publish_module, "remote_create_change", fake_create_change)
    monkeypatch.setattr(publish_module, "remote_publish_patchset", fake_publish_patchset)

    payload = queue_module._workflow_publish_payload(
        _build_ctx(),
        task_id="T-1",
        summary="summary",
        remote_name="origin",
        base_snapshot_id=None,
        base_line_name=None,
        target_line="main",
        change_title=None,
        snapshot_message=None,
        author_mode=None,
    )

    assert payload["promotion_mode"] == "local_first_final_remote_land"
    assert ("create_change", "main", "SNP-MAIN", "main") in calls
    assert ("publish_patchset", "C-FINAL", "SNP-MAIN", "SNP-REV", "repo-alpha", True) in calls


def test_workflow_refresh_patchset_for_land_auto_rebases_before_publish(monkeypatch):
    calls: list[str] = []

    def fake_remote_sync(ctx, remote_name):
        return {"url": "http://example.test"}, "repo-alpha"

    monkeypatch.setattr(publish_module, "_sync_remote_repository_defaults", fake_remote_sync)
    monkeypatch.setattr(
        publish_module,
        "remote_get_change",
        lambda base_url, change_id, *, repo_name=None: {"change_id": change_id, "base_line": "main"},
    )
    monkeypatch.setattr(
        publish_module,
        "_workflow_publish_auto_rebase_if_needed",
        lambda ctx, target_line: calls.append(f"rebase:{target_line}") or {"rebase": {"status": "applied"}},
    )
    monkeypatch.setattr(
        publish_module,
        "_publish_patchset_from_current_line",
        lambda ctx, **kwargs: calls.append(f"publish:{kwargs['change_id']}") or {"patchset_id": "P-1"},
    )

    result = queue_module._workflow_refresh_patchset_for_land(
        _build_ctx(),
        change_id="C-1",
        summary="summary",
        remote_name="origin",
        author_mode=None,
    )

    assert calls == ["rebase:main", "publish:C-1"]
    assert result["patchset_id"] == "P-1"
    assert result["auto_rebase"]["rebase"]["status"] == "applied"


def test_task_dag_readiness_from_remote_inventory_forwards_repo_name(monkeypatch):
    calls: list[tuple] = []
    graph = {"source_plan": {"plan_id": "PL-1"}, "nodes": []}

    def fake_remote_get_plan(base_url, plan_id):
        calls.append(("get_plan", base_url, plan_id))
        return {"head_revision": {"plan_revision_id": "PR-1"}}

    def fake_remote_list_tasks(base_url, repo_name):
        calls.append(("list_tasks", base_url, repo_name))
        return []

    def fake_remote_list_changes(base_url, repo_name):
        calls.append(("list_changes", base_url, repo_name))
        return [{"change_id": "C-1"}]

    def fake_remote_get_change(base_url, change_id, *, repo_name=None):
        calls.append(("get_change", base_url, change_id, repo_name))
        return {"change_id": change_id}

    def fake_remote_list_patchsets(base_url, change_id, *, repo_name=None):
        calls.append(("list_patchsets", base_url, change_id, repo_name))
        return [{"patchset_id": "P-1"}]

    def fake_remote_list_reviews(base_url, change_id, *, repo_name=None):
        calls.append(("list_reviews", base_url, change_id, repo_name))
        return {"reviews": []}

    def fake_remote_get_policy(base_url, patchset_id, *, repo_name=None):
        calls.append(("get_policy", base_url, patchset_id, repo_name))
        return {"patchset_id": patchset_id, "decision": "pass"}

    def fake_compute_readiness(graph_value, remote_payload, *, current_plan_revision_id=None):
        calls.append(("compute_readiness", current_plan_revision_id))
        return {"summary": {}, "counts": {}}

    monkeypatch.setattr(cli_app_module, "remote_get_plan", fake_remote_get_plan)
    monkeypatch.setattr(cli_app_module, "remote_list_tasks", fake_remote_list_tasks)
    monkeypatch.setattr(cli_app_module, "remote_list_changes", fake_remote_list_changes)
    monkeypatch.setattr(cli_app_module, "remote_list_sessions", lambda base_url, repo_name: [])
    monkeypatch.setattr(cli_app_module, "remote_list_session_checkpoints", lambda base_url, session_id, repo_name=None: [])
    monkeypatch.setattr(cli_app_module, "remote_get_change", fake_remote_get_change)
    monkeypatch.setattr(cli_app_module, "remote_list_patchsets", fake_remote_list_patchsets)
    monkeypatch.setattr(cli_app_module, "remote_list_reviews", fake_remote_list_reviews)
    monkeypatch.setattr(cli_app_module, "remote_get_policy", fake_remote_get_policy)
    monkeypatch.setattr(cli_app_module, "compute_task_graph_readiness", fake_compute_readiness)

    cli_app_module._task_dag_readiness_from_remote_inventory({"url": "http://example.test"}, "repo-alpha", graph)

    assert ("list_tasks", "http://example.test", "repo-alpha") in calls
    assert ("list_changes", "http://example.test", "repo-alpha") in calls
    assert ("get_change", "http://example.test", "C-1", "repo-alpha") in calls
    assert ("list_patchsets", "http://example.test", "C-1", "repo-alpha") in calls
    assert ("list_reviews", "http://example.test", "C-1", "repo-alpha") in calls
    assert ("get_policy", "http://example.test", "P-1", "repo-alpha") in calls


def test_remote_task_dag_readiness_compatibility_helper_supports_repo_scoped_and_legacy_signatures(monkeypatch):
    calls: list[tuple] = []

    def fake_supports_repo_name(base_url, graph, *, repo_name=None, current_plan_revision_id=None):
        calls.append(("modern", repo_name, current_plan_revision_id))
        return {"ok": True}

    monkeypatch.setattr(cli_app_module, "remote_read_task_dag_readiness", fake_supports_repo_name)
    result = cli_app_module._remote_read_task_dag_readiness(
        "http://example.test",
        {"nodes": []},
        repo_name="repo-alpha",
        current_plan_revision_id="PR-1",
    )
    assert result["ok"] is True
    assert ("modern", "repo-alpha", "PR-1") in calls

    calls.clear()

    def fake_without_repo_name(base_url, graph, current_plan_revision_id=None):
        calls.append(("legacy", current_plan_revision_id))
        return {"legacy": True}

    monkeypatch.setattr(cli_app_module, "remote_read_task_dag_readiness", fake_without_repo_name)
    result = cli_app_module._remote_read_task_dag_readiness(
        "http://example.test",
        {"nodes": []},
        repo_name="repo-alpha",
        current_plan_revision_id="PR-1",
    )
    assert result["legacy"] is True
    assert ("legacy", "PR-1") in calls


def test_review_submission_latest_patchset_id_forwards_repo_name(monkeypatch):
    calls: list[tuple] = []

    def fake_list_patchsets(base_url, change_id, *, repo_name=None):
        calls.append(("list_patchsets", base_url, change_id, repo_name))
        return [{"patchset_id": "PS-1"}]

    monkeypatch.setattr(review_submission_helpers, "remote_list_patchsets", fake_list_patchsets)
    assert review_submission_helpers._latest_patchset_id("http://example.test", "C-1", "repo-alpha") == "PS-1"
    assert ("list_patchsets", "http://example.test", "C-1", "repo-alpha") in calls


def test_review_action_result_forwards_repo_name_to_record_review(monkeypatch):
    calls: list[tuple] = []

    def fake_remote_tuple(ctx, remote_name):
        return {"url": "http://example.test"}, "repo-alpha"

    def fake_reviewer_identity(ctx, reviewer):
        return "reviewer@example.com"

    def fake_latest_patchset(base_url, change_id, repo_name):
        calls.append(("latest_patchset_id", base_url, change_id, repo_name))
        return "PS-1"

    def fake_record_review(
        base_url,
        change_id,
        patchset_id,
        reviewer,
        action,
        comment,
        blocking,
        *,
        repo_name=None,
    ):
        calls.append(("record_review", base_url, change_id, patchset_id, reviewer, action, comment, blocking, repo_name))
        return {}

    monkeypatch.setattr(review_submission_helpers, "_remote_tuple", fake_remote_tuple)
    monkeypatch.setattr(review_submission_helpers, "_effective_reviewer_identity", fake_reviewer_identity)
    monkeypatch.setattr(review_submission_helpers, "_latest_patchset_id", fake_latest_patchset)
    monkeypatch.setattr(review_submission_helpers, "remote_record_review", fake_record_review)

    review_submission_helpers._review_action_result(
        _build_ctx(),
        change_id="C-1",
        reviewer=None,
        action="approve",
        blocking=False,
        patchset=None,
        message=None,
        remote="origin",
    )

    assert ("latest_patchset_id", "http://example.test", "C-1", "repo-alpha") in calls
    assert ("record_review", "http://example.test", "C-1", "PS-1", "reviewer@example.com", "approve", None, False, "repo-alpha") in calls


def test_request_team_review_result_forwards_repo_name(monkeypatch):
    calls: list[tuple] = []

    def fake_remote_tuple(ctx, remote_name):
        return {"url": "http://example.test"}, "repo-alpha"

    def fake_latest_patchset(base_url, change_id, repo_name):
        calls.append(("latest_patchset_id", base_url, change_id, repo_name))
        return "PS-2"

    def fake_request_review(base_url, change_id, patchset_id, group, note, *, repo_name=None):
        calls.append(("request_review", base_url, change_id, patchset_id, tuple(group), note, repo_name))
        return {"requested": True}

    monkeypatch.setattr(review_submission_helpers, "_remote_tuple", fake_remote_tuple)
    monkeypatch.setattr(review_submission_helpers, "_latest_patchset_id", fake_latest_patchset)
    monkeypatch.setattr(review_submission_helpers, "remote_request_review", fake_request_review)

    payload = review_submission_helpers._request_team_review_result(
        _build_ctx(),
        change_id="C-2",
        group=["eng", "ops"],
        patchset=None,
        note="please review",
        remote="origin",
    )

    assert payload == {"requested": True}
    assert ("latest_patchset_id", "http://example.test", "C-2", "repo-alpha") in calls
    assert ("request_review", "http://example.test", "C-2", "PS-2", ("eng", "ops"), "please review", "repo-alpha") in calls
