from __future__ import annotations

from datetime import datetime, timezone
from pathlib import Path

from ait import local_content, store
from ait.aitk_export import (
    attach_snapshot_parent_diffs,
    build_aitk_history_payload,
    build_aitk_payload_from_rows,
    build_markdown_docs,
    build_plan_links,
    snapshot_chain,
    snapshot_distance_descendant_to_ancestor,
    snapshot_health_relation_to_main,
    snapshot_is_ancestor,
)


def _snapshot_rows() -> tuple[list[dict], list[dict]]:
    snapshots = [
        {
            "snapshot_id": "S1",
            "parent_snapshot_id": None,
            "created_at": "2026-04-10T00:00:00+00:00",
            "message": "root",
        },
        {
            "snapshot_id": "S2",
            "parent_snapshot_id": "S1",
            "created_at": "2026-04-11T00:00:00+00:00",
            "message": "main-2",
        },
        {
            "snapshot_id": "S3",
            "parent_snapshot_id": "S2",
            "created_at": "2026-04-12T00:00:00+00:00",
            "message": "main-3",
        },
        {
            "snapshot_id": "S4",
            "parent_snapshot_id": "S3",
            "created_at": "2026-04-13T00:00:00+00:00",
            "message": "feature-ahead",
        },
        {
            "snapshot_id": "S5",
            "parent_snapshot_id": None,
            "created_at": "2026-04-01T00:00:00+00:00",
            "message": "unrelated",
        },
        {
            "snapshot_id": "S6",
            "parent_snapshot_id": "S2",
            "created_at": "2026-04-14T00:00:00+00:00",
            "message": "feature-diverged-from-older-main",
        },
    ]
    lines = [
        {
            "line_name": "main",
            "status": "active",
            "created_at": "2026-04-12T00:00:00+00:00",
            "updated_at": "2026-04-12T12:00:00+00:00",
            "head_snapshot_id": "S3",
        },
        {
            "line_name": "feature/contained",
            "status": "active",
            "created_at": "2026-04-10T00:00:00+00:00",
            "updated_at": "2026-04-18T00:00:00+00:00",
            "head_snapshot_id": "S2",
        },
        {
            "line_name": "feature/newer",
            "status": "active",
            "created_at": "2026-04-10T00:00:00+00:00",
            "updated_at": "2026-04-19T00:00:00+00:00",
            "head_snapshot_id": "S4",
        },
        {
            "line_name": "feature/stale",
            "status": "active",
            "created_at": "2026-04-01T00:00:00+00:00",
            "updated_at": "2026-04-01T00:00:00+00:00",
            "head_snapshot_id": "S5",
        },
        {
            "line_name": "feature/diverged",
            "status": "active",
            "created_at": "2026-04-10T00:00:00+00:00",
            "updated_at": "2026-04-19T00:00:00+00:00",
            "head_snapshot_id": "S6",
        },
    ]
    return snapshots, lines


def test_snapshot_chain_and_ancestor_detection():
    snapshot_rows, _ = _snapshot_rows()
    parent_by_id = {row["snapshot_id"]: row["parent_snapshot_id"] for row in snapshot_rows}

    assert snapshot_chain("S4", parent_by_id) == ["S4", "S3", "S2", "S1"]
    assert snapshot_is_ancestor("S2", "S3", parent_by_id) is True
    assert snapshot_is_ancestor("S2", "S4", parent_by_id) is True
    assert snapshot_is_ancestor("S4", "S2", parent_by_id) is False


