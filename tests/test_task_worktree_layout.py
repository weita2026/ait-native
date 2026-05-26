from __future__ import annotations

import plistlib
from importlib import import_module
from pathlib import Path

from typer.testing import CliRunner

from ait import local_content
from ait.cli.app import app
from ait.repo_paths import RepoContext
from ait.task_worktree_layout import (
    detect_init_task_worktree_defaults,
    path_is_memory_backed,
    resolve_main_seed_mirror_location,
    resolve_task_auto_worktree_location,
)


runner = CliRunner()


def _init_repo(tmp_path: Path, monkeypatch, name: str) -> tuple[Path, RepoContext]:
    repo = tmp_path / name
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)
    layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(layout, "_macos_ram_volume_roots", lambda: [])
    monkeypatch.setattr(layout, "_macos_ram_volume_specs", lambda: [])
    monkeypatch.setattr(layout, "_linux_detected_memory_roots", lambda: [])
    monkeypatch.setattr(layout, "_windows_ram_disk_roots", lambda: [])
    init_out = runner.invoke(app, ["init", "--name", name], catch_exceptions=False)
    assert init_out.exit_code == 0, init_out.stdout
    return repo, RepoContext.discover(repo)


def test_resolve_task_auto_worktree_location_prefers_configured_ephemeral_root_on_macos(tmp_path: Path, monkeypatch):
    repo, ctx = _init_repo(tmp_path, monkeypatch, "housekeeper-layout-macos-configured-root")
    layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(layout.sys, "platform", "darwin")

    location = resolve_task_auto_worktree_location(
        ctx,
        worktree_name="t-1234",
        ephemeral_root=".ram-root",
        alias_root=None,
    )

    assert location["root_source"] == "configured_ephemeral_root"
    assert location["target_path"] == (repo / ".ram-root" / "housekeeper-layout-macos-configured-root" / "t-1234").resolve()
    assert location["alias_path"] == (repo / ".ait" / "worktree-links" / "t-1234").resolve()
    assert location["preferred_path"] == location["alias_path"]


def test_resolve_main_seed_mirror_location_uses_hidden_internal_root(tmp_path: Path, monkeypatch):
    _, ctx = _init_repo(tmp_path, monkeypatch, "housekeeper-layout-main-seed")
    layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(layout.sys, "platform", "darwin")

    location = resolve_main_seed_mirror_location(
        ctx,
        seed_name="main-seed",
        ephemeral_root=tmp_path / "ram-root",
    )

    assert location is not None
    assert location["root_source"] == "configured_ephemeral_root"
    assert location["target_path"] == (
        tmp_path / "ram-root" / "housekeeper-layout-main-seed" / ".ait-internal" / "main-seed"
    ).resolve()
    assert location["preferred_path"] == location["target_path"]


def test_main_seed_ram_budget_status_reports_exceeded_when_head_snapshot_exceeds_budget(tmp_path: Path, monkeypatch):
    _, ctx = _init_repo(tmp_path, monkeypatch, "housekeeper-layout-main-seed-budget-status")
    layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(
        local_content,
        "get_line",
        lambda _ctx, line_name: {"line_name": line_name, "head_snapshot_id": "SNP-123"},
    )
    monkeypatch.setattr(
        local_content,
        "get_snapshot",
        lambda _ctx, snapshot_id: {"snapshot_id": snapshot_id, "total_bytes": 5},
    )

    status = layout.main_seed_ram_budget_status(ctx, main_seed_ram_max_bytes=1)

    assert status == {
        "default_line": "main",
        "seed_snapshot_id": "SNP-123",
        "seed_snapshot_total_bytes": 5,
        "main_seed_ram_max_bytes": 1,
        "exceeded": True,
    }


