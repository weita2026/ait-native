from __future__ import annotations

import base64
import hashlib
from pathlib import Path

from ait_server import read_models, server_store
from ait_server.read_models_domains import workflow_detail
from tests.postgres_fake import fake_postgres_context


def _snapshot_bundle(
    repo_name: str,
    snapshot_id: str,
    *,
    parent_snapshot_id: str | None,
    line_name: str,
    message: str,
    files: dict[str, bytes],
) -> dict:
    file_rows = []
    for path, data in files.items():
        blob_id = f"BLB-{snapshot_id}-{path.replace('/', '_')}"
        file_rows.append(
            {
                "path": path,
                "blob_id": blob_id,
                "size_bytes": len(data),
                "mode": "100644",
                "sha256": hashlib.sha256(data).hexdigest(),
                "content_b64": base64.b64encode(data).decode("ascii"),
            }
        )
    return {
        "snapshot_id": snapshot_id,
        "repo_name": repo_name,
        "parent_snapshot_id": parent_snapshot_id,
        "line_name": line_name,
        "message": message,
        "files": file_rows,
    }


def _publish_ready_change(ctx, repo_name: str, task_title: str, change_title: str, suffix: str):
    task = server_store.create_task(ctx, repo_name, task_title, "compatibility sweep", "medium")
    change = server_store.create_change(
        ctx,
        repo_name,
        task["task_id"],
        change_title,
        "main",
        "medium",
    )
    base_snapshot = server_store.import_snapshot(
        ctx,
        repo_name,
        _snapshot_bundle(
            repo_name,
            f"SNP-{suffix}-BASE",
            parent_snapshot_id=None,
            line_name="main",
            message="base",
            files={"README.md": b"base\n"},
        ),
    )
    revision_snapshot = server_store.import_snapshot(
        ctx,
        repo_name,
        _snapshot_bundle(
            repo_name,
            f"SNP-{suffix}-REV",
            parent_snapshot_id=base_snapshot["snapshot_id"],
            line_name="main",
            message="revision",
            files={"README.md": f"base\n{suffix}\n".encode("utf-8")},
        ),
    )
    server_store.update_line(ctx, repo_name, "main", base_snapshot["snapshot_id"])
    patchset = server_store.publish_patchset(
        ctx,
        change["change_id"],
        base_snapshot["snapshot_id"],
        revision_snapshot["snapshot_id"],
        f"patchset {suffix}",
        "human_only",
    )
    return task, change, patchset


def test_get_change_for_repo_no_longer_delegates_to_global_get_change(tmp_path: Path, monkeypatch):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)
    server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    _, change, _ = _publish_ready_change(ctx, "repo-a", "Repo task", "Repo change", "001")

    calls = {"count": 0}

    def fail_global_get_change(_ctx, change_id):  # pragma: no cover
        calls["count"] += 1
        raise AssertionError(f"global get_change called with {change_id}")

    monkeypatch.setattr(server_store, "get_change", fail_global_get_change)
    result = server_store.get_change_for_repo(ctx, "repo-a", change["change_id"])

    assert result["change_id"] == change["change_id"]
    assert calls["count"] == 0


def test_reviewer_inbox_uses_repo_scoped_loaders(tmp_path: Path, monkeypatch):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)
    server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    _, _, _ = _publish_ready_change(ctx, "repo-a", "Compat task", "Compat change", "R01")

    calls = {"task_for_repo": 0, "patchset_for_repo": 0}
    original_task_for_repo = read_models.get_task_for_repo
    original_patchset_for_repo = read_models.get_patchset_for_repo

    monkeypatch.setattr(read_models, "get_change", lambda *_args, **_kwargs: (_ for _ in ()).throw(AssertionError("global get_change called")))
    monkeypatch.setattr(
        read_models,
        "get_change_for_repo",
        lambda *_args, **_kwargs: (_ for _ in ()).throw(AssertionError("repo-scoped get_change called")),
    )
    monkeypatch.setattr(read_models, "get_task", lambda *_args, **_kwargs: (_ for _ in ()).throw(AssertionError("global get_task called")))
    monkeypatch.setattr(
        read_models,
        "get_patchset",
        lambda *_args, **_kwargs: (_ for _ in ()).throw(AssertionError("global get_patchset called")),
    )

    def counted_task_for_repo(*args, **kwargs):
        calls["task_for_repo"] += 1
        return original_task_for_repo(*args, **kwargs)

    def counted_patchset_for_repo(*args, **kwargs):
        calls["patchset_for_repo"] += 1
        return original_patchset_for_repo(*args, **kwargs)

    monkeypatch.setattr(read_models, "get_task_for_repo", counted_task_for_repo)
    monkeypatch.setattr(read_models, "get_patchset_for_repo", counted_patchset_for_repo)

    inbox = read_models.reviewer_inbox(ctx, repo_name="repo-a")

    assert inbox["count"] == 1
    assert calls["task_for_repo"] >= 1
    assert calls["patchset_for_repo"] >= 1