def test_snapshot_distance_mainline_health_relations():
    snapshot_rows, _ = _snapshot_rows()
    parent_by_id = {row["snapshot_id"]: row["parent_snapshot_id"] for row in snapshot_rows}

    assert snapshot_distance_descendant_to_ancestor("S4", "S3", parent_by_id) == 1
    assert snapshot_distance_descendant_to_ancestor("S3", "S1", parent_by_id) == 2
    assert snapshot_distance_descendant_to_ancestor("S5", "S3", parent_by_id) is None

    contained = snapshot_health_relation_to_main("S3", "S1", parent_by_id)
    assert contained["is_contained_in_main"] is True
    assert contained["ahead_count"] == 0
    assert contained["behind_count"] == 2
    assert contained["base_distance"] == 2

    ahead = snapshot_health_relation_to_main("S3", "S4", parent_by_id)
    assert ahead["is_contained_in_main"] is False
    assert ahead["ahead_count"] == 1
    assert ahead["behind_count"] == 0
    assert ahead["base_distance"] == 1
    assert ahead["base_snapshot_id"] == "S3"

    diverged = snapshot_health_relation_to_main("S3", "S6", parent_by_id)
    assert diverged["is_contained_in_main"] is False
    assert diverged["ahead_count"] == 1
    assert diverged["behind_count"] == 1
    assert diverged["base_distance"] == 1
    assert diverged["base_snapshot_id"] == "S2"

    unrelated = snapshot_health_relation_to_main("S3", "S5", parent_by_id)
    assert unrelated["is_contained_in_main"] is False
    assert unrelated["ahead_count"] is None
    assert unrelated["behind_count"] is None
    assert unrelated["base_distance"] is None
    assert unrelated["base_snapshot_id"] is None


def test_build_payload_includes_history_line_health_and_summary():
    now = datetime(2026, 4, 21, 0, 0, tzinfo=timezone.utc)
    snapshot_rows, line_rows = _snapshot_rows()
    payload = build_aitk_payload_from_rows(snapshot_rows, line_rows, now=now, stale_days=3)

    assert payload["main_head_snapshot_id"] == "S3"
    assert payload["main_line_name"] == "main"
    assert len(payload["history_rows"]) == 6
    assert len(payload["line_health_rows"]) == 5
    assert payload["plan_links"] == []

    history_index = {row["snapshot_id"]: row for row in payload["history_rows"]}
    assert history_index["S3"]["is_main_head"] is True
    assert history_index["S3"]["is_head"] is True
    assert history_index["S3"]["marker"] == "@"
    assert isinstance(history_index["S3"]["graph_column"], int)
    assert "graph_segments" in history_index["S4"]
    assert history_index["S3"]["age_days"] == 9.0
    assert history_index["S3"]["provenance_badges"] == []

    line_health_by_name = {row["line_name"]: row for row in payload["line_health_rows"]}
    assert line_health_by_name["feature/contained"]["is_contained_in_main"] is True
    assert line_health_by_name["feature/contained"]["ahead_count"] == 0
    assert line_health_by_name["feature/contained"]["behind_count"] == 1
    assert line_health_by_name["feature/newer"]["is_contained_in_main"] is False
    assert line_health_by_name["feature/newer"]["ahead_count"] == 1
    assert line_health_by_name["feature/diverged"]["is_contained_in_main"] is False
    assert line_health_by_name["feature/diverged"]["ahead_count"] == 1
    assert line_health_by_name["feature/diverged"]["behind_count"] == 1
    assert line_health_by_name["feature/diverged"]["base_snapshot_id"] == "S2"
    assert line_health_by_name["feature/stale"]["is_stale"] is True
    assert line_health_by_name["feature/stale"]["ahead_count"] is None
    assert line_health_by_name["feature/stale"]["behind_count"] is None
    assert line_health_by_name["feature/stale"]["is_contained_in_main"] is False

    summary = payload["summary"]
    assert summary["history_count"] == 6
    assert summary["line_count"] == 5
    assert summary["plan_link_count"] == 0
    assert summary["active_line_count"] == 5
    assert summary["contained_line_count"] == 2
    assert summary["uncontained_line_count"] == 3
    assert summary["stale_uncontained_line_count"] == 1
    assert summary["stale_threshold_days"] == 3
    assert summary["generated_at"] == now.isoformat()


def test_build_payload_tolerates_archived_line_with_unknown_head():
    now = datetime(2026, 4, 21, 0, 0, tzinfo=timezone.utc)
    snapshot_rows, line_rows = _snapshot_rows()
    line_rows.append(
        {
            "line_name": "feature/archived-bad-ref",
            "status": "archived",
            "archived_at": "2026-04-20T00:00:00+00:00",
            "created_at": "2026-04-20T00:00:00+00:00",
            "updated_at": "2026-04-20T00:00:00+00:00",
            "head_snapshot_id": "--json",
        }
    )

    payload = build_aitk_payload_from_rows(snapshot_rows, line_rows, now=now)

    line_health_by_name = {row["line_name"]: row for row in payload["line_health_rows"]}
    bad_line = line_health_by_name["feature/archived-bad-ref"]
    assert bad_line["head_snapshot_id"] == "--json"
    assert bad_line["head_snapshot_missing"] is True
    assert "Unknown snapshot id" in bad_line["head_snapshot_error"]
    assert bad_line["ahead_count"] is None
    assert bad_line["behind_count"] is None
    assert bad_line["base_snapshot_id"] is None