def test_resolve_task_auto_worktree_location_falls_back_when_main_seed_ram_budget_is_exceeded(
    tmp_path: Path,
    monkeypatch,
):
    _, ctx = _init_repo(tmp_path, monkeypatch, "housekeeper-layout-linux-main-seed-budget")
    layout = import_module("ait.task_worktree_layout")
    runtime_root = (tmp_path / "runtime-root").resolve()
    runtime_root.mkdir()
    monkeypatch.setattr(layout.sys, "platform", "linux")
    monkeypatch.setattr(layout, "_linux_detected_memory_roots", lambda: [runtime_root])
    monkeypatch.setattr(
        layout,
        "main_seed_ram_budget_status",
        lambda *_args, **_kwargs: {
            "default_line": "main",
            "seed_snapshot_id": "SNP-123",
            "seed_snapshot_total_bytes": 5,
            "main_seed_ram_max_bytes": 1,
            "exceeded": True,
        },
    )

    location = resolve_task_auto_worktree_location(
        ctx,
        worktree_name="t-1234",
        ephemeral_root=None,
        alias_root=None,
        main_seed_ram_max_bytes=1,
    )

    assert location["root_source"] == "repo_internal_fallback"
    assert location["fallback_reason"] == "main_seed_ram_budget_exceeded"
    assert location["target_path"] == (ctx.task_worktree_dir / "t-1234").resolve()
    assert location["alias_path"] is None
    assert location["seed_snapshot_total_bytes"] == 5
    assert location["main_seed_ram_max_bytes"] == 1


def test_resolve_main_seed_mirror_location_returns_none_when_main_seed_ram_budget_is_exceeded(
    tmp_path: Path,
    monkeypatch,
):
    _, ctx = _init_repo(tmp_path, monkeypatch, "housekeeper-layout-main-seed-budget")
    layout = import_module("ait.task_worktree_layout")
    runtime_root = (tmp_path / "runtime-root").resolve()
    runtime_root.mkdir()
    monkeypatch.setattr(layout.sys, "platform", "linux")
    monkeypatch.setattr(layout, "_linux_detected_memory_roots", lambda: [runtime_root])
    monkeypatch.setattr(
        layout,
        "main_seed_ram_budget_status",
        lambda *_args, **_kwargs: {
            "default_line": "main",
            "seed_snapshot_id": "SNP-123",
            "seed_snapshot_total_bytes": 5,
            "main_seed_ram_max_bytes": 1,
            "exceeded": True,
        },
    )

    location = resolve_main_seed_mirror_location(
        ctx,
        seed_name="main-seed",
        ephemeral_root=None,
        main_seed_ram_max_bytes=1,
    )

    assert location is None


def test_detect_init_task_worktree_defaults_persists_first_macos_ram_volume(tmp_path: Path, monkeypatch, host_ram_root: Path):
    repo, ctx = _init_repo(tmp_path, monkeypatch, "housekeeper-layout-macos-init-defaults")
    layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(layout.sys, "platform", "darwin")
    first_volume = {
        "kind": "macos_ram_volume",
        "root": str(host_ram_root),
        "volume_name": host_ram_root.name,
        "sector_count": 4194304,
    }
    second_root = (tmp_path / "Z_RAM").resolve()
    monkeypatch.setattr(
        layout,
        "_macos_ram_volume_specs",
        lambda: [dict(first_volume), {"kind": "macos_ram_volume", "root": str(second_root), "volume_name": second_root.name}],
    )
    monkeypatch.setattr(layout, "_macos_ram_volume_roots", lambda: [host_ram_root, second_root])

    defaults = detect_init_task_worktree_defaults(ctx)

    assert defaults == {
        "memory_root": first_volume,
    }
    expected_root = layout._auto_detected_ephemeral_root(ctx, host_ram_root)
    assert str(expected_root).startswith(str((host_ram_root / ".ait-repos").resolve()))
    assert str(expected_root) != str((repo / ".ait").resolve())


def test_decode_mountinfo_path_unescapes_spaces():
    from ait.task_worktree_layout import _decode_mountinfo_path

    assert _decode_mountinfo_path("/run/user/501/My\\040RAM") == "/run/user/501/My RAM"


def test_linux_mount_fstype_for_path_uses_deepest_mount(monkeypatch, tmp_path: Path):
    layout = import_module("ait.task_worktree_layout")
    target = tmp_path / "run" / "user" / "501"
    target.mkdir(parents=True)
    mountinfo = "\n".join(
        [
            "30 25 0:28 / / rw,relatime - apfs /dev/disk3s1 rw",
            f"31 30 0:44 / {target} rw,nosuid,nodev - tmpfs tmpfs rw",
        ]
    )
    monkeypatch.setattr(layout.Path, "read_text", lambda self, encoding='utf-8': mountinfo)

    assert layout._linux_mount_fstype_for_path(target / "ait-worktrees") == "tmpfs"


