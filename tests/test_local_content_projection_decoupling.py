from __future__ import annotations

from pathlib import Path

from ait import local_content
from ait import local_content_projection
from ait import store


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]

LOCAL_CONTENT_PROJECTION_EXPORTS = (
    "_normalize_markdown_artifact_path",
    "_is_lineage_only_markdown_artifact_path",
    "_effective_workspace_ignore_rules",
    "_path_is_projected_out_for_workspace",
    "allow_lineage_only_markdown_paths",
    "_filter_snapshot_file_map_for_workspace",
    "_filter_workspace_state_for_workspace",
)


def test_local_content_projection_helpers_match_facade(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    (repo / "seed.txt").write_text("seed\n", encoding="utf-8")
    ctx = store.init_repo(repo, "repo", "main")

    for name in LOCAL_CONTENT_PROJECTION_EXPORTS:
        assert getattr(local_content_projection, name) is getattr(local_content, name), name

    assert local_content_projection._normalize_markdown_artifact_path(r"docs\plan.md") == "docs/plan.md"
    assert local_content_projection._is_lineage_only_markdown_artifact_path("docs/plan.md") is True
    assert local_content_projection._path_is_projected_out_for_workspace(ctx, "docs/plan.md") is True

    filtered = local_content_projection._filter_snapshot_file_map_for_workspace(
        ctx,
        {
            "seed.txt": {"path": "seed.txt"},
            "docs/plan.md": {"path": "docs/plan.md"},
        },
    )
    assert filtered == {"seed.txt": {"path": "seed.txt"}}


def test_local_content_projection_helpers_are_extracted_from_facade() -> None:
    local_content_text = (WORKSPACE_ROOT / "src/ait/local_content.py").read_text(encoding="utf-8")
    helper_text = (WORKSPACE_ROOT / "src/ait/local_content_projection.py").read_text(encoding="utf-8")

    assert "from .local_content_projection import (" in local_content_text
    assert "def _normalize_markdown_artifact_path(" not in local_content_text
    assert "def _is_markdown_artifact_path(" not in local_content_text
    assert "def _is_lineage_only_markdown_artifact_path(" not in local_content_text
    assert "def _effective_workspace_ignore_rules(" not in local_content_text
    assert "def _path_is_projected_out_for_workspace(" not in local_content_text
    assert "def allow_lineage_only_markdown_paths(" not in local_content_text
    assert "def _filter_snapshot_file_map_for_workspace(" not in local_content_text
    assert "def _filter_workspace_state_for_workspace(" not in local_content_text
    assert "from .local_content import (" not in helper_text
