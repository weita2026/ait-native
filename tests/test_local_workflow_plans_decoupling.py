from __future__ import annotations

from pathlib import Path

from ait import local_control
from ait import local_workflow_plans
from ait import store


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]

LOCAL_WORKFLOW_PLAN_EXPORTS = (
    "create_workflow_plan",
    "list_workflow_plans",
    "get_workflow_plan",
    "list_workflow_plan_revisions",
    "get_workflow_plan_revision",
    "get_workflow_plan_revision_by_id",
    "revise_workflow_plan",
    "close_workflow_plan",
    "mark_workflow_plan_published",
)


def test_local_workflow_plan_helpers_match_local_control_facade() -> None:
    for name in LOCAL_WORKFLOW_PLAN_EXPORTS:
        assert getattr(local_workflow_plans, name) is getattr(local_control, name), name


def test_local_workflow_plan_domain_is_extracted_from_local_control_facade() -> None:
    local_control_text = (WORKSPACE_ROOT / "src/ait/local_control.py").read_text(encoding="utf-8")
    plan_text = (WORKSPACE_ROOT / "src/ait/local_workflow_plans.py").read_text(encoding="utf-8")

    assert "from .local_workflow_plans import (" in local_control_text
    assert "def create_workflow_plan(" not in local_control_text
    assert "def list_workflow_plans(" not in local_control_text
    assert "def get_workflow_plan(" not in local_control_text
    assert "def list_workflow_plan_revisions(" not in local_control_text
    assert "def get_workflow_plan_revision(" not in local_control_text
    assert "def get_workflow_plan_revision_by_id(" not in local_control_text
    assert "def revise_workflow_plan(" not in local_control_text
    assert "def close_workflow_plan(" not in local_control_text
    assert "def mark_workflow_plan_published(" not in local_control_text
    assert "from .local_control import (" not in plan_text


def test_local_workflow_plan_helpers_keep_plan_crud_behavior(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    ctx = store.init_repo(repo, "repo", "main")

    created = local_workflow_plans.create_workflow_plan(
        ctx,
        "PL-1000",
        "PR-1000",
        "repo",
        "Plan title",
        "docs/sprints/plan.md",
        None,
        "Plan title",
        [
            {
                "plan_item_ref": "plan/ref-1",
                "title": "First item",
                "status": "pending",
            }
        ],
        summary="initial summary",
    )

    assert created["plan_id"] == "PL-1000"
    assert local_workflow_plans.get_workflow_plan(ctx, "PL-1000") == created
    revisions = local_workflow_plans.list_workflow_plan_revisions(ctx, "PL-1000")
    assert [row["plan_revision_id"] for row in revisions] == ["PR-1000"]
    assert local_workflow_plans.get_workflow_plan_revision(ctx, "PL-1000", "PR-1000")["revision_number"] == 1
    assert local_workflow_plans.get_workflow_plan_revision_by_id(ctx, "PR-1000")["plan_id"] == "PL-1000"

    revised = local_workflow_plans.revise_workflow_plan(
        ctx,
        "PL-1000",
        "PR-1001",
        "docs/sprints/plan.md",
        None,
        "Plan title",
        [
            {
                "plan_item_ref": "plan/ref-1",
                "title": "Updated item",
                "status": "in_progress",
            }
        ],
        title="Plan title v2",
    )

    assert revised["head_revision_id"] == "PR-1001"
    assert local_workflow_plans.get_workflow_plan_revision_by_id(ctx, "PR-1001")["revision_number"] == 2

    published = local_workflow_plans.mark_workflow_plan_published(
        ctx,
        "PL-1000",
        remote_name="origin",
        published_plan_id="remote-plan-1000",
        published_head_revision_id="remote-plan-rev-1001",
        revision_mappings=[
            ("PR-1000", "remote-plan-rev-1000"),
            ("PR-1001", "remote-plan-rev-1001"),
        ],
    )

    assert published["publication_state"] == "published"
    assert published["published_plan_id"] == "remote-plan-1000"
    assert (
        local_workflow_plans.get_workflow_plan_revision_by_id(ctx, "PR-1001")["published_plan_revision_id"]
        == "remote-plan-rev-1001"
    )

    closed = local_workflow_plans.close_workflow_plan(ctx, "PL-1000", "archived")
    assert closed["status"] == "archived"