def test_linux_detected_memory_roots_prefers_verified_runtime_and_dev_shm(monkeypatch, tmp_path: Path):
    layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(layout.sys, "platform", "linux")
    runtime_root = (tmp_path / "runtime-root").resolve()
    runtime_root.mkdir()
    dev_shm_root = (tmp_path / "dev-shm").resolve()
    dev_shm_root.mkdir()
    tmp_root = (tmp_path / "tmp-root").resolve()
    tmp_root.mkdir()
    monkeypatch.setenv("XDG_RUNTIME_DIR", str(runtime_root))
    fstype_by_path = {
        str(runtime_root): "tmpfs",
        str(dev_shm_root): "tmpfs",
        str(tmp_root): "ext4",
    }
    monkeypatch.setattr(
        layout,
        "_linux_mount_fstype_for_path",
        lambda path: fstype_by_path.get(str(path.resolve())),
    )
    monkeypatch.setattr(layout, "Path", Path)

    original_path = layout.Path

    def fake_path(value: str | Path):
        text = str(value)
        if text == "/dev/shm":
            return dev_shm_root
        if text == "/tmp":
            return tmp_root
        return original_path(value)

    monkeypatch.setattr(layout, "Path", fake_path)

    roots = layout._linux_detected_memory_roots()

    assert roots == [runtime_root, dev_shm_root]


def test_detect_init_task_worktree_defaults_persists_first_linux_memory_root(tmp_path: Path, monkeypatch):
    _, ctx = _init_repo(tmp_path, monkeypatch, "housekeeper-layout-linux-init-defaults")
    layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(layout.sys, "platform", "linux")
    memory_root = (tmp_path / "run-user-501").resolve()
    monkeypatch.setattr(layout, "_linux_detected_memory_roots", lambda: [memory_root, (tmp_path / "dev-shm").resolve()])

    defaults = detect_init_task_worktree_defaults(ctx)

    assert defaults == {
        "memory_root": {"kind": "linux_memory_root", "root": str(memory_root)},
    }


def test_macos_ram_volume_roots_parse_hdiutil_plist(monkeypatch):
    layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(layout.sys, "platform", "darwin")
    payload = {
        "images": [
            {
                "image-path": "/Users/example/Installer.dmg",
                "system-entities": [{"mount-point": "/Volumes/Installer"}],
                "writeable": False,
            },
            {
                "image-path": "ram://4194304",
                "system-entities": [{"dev-entry": "/dev/disk10", "mount-point": "/Volumes/AIT_RAM"}],
                "writeable": True,
            },
            {
                "image-path": "ram://2048",
                "system-entities": [{"dev-entry": "/dev/disk11"}],
                "writeable": True,
            },
        ]
    }
    monkeypatch.setattr(layout.subprocess, "check_output", lambda *args, **kwargs: plistlib.dumps(payload))

    roots = layout._macos_ram_volume_roots()

    assert roots == [Path("/Volumes/AIT_RAM").resolve()]


def test_resolve_task_auto_worktree_location_reprovisions_macos_ram_volume_from_saved_spec(
    tmp_path: Path,
    monkeypatch,
):
    _, ctx = _init_repo(tmp_path, monkeypatch, "housekeeper-layout-macos-reprovision")
    layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(layout.sys, "platform", "darwin")
    ram_root = (tmp_path / "Volumes" / "AIT_RAM").resolve()
    mounted_roots: list[Path] = []

    def fake_specs():
        if not mounted_roots:
            return []
        return [
            {
                "kind": "macos_ram_volume",
                "root": str(mounted_roots[0]),
                "volume_name": mounted_roots[0].name,
                "sector_count": 4194304,
            }
        ]

    def fake_provision(spec: dict[str, object]) -> bool:
        mounted_roots[:] = [ram_root]
        ram_root.mkdir(parents=True, exist_ok=True)
        return True

    monkeypatch.setattr(layout, "_macos_ram_volume_specs", fake_specs)
    monkeypatch.setattr(layout, "_macos_ram_volume_roots", lambda: list(mounted_roots))
    monkeypatch.setattr(layout, "_provision_macos_ram_volume", fake_provision)

    location = resolve_task_auto_worktree_location(
        ctx,
        worktree_name="rt-1551",
        ephemeral_root=None,
        alias_root=None,
        memory_root={
            "kind": "macos_ram_volume",
            "root": str(ram_root),
            "volume_name": "AIT_RAM",
            "sector_count": 4194304,
        },
    )

    assert location["root_source"] == "macos_ram_volume"
    assert location["target_path"] == (
        layout._auto_detected_ephemeral_root(ctx, ram_root) / "housekeeper-layout-macos-reprovision" / "rt-1551"
    ).resolve()
    assert location["preferred_path"] == location["alias_path"]


