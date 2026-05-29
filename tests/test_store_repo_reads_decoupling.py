from __future__ import annotations

from pathlib import Path

from ait import store
from ait import store_repo_reads


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]

STORE_REPO_READ_EXPORTS = (
    "get_line",
    "iter_workspace_files",
    "snapshot_exists",
    "list_snapshots",
    "get_snapshot",
    "move_ref",
    "ref_history",
    "repo_status",
    "collect_snapshot_chain",
    "ensure_snapshot_chain",
)

STORE_REPO_READ_CALLERS = (
    "src/ait/aitk_export.py",
    "src/ait/release_ops.py",
)


def test_store_repo_reads_helpers_match_store_facade() -> None:
    for name in STORE_REPO_READ_EXPORTS:
        assert getattr(store_repo_reads, name) is getattr(store, name), name


def test_store_repo_reads_are_extracted_from_store_facade() -> None:
    store_text = (WORKSPACE_ROOT / "src/ait/store.py").read_text(encoding="utf-8")
    reads_text = (WORKSPACE_ROOT / "src/ait/store_repo_reads.py").read_text(encoding="utf-8")

    assert "from .store_repo_reads import (" in store_text
    assert "def get_line(" not in store_text
    assert "def iter_workspace_files(" not in store_text
    assert "def snapshot_exists(" not in store_text
    assert "def list_snapshots(" not in store_text
    assert "def get_snapshot(" not in store_text
    assert "def move_ref(" not in store_text
    assert "def ref_history(" not in store_text
    assert "def repo_status(" not in store_text
    assert "def collect_snapshot_chain(" not in store_text
    assert "def ensure_snapshot_chain(" not in store_text
    assert "from .store import (" not in reads_text
    assert "local_content_snapshots" in reads_text


def test_store_repo_read_callers_use_narrow_seam() -> None:
    for relative_path in STORE_REPO_READ_CALLERS:
        text = (WORKSPACE_ROOT / relative_path).read_text(encoding="utf-8")
        assert "from .store_repo_reads import (" in text, relative_path
