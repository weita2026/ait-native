from __future__ import annotations

from pathlib import Path

from ait import store_worktree_filesystem
from ait import store_worktrees


def test_create_directory_link_prefers_windows_junction_helper(tmp_path: Path, monkeypatch):
    target = tmp_path / "target"
    target.mkdir()
    link = tmp_path / "alias"
    calls: list[tuple[Path, Path]] = []

    monkeypatch.setattr(store_worktree_filesystem, "_is_windows_platform", lambda: True)

    def fake_create_windows_directory_junction(link_path: Path, target_path: Path) -> None:
        calls.append((link_path, target_path))
        link_path.symlink_to(target_path, target_is_directory=True)

    monkeypatch.setattr(store_worktree_filesystem, "_create_windows_directory_junction", fake_create_windows_directory_junction)

    store_worktrees._create_directory_link(link, target)

    assert calls == [(link, target)]
    assert link.is_symlink()
    assert link.resolve() == target.resolve()


def test_create_directory_link_falls_back_to_symlink_when_windows_junction_fails(tmp_path: Path, monkeypatch):
    target = tmp_path / "target"
    target.mkdir()
    link = tmp_path / "alias"

    monkeypatch.setattr(store_worktree_filesystem, "_is_windows_platform", lambda: True)
    monkeypatch.setattr(
        store_worktree_filesystem,
        "_create_windows_directory_junction",
        lambda *_args, **_kwargs: (_ for _ in ()).throw(OSError("junction unavailable")),
    )

    store_worktrees._create_directory_link(link, target)

    assert link.is_symlink()
    assert link.resolve() == target.resolve()