def test_resolve_task_auto_worktree_location_bootstraps_default_macos_ram_volume_without_saved_spec(
    tmp_path: Path,
    monkeypatch,
):
    _, ctx = _init_repo(tmp_path, monkeypatch, "housekeeper-layout-macos-default-bootstrap")
    layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(layout.sys, "platform", "darwin")
    ram_root = (tmp_path / "Volumes" / "AIT_RAM").resolve()
    mounted_roots: list[Path] = []
    default_spec = {
        "kind": "macos_ram_volume",
        "root": str(ram_root),
        "volume_name": "AIT_RAM",
        "sector_count": 4194304,
    }

    def fake_specs():
        if not mounted_roots:
            return []
        return [dict(default_spec)]

    def fake_provision(spec: dict[str, object]) -> bool:
        assert spec == default_spec
        mounted_roots[:] = [ram_root]
        ram_root.mkdir(parents=True, exist_ok=True)
        return True

    monkeypatch.setattr(layout, "_macos_ram_volume_specs", fake_specs)
    monkeypatch.setattr(layout, "_default_macos_ram_volume_spec", lambda: dict(default_spec))
    monkeypatch.setattr(layout, "_provision_macos_ram_volume", fake_provision)

    location = resolve_task_auto_worktree_location(
        ctx,
        worktree_name="rt-1554",
        ephemeral_root=None,
        alias_root=None,
    )

    assert location["root_source"] == "macos_ram_volume"
    assert location["target_path"] == (
        layout._auto_detected_ephemeral_root(ctx, ram_root) / "housekeeper-layout-macos-default-bootstrap" / "rt-1554"
    ).resolve()
    assert location["preferred_path"] == location["alias_path"]


def test_resolve_main_seed_mirror_location_bootstraps_default_macos_ram_volume_without_saved_spec(
    tmp_path: Path,
    monkeypatch,
):
    _, ctx = _init_repo(tmp_path, monkeypatch, "housekeeper-layout-macos-main-seed-bootstrap")
    layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(layout.sys, "platform", "darwin")
    ram_root = (tmp_path / "Volumes" / "AIT_RAM").resolve()
    mounted_roots: list[Path] = []
    default_spec = {
        "kind": "macos_ram_volume",
        "root": str(ram_root),
        "volume_name": "AIT_RAM",
        "sector_count": 4194304,
    }

    def fake_specs():
        if not mounted_roots:
            return []
        return [dict(default_spec)]

    def fake_provision(spec: dict[str, object]) -> bool:
        assert spec == default_spec
        mounted_roots[:] = [ram_root]
        ram_root.mkdir(parents=True, exist_ok=True)
        return True

    monkeypatch.setattr(layout, "_macos_ram_volume_specs", fake_specs)
    monkeypatch.setattr(layout, "_default_macos_ram_volume_spec", lambda: dict(default_spec))
    monkeypatch.setattr(layout, "_provision_macos_ram_volume", fake_provision)

    location = resolve_main_seed_mirror_location(
        ctx,
        seed_name="main-seed",
        ephemeral_root=None,
    )

    assert location is not None
    assert location["root_source"] == "macos_ram_volume"
    assert location["target_path"] == (
        layout._auto_detected_ephemeral_root(ctx, ram_root)
        / "housekeeper-layout-macos-main-seed-bootstrap"
        / ".ait-internal"
        / "main-seed"
    ).resolve()
    assert location["preferred_path"] == location["target_path"]


def test_path_is_memory_backed_on_macos_when_path_resolves_under_ram_volume(tmp_path: Path, monkeypatch):
    layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(layout.sys, "platform", "darwin")
    ram_root = (tmp_path / "ram-volume").resolve()
    target_path = ram_root / ".ait-repos" / "repo" / "rt-1550"
    target_path.mkdir(parents=True)
    monkeypatch.setattr(layout, "_macos_ram_volume_roots", lambda: [ram_root])

    assert path_is_memory_backed(target_path) is True


