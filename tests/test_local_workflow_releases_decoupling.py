from __future__ import annotations

from pathlib import Path

from ait import local_control
from ait import local_workflow_releases
from ait import store


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]

LOCAL_WORKFLOW_RELEASE_EXPORTS = (
    "create_workflow_release",
    "list_workflow_releases",
    "get_workflow_release",
    "update_workflow_release",
)


def test_local_workflow_release_helpers_match_local_control_facade() -> None:
    for name in LOCAL_WORKFLOW_RELEASE_EXPORTS:
        assert getattr(local_workflow_releases, name) is getattr(local_control, name), name


def test_local_workflow_release_domain_is_extracted_from_local_control_facade() -> None:
    local_control_text = (WORKSPACE_ROOT / "src/ait/local_control.py").read_text(encoding="utf-8")
    release_text = (WORKSPACE_ROOT / "src/ait/local_workflow_releases.py").read_text(encoding="utf-8")

    assert "from .local_workflow_releases import (" in local_control_text
    assert "def create_workflow_release(" not in local_control_text
    assert "def list_workflow_releases(" not in local_control_text
    assert "def get_workflow_release(" not in local_control_text
    assert "def update_workflow_release(" not in local_control_text
    assert "from .local_control import (" not in release_text


def test_local_workflow_release_helpers_keep_release_crud_behavior(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    ctx = store.init_repo(repo, "repo", "main")

    created = local_control.create_workflow_release(
        ctx,
        "RL-1000",
        "repo",
        "1.2.3",
        "main",
        "SNP-1000",
        "abc123",
        "self_hosted_public",
        metadata={"owner": "tests"},
    )

    assert created["release_id"] == "RL-1000"
    assert local_workflow_releases.get_workflow_release(ctx, "RL-1000") == created
    assert [row["release_id"] for row in local_workflow_releases.list_workflow_releases(ctx)] == ["RL-1000"]

    updated = local_control.update_workflow_release(
        ctx,
        "RL-1000",
        status="verified",
        checks=[{"name": "tests", "status": "passed"}],
        artifacts=[{"path": "dist/app.whl"}],
        formula={"url": "https://example.invalid/app.whl"},
    )

    assert updated["status"] == "verified"
    assert updated["checks_json"] == '[{"name": "tests", "status": "passed"}]'
    assert updated["artifacts_json"] == '[{"path": "dist/app.whl"}]'
    assert updated["formula_json"] == '{"url": "https://example.invalid/app.whl"}'
    assert updated["metadata_json"] == '{"owner": "tests"}'