def test_reviewer_inbox_uses_global_fallback_for_drifted_change_repo_name(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)
    server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    _, change, _ = _publish_ready_change(ctx, "repo-a", "Compat task", "Compat change", "R02")

    with server_store.connect(ctx) as conn:
        conn.execute("update changes set repo_name = ? where change_id = ?", ("repo-a-legacy", change["change_id"]))
        conn.commit()

    inbox = read_models.reviewer_inbox(ctx, repo_name="repo-a")
    assert inbox["count"] == 1
    assert inbox["items"][0]["change_id"] == change["change_id"]


def test_task_queue_uses_repo_scoped_change_and_patchset_loaders(tmp_path: Path, monkeypatch):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)
    server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    _, _, _ = _publish_ready_change(ctx, "repo-a", "Compat task", "Compat change", "Q01")

    calls = {"patchset_for_repo": 0}
    original_patchset_for_repo = read_models.get_patchset_for_repo

    monkeypatch.setattr(
        read_models,
        "get_change",
        lambda *_args, **_kwargs: (_ for _ in ()).throw(AssertionError("global get_change called")),
    )
    monkeypatch.setattr(
        read_models,
        "get_change_for_repo",
        lambda *_args, **_kwargs: (_ for _ in ()).throw(AssertionError("repo-scoped get_change called")),
    )

    def counted_patchset_for_repo(*args, **kwargs):
        calls["patchset_for_repo"] += 1
        return original_patchset_for_repo(*args, **kwargs)

    monkeypatch.setattr(read_models, "get_patchset_for_repo", counted_patchset_for_repo)

    payload = read_models.task_queue(ctx, repo_name="repo-a")

    assert payload["count"] == 1
    assert calls["patchset_for_repo"] >= 1


def test_task_queue_and_reviewer_inbox_share_cached_repo_scoped_change_hydration(tmp_path: Path, monkeypatch):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)
    server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    _, _, _ = _publish_ready_change(ctx, "repo-a", "Compat task", "Compat change", "CACHE01")

    calls = {"list_changes": 0}
    original_list_changes = read_models.list_changes

    def counted_list_changes(*args, **kwargs):
        calls["list_changes"] += 1
        return original_list_changes(*args, **kwargs)

    monkeypatch.setattr(read_models, "list_changes", counted_list_changes)

    cache: dict[str, dict[object, object]] = {}
    queue_payload = read_models.task_queue(ctx, repo_name="repo-a", cache=cache)
    inbox_payload = read_models.reviewer_inbox(ctx, repo_name="repo-a", cache=cache)

    assert queue_payload["count"] == 1
    assert inbox_payload["count"] == 1
    assert calls["list_changes"] == 1


def test_task_audit_uses_repo_scoped_change_and_patchset_loaders(tmp_path: Path, monkeypatch):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)
    server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    task, _, _ = _publish_ready_change(ctx, "repo-a", "Compat task", "Compat change", "A01")

    monkeypatch.setattr(
        read_models,
        "get_change",
        lambda *_args, **_kwargs: (_ for _ in ()).throw(AssertionError("global get_change called")),
    )
    monkeypatch.setattr(
        read_models,
        "get_patchset",
        lambda *_args, **_kwargs: (_ for _ in ()).throw(AssertionError("global get_patchset called")),
    )

    audit = read_models.task_audit(ctx, task["task_id"])
    assert audit["task"]["task_id"] == task["task_id"]
    assert len(audit["changes"]) == 1