def test_path_is_memory_backed_false_on_macos_when_path_is_not_under_ram_volume(tmp_path: Path, monkeypatch):
    layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(layout.sys, "platform", "darwin")
    ram_root = (tmp_path / "ram-volume").resolve()
    disk_root = (tmp_path / "disk-volume").resolve()
    target_path = disk_root / ".ait-repos" / "repo" / "rt-1550"
    target_path.mkdir(parents=True)
    monkeypatch.setattr(layout, "_macos_ram_volume_roots", lambda: [ram_root])

    assert path_is_memory_backed(target_path) is False


def test_auto_detected_ephemeral_root_scopes_by_repo_path(tmp_path: Path, monkeypatch, host_ram_root: Path):
    repo_a, ctx_a = _init_repo(tmp_path, monkeypatch, "housekeeper-layout-scope-a")
    repo_b = tmp_path / "housekeeper-layout-scope-b"
    repo_b.mkdir()
    (repo_b / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo_b)
    layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(layout, "_macos_ram_volume_roots", lambda: [])
    monkeypatch.setattr(layout, "_macos_ram_volume_specs", lambda: [])
    init_out = runner.invoke(app, ["init", "--name", "housekeeper-layout-scope-a"], catch_exceptions=False)
    assert init_out.exit_code == 0, init_out.stdout
    ctx_b = RepoContext.discover(repo_b)

    scoped_a = layout._auto_detected_ephemeral_root(ctx_a, host_ram_root)
    scoped_b = layout._auto_detected_ephemeral_root(ctx_b, host_ram_root)

    assert scoped_a != scoped_b
    assert scoped_a.parent == scoped_b.parent == (host_ram_root / ".ait-repos").resolve()


def test_resolve_task_auto_worktree_location_macos_legacy_missing_volume_reprovisions_from_path(
    tmp_path: Path,
    monkeypatch,
):
    _, ctx = _init_repo(tmp_path, monkeypatch, "housekeeper-layout-macos-legacy-fallback")
    layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(layout.sys, "platform", "darwin")
    missing_root = Path("/Volumes/AIT_RAM")
    expected_spec = {
        "kind": "macos_ram_volume",
        "root": str(missing_root.resolve()),
        "volume_name": "AIT_RAM",
        "sector_count": 4194304,
    }
    provisioned = False

    def fake_specs():
        return [dict(expected_spec)] if provisioned else []

    def fake_provision(spec: dict[str, object]) -> bool:
        nonlocal provisioned
        assert spec == expected_spec
        provisioned = True
        return True

    monkeypatch.setattr(layout, "_macos_ram_volume_specs", fake_specs)
    monkeypatch.setattr(layout, "_provision_macos_ram_volume", fake_provision)
    monkeypatch.setattr(layout, "_ensure_root_candidate", lambda path: path.resolve())

    location = resolve_task_auto_worktree_location(
        ctx,
        worktree_name="rt-1551",
        ephemeral_root=str(layout._auto_detected_ephemeral_root(ctx, missing_root)),
        alias_root=None,
    )

    expected_target_root = layout._auto_detected_ephemeral_root(ctx, missing_root) / "housekeeper-layout-macos-legacy-fallback"
    assert location["root_source"] == "configured_ephemeral_root"
    assert location["target_path"] == (expected_target_root / "rt-1551").resolve()
    assert location["alias_path"] == (ctx.repo_root / ".ait" / "worktree-links" / "rt-1551").resolve()
    assert location["preferred_path"] == location["alias_path"]


def test_windows_ram_disk_roots_prefers_env_roots_and_dedupes(monkeypatch):
    layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(layout.sys, "platform", "win32")
    monkeypatch.setenv("LOCALAPPDATA", "R:\\Users\\Alice\\AppData\\Local")
    monkeypatch.setenv("TEMP", "R:\\Temp")
    monkeypatch.setenv("TMP", "C:\\Temp")
    monkeypatch.setattr(layout.tempfile, "gettempdir", lambda: "R:\\Temp")
    monkeypatch.setattr(layout, "_windows_list_drive_roots", lambda: [Path("R:\\"), Path("S:\\"), Path("C:\\")])
    drive_type = {
        "R:\\": layout._WINDOWS_DRIVE_RAMDISK,
        "S:\\": layout._WINDOWS_DRIVE_RAMDISK,
        "C:\\": 3,
    }
    monkeypatch.setattr(layout, "_windows_get_drive_type", lambda root: drive_type.get(str(root), 3))

    roots = layout._windows_ram_disk_roots()

    assert roots == [Path("R:\\").resolve(), Path("S:\\").resolve()]


