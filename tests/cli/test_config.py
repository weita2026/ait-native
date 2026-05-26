from __future__ import annotations

import pytest

from ._shared import *  # noqa: F401,F403


@pytest.fixture(autouse=True)
def _disable_host_macos_ram_volume_detection(monkeypatch):
    task_worktree_layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(task_worktree_layout, "_macos_ram_volume_roots", lambda: [])
    monkeypatch.setattr(task_worktree_layout, "_macos_ram_volume_specs", lambda: [])
    monkeypatch.setattr(task_worktree_layout, "_linux_detected_memory_roots", lambda: [])
    monkeypatch.setattr(task_worktree_layout, "_windows_ram_disk_roots", lambda: [])


def _task_worktree_summary(
    *,
    ephemeral_root: str | None = None,
    ephemeral_root_source: str = "built_in",
    alias_root: str = ".ait/worktree-links",
    alias_root_source: str = "built_in",
    memory_root: dict[str, object] | None = None,
    memory_root_source: str = "built_in",
    main_seed_ram_max_bytes: int | None = None,
    main_seed_ram_max_bytes_source: str = "built_in",
) -> dict[str, dict[str, object | None]]:
    return {
        "ephemeral_root": {"value": ephemeral_root, "source": ephemeral_root_source},
        "alias_root": {"value": alias_root, "source": alias_root_source},
        "memory_root": {"value": memory_root, "source": memory_root_source},
        "main_seed_ram_max_bytes": {
            "value": main_seed_ram_max_bytes,
            "source": main_seed_ram_max_bytes_source,
        },
    }


