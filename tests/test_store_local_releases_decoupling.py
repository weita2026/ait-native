from __future__ import annotations

from pathlib import Path

from ait import store
from ait import store_local_releases
from ait import store_local_workflow


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]

STORE_LOCAL_RELEASE_EXPORTS = (
    "create_local_release",
    "list_local_releases",
    "get_local_release",
    "update_local_release",
)

STORE_LOCAL_RELEASE_CALLERS = (
    "src/ait/release_ops.py",
    "src/ait/cli/commands/release.py",
)


def test_store_local_release_helpers_match_facades() -> None:
    for name in STORE_LOCAL_RELEASE_EXPORTS:
        assert getattr(store_local_releases, name) is getattr(store, name), name
        assert getattr(store_local_releases, name) is getattr(store_local_workflow, name), name


def test_store_local_release_helpers_are_extracted_from_facades() -> None:
    store_text = (WORKSPACE_ROOT / "src/ait/store.py").read_text(encoding="utf-8")
    workflow_text = (WORKSPACE_ROOT / "src/ait/store_local_workflow.py").read_text(encoding="utf-8")
    releases_text = (WORKSPACE_ROOT / "src/ait/store_local_releases.py").read_text(encoding="utf-8")

    assert "from .store_local_releases import (" in store_text
    assert "from .store_local_releases import (" in workflow_text
    assert "def create_local_release(" not in workflow_text
    assert "def list_local_releases(" not in workflow_text
    assert "def get_local_release(" not in workflow_text
    assert "def update_local_release(" not in workflow_text
    assert "from .store import (" not in releases_text


def test_store_local_release_callers_use_narrow_seam() -> None:
    for relative_path in STORE_LOCAL_RELEASE_CALLERS:
        text = (WORKSPACE_ROOT / relative_path).read_text(encoding="utf-8")
        assert "store_local_releases import" in text, relative_path