def test_detect_init_task_worktree_defaults_persists_first_windows_ram_disk(tmp_path: Path, monkeypatch):
    _, ctx = _init_repo(tmp_path, monkeypatch, "housekeeper-layout-windows-init-defaults")
    layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(layout.sys, "platform", "win32")
    ram_root = (tmp_path / "RamDisk").resolve()
    monkeypatch.setattr(layout, "_windows_ram_disk_roots", lambda: [ram_root, (tmp_path / "OtherRamDisk").resolve()])

    defaults = detect_init_task_worktree_defaults(ctx)

    assert defaults == {
        "memory_root": {"kind": "windows_ramdisk", "root": str(ram_root)},
    }


def test_resolve_task_auto_worktree_location_linux_falls_back_through_candidate_chain(tmp_path: Path, monkeypatch):
    _, ctx = _init_repo(tmp_path, monkeypatch, "housekeeper-layout-linux-fallback-chain")
    layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(layout.sys, "platform", "linux")

    candidate_paths = [
        (tmp_path / "runtime-root" / "ait-worktrees" / "housekeeper-layout-linux-fallback-chain", "linux_xdg_runtime_dir"),
        (tmp_path / "dev-shm" / "ait-worktrees" / "housekeeper-layout-linux-fallback-chain", "linux_dev_shm"),
        (tmp_path / "tmp-root" / "ait-worktrees" / "housekeeper-layout-linux-fallback-chain", "linux_tmp"),
    ]
    monkeypatch.setattr(layout, "_linux_ephemeral_root_candidates", lambda _: candidate_paths)

    def fake_ensure_root_candidate(path: Path) -> Path | None:
        if path == candidate_paths[0][0]:
            return None
        path.mkdir(parents=True, exist_ok=True)
        return path.resolve()

    monkeypatch.setattr(layout, "_ensure_root_candidate", fake_ensure_root_candidate)

    location = resolve_task_auto_worktree_location(
        ctx,
        worktree_name="t-1234",
        ephemeral_root=None,
        alias_root=None,
    )

    assert location["root_source"] == "linux_dev_shm"
    assert location["target_path"] == (candidate_paths[1][0] / "t-1234").resolve()
    assert location["preferred_path"] == location["alias_path"]


def test_resolve_task_auto_worktree_location_linux_falls_back_to_workspace_when_candidates_fail(tmp_path: Path, monkeypatch):
    repo, ctx = _init_repo(tmp_path, monkeypatch, "housekeeper-layout-linux-workspace-fallback")
    layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(layout.sys, "platform", "linux")
    monkeypatch.setattr(
        layout,
        "_linux_ephemeral_root_candidates",
        lambda _: [
            (tmp_path / "runtime-root" / "ait-worktrees" / "housekeeper-layout-linux-workspace-fallback", "linux_xdg_runtime_dir"),
            (tmp_path / "dev-shm" / "ait-worktrees" / "housekeeper-layout-linux-workspace-fallback", "linux_dev_shm"),
            (tmp_path / "tmp-root" / "ait-worktrees" / "housekeeper-layout-linux-workspace-fallback", "linux_tmp"),
        ],
    )
    monkeypatch.setattr(layout, "_ensure_root_candidate", lambda _: None)

    location = resolve_task_auto_worktree_location(
        ctx,
        worktree_name="t-1234",
        ephemeral_root=None,
        alias_root=None,
    )

    assert location["root_source"] == "repo_internal_fallback"
    assert location["target_path"] == (repo / ".ait" / "worktree" / "t-1234").resolve()
    assert location["alias_path"] is None
    assert location["preferred_path"] == location["target_path"]