def test_build_payload_can_attach_provenance_badges():
    now = datetime(2026, 4, 21, 0, 0, tzinfo=timezone.utc)
    snapshot_rows, line_rows = _snapshot_rows()
    payload = build_aitk_payload_from_rows(
        snapshot_rows,
        line_rows,
        now=now,
        provenance={
            "changes": [{"change_id": "C-1", "revision_snapshot_id": "S4", "title": "feature change"}],
            "sessions": [{"session_id": "S-1", "head_snapshot_id": "S4", "summary": "session"}],
        },
    )

    by_id = {row["snapshot_id"]: row for row in payload["history_rows"]}
    assert by_id["S4"]["provenance_badges"] == ["change", "session"]
    assert payload["provenance_overlays"]["S4"]["items"][0]["kind"] == "change"


def test_build_payload_attaches_selected_snapshot_plan_context():
    now = datetime(2026, 4, 21, 0, 0, tzinfo=timezone.utc)
    snapshot_rows, line_rows = _snapshot_rows()
    payload = build_aitk_payload_from_rows(
        snapshot_rows,
        line_rows,
        now=now,
        provenance={
            "tasks": [
                {
                    "task_id": "T-1",
                    "title": "Show row plan context",
                    "intent": "Render the selected snapshot task's plan item beside the graph.",
                    "status": "active",
                    "plan_id": "PL-1",
                    "origin_plan_revision_id": "PR-1",
                    "plan_item_ref": "aitk/context-pane",
                }
            ],
            "changes": [{"change_id": "C-1", "task_id": "T-1", "title": "Plan context change"}],
            "patchsets": [
                {
                    "patchset_id": "P-1",
                    "change_id": "C-1",
                    "revision_snapshot_id": "S4",
                    "base_snapshot_id": "S3",
                    "summary": "Patchset summary",
                }
            ],
        },
        plan_links=[
            {
                "kind": "plan",
                "plan_id": "PL-1",
                "title": "aitk Plan",
                "status": "draft",
                "head_revision_id": "PR-1",
                "artifact_path": "docs/sprints/aitk.md",
                "artifact_selector": "aitk/root",
                "display_path": "docs/sprints/aitk.md#aitk/root",
                "items": [
                    {
                        "plan_item_ref": "aitk/context-pane",
                        "text": "Show the selected row's task plan context.",
                        "checkbox_state": " ",
                        "heading_path": ["aitk Plan"],
                        "line_number": 12,
                    }
                ],
            }
        ],
    )

    by_id = {row["snapshot_id"]: row for row in payload["history_rows"]}
    assert by_id["S4"]["plan_context_count"] == 1
    context = payload["plan_context_by_snapshot"]["S4"][0]
    assert context["task_id"] == "T-1"
    assert context["change_id"] == "C-1"
    assert context["patchset_id"] == "P-1"
    assert context["plan_id"] == "PL-1"
    assert context["plan_item_ref"] == "aitk/context-pane"
    assert context["plan_item_text"] == "Show the selected row's task plan context."
    assert context["display_path"] == "docs/sprints/aitk.md#aitk/root"
    assert payload["summary"]["plan_context_snapshot_count"] == 1


def test_build_aitk_history_payload_calls_local_apis(monkeypatch):
    snapshot_rows, line_rows = _snapshot_rows()
    fake_ctx = object()
    called = {"snapshots": 0, "lines": 0, "current_line": 0, "plans": 0}

    monkeypatch.setattr(
        "ait.aitk_export.list_snapshots",
        lambda ctx: called.__setitem__("snapshots", called["snapshots"] + 1) or snapshot_rows,
    )
    monkeypatch.setattr(
        "ait.aitk_export.list_lines",
        lambda ctx: called.__setitem__("lines", called["lines"] + 1) or line_rows,
    )
    monkeypatch.setattr(
        "ait.aitk_export.current_line",
        lambda ctx: called.__setitem__("current_line", called["current_line"] + 1) or "main",
    )
    monkeypatch.setattr(
        "ait.aitk_export.list_local_plans",
        lambda ctx: called.__setitem__("plans", called["plans"] + 1)
        or [
            {
                "plan_id": "PL-1",
                "title": "Plan one",
                "status": "draft",
                "head_revision_id": "PR-1",
                "head_artifact_path": "docs/sprints/plan_one.md",
                "head_artifact_selector": "plan-one/root",
                "head_artifact_heading": "Plan One",
            }
        ],
    )

    payload = build_aitk_history_payload(fake_ctx)

    assert called == {"snapshots": 1, "lines": 1, "current_line": 1, "plans": 1}
    assert payload["main_head_snapshot_id"] == "S3"
    assert payload["plan_links"][0]["display_path"] == "docs/sprints/plan_one.md#plan-one/root"
    assert payload["markdown_docs"] == []