def test_task_queue_and_task_audit_report_stale_patchset_refresh_instead_of_blocking_review(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)
    server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    task, change, patchset = _publish_ready_change(ctx, "repo-a", "Compat task", "Compat change", "STALE01")

    server_store.upsert_attestation(
        ctx,
        patchset["patchset_id"],
        "human_only",
        {"tests": "pass"},
        {"policy_readable": True},
    )
    server_store.record_review(ctx, change["change_id"], patchset["patchset_id"], "alice@example.com", "approve", None)
    assert server_store.evaluate_policy(ctx, patchset["patchset_id"])["decision"] == "pass"

    unrelated_head = server_store.import_snapshot(
        ctx,
        "repo-a",
        _snapshot_bundle(
            "repo-a",
            "SNP-STALE01-MAIN",
            parent_snapshot_id=patchset["base_snapshot_id"],
            line_name="main",
            message="main moved",
            files={"README.md": b"base\nmain moved\n"},
        ),
    )
    server_store.update_line(
        ctx,
        "repo-a",
        "main",
        unrelated_head["snapshot_id"],
        expected_head_snapshot_id=patchset["base_snapshot_id"],
    )
    assert server_store.evaluate_policy(ctx, patchset["patchset_id"])["decision"] == "pass"
    assert server_store.get_change_for_repo(ctx, "repo-a", change["change_id"])["status"] == "blocked"

    queue_payload = read_models.task_queue(ctx, repo_name="repo-a")
    queue_item = queue_payload["items"][0]
    assert queue_item["workflow"]["state"] == "attention_required"
    assert queue_item["workflow"]["reason"] == "The base line moved after this patchset was published."
    assert queue_item["next_action"]["code"] == "refresh_patchset"
    assert queue_item["next_action"]["label"] == "Open task and refresh the patchset"
    assert queue_item["next_action"]["detail"] == "The base line moved after this patchset was published."

    audit = read_models.task_audit(ctx, task["task_id"])
    assert audit["workflow"]["state"] == "attention_required"
    assert audit["workflow"]["reason"] == "The base line moved after this patchset was published."
    assert audit["recommended_action"]["code"] == "refresh_patchset"
    assert audit["recommended_action"]["label"] == "Open task and refresh the patchset"
    assert audit["recommended_action"]["detail"] == "The base line moved after this patchset was published."


def test_workflow_detail_task_detail_uses_repo_scoped_patchset_loader(tmp_path: Path, monkeypatch):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)
    server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    task, _, patchset = _publish_ready_change(ctx, "repo-a", "Compat task", "Compat change", "W01")

    monkeypatch.setattr(
        workflow_detail,
        "get_patchset",
        lambda *_args, **_kwargs: (_ for _ in ()).throw(AssertionError("global get_patchset called")),
    )

    payload = workflow_detail.task_detail(ctx, task["task_id"])
    assert payload["changes"][0]["current_patchset"]["patchset_id"] == patchset["patchset_id"]


def test_workflow_detail_change_and_stack_detail_use_repo_scoped_loaders(tmp_path: Path, monkeypatch):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)
    server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    task, change, _ = _publish_ready_change(ctx, "repo-a", "Compat task", "Compat change", "W02")
    stack = server_store.create_stack(ctx, "repo-a", "Compat stack", [change["change_id"]])

    calls = {"task_for_repo": 0, "patchset_for_repo": 0, "change_for_repo": 0}
    original_task_for_repo = workflow_detail.get_task_for_repo
    original_patchset_for_repo = workflow_detail.get_patchset_for_repo
    original_change_for_repo = workflow_detail.get_change_for_repo

    monkeypatch.setattr(
        workflow_detail,
        "get_task",
        lambda *_args, **_kwargs: (_ for _ in ()).throw(AssertionError("global get_task called")),
    )
    monkeypatch.setattr(
        workflow_detail,
        "get_patchset",
        lambda *_args, **_kwargs: (_ for _ in ()).throw(AssertionError("global get_patchset called")),
    )

    def counted_task_for_repo(*args, **kwargs):
        calls["task_for_repo"] += 1
        return original_task_for_repo(*args, **kwargs)

    def counted_patchset_for_repo(*args, **kwargs):
        calls["patchset_for_repo"] += 1
        return original_patchset_for_repo(*args, **kwargs)

    def counted_change_for_repo(*args, **kwargs):
        calls["change_for_repo"] += 1
        return original_change_for_repo(*args, **kwargs)

    monkeypatch.setattr(workflow_detail, "get_task_for_repo", counted_task_for_repo)
    monkeypatch.setattr(workflow_detail, "get_patchset_for_repo", counted_patchset_for_repo)
    monkeypatch.setattr(workflow_detail, "get_change_for_repo", counted_change_for_repo)

    change_payload = workflow_detail.change_detail(ctx, change["change_id"])
    stack_payload = workflow_detail.stack_detail(ctx, stack["stack_id"])

    assert change_payload["change"]["change_id"] == change["change_id"]
    assert change_payload["task"]["task_id"] == task["task_id"]
    assert stack_payload["changes"][0]["change_id"] == change["change_id"]
    assert calls["task_for_repo"] >= 1
    assert calls["patchset_for_repo"] >= 1
    assert calls["change_for_repo"] >= 1


def test_workflow_detail_change_detail_uses_global_fallback_when_repo_name_drifted(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)
    server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    _, change, _ = _publish_ready_change(ctx, "repo-a", "Compat task", "Compat change", "W03")

    with server_store.connect(ctx) as conn:
        conn.execute("update changes set repo_name = ? where change_id = ?", ("repo-a-legacy", change["change_id"]))
        conn.commit()

    payload = workflow_detail.change_detail(ctx, change["change_id"])
    assert payload["change"]["change_id"] == change["change_id"]
    assert payload["task"]["task_id"] == payload["change"]["task_id"]
    assert payload["current_patchset"] is not None