def test_resolve_task_auto_worktree_location_windows_falls_back_through_candidate_chain(tmp_path: Path, monkeypatch):
    _, ctx = _init_repo(tmp_path, monkeypatch, "housekeeper-layout-windows-fallback-chain")
    layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(layout.sys, "platform", "win32")

    candidate_paths = [
        (tmp_path / "LocalAppData" / "Temp" / "ait-worktrees" / "housekeeper-layout-windows-fallback-chain", "windows_localappdata_temp"),
        (tmp_path / "TempRoot" / "ait-worktrees" / "housekeeper-layout-windows-fallback-chain", "windows_temp"),
        (tmp_path / "PyTemp" / "ait-worktrees" / "housekeeper-layout-windows-fallback-chain", "windows_tempfile"),
    ]
    monkeypatch.setattr(layout, "_windows_ephemeral_root_candidates", lambda _: candidate_paths)

    def fake_ensure_root_candidate(path: Path) -> Path | None:
        if path == candidate_paths[0][0]:
            return None
        path.mkdir(parents=True, exist_ok=True)
        return path.resolve()

    monkeypatch.setattr(layout, "_ensure_root_candidate", fake_ensure_root_candidate)

    location = resolve_task_auto_worktree_location(
        ctx,
        worktree_name="t-1234",
        ephemeral_root=None,
        alias_root=None,
    )

    assert location["root_source"] == "windows_temp"
    assert location["target_path"] == (candidate_paths[1][0] / "t-1234").resolve()
    assert location["preferred_path"] == location["alias_path"]


def test_resolve_task_auto_worktree_location_windows_skips_stale_configured_ramdisk(
    tmp_path: Path,
    monkeypatch,
):
    _, ctx = _init_repo(tmp_path, monkeypatch, "housekeeper-layout-windows-stale-configured-root")
    layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(layout.sys, "platform", "win32")
    current_root = (tmp_path / "CurrentRamDisk").resolve()
    monkeypatch.setattr(layout, "_windows_ram_disk_roots", lambda: [current_root])

    location = resolve_task_auto_worktree_location(
        ctx,
        worktree_name="t-1234",
        ephemeral_root=str(layout._auto_detected_ephemeral_root(ctx, tmp_path / "MissingRamDisk")),
        alias_root=None,
        memory_root={"kind": "windows_ramdisk", "root": str((tmp_path / "MissingRamDisk").resolve())},
    )

    assert location["root_source"] == "windows_ramdisk"
    assert location["target_path"] == (
        layout._auto_detected_ephemeral_root(ctx, current_root) / "housekeeper-layout-windows-stale-configured-root" / "t-1234"
    ).resolve()


def test_resolve_task_auto_worktree_location_windows_falls_back_to_workspace_when_candidates_fail(tmp_path: Path, monkeypatch):
    repo, ctx = _init_repo(tmp_path, monkeypatch, "housekeeper-layout-windows-workspace-fallback")
    layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(layout.sys, "platform", "win32")
    monkeypatch.setattr(
        layout,
        "_windows_ephemeral_root_candidates",
        lambda _: [
            (tmp_path / "LocalAppData" / "Temp" / "ait-worktrees" / "housekeeper-layout-windows-workspace-fallback", "windows_localappdata_temp"),
            (tmp_path / "TempRoot" / "ait-worktrees" / "housekeeper-layout-windows-workspace-fallback", "windows_temp"),
        ],
    )
    monkeypatch.setattr(layout, "_ensure_root_candidate", lambda _: None)

    location = resolve_task_auto_worktree_location(
        ctx,
        worktree_name="t-1234",
        ephemeral_root=None,
        alias_root=None,
    )

    assert location["root_source"] == "repo_internal_fallback"
    assert location["target_path"] == (repo / ".ait" / "worktree" / "t-1234").resolve()
    assert location["alias_path"] is None
    assert location["preferred_path"] == location["target_path"]


def test_resolve_task_auto_worktree_location_linux_skips_stale_configured_memory_root(
    tmp_path: Path,
    monkeypatch,
):
    _, ctx = _init_repo(tmp_path, monkeypatch, "housekeeper-layout-linux-stale-configured-root")
    layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(layout.sys, "platform", "linux")
    current_root = (tmp_path / "runtime-root").resolve()
    monkeypatch.setattr(layout, "_linux_detected_memory_roots", lambda: [current_root])

    location = resolve_task_auto_worktree_location(
        ctx,
        worktree_name="t-1234",
        ephemeral_root=str(layout._auto_detected_ephemeral_root(ctx, tmp_path / "missing-runtime-root")),
        alias_root=None,
        memory_root={"kind": "linux_memory_root", "root": str((tmp_path / "missing-runtime-root").resolve())},
    )

    assert location["root_source"] == "linux_memory_root"
    assert location["target_path"] == (
        layout._auto_detected_ephemeral_root(ctx, current_root) / "housekeeper-layout-linux-stale-configured-root" / "t-1234"
    ).resolve()