def test_init_writes_prototype_policy_by_default(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper"
    repo.mkdir()
    monkeypatch.chdir(repo)

    result = runner.invoke(app, ["init", "--name", "housekeeper", "--json"], catch_exceptions=False)
    assert result.exit_code == 0, result.stdout
    payload = json.loads(result.stdout)
    assert payload["policy_profile"] == "prototype"
    assert payload["task_worktree"] == _task_worktree_summary()

    policy_text = (repo / ".ait" / "policy.yaml").read_text(encoding="utf-8")
    assert "policy_id: prototype" in policy_text
    assert "require_tests: true" in policy_text
    assert "require_lint: false" in policy_text
    assert "require_security_scan: false" in policy_text
    assert "require_license_scan: false" in policy_text
    assert "require_ai_provenance: false" in policy_text
    assert "class_overrides:" in policy_text
    assert "content_class: docs_only" in policy_text

    config_data = json.loads((repo / ".ait" / "config.json").read_text(encoding="utf-8"))
    assert config_data["id_namespace_prefix"] == ""
    assert "task_worktree" not in config_data


def test_init_persists_empty_id_namespace_prefix_by_default(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-empty-namespace-default"
    repo.mkdir()
    monkeypatch.chdir(repo)

    result = runner.invoke(app, ["init", "--name", "housekeeper", "--json"], catch_exceptions=False)
    assert result.exit_code == 0, result.stdout

    config_data = json.loads((repo / ".ait" / "config.json").read_text(encoding="utf-8"))
    assert config_data["id_namespace_prefix"] == ""

    show_result = runner.invoke(app, ["config", "show", "--json"], catch_exceptions=False)
    assert show_result.exit_code == 0, show_result.stdout
    shown = json.loads(show_result.stdout)
    assert shown["id_namespace_prefix"] == {
        "value": "",
        "source": "repo_config",
    }


def test_init_persists_detected_macos_ram_volume_as_ephemeral_root(tmp_path: Path, monkeypatch, host_ram_root: Path):
    repo = tmp_path / "housekeeper-macos-init-ram-root"
    repo.mkdir()
    monkeypatch.chdir(repo)

    task_worktree_layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(task_worktree_layout.sys, "platform", "darwin")
    memory_root = {
        "kind": "macos_ram_volume",
        "root": str(host_ram_root),
        "volume_name": host_ram_root.name,
        "sector_count": 4194304,
    }
    monkeypatch.setattr(task_worktree_layout, "_macos_ram_volume_specs", lambda: [dict(memory_root)])
    monkeypatch.setattr(task_worktree_layout, "_macos_ram_volume_roots", lambda: [host_ram_root])

    result = runner.invoke(app, ["init", "--name", "housekeeper", "--json"], catch_exceptions=False)
    assert result.exit_code == 0, result.stdout
    payload = json.loads(result.stdout)
    configured_root = payload["task_worktree"]["ephemeral_root"]["value"]
    assert configured_root is not None
    assert str(configured_root).startswith(str((host_ram_root / ".ait-repos").resolve()))
    assert payload["task_worktree"] == _task_worktree_summary(
        ephemeral_root=configured_root,
        ephemeral_root_source="derived_from_memory_root",
        memory_root=memory_root,
        memory_root_source="repo_config",
    )

    config_data = json.loads((repo / ".ait" / "config.json").read_text(encoding="utf-8"))
    assert config_data["task_worktree"] == {
        "memory_root": memory_root,
    }


def test_init_persists_detected_linux_memory_root_as_ephemeral_root(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-linux-init-ram-root"
    repo.mkdir()
    monkeypatch.chdir(repo)

    task_worktree_layout = import_module("ait.task_worktree_layout")
    memory_root = (tmp_path / "run-user-501").resolve()
    monkeypatch.setattr(task_worktree_layout.sys, "platform", "linux")
    monkeypatch.setattr(task_worktree_layout, "_linux_detected_memory_roots", lambda: [memory_root])

    result = runner.invoke(app, ["init", "--name", "housekeeper", "--json"], catch_exceptions=False)
    assert result.exit_code == 0, result.stdout
    payload = json.loads(result.stdout)
    configured_root = payload["task_worktree"]["ephemeral_root"]["value"]
    assert configured_root is not None
    assert str(configured_root).startswith(str((memory_root / ".ait-repos").resolve()))
    assert payload["task_worktree"] == _task_worktree_summary(
        ephemeral_root=configured_root,
        ephemeral_root_source="derived_from_memory_root",
        memory_root={"kind": "linux_memory_root", "root": str(memory_root)},
        memory_root_source="repo_config",
    )

    config_data = json.loads((repo / ".ait" / "config.json").read_text(encoding="utf-8"))
    assert config_data["task_worktree"] == {
        "memory_root": {"kind": "linux_memory_root", "root": str(memory_root)},
    }


def test_init_persists_detected_windows_ram_disk_as_ephemeral_root(tmp_path: Path, monkeypatch, host_ram_root: Path):
    repo = tmp_path / "housekeeper-windows-init-ram-root"
    repo.mkdir()
    monkeypatch.chdir(repo)

    task_worktree_layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(task_worktree_layout.sys, "platform", "win32")
    monkeypatch.setattr(task_worktree_layout, "_windows_ram_disk_roots", lambda: [host_ram_root])

    result = runner.invoke(app, ["init", "--name", "housekeeper", "--json"], catch_exceptions=False)
    assert result.exit_code == 0, result.stdout
    payload = json.loads(result.stdout)
    configured_root = payload["task_worktree"]["ephemeral_root"]["value"]
    assert configured_root is not None
    assert str(configured_root).startswith(str((host_ram_root / ".ait-repos").resolve()))
    assert payload["task_worktree"] == _task_worktree_summary(
        ephemeral_root=configured_root,
        ephemeral_root_source="derived_from_memory_root",
        memory_root={"kind": "windows_ramdisk", "root": str(host_ram_root)},
        memory_root_source="repo_config",
    )

    config_data = json.loads((repo / ".ait" / "config.json").read_text(encoding="utf-8"))
    assert config_data["task_worktree"] == {
        "memory_root": {"kind": "windows_ramdisk", "root": str(host_ram_root)},
    }


def test_config_set_rejects_removed_task_worktree_ephemeral_root_options(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-config-set-removed-task-worktree-root"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    set_out = runner.invoke(app, ["config", "set", "--task-worktree-ephemeral-root", "/tmp/ait-ram"], catch_exceptions=False)
    assert set_out.exit_code != 0

    clear_out = runner.invoke(app, ["config", "set", "--clear-task-worktree-ephemeral-root"], catch_exceptions=False)
    assert clear_out.exit_code != 0


def test_config_set_alias_root_prunes_derived_ephemeral_root_compat_entry(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-config-set-prune-derived-root"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    repo_ctx = RepoContext.discover(repo)
    task_worktree_layout = import_module("ait.task_worktree_layout")
    derived_root = str(task_worktree_layout._auto_detected_ephemeral_root(repo_ctx, Path("/Volumes/AIT_RAM")))
    config_path = repo / ".ait" / "config.json"
    saved = json.loads(config_path.read_text(encoding="utf-8"))
    saved["task_worktree"] = {
        "ephemeral_root": derived_root,
        "memory_root": {
            "kind": "macos_ram_volume",
            "root": "/Volumes/AIT_RAM",
            "volume_name": "AIT_RAM",
            "sector_count": 4194304,
        },
    }
    config_path.write_text(json.dumps(saved, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    set_out = runner.invoke(
        app,
        ["config", "set", "--task-worktree-alias-root", ".ait/ram-links", "--json"],
        catch_exceptions=False,
    )
    assert set_out.exit_code == 0, set_out.stdout

    updated = json.loads(config_path.read_text(encoding="utf-8"))
    assert updated["task_worktree"] == {
        "alias_root": ".ait/ram-links",
        "memory_root": {
            "kind": "macos_ram_volume",
            "root": "/Volumes/AIT_RAM",
            "volume_name": "AIT_RAM",
            "sector_count": 4194304,
        },
    }


def test_repo_config_update_path_preserves_default_remote_after_earlier_write(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-config-update-preserves-default-remote"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    store_module = import_module("ait.store")
    store_repo_config = import_module("ait.store_repo_config")
    config_module = import_module("ait.cli.commands.config")
    ctx = RepoContext.discover(repo)

    store_module.add_remote(ctx, "origin", "http://example.invalid", "housekeeper", make_default=True)

    store_repo_config.update_config(
        ctx,
        lambda cfg: config_module._apply_config_set_updates(
            ctx,
            cfg,
            default_author_mode=None,
            clear_default_author_mode=False,
            default_model=None,
            clear_default_model=False,
            task_tracking=None,
            command_profiling=None,
            task_dag_allow_multi_worker=None,
            task_worktree_alias_root=None,
            clear_task_worktree_alias_root=False,
            task_worktree_main_seed_ram_max_bytes=None,
            clear_task_worktree_main_seed_ram_max_bytes=False,
            legacy_task_auto_worktree=None,
            workflow_mode="solo_remote",
            workflow_default_scope=None,
            clear_workflow_default_scope=False,
            task_default_scope=None,
            clear_task_default_scope=False,
            change_default_scope=None,
            clear_change_default_scope=False,
            id_namespace_prefix=None,
            clear_id_namespace_prefix=False,
            plan_task_binding_mode=None,
            clear_plan_task_binding=False,
            user_name=None,
            clear_user_name=False,
            user_email=None,
            clear_user_email=False,
        ),
    )

    saved = json.loads((repo / ".ait" / "config.json").read_text(encoding="utf-8"))
    assert saved["default_remote"] == "origin"
    assert saved["workflow_mode"] == "solo_remote"
    assert saved["workflow_default_scope"] == "remote"
    assert saved["task_default_scope"] == "remote"
    assert saved["change_default_scope"] == "remote"


def test_policy_yaml_roundtrip_supports_class_overrides():
    policy_text = """
version: 1
policy_id: prototype
defaults:
  require_attestation: true
  require_tests: true
  require_lint: false
  require_security_scan: false
  require_license_scan: false
  require_ai_provenance: false
class_overrides:
  - when:
      content_class: docs_only
    set:
      require_tests: false
      require_lint: false
  - when:
      author_class: ai_related
    set:
      require_attestation: true
      require_ai_provenance: true
""".strip()
    parsed = parse_policy_yaml(policy_text)
    assert parsed["defaults"]["require_ai_provenance"] is False
    assert parsed["class_overrides"][0]["when"]["content_class"] == "docs_only"
    assert parsed["class_overrides"][0]["set"]["require_tests"] is False
    assert parsed["class_overrides"][1]["when"]["author_class"] == "ai_related"
    assert parsed["class_overrides"][1]["set"]["require_ai_provenance"] is True

    roundtrip = parse_policy_yaml(policy_to_yaml(parsed))
    assert roundtrip["defaults"] == parsed["defaults"]
    assert roundtrip["class_overrides"] == parsed["class_overrides"]


def test_init_and_config_manage_provenance_defaults(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper"
    repo.mkdir()
    monkeypatch.chdir(repo)
    monkeypatch.delenv("AIT_MODEL", raising=False)
    monkeypatch.delenv("CODEX_MODEL", raising=False)
    monkeypatch.delenv("OPENAI_MODEL", raising=False)

    result = runner.invoke(
        app,
        [
            "init",
            "--name",
            "housekeeper",
            "--default-author-mode",
            "human_with_ai_assist",
            "--default-model",
            "codex",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert result.exit_code == 0, result.stdout
    payload = json.loads(result.stdout)
    assert payload["default_author_mode"] == "human_with_ai_assist"
    assert payload["default_model"] == "codex"

    config_data = json.loads((repo / ".ait" / "config.json").read_text(encoding="utf-8"))
    assert config_data["default_author_mode"] == "human_with_ai_assist"
    assert config_data["default_model"] == "codex"

    show_out = runner.invoke(app, ["config", "show", "--json"], catch_exceptions=False)
    assert show_out.exit_code == 0, show_out.stdout
    shown = json.loads(show_out.stdout)
    assert shown["effective_author_mode"] == "human_with_ai_assist"
    assert shown["effective_model"] == "codex"
    assert shown["effective_actor"] is None
    assert shown["user_name"] is None
    assert shown["user_email"] is None
    assert shown["effective_reviewer"] is None

    set_out = runner.invoke(
        app,
        [
            "config",
            "set",
            "--default-author-mode",
            "human_only",
            "--default-model",
            "gpt-5.4",
            "--user-name",
            "Alice Example",
            "--user-email",
            "alice@example.com",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert set_out.exit_code == 0, set_out.stdout
    updated = json.loads(set_out.stdout)
    assert updated["default_author_mode"] == "human_only"
    assert updated["default_model"] == "gpt-5.4"
    assert updated["user_name"] == "Alice Example"
    assert updated["user_email"] == "alice@example.com"
    assert updated["effective_actor"] == "alice@example.com"
    assert updated["effective_reviewer"] == "Alice Example <alice@example.com>"
    assert updated["effective_author_mode"] == "human_only"
    assert updated["effective_model"] == "gpt-5.4"

    clear_out = runner.invoke(
        app,
        ["config", "set", "--clear-default-model", "--clear-user-name", "--clear-user-email", "--json"],
        catch_exceptions=False,
    )
    assert clear_out.exit_code == 0, clear_out.stdout
    cleared = json.loads(clear_out.stdout)
    assert cleared["default_model"] is None
    assert cleared["user_name"] is None
    assert cleared["user_email"] is None
    assert cleared["effective_actor"] is None
    assert cleared["effective_reviewer"] is None


def test_config_set_task_tracking_mode_and_clear_binding(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-tracking-config"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    set_out = runner.invoke(app, ["config", "set", "--task-tracking", "on", "--json"], catch_exceptions=False)
    assert set_out.exit_code == 0, set_out.stdout
    shown = json.loads(set_out.stdout)
    assert shown["task_tracking"] == "on"
    assert shown["tracked_session"] is None
    assert runner.invoke(
        app,
        ["config", "set", "--plan-task-binding-mode", "advisory", "--json"],
        catch_exceptions=False,
    ).exit_code == 0

    task_out = runner.invoke(
        app,
        ["task", "start", "--task-only", "--local", "--title", "Tracked task", "--intent", "exercise config binding", "--risk", "low", "--json"],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task = json.loads(task_out.stdout)
    assert task["tracking"]["session_scope"] == "local"

    off_out = runner.invoke(app, ["config", "set", "--task-tracking", "off", "--json"], catch_exceptions=False)
    assert off_out.exit_code == 0, off_out.stdout
    disabled = json.loads(off_out.stdout)
    assert disabled["task_tracking"] == "off"
    assert disabled["tracked_session"] is None


def test_config_set_command_profiling_mode(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-command-profiling-config"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    shown = json.loads(runner.invoke(app, ["config", "show", "--json"], catch_exceptions=False).stdout)
    assert shown["command_profiling"] == "off"

    set_out = runner.invoke(app, ["config", "set", "--command-profiling", "on", "--json"], catch_exceptions=False)
    assert set_out.exit_code == 0, set_out.stdout
    updated = json.loads(set_out.stdout)
    assert updated["command_profiling"] == "on"

    config_data = json.loads((repo / ".ait" / "config.json").read_text(encoding="utf-8"))
    assert config_data["command_profiling"] == "on"

    off_out = runner.invoke(app, ["config", "set", "--command-profiling", "off", "--json"], catch_exceptions=False)
    assert off_out.exit_code == 0, off_out.stdout
    disabled = json.loads(off_out.stdout)
    assert disabled["command_profiling"] == "off"


def test_config_show_reports_staged_default_plan_task_binding(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-task-binding-defaults"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    show_out = runner.invoke(app, ["config", "show", "--json"], catch_exceptions=False)
    assert show_out.exit_code == 0, show_out.stdout
    shown = json.loads(show_out.stdout)
    assert shown["plan_task_binding"] == {
        "mode": "required",
        "source": "staged_default",
    }
    assert shown["task_dag"] == {
        "allow_multi_worker": {
            "value": "off",
            "source": "built_in",
        }
    }


def test_config_set_task_dag_allow_multi_worker(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-dag-config"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    set_out = runner.invoke(
        app,
        ["config", "set", "--task-dag-allow-multi-worker", "on", "--json"],
        catch_exceptions=False,
    )
    assert set_out.exit_code == 0, set_out.stdout
    updated = json.loads(set_out.stdout)
    assert updated["task_dag"] == {
        "allow_multi_worker": {
            "value": "on",
            "source": "repo_config",
        }
    }

    config_data = json.loads((repo / ".ait" / "config.json").read_text(encoding="utf-8"))
    assert config_data["task_dag"] == {"allow_multi_worker": "on"}

    off_out = runner.invoke(
        app,
        ["config", "set", "--task-dag-allow-multi-worker", "off", "--json"],
        catch_exceptions=False,
    )
    assert off_out.exit_code == 0, off_out.stdout
    disabled = json.loads(off_out.stdout)
    assert disabled["task_dag"] == {
        "allow_multi_worker": {
            "value": "off",
            "source": "repo_config",
        }
    }


def test_config_set_plan_task_binding_mode(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-task-binding-config"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    set_out = runner.invoke(
        app,
        [
            "config",
            "set",
            "--plan-task-binding-mode",
            "strict",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert set_out.exit_code == 0, set_out.stdout
    updated = json.loads(set_out.stdout)
    assert updated["plan_task_binding"] == {
        "mode": "strict",
        "source": "repo_config",
    }

    clear_out = runner.invoke(
        app,
        ["config", "set", "--clear-plan-task-binding", "--json"],
        catch_exceptions=False,
    )
    assert clear_out.exit_code == 0, clear_out.stdout
    cleared = json.loads(clear_out.stdout)
    assert cleared["plan_task_binding"] == {
        "mode": "required",
        "source": "staged_default",
    }


def test_config_set_plan_task_binding_mode_accepts_required(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-task-binding-required"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    set_out = runner.invoke(
        app,
        ["config", "set", "--plan-task-binding-mode", "required", "--json"],
        catch_exceptions=False,
    )
    assert set_out.exit_code == 0, set_out.stdout
    updated = json.loads(set_out.stdout)
    assert updated["plan_task_binding"] == {
        "mode": "required",
        "source": "repo_config",
    }


def test_config_set_accepts_hidden_legacy_task_auto_worktree_flag_as_noop(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-auto-worktree-legacy-noop"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    set_out = runner.invoke(
        app,
        ["config", "set", "--task-auto-worktree", "on", "--json"],
        catch_exceptions=False,
    )
    assert set_out.exit_code == 0, set_out.stdout
    payload = json.loads(set_out.stdout)
    assert payload["task_worktree"] == _task_worktree_summary()

    config_data = json.loads((repo / ".ait" / "config.json").read_text(encoding="utf-8"))
    task_worktree_cfg = config_data.get("task_worktree") if isinstance(config_data.get("task_worktree"), dict) else {}
    assert "auto_create" not in task_worktree_cfg
    assert "root_mode" not in task_worktree_cfg

    help_out = runner.invoke(app, ["config", "set", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "--task-auto-worktree" not in help_out.stdout


def test_config_set_plan_task_binding_mode_drops_legacy_fields(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-task-binding-legacy"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    config_path = repo / ".ait" / "config.json"
    cfg = json.loads(config_path.read_text(encoding="utf-8"))
    cfg["plan_task_binding"] = {
        "mode": "advisory",
        "allow_head_fallback": False,
        "drift_gate": "warn",
    }
    config_path.write_text(json.dumps(cfg, indent=2) + "\n", encoding="utf-8")

    set_out = runner.invoke(
        app,
        ["config", "set", "--plan-task-binding-mode", "strict", "--json"],
        catch_exceptions=False,
    )
    assert set_out.exit_code == 0, set_out.stdout
    updated = json.loads(set_out.stdout)
    assert updated["plan_task_binding"] == {
        "mode": "strict",
        "source": "repo_config",
    }

    saved = json.loads(config_path.read_text(encoding="utf-8"))
    assert saved["plan_task_binding"] == {"mode": "strict"}


def test_config_set_workflow_default_scope_modes(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-workflow-scope-config"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    show_out = runner.invoke(app, ["config", "show", "--json"], catch_exceptions=False)
    assert show_out.exit_code == 0, show_out.stdout
    shown = json.loads(show_out.stdout)
    assert shown["workflow_default_scope"] == {
        "workflow": {"value": "local", "source": "built_in"},
        "task": {"value": "local", "source": "built_in"},
        "change": {"value": "local", "source": "built_in"},
    }

    workflow_out = runner.invoke(
        app,
        ["config", "set", "--workflow-default-scope", "local", "--json"],
        catch_exceptions=False,
    )
    assert workflow_out.exit_code == 0, workflow_out.stdout
    workflow = json.loads(workflow_out.stdout)
    assert workflow["workflow_default_scope"] == {
        "workflow": {"value": "local", "source": "repo_config"},
        "task": {"value": "local", "source": "workflow_default_scope"},
        "change": {"value": "local", "source": "workflow_default_scope"},
    }

    task_out = runner.invoke(
        app,
        ["config", "set", "--task-default-scope", "remote", "--json"],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task_scope = json.loads(task_out.stdout)
    assert task_scope["workflow_default_scope"]["task"] == {"value": "remote", "source": "repo_config"}
    assert task_scope["workflow_default_scope"]["change"] == {"value": "local", "source": "workflow_default_scope"}

    clear_out = runner.invoke(
        app,
        ["config", "set", "--clear-workflow-default-scope", "--clear-task-default-scope", "--json"],
        catch_exceptions=False,
    )
    assert clear_out.exit_code == 0, clear_out.stdout
    cleared = json.loads(clear_out.stdout)
    assert cleared["workflow_default_scope"] == {
        "workflow": {"value": "local", "source": "built_in"},
        "task": {"value": "local", "source": "built_in"},
        "change": {"value": "local", "source": "built_in"},
    }


def test_config_help_frames_workflow_mode_as_primary_selector():
    config_help = runner.invoke(app, ["config", "--help"], catch_exceptions=False)
    assert config_help.exit_code == 0, config_help.stdout
    config_text = " ".join(config_help.stdout.split())
    assert "Inspect effective workflow modes and update local repository defaults" in config_text

    show_help = runner.invoke(app, ["config", "show", "--help"], catch_exceptions=False)
    assert show_help.exit_code == 0, show_help.stdout
    show_text = " ".join(show_help.stdout.split())
    assert "Show the effective workflow mode, actor defaults, and advanced local overrides." in show_text

    set_help = runner.invoke(app, ["config", "set", "--help"], catch_exceptions=False)
    assert set_help.exit_code == 0, set_help.stdout
    set_text = " ".join(set_help.stdout.split())
    assert "Set the primary workflow-mode preset or advanced local overrides." in set_text
    assert "--workflow-mode" in set_help.stdout
    assert "Primary workflow" in set_text
    assert "solo_local" in set_text
    assert "solo_remote" in set_text
    assert "team_remote" in set_text


def test_config_set_workflow_mode_presets(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-workflow-mode-presets"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    shown = json.loads(runner.invoke(app, ["config", "show", "--json"], catch_exceptions=False).stdout)
    assert shown["workflow_mode"] == {
        "value": "solo_local",
        "source": "derived_from_effective_config",
        "dag_default": "local_execution_dag",
        "change_strategy": "promote_reviewable_outputs_late",
    }

    solo_local = runner.invoke(
        app,
        ["config", "set", "--workflow-mode", "solo_local", "--json"],
        catch_exceptions=False,
    )
    assert solo_local.exit_code == 0, solo_local.stdout
    solo_local_data = json.loads(solo_local.stdout)
    assert solo_local_data["workflow_mode"]["value"] == "solo_local"
    assert solo_local_data["workflow_mode"]["dag_default"] == "local_execution_dag"
    assert solo_local_data["workflow_default_scope"] == {
        "workflow": {"value": "local", "source": "repo_config"},
        "task": {"value": "local", "source": "repo_config"},
        "change": {"value": "local", "source": "repo_config"},
    }
    assert solo_local_data["plan_task_binding"] == {
        "mode": "required",
        "source": "repo_config",
    }
    saved_cfg = json.loads((repo / ".ait" / "config.json").read_text(encoding="utf-8"))
    assert saved_cfg["workflow_mode"] == "solo_local"

    solo_remote = runner.invoke(
        app,
        ["config", "set", "--workflow-mode", "solo_remote", "--json"],
        catch_exceptions=False,
    )
    assert solo_remote.exit_code == 0, solo_remote.stdout
    solo_remote_data = json.loads(solo_remote.stdout)
    assert solo_remote_data["workflow_mode"] == {
        "value": "solo_remote",
        "source": "repo_config",
        "dag_default": "local_execution_dag_with_selective_promotion",
        "change_strategy": "remote_backed_selective_promotion",
    }
    assert solo_remote_data["workflow_default_scope"] == {
        "workflow": {"value": "remote", "source": "repo_config"},
        "task": {"value": "remote", "source": "repo_config"},
        "change": {"value": "remote", "source": "repo_config"},
    }
    assert solo_remote_data["plan_task_binding"] == {
        "mode": "required",
        "source": "repo_config",
    }

    team_remote = runner.invoke(
        app,
        ["config", "set", "--workflow-mode", "team_remote", "--json"],
        catch_exceptions=False,
    )
    assert team_remote.exit_code == 0, team_remote.stdout
    team_remote_data = json.loads(team_remote.stdout)
    assert team_remote_data["workflow_mode"] == {
        "value": "team_remote",
        "source": "repo_config",
        "dag_default": "shared_workflow_dag",
        "change_strategy": "per_slice_reviewable_changes",
    }
    assert team_remote_data["workflow_default_scope"] == {
        "workflow": {"value": "remote", "source": "repo_config"},
        "task": {"value": "remote", "source": "repo_config"},
        "change": {"value": "remote", "source": "repo_config"},
    }
    assert team_remote_data["plan_task_binding"] == {
        "mode": "required",
        "source": "repo_config",
    }


def test_config_show_reports_agent_runtime_resolution(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-agent-runtime"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    local_show = json.loads(runner.invoke(app, ["config", "show", "--json"], catch_exceptions=False).stdout)
    assert local_show["agent_runtime"]["mode"] == "local"
    assert local_show["agent_runtime"]["workflow_mode"] == "solo_local"
    assert local_show["agent_runtime"]["remote_name"] is None
    assert local_show["agent_runtime"]["server_url"] is None

    assert runner.invoke(
        app,
        ["remote", "add", "origin", "http://example.test:8088", "--repo-name", "housekeeper", "--default"],
        catch_exceptions=False,
    ).exit_code == 0
    remote_show = json.loads(
        runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote", "--json"], catch_exceptions=False).stdout
    )
    assert remote_show["agent_runtime"]["mode"] == "remote"
    assert remote_show["agent_runtime"]["workflow_mode"] == "solo_remote"
    assert remote_show["agent_runtime"]["remote_name"] == "origin"
    assert remote_show["agent_runtime"]["server_url"] == "http://example.test:8088"


def test_agent_runtime_mode_uses_shared_workflow_mode_resolver(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-agent-runtime-shared-resolver"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    runtime_backend = import_module("ait_agent.runtime_backend")
    observed_roots: list[Path] = []

    def fake_shared_effective_workflow_mode(ctx):
        observed_roots.append(ctx.root)
        return {"value": "solo_remote"}

    monkeypatch.setattr(runtime_backend, "_shared_effective_workflow_mode", fake_shared_effective_workflow_mode)

    repo_ctx = RepoContext.discover(repo)
    assert runtime_backend.effective_agent_workflow_mode(repo_ctx) == "solo_remote"
    assert observed_roots == [repo.resolve()]


def test_agent_runtime_mode_rejects_non_preset_shared_workflow_mode(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-agent-runtime-custom-mode"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    runtime_backend = import_module("ait_agent.runtime_backend")
    monkeypatch.setattr(runtime_backend, "_shared_effective_workflow_mode", lambda _ctx: {"value": "custom"})

    repo_ctx = RepoContext.discover(repo)
    with pytest.raises(runtime_backend.AgentRuntimeConfigError, match="repo workflow preset"):
        runtime_backend.effective_agent_workflow_mode(repo_ctx)


def test_config_set_workflow_mode_rejects_manual_scope_mix(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-workflow-mode-mix"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    mixed = runner.invoke(
        app,
        ["config", "set", "--workflow-mode", "solo_local", "--task-default-scope", "remote"],
        catch_exceptions=False,
    )
    assert mixed.exit_code != 0


def test_config_set_task_worktree_lifecycle_modes_without_auto_create_toggle(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-worktree-config"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    show_out = runner.invoke(app, ["config", "show", "--json"], catch_exceptions=False)
    assert show_out.exit_code == 0, show_out.stdout
    shown = json.loads(show_out.stdout)
    assert shown["task_worktree"] == _task_worktree_summary()

    set_out = runner.invoke(
        app,
        [
            "config",
            "set",
            "--task-worktree-alias-root",
            ".ait/ram-links",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert set_out.exit_code == 0, set_out.stdout
    updated = json.loads(set_out.stdout)
    assert updated["task_worktree"] == _task_worktree_summary(
        alias_root=".ait/ram-links",
        alias_root_source="repo_config",
    )

    config_path = repo / ".ait" / "config.json"
    saved = json.loads(config_path.read_text(encoding="utf-8"))
    assert saved["task_worktree"] == {
        "alias_root": ".ait/ram-links",
    }

    clear_out = runner.invoke(
        app,
        [
            "config",
            "set",
            "--clear-task-worktree-alias-root",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert clear_out.exit_code == 0, clear_out.stdout
    cleared = json.loads(clear_out.stdout)
    assert cleared["task_worktree"] == _task_worktree_summary()


def test_config_set_main_seed_ram_budget_updates_task_worktree_policy(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-worktree-main-seed-budget"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    set_out = runner.invoke(
        app,
        [
            "config",
            "set",
            "--task-worktree-main-seed-ram-max-bytes",
            "52428800",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert set_out.exit_code == 0, set_out.stdout
    updated = json.loads(set_out.stdout)
    assert updated["task_worktree"] == _task_worktree_summary(
        main_seed_ram_max_bytes=52428800,
        main_seed_ram_max_bytes_source="repo_config",
    )

    config_path = repo / ".ait" / "config.json"
    saved = json.loads(config_path.read_text(encoding="utf-8"))
    assert saved["task_worktree"] == {
        "main_seed_ram_max_bytes": 52428800,
    }

    clear_out = runner.invoke(
        app,
        [
            "config",
            "set",
            "--clear-task-worktree-main-seed-ram-max-bytes",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert clear_out.exit_code == 0, clear_out.stdout
    cleared = json.loads(clear_out.stdout)
    assert cleared["task_worktree"] == _task_worktree_summary()


def test_task_start_auto_creates_ephemeral_bound_worktree_alias_on_linux(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-start-ephemeral-auto-worktree"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    snapshot_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert snapshot_out.exit_code == 0, snapshot_out.stdout
    runtime_root = (tmp_path / "runtime-root").resolve()
    monkeypatch.setenv("XDG_RUNTIME_DIR", str(runtime_root))
    task_worktree_layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(task_worktree_layout.sys, "platform", "linux")
    monkeypatch.setattr(task_worktree_layout, "_linux_detected_memory_roots", lambda: [runtime_root])
    set_out = runner.invoke(
        app,
        [
            "config",
            "set",
            "--plan-task-binding-mode",
            "advisory",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert set_out.exit_code == 0, set_out.stdout

    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Bootstrap isolated workflow",
            "--intent",
            "open a bound worktree together with the task",
            "--base-line",
            "main",
            "--risk",
            "medium",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    payload = json.loads(start_out.stdout)
    worktree = payload["worktree"]
    worktree_path = Path(worktree["path"])
    alias_path = Path(worktree["alias_path"])
    expected_worktree_name = payload["task_id"].lower()
    expected_target_root = (
        task_worktree_layout._auto_detected_ephemeral_root(RepoContext.discover(repo), runtime_root) / "housekeeper"
    )

    assert worktree["name"] == expected_worktree_name
    assert worktree["root_source"] == "linux_xdg_runtime_dir"
    assert worktree_path.parent == expected_target_root
    assert alias_path == (repo / ".ait" / "worktree-links" / expected_worktree_name)
    assert alias_path.is_symlink()
    assert alias_path.resolve() == worktree_path.resolve()
    assert worktree["open_path"] == str(alias_path)
    assert worktree["cd_command"] == f"cd {shlex.quote(str(alias_path))}"
    guidance = payload["worktree_guidance"]
    assert guidance["target_workspace_root"] == str(alias_path)
    assert guidance["cd_command"] == worktree["cd_command"]


def test_task_start_auto_bootstraps_macos_ram_volume_when_none_is_pre_mounted(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-start-macos-ram-bootstrap"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    snapshot_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert snapshot_out.exit_code == 0, snapshot_out.stdout
    task_worktree_layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(task_worktree_layout.sys, "platform", "darwin")
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

    monkeypatch.setattr(task_worktree_layout, "_macos_ram_volume_specs", fake_specs)
    monkeypatch.setattr(task_worktree_layout, "_default_macos_ram_volume_spec", lambda: dict(default_spec))
    monkeypatch.setattr(task_worktree_layout, "_provision_macos_ram_volume", fake_provision)
    set_out = runner.invoke(
        app,
        [
            "config",
            "set",
            "--plan-task-binding-mode",
            "advisory",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert set_out.exit_code == 0, set_out.stdout

    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Bootstrap macOS RAM workflow",
            "--intent",
            "open a bound worktree after provisioning the managed RAM volume",
            "--base-line",
            "main",
            "--risk",
            "medium",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    payload = json.loads(start_out.stdout)
    worktree = payload["worktree"]
    worktree_path = Path(worktree["path"])
    alias_path = Path(worktree["alias_path"])
    expected_worktree_name = payload["task_id"].lower()
    expected_target_root = task_worktree_layout._auto_detected_ephemeral_root(RepoContext.discover(repo), ram_root) / "housekeeper"

    assert worktree["name"] == expected_worktree_name
    assert worktree["root_source"] == "macos_ram_volume"
    assert worktree_path.parent == expected_target_root
    assert alias_path == (repo / ".ait" / "worktree-links" / expected_worktree_name)
    assert alias_path.is_symlink()
    assert alias_path.resolve() == worktree_path.resolve()
    assert worktree["open_path"] == str(alias_path)
    assert worktree["cd_command"] == f"cd {shlex.quote(str(alias_path))}"
    guidance = payload["worktree_guidance"]
    assert guidance["target_workspace_root"] == str(alias_path)
    assert guidance["cd_command"] == worktree["cd_command"]


def test_task_start_auto_creates_windows_ephemeral_bound_worktree_links_without_symlink_privilege(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-start-ephemeral-auto-worktree-windows"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / ".venv").mkdir()
    (repo / ".venv" / "pyvenv.cfg").write_text("home = test\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    snapshot_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert snapshot_out.exit_code == 0, snapshot_out.stdout
    ram_root = (tmp_path / "RamDisk").resolve()
    task_worktree_layout = import_module("ait.task_worktree_layout")
    monkeypatch.setattr(task_worktree_layout.sys, "platform", "win32")
    monkeypatch.setattr(task_worktree_layout, "_windows_ram_disk_roots", lambda: [ram_root])
    store_worktree_filesystem = import_module("ait.store_worktree_filesystem")
    monkeypatch.setattr(store_worktree_filesystem, "_is_windows_platform", lambda: True)

    def fake_windows_junction(link_path: Path, target_path: Path) -> None:
        link_path.parent.mkdir(parents=True, exist_ok=True)
        link_path.symlink_to(target_path, target_is_directory=True)

    monkeypatch.setattr(store_worktree_filesystem, "_create_windows_directory_junction", fake_windows_junction)
    set_out = runner.invoke(
        app,
        [
            "config",
            "set",
            "--plan-task-binding-mode",
            "advisory",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert set_out.exit_code == 0, set_out.stdout

    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Bootstrap Windows RAM workflow",
            "--intent",
            "open a Windows-compatible bound worktree together with the task",
            "--base-line",
            "main",
            "--risk",
            "medium",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    payload = json.loads(start_out.stdout)
    worktree = payload["worktree"]
    worktree_path = Path(worktree["path"])
    alias_path = Path(worktree["alias_path"])
    expected_target_root = task_worktree_layout._auto_detected_ephemeral_root(RepoContext.discover(repo), ram_root) / "housekeeper"

    assert worktree["root_source"] == "windows_ramdisk"
    assert worktree_path.parent == expected_target_root
    assert alias_path.is_symlink()
    assert alias_path.resolve() == worktree_path.resolve()
    assert worktree["open_path"] == str(alias_path)
    assert (worktree_path / ".ait").is_symlink()
    assert (worktree_path / ".ait").resolve() == (repo / ".ait").resolve()
    assert (worktree_path / ".venv").is_symlink()
    assert (worktree_path / ".venv").resolve() == (repo / ".venv").resolve()
    guidance = payload["worktree_guidance"]
    assert guidance["target_workspace_root"] == str(alias_path)


def test_task_start_auto_creates_bound_worktree_by_default(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-start-auto-worktree"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    snapshot_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert snapshot_out.exit_code == 0, snapshot_out.stdout
    set_out = runner.invoke(
        app,
        ["config", "set", "--plan-task-binding-mode", "advisory", "--json"],
        catch_exceptions=False,
    )
    assert set_out.exit_code == 0, set_out.stdout

    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Bootstrap isolated workflow",
            "--intent",
            "open a bound worktree together with the task",
            "--base-line",
            "main",
            "--risk",
            "medium",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    payload = json.loads(start_out.stdout)
    worktree = payload["worktree"]
    worktree_path = Path(worktree["path"])
    expected_worktree_name = payload["task_id"].lower()
    feature_line_name = f"feature/{payload['task_id'].lower()}"

    assert worktree["name"] == expected_worktree_name
    assert worktree["bound_task_id"] == payload["task_id"]
    assert worktree["bound_change_id"] == payload["change"]["change_id"]
    assert worktree["auto_created_for_task"] is True
    assert worktree["registered_line_name"] == feature_line_name
    assert worktree["current_line"] == feature_line_name
    assert worktree["workspace_status"] == "clean"
    assert worktree["cd_command"] == f"cd {shlex.quote(str(worktree_path))}"
    assert worktree["shell_command"].startswith(f"cd {shlex.quote(str(worktree_path))} &&")
    guidance = payload["worktree_guidance"]
    assert guidance["switch_required"] is True
    assert guidance["current_workspace_root"] == str(repo.resolve())
    assert guidance["target_workspace_root"] == str(worktree_path.resolve())
    assert guidance["cd_command"] == worktree["cd_command"]
    assert "Your current shell has not been switched automatically." in guidance["message"]
    assert "dirty_source_warning" not in guidance
    assert worktree_path.is_dir()
    assert (worktree_path / ".ait-worktree.json").exists()

    show_out = runner.invoke(app, ["worktree", "show", worktree["name"], "--json"], catch_exceptions=False)
    assert show_out.exit_code == 0, show_out.stdout
    shown = json.loads(show_out.stdout)
    assert shown["name"] == expected_worktree_name
    assert shown["bound_task_id"] == payload["task_id"]
    assert shown["bound_change_id"] == payload["change"]["change_id"]
    assert shown["auto_created_for_task"] is True
    assert shown["registered_line_name"] == feature_line_name
    assert shown["current_line"] == feature_line_name


def test_task_and_change_default_to_repo_default_line(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-default-line-bootstrap"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    init_out = runner.invoke(
        app,
        ["init", "--name", "housekeeper", "--default-line", "release-main", "--json"],
        catch_exceptions=False,
    )
    assert init_out.exit_code == 0, init_out.stdout
    assert json.loads(init_out.stdout)["default_line"] == "release-main"

    config_out = runner.invoke(
        app,
        ["config", "set", "--plan-task-binding-mode", "advisory", "--json"],
        catch_exceptions=False,
    )
    assert config_out.exit_code == 0, config_out.stdout

    snapshot_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert snapshot_out.exit_code == 0, snapshot_out.stdout

    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Default line bootstrap",
            "--intent",
            "use the repository default line when no base line is supplied",
            "--risk",
            "low",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    started = json.loads(start_out.stdout)
    assert started["change"]["base_line"] == "release-main"

    task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--local",
            "--title",
            "Default line follow-up",
            "--intent",
            "open a later change without restating the base line",
            "--risk",
            "low",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task = json.loads(task_out.stdout)
    second_worktree_path = Path(task["worktree"]["path"])
    assert task["worktree"]["target_base_line"] == "release-main"
    assert task["worktree"]["forked_from_line"] == "release-main"

    monkeypatch.chdir(second_worktree_path)
    change_out = runner.invoke(
        app,
        [
            "change",
            "create",
            "--local",
            "--task",
            task["task_id"],
            "--title",
            "Default line follow-up change",
            "--risk",
            "low",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert change_out.exit_code == 0, change_out.stdout
    assert json.loads(change_out.stdout)["base_line"] == "release-main"


def test_task_create_auto_worktree_uses_repo_default_line_from_non_default_workspace(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-create-default-line-worktree"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    init_out = runner.invoke(
        app,
        ["init", "--name", "housekeeper", "--default-line", "release-main", "--json"],
        catch_exceptions=False,
    )
    assert init_out.exit_code == 0, init_out.stdout
    assert json.loads(init_out.stdout)["default_line"] == "release-main"

    config_out = runner.invoke(
        app,
        ["config", "set", "--plan-task-binding-mode", "advisory", "--json"],
        catch_exceptions=False,
    )
    assert config_out.exit_code == 0, config_out.stdout

    snapshot_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert snapshot_out.exit_code == 0, snapshot_out.stdout

    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Default line bootstrap",
            "--intent",
            "open the first bound task worktree from the repository default line",
            "--risk",
            "low",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    started = json.loads(start_out.stdout)
    first_worktree_path = Path(started["worktree"]["path"])

    monkeypatch.chdir(first_worktree_path)
    task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--local",
            "--title",
            "Default line follow-up",
            "--intent",
            "keep new task worktrees fresh from the repository default line even when the current workspace is elsewhere",
            "--risk",
            "low",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task = json.loads(task_out.stdout)
    worktree = task["worktree"]
    feature_line_name = f"feature/{task['task_id'].lower()}"

    assert worktree["forked_from_line"] == "release-main"
    assert worktree["target_base_line"] == "release-main"
    assert worktree["registered_line_name"] == feature_line_name
    assert worktree["current_line"] == feature_line_name


def test_land_submit_auto_removes_bound_task_worktree_when_configured(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-land-submit-auto-worktree-cleanup"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-land-submit-auto-worktree-cleanup") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            [
                "config",
                "set",
                "--plan-task-binding-mode",
                "advisory",
                "--json",
            ],
            catch_exceptions=False,
        ).exit_code == 0

        start_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--title",
                "Workflow land with cleanup",
                "--intent",
                "clean the bound worktree after remote land when the task is ready",
                "--base-line",
                "main",
                "--risk",
                "medium",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert start_out.exit_code == 0, start_out.stdout
        start_payload = json.loads(start_out.stdout)
        change = start_payload["change"]
        bound_worktree = start_payload["worktree"]
        bound_worktree_path = Path(bound_worktree["path"])
        assert bound_worktree_path.is_dir()

        (repo / "app.py").write_text("print('stale root copy')\n", encoding="utf-8")
        monkeypatch.chdir(bound_worktree_path)
        assert runner.invoke(app, ["line", "create", "feature/land-cleanup"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/land-cleanup"], catch_exceptions=False).exit_code == 0
        (bound_worktree_path / "app.py").write_text("print('cleanup ready')\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "cleanup ready"], catch_exceptions=False).exit_code == 0

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "cleanup patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)

        assert runner.invoke(app, ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--json"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["review", "approve", change["change_id"], "--patchset", patchset["patchset_id"], "--reviewer", "reviewer@example.com", "--json"],
            catch_exceptions=False,
        ).exit_code == 0
        _submit_passing_code_review_summary(
            change["change_id"],
            patchset["patchset_id"],
            reviewer="reviewer@example.com",
            reviewed_files="app.py",
        )
        policy_out = runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert policy_out.exit_code == 0, policy_out.stdout
        assert json.loads(policy_out.stdout)["decision"] == "pass"

        monkeypatch.chdir(bound_worktree_path)
        land_out = runner.invoke(
            app,
            ["land", "submit", change["change_id"], "--patchset", patchset["patchset_id"], "--target", "main", "--mode", "direct", "--json"],
            catch_exceptions=False,
        )
        assert land_out.exit_code == 0, land_out.stdout
        landed = json.loads(land_out.stdout)

        assert landed["status"] == "succeeded"
        assert landed["local_sync"]["status"] == "synced"
        assert landed["local_sync"]["workspace_restore"]["status"] == "restored"
        assert landed["local_sync"]["workspace_restore"]["force"] is True
        assert landed["local_sync"]["workspace_restore"]["applied"] is True
        assert landed["local_sync"]["workspace_restore"]["dirty_workspace"]["changed_count"] > 0
        assert landed["bound_worktree_cleanup"]["status"] == "skipped"
        assert landed["bound_worktree_cleanup"]["reason"] == "current_worktree"
        assert landed["bound_worktree_cleanup"]["task_id"] == start_payload["task_id"]
        assert landed["bound_worktree_cleanup"]["worktree_name"] == bound_worktree["name"]
        assert bound_worktree_path.exists()
        assert (repo / "app.py").read_text(encoding="utf-8") == "print('cleanup ready')\n"

        list_out = runner.invoke(app, ["worktree", "list", "--json"], catch_exceptions=False)
        assert list_out.exit_code == 0, list_out.stdout
        names = {row["name"] for row in json.loads(list_out.stdout)}
        assert bound_worktree["name"] in names


def test_workflow_land_apply_auto_removes_bound_task_worktree_when_configured(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-workflow-land-apply-auto-worktree-cleanup"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-workflow-land-apply-auto-worktree-cleanup") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            [
                "config",
                "set",
                "--plan-task-binding-mode",
                "advisory",
                "--json",
            ],
            catch_exceptions=False,
        ).exit_code == 0

        start_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--title",
                "Workflow land apply cleanup",
                "--intent",
                "remove the bound worktree from workflow land apply",
                "--base-line",
                "main",
                "--risk",
                "medium",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert start_out.exit_code == 0, start_out.stdout
        start_payload = json.loads(start_out.stdout)
        change = start_payload["change"]
        bound_worktree = start_payload["worktree"]
        bound_worktree_path = Path(bound_worktree["path"])
        assert bound_worktree_path.is_dir()

        monkeypatch.chdir(bound_worktree_path)
        assert runner.invoke(app, ["line", "create", "feature/workflow-land-apply-cleanup"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/workflow-land-apply-cleanup"], catch_exceptions=False).exit_code == 0
        (bound_worktree_path / "app.py").write_text("print('workflow apply cleanup')\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "workflow apply cleanup"], catch_exceptions=False).exit_code == 0

        apply_out = runner.invoke(
            app,
            [
                "workflow",
                "land",
                change["change_id"],
                "--apply",
                "--summary",
                "guided land cleanup patchset",
                "--tests",
                "pass",
                "--reviewer",
                "reviewer@example.com",
                "--review-message",
                "Reviewed files: app.py; Findings: no blocking findings; Risks: low; Tests: pytest focused suite passed; Recommendation: safe to land.",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert apply_out.exit_code == 0, apply_out.stdout
        applied = json.loads(apply_out.stdout)

        assert applied["apply_status"] == "done"
        submit_result = next(row["result"] for row in applied["applied_actions"] if row["code"] == "submit_land")
        complete_result = next(row["result"] for row in applied["applied_actions"] if row["code"] == "complete_task")
        assert submit_result["local_sync"]["workspace_restore"]["status"] == "restored"
        assert submit_result["bound_worktree_cleanup"]["status"] == "skipped"
        assert submit_result["bound_worktree_cleanup"]["reason"] == "current_worktree"
        assert submit_result["bound_worktree_cleanup"]["task_id"] == start_payload["task_id"]
        assert submit_result["bound_worktree_cleanup"]["worktree_name"] == bound_worktree["name"]
        assert complete_result["status"] == "completed"
        assert complete_result["bound_worktree_cleanup"]["status"] == "skipped"
        assert complete_result["bound_worktree_cleanup"]["reason"] == "current_worktree"
        assert complete_result["bound_worktree_cleanup"]["task_id"] == start_payload["task_id"]
        assert bound_worktree_path.exists()

        monkeypatch.chdir(repo)
        config_payload = json.loads((repo / ".ait" / "config.json").read_text(encoding="utf-8"))
        assert config_payload.get("worktree_name") is None

        cleanup_out = runner.invoke(app, ["worktree", "cleanup", "--yes", "--json"], catch_exceptions=False)
        assert cleanup_out.exit_code == 0, cleanup_out.stdout
        cleanup_payload = json.loads(cleanup_out.stdout)
        assert cleanup_payload["removed_count"] == 1
        assert cleanup_payload["removed_rows"][0]["name"] == bound_worktree["name"]
        assert not bound_worktree_path.exists()


def test_remote_land_preserves_unrelated_repo_root_dirty_and_untracked_paths(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-land-preserve-root-paths"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")
    (repo / "notes.txt").write_text("tracked base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-remote-land-preserve-root-paths") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            [
                "config",
                "set",
                "--plan-task-binding-mode",
                "advisory",
                "--id-namespace-prefix",
                "AIT",
                "--json",
            ],
            catch_exceptions=False,
        ).exit_code == 0

        start_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--title",
                "Preserve repo-root paths after land",
                "--intent",
                "keep unrelated repo-root dirty and untracked paths during remote land local sync",
                "--base-line",
                "main",
                "--risk",
                "medium",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert start_out.exit_code == 0, start_out.stdout
        start_payload = json.loads(start_out.stdout)
        change = start_payload["change"]
        bound_worktree = start_payload["worktree"]
        bound_worktree_path = Path(bound_worktree["path"])
        assert bound_worktree_path.is_dir()

        (repo / "notes.txt").write_text("tracked base\nlocal dirty note\n", encoding="utf-8")
        untracked_path = repo / "docs" / "benchmarks" / "runs" / "land-preserve" / "evidence" / "worker_session.jsonl"
        untracked_path.parent.mkdir(parents=True, exist_ok=True)
        untracked_path.write_text("{\"status\":\"keep\"}\n", encoding="utf-8")

        monkeypatch.chdir(bound_worktree_path)
        (bound_worktree_path / "app.py").write_text("print('landed change')\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "landed change"], catch_exceptions=False).exit_code == 0

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "preserve root paths patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)

        assert runner.invoke(app, ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--json"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["review", "approve", change["change_id"], "--patchset", patchset["patchset_id"], "--reviewer", "reviewer@example.com", "--json"],
            catch_exceptions=False,
        ).exit_code == 0
        _submit_passing_code_review_summary(
            change["change_id"],
            patchset["patchset_id"],
            reviewer="reviewer@example.com",
            reviewed_files="app.py",
        )
        policy_out = runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert policy_out.exit_code == 0, policy_out.stdout
        assert json.loads(policy_out.stdout)["decision"] == "pass"

        monkeypatch.chdir(bound_worktree_path)
        land_out = runner.invoke(
            app,
            ["land", "submit", change["change_id"], "--patchset", patchset["patchset_id"], "--target", "main", "--mode", "direct", "--json"],
            catch_exceptions=False,
        )
        assert land_out.exit_code == 0, land_out.stdout
        landed = json.loads(land_out.stdout)
        workspace_restore = landed["local_sync"]["workspace_restore"]
        expected_unrelated_paths = [
            "docs/benchmarks/runs/land-preserve/evidence/worker_session.jsonl",
            "notes.txt",
        ]

        assert landed["status"] == "succeeded"
        assert landed["local_sync"]["status"] == "synced"
        assert workspace_restore["status"] == "restored"
        assert sorted(workspace_restore["unrelated_paths"]) == expected_unrelated_paths
        assert sorted(workspace_restore["preserved_unrelated_paths"]) == expected_unrelated_paths
        assert sorted(workspace_restore["remaining_paths"]) == expected_unrelated_paths
        assert "app.py" in workspace_restore["landed_diff_paths"]
        assert (repo / "app.py").read_text(encoding="utf-8") == "print('landed change')\n"
        assert (repo / "notes.txt").read_text(encoding="utf-8") == "tracked base\nlocal dirty note\n"
        assert untracked_path.read_text(encoding="utf-8") == "{\"status\":\"keep\"}\n"
        assert landed["bound_worktree_cleanup"]["status"] == "skipped"
        assert landed["bound_worktree_cleanup"]["reason"] == "current_worktree"
        assert landed["bound_worktree_cleanup"]["worktree_name"] == bound_worktree["name"]
        assert bound_worktree_path.exists()

        monkeypatch.chdir(repo)
        status_out = runner.invoke(app, ["workspace", "status", "--json"], catch_exceptions=False)
        assert status_out.exit_code == 0, status_out.stdout
        status = json.loads(status_out.stdout)
        assert status["clean"] is False
        assert status["modified_paths"] == ["notes.txt"]
        assert status["untracked_paths"] == ["docs/benchmarks/runs/land-preserve/evidence/worker_session.jsonl"]


def test_workflow_land_auto_removes_bound_task_worktree_with_lr_prefixed_publish_ids(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-workflow-land-alias-auto-worktree-cleanup"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-workflow-land-alias-auto-worktree-cleanup") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            [
                "config",
                "set",
                "--plan-task-binding-mode",
                "advisory",
                "--id-namespace-prefix",
                "AIT",
                "--json",
            ],
            catch_exceptions=False,
        ).exit_code == 0

        seed_task_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--task-only",
                "--title",
                "Seed remote task sequence",
                "--intent",
                "seed a remote-prefixed task before publishing a local-prefixed task",
                "--risk",
                "low",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert seed_task_out.exit_code == 0, seed_task_out.stdout
        seed_task = json.loads(seed_task_out.stdout)
        assert seed_task["task_id"] == "RAITT-0001"
        seed_worktree = seed_task["worktree"]
        assert seed_worktree["bound_task_id"] == seed_task["task_id"]
        remove_seed_worktree_out = runner.invoke(
            app,
            ["worktree", "remove", seed_worktree["name"], "--delete-path", "--json"],
            catch_exceptions=False,
        )
        assert remove_seed_worktree_out.exit_code == 0, remove_seed_worktree_out.stdout

        start_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--local",
                "--title",
                "Workflow land cleanup with local-prefixed publish id",
                "--intent",
                "remove the auto-created bound worktree after land when the published task keeps the local-prefixed canonical id",
                "--base-line",
                "main",
                "--risk",
                "medium",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert start_out.exit_code == 0, start_out.stdout
        start_payload = json.loads(start_out.stdout)
        assert start_payload["task_id"] == "LAITT-0001"
        change = start_payload["change"]
        assert change["change_id"] == "LAITC-0001"
        bound_worktree = start_payload["worktree"]
        bound_worktree_path = Path(bound_worktree["path"])
        assert bound_worktree_path.is_dir()

        monkeypatch.chdir(bound_worktree_path)
        assert runner.invoke(app, ["line", "create", "feature/published-task-alias-cleanup"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/published-task-alias-cleanup"], catch_exceptions=False).exit_code == 0
        (bound_worktree_path / "app.py").write_text("print('alias cleanup ready')\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "alias cleanup ready"], catch_exceptions=False).exit_code == 0

        task_publish_out = runner.invoke(app, ["task", "publish", start_payload["task_id"], "--json"], catch_exceptions=False)
        assert task_publish_out.exit_code == 0, task_publish_out.stdout
        published_task = json.loads(task_publish_out.stdout)
        assert published_task["published_task_id"] == start_payload["task_id"]

        change_publish_out = runner.invoke(app, ["change", "publish", change["change_id"], "--json"], catch_exceptions=False)
        assert change_publish_out.exit_code == 0, change_publish_out.stdout
        published_change = json.loads(change_publish_out.stdout)
        assert published_change["published_change_id"] == change["change_id"]

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "alias cleanup patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)

        assert runner.invoke(app, ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--json"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            [
                "review",
                "approve",
                published_change["published_change_id"],
                "--patchset",
                patchset["patchset_id"],
                "--reviewer",
                "reviewer@example.com",
                "--json",
            ],
            catch_exceptions=False,
        ).exit_code == 0
        _submit_passing_code_review_summary(
            published_change["published_change_id"],
            patchset["patchset_id"],
            reviewer="reviewer@example.com",
            reviewed_files="app.py",
        )
        policy_out = runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert policy_out.exit_code == 0, policy_out.stdout
        assert json.loads(policy_out.stdout)["decision"] == "pass"

        monkeypatch.chdir(bound_worktree_path)
        land_out = runner.invoke(
            app,
            [
                "land",
                "submit",
                published_change["published_change_id"],
                "--patchset",
                patchset["patchset_id"],
                "--target",
                "main",
                "--mode",
                "direct",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert land_out.exit_code == 0, land_out.stdout
        landed = json.loads(land_out.stdout)

        assert landed["status"] == "succeeded"
        assert landed["bound_worktree_cleanup"]["status"] == "skipped"
        assert landed["bound_worktree_cleanup"]["reason"] == "current_worktree"
        assert landed["bound_worktree_cleanup"]["task_id"] == published_task["published_task_id"]
        assert landed["bound_worktree_cleanup"]["worktree_name"] == bound_worktree["name"]
        assert bound_worktree_path.exists()


def test_worktree_cleanup_can_remove_current_worktree_land_after_task_completion(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-post-land-worktree-cleanup"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-post-land-worktree-cleanup") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            [
                "config",
                "set",
                "--plan-task-binding-mode",
                "advisory",
                "--json",
            ],
            catch_exceptions=False,
        ).exit_code == 0

        start_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--title",
                "Current worktree cleanup fallback",
                "--intent",
                "allow cleanup after landing from the bound worktree and completing the task",
                "--base-line",
                "main",
                "--risk",
                "medium",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert start_out.exit_code == 0, start_out.stdout
        start_payload = json.loads(start_out.stdout)
        change = start_payload["change"]
        bound_worktree = start_payload["worktree"]
        bound_worktree_path = Path(bound_worktree["path"])
        assert bound_worktree_path.is_dir()

        monkeypatch.chdir(bound_worktree_path)
        assert runner.invoke(app, ["line", "create", "feature/current-worktree-cleanup"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/current-worktree-cleanup"], catch_exceptions=False).exit_code == 0
        (bound_worktree_path / "app.py").write_text("print('cleanup fallback')\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "cleanup fallback"], catch_exceptions=False).exit_code == 0

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "cleanup patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)

        assert runner.invoke(app, ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--json"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["review", "approve", change["change_id"], "--patchset", patchset["patchset_id"], "--reviewer", "reviewer@example.com", "--json"],
            catch_exceptions=False,
        ).exit_code == 0
        _submit_passing_code_review_summary(
            change["change_id"],
            patchset["patchset_id"],
            reviewer="reviewer@example.com",
            reviewed_files="app.py",
        )
        policy_out = runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert policy_out.exit_code == 0, policy_out.stdout
        assert json.loads(policy_out.stdout)["decision"] == "pass"

        land_out = runner.invoke(
            app,
            ["land", "submit", change["change_id"], "--patchset", patchset["patchset_id"], "--target", "main", "--mode", "direct", "--json"],
            catch_exceptions=False,
        )
        assert land_out.exit_code == 0, land_out.stdout
        landed = json.loads(land_out.stdout)
        assert landed["status"] == "succeeded"
        assert landed["bound_worktree_cleanup"]["status"] == "skipped"
        assert landed["bound_worktree_cleanup"]["reason"] == "current_worktree"
        assert bound_worktree_path.exists()

        monkeypatch.chdir(repo)
        complete_out = runner.invoke(
            app,
            ["task", "complete", start_payload["task_id"], "--json"],
            catch_exceptions=False,
        )
        assert complete_out.exit_code == 0, complete_out.stdout
        completed_task = json.loads(complete_out.stdout)
        assert completed_task["status"] == "completed"
        config_path = repo / ".ait" / "config.json"
        assert completed_task["bound_worktree_cleanup"]["status"] == "removed"
        assert completed_task["bound_worktree_cleanup"]["task_id"] == start_payload["task_id"]
        assert completed_task["bound_worktree_cleanup"]["worktree"]["name"] == bound_worktree["name"]
        config_payload = json.loads(config_path.read_text(encoding="utf-8"))
        assert config_payload.get("worktree_name") is None
        assert not bound_worktree_path.exists()


def test_task_complete_from_current_worktree_clears_root_binding_for_repo_cleanup(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-current-worktree-task-complete-root-binding"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-current-worktree-task-complete-root-binding") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            [
                "config",
                "set",
                "--plan-task-binding-mode",
                "advisory",
                "--json",
            ],
            catch_exceptions=False,
        ).exit_code == 0

        start_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--title",
                "Current worktree task complete clears root binding",
                "--intent",
                "keep repo-root cleanup viable after current-worktree task completion",
                "--base-line",
                "main",
                "--risk",
                "medium",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert start_out.exit_code == 0, start_out.stdout
        start_payload = json.loads(start_out.stdout)
        change = start_payload["change"]
        bound_worktree = start_payload["worktree"]
        bound_worktree_path = Path(bound_worktree["path"])
        assert bound_worktree_path.is_dir()

        monkeypatch.chdir(bound_worktree_path)
        assert runner.invoke(app, ["line", "create", "feature/current-worktree-task-complete"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/current-worktree-task-complete"], catch_exceptions=False).exit_code == 0
        (bound_worktree_path / "app.py").write_text("print('current worktree complete')\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "current worktree complete"], catch_exceptions=False).exit_code == 0

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "current worktree complete patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)

        assert runner.invoke(app, ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--json"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["review", "approve", change["change_id"], "--patchset", patchset["patchset_id"], "--reviewer", "reviewer@example.com", "--json"],
            catch_exceptions=False,
        ).exit_code == 0
        _submit_passing_code_review_summary(
            change["change_id"],
            patchset["patchset_id"],
            reviewer="reviewer@example.com",
            reviewed_files="app.py",
        )
        policy_out = runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert policy_out.exit_code == 0, policy_out.stdout
        assert json.loads(policy_out.stdout)["decision"] == "pass"

        land_out = runner.invoke(
            app,
            ["land", "submit", change["change_id"], "--patchset", patchset["patchset_id"], "--target", "main", "--mode", "direct", "--json"],
            catch_exceptions=False,
        )
        assert land_out.exit_code == 0, land_out.stdout
        landed = json.loads(land_out.stdout)
        assert landed["status"] == "succeeded"
        assert landed["bound_worktree_cleanup"]["status"] == "skipped"
        assert landed["bound_worktree_cleanup"]["reason"] == "current_worktree"
        assert bound_worktree_path.exists()

        complete_out = runner.invoke(
            app,
            ["task", "complete", start_payload["task_id"], "--json"],
            catch_exceptions=False,
        )
        assert complete_out.exit_code == 0, complete_out.stdout
        completed_task = json.loads(complete_out.stdout)
        assert completed_task["status"] == "completed"
        assert completed_task["bound_worktree_cleanup"]["status"] == "skipped"
        assert completed_task["bound_worktree_cleanup"]["reason"] == "current_worktree"
        assert completed_task["bound_worktree_cleanup"]["worktree_name"] == bound_worktree["name"]

        monkeypatch.chdir(repo)
        config_payload = json.loads((repo / ".ait" / "config.json").read_text(encoding="utf-8"))
        assert config_payload.get("worktree_name") is None

        cleanup_out = runner.invoke(app, ["worktree", "cleanup", "--yes", "--json"], catch_exceptions=False)
        assert cleanup_out.exit_code == 0, cleanup_out.stdout
        cleanup_payload = json.loads(cleanup_out.stdout)
        assert cleanup_payload["removed_count"] == 1
        assert cleanup_payload["removed_rows"][0]["name"] == bound_worktree["name"]
        assert not bound_worktree_path.exists()


def test_patchset_and_attestation_use_config_defaults_and_detected_model(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-provenance-defaults"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-provenance-defaults") as base_url:
        monkeypatch.chdir(repo)
        monkeypatch.delenv("AIT_MODEL", raising=False)
        monkeypatch.setenv("CODEX_MODEL", "gpt-5.4-codex")
        monkeypatch.delenv("OPENAI_MODEL", raising=False)
        monkeypatch.setenv("AIT_SESSION_ID", "session-defaults")
        monkeypatch.setenv("AIT_CHECKPOINT_ID", "checkpoint-defaults")
        assert runner.invoke(
            app,
            ["init", "--name", "housekeeper", "--default-author-mode", "human_with_ai_assist"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0

        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Default provenance", "--intent", "verify config defaults", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        workspace = _bind_task_worktree(task["task_id"], monkeypatch)

        assert runner.invoke(app, ["line", "create", "feature/provenance-defaults"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/provenance-defaults"], catch_exceptions=False).exit_code == 0
        (workspace / "app.py").write_text("print('feature')\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "feature work"], catch_exceptions=False).exit_code == 0

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Default provenance", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "default provenance patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)
        assert patchset["author_mode"] == "human_with_ai_assist"

        attest_out = runner.invoke(app, ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--json"], catch_exceptions=False)
        assert attest_out.exit_code == 0, attest_out.stdout
        attestation = json.loads(attest_out.stdout)
        assert attestation["author_mode"] == "human_with_ai_assist"
        assert attestation["provenance_summary"]["model_name"] == "gpt-5.4-codex"
        assert attestation["provenance_summary"]["session_id"] == "session-defaults"
        assert attestation["provenance_summary"]["checkpoint_id"] == "checkpoint-defaults"
        assert attestation["provenance_summary"]["evidence_readiness"] == "complete"
        assert attestation["provenance_summary"]["policy_readable"] is True
        assert attestation["detail"]["minimum_evidence"]["missing_fields"] == []


def test_review_approve_can_use_configured_user_identity(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-review-defaults"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-review-defaults") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["config", "set", "--user-name", "Alice Example", "--user-email", "alice@example.com"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0

        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Review defaults", "--intent", "use configured reviewer", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        workspace = _bind_task_worktree(task["task_id"], monkeypatch)

        assert runner.invoke(app, ["line", "create", "feature/review-defaults"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/review-defaults"], catch_exceptions=False).exit_code == 0
        (workspace / "app.py").write_text("print('feature')\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "feature work"], catch_exceptions=False).exit_code == 0

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Review defaults", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "review defaults patchset", "--json"],
            catch_exceptions=False,
        )
        patchset = json.loads(patchset_out.stdout)

        review_out = runner.invoke(
            app,
            ["review", "approve", change["change_id"], "--patchset", patchset["patchset_id"], "--json"],
            catch_exceptions=False,
        )
        assert review_out.exit_code == 0, review_out.stdout
        review = json.loads(review_out.stdout)
        assert review["reviewer"] == "Alice Example <alice@example.com>"
        assert review["action"] == "approve"