def test_build_plan_links_scans_local_plans_and_markdown_artifacts(tmp_path: Path):
    ctx = store.RepoContext(
        root=tmp_path,
        ait_dir=tmp_path / ".ait",
        content_db_path=tmp_path / ".ait" / "content.db",
        control_db_path=tmp_path / ".ait" / "control.db",
        config_path=tmp_path / ".ait" / "config.json",
    )
    plans_dir = tmp_path / "docs" / "sprints"
    plans_dir.mkdir(parents=True)
    (plans_dir / "extra_plan.md").write_text("# Extra Plan\n", encoding="utf-8")

    links = build_plan_links(
        ctx,
        plans=[
            {
                "plan_id": "PL-1",
                "title": "Durable plan",
                "status": "open",
                "head_revision_id": "PR-1",
                "head_revision_number": 2,
                "head_artifact_path": "docs/sprints/durable_plan.md",
                "head_artifact_selector": "durable/root",
                "head_artifact_heading": "Durable Plan",
            }
        ],
    )

    assert links[0]["plan_id"] == "PL-1"
    assert links[0]["display_path"] == "docs/sprints/durable_plan.md#durable/root"
    assert links[1]["kind"] == "plan_artifact"
    assert links[1]["source"] == "docs/sprints"
    assert links[1]["title"] == "Extra Plan"


def test_build_markdown_docs_scans_all_docs_markdown(tmp_path: Path):
    ctx = store.RepoContext(
        root=tmp_path,
        ait_dir=tmp_path / ".ait",
        content_db_path=tmp_path / ".ait" / "content.db",
        control_db_path=tmp_path / ".ait" / "control.db",
        config_path=tmp_path / ".ait" / "config.json",
    )
    (tmp_path / "docs" / "sprints").mkdir(parents=True)
    (tmp_path / "docs" / "ait_native_quickstart.md").write_text("# Quickstart\n", encoding="utf-8")
    (tmp_path / "docs" / "sprints" / "feature.md").write_text("# Feature Plan\n", encoding="utf-8")
    (tmp_path / "docs" / "notes.txt").write_text("not markdown\n", encoding="utf-8")

    docs = build_markdown_docs(ctx)

    assert [doc["path"] for doc in docs] == ["docs/ait_native_quickstart.md", "docs/sprints/feature.md"]
    assert [doc["title"] for doc in docs] == ["Quickstart", "Feature Plan"]


def test_attach_snapshot_parent_diffs_includes_bounded_text_diff(tmp_path: Path):
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    ctx = store.init_repo(repo, "repo", "main")
    (repo / "a.txt").write_text("hello\nfoo\n", encoding="utf-8")
    first = local_content.create_snapshot(ctx, "repo", "main", "first")

    (repo / "a.txt").write_text("hello\nbar\n", encoding="utf-8")
    second = local_content.create_snapshot(ctx, "repo", "main", "second")

    payload = {
        "history_rows": [
            {
                "snapshot_id": second["snapshot_id"],
                "parent_snapshot_id": first["snapshot_id"],
            }
        ]
    }

    attach_snapshot_parent_diffs(ctx, payload, include_text=True, max_bytes=1_000_000)

    row = payload["history_rows"][0]
    assert row["changed_files"] == ["a.txt"]
    assert row["parent_diff"]["summary"]["files_changed"] == 1
    file_row = row["parent_diff"]["files"][0]
    assert file_row["path"] == "a.txt"
    assert file_row["diff"]["status"] == "text"
    assert "+bar" in (file_row["diff"]["text"] or "")
