from __future__ import annotations

from importlib.metadata import metadata
import json
from pathlib import Path
import tomllib

import ait_py
import ait_python


ROOT = Path(__file__).parents[1]


def test_package_builds_the_pinned_pyo3_extension() -> None:
    pyproject = tomllib.loads(
        (ROOT / "pyproject.toml").read_text(encoding="utf-8")
    )

    assert pyproject["build-system"]["build-backend"] == "maturin"
    assert pyproject["build-system"]["requires"] == ["maturin==1.13.3"]
    assert "scripts" not in pyproject["project"]
    assert (
        pyproject["tool"]["maturin"]["manifest-path"]
        == ".ait-external/ait-core/rust/crates/ait-py/Cargo.toml"
    )
    assert pyproject["tool"]["maturin"]["module-name"] == "ait_py"
    assert pyproject["tool"]["maturin"]["features"] == [
        "pyo3/abi3-py311"
    ]
    assert pyproject["tool"]["maturin"]["python-packages"] == [
        "ait_py",
        "ait_python",
    ]
    assert "config" not in pyproject["tool"]["maturin"]

    cargo_config_path = ROOT / ".cargo" / "config.toml"
    cargo_config_text = cargo_config_path.read_text(encoding="utf-8")
    cargo_config = tomllib.loads(cargo_config_text)
    target_dir = Path(cargo_config["build"]["target-dir"])
    build_dir = Path(cargo_config["build"]["build-dir"])

    if cargo_config_text.startswith("# AIT source policy:"):
        assert target_dir.parts[-2:] == (".ait", "cargo-target")
        assert "cargo-build" in build_dir.parts
        assert "{workspace-path-hash}" in build_dir.parts
    else:
        assert cargo_config_text.startswith(
            "# Managed by ait: workspace-isolated final artifacts and intermediates."
        )
        worktree = json.loads(
            (ROOT / ".ait-worktree.json").read_text(encoding="utf-8")
        )
        worktree_name = worktree["worktree_name"]
        assert target_dir.parts[-3] == "cargo-target"
        assert target_dir.parts[-2] == "task-workspaces"
        assert target_dir.name == worktree_name
        assert "cargo-build" in build_dir.parts
        assert build_dir.parts[-2] == "task-workspaces"
        assert build_dir.name == worktree_name != "main-seed"
        assert target_dir != build_dir


def test_package_declares_the_apache_rc_identity() -> None:
    pyproject = tomllib.loads(
        (ROOT / "pyproject.toml").read_text(encoding="utf-8")
    )
    project = pyproject["project"]

    assert project["version"] == ait_python.__version__ == "1.1.1"
    assert project["license"] == "Apache-2.0"
    assert project["license-files"] == ["LICENSE", "NOTICE"]
    assert "Apache License" in (ROOT / "LICENSE").read_text(encoding="utf-8")
    notice = (ROOT / "NOTICE").read_text(encoding="utf-8")
    assert "ait-python" in notice
    assert "native `ait_py` extension" in notice
    assert notice.count(
        "----- BEGIN GENERATED THIRD-PARTY NOTICES -----"
    ) == 1
    assert "/.cargo/registry/" not in notice
    assert "/Users/" not in notice
    assert "/Volumes/" not in notice

    core_lock = tomllib.loads(
        (
            ROOT / ".ait-external" / "ait-core" / "rust" / "Cargo.lock"
        ).read_text(encoding="utf-8")
    )
    notice_lines = notice.splitlines()
    for package in core_lock["package"]:
        if "source" not in package:
            continue
        prefix = f"{package['name']}\t{package['version']}\t"
        assert any(line.startswith(prefix) for line in notice_lines), (
            f"NOTICE is missing {package['name']} {package['version']}"
        )

    generator = (ROOT / "ci" / "generate_notice.sh").read_text(
        encoding="utf-8"
    )
    assert ".ait-external/ait-core/ci/generate_rust_notice.sh" in generator
    assert ".ait-external/ait-core/rust/Cargo.toml" in generator

    installed = metadata("ait-python")
    assert installed["Version"] == "1.1.1"
    assert installed["License-Expression"] == "Apache-2.0"
    assert installed.get_all("License-File") == ["LICENSE", "NOTICE"]


def test_materialized_core_matches_the_external_lock() -> None:
    lock = tomllib.loads(
        (ROOT / "ait-external.lock").read_text(encoding="utf-8")
    )
    marker = json.loads(
        (
            ROOT
            / ".ait-external"
            / "ait-core"
            / ".ait-external-marker.json"
        ).read_text(encoding="utf-8")
    )
    node = lock["node"][0]

    assert node["repository_index"] == 0
    assert node["snapshot"] == "SNP-8F6FBEF7B117"
    assert marker["format"] == "ait.external.materialized"
    assert marker["name"] == node["name"] == "ait-core"
    assert marker["repo_name"] == node["repo_name"] == "ait-core"
    assert marker["repository_index"] == node["repository_index"]
    assert marker["snapshot"] == node["snapshot"]
    assert marker["materialize_to"] == node["materialize_to"]
    assert (
        ROOT
        / ".ait-external"
        / "ait-core"
        / "rust"
        / "crates"
        / "ait-py"
        / "Cargo.toml"
    ).is_file()


def test_patchset_ci_keeps_dependent_build_steps_in_one_workspace() -> None:
    catalog = json.loads(
        (ROOT / "ci" / "patch_ci.json").read_text(encoding="utf-8")
    )
    commands = catalog["suites"][0]["runner"]["commands"]

    assert commands == ["./ci/run.sh patchset"]

    entrypoint = (ROOT / "ci" / "run.sh").read_text(encoding="utf-8")
    assert "patchset | repo | all" in entrypoint
    assert "PIP_NO_CACHE_DIR=1" in entrypoint
    assert "PYTHONPYCACHEPREFIX" in entrypoint
    assert "CARGO_TARGET_DIR" in entrypoint
    assert "python\" -m pytest -p no:cacheprovider" in entrypoint
    assert "python\" -m pip check" in entrypoint


def test_windows_ci_entrypoint_mirrors_the_attempt_owned_contract() -> None:
    entrypoint = (ROOT / "ci" / "run.ps1").read_text(encoding="utf-8")

    assert '@("patchset", "repo", "all")' in entrypoint
    assert "AIT_RUNNER_ATTEMPT_ROOT" in entrypoint
    assert '"ait-python-ci." + [Guid]::NewGuid()' in entrypoint
    assert "PIP_NO_CACHE_DIR" in entrypoint
    assert "PYTHONPYCACHEPREFIX" in entrypoint
    assert "CARGO_TARGET_DIR" in entrypoint
    assert '"Scripts/python.exe"' in entrypoint
    assert '"-m", "pytest", "-p", "no:cacheprovider"' in entrypoint
    assert '"-m", "pip", "check"' in entrypoint
    assert "Remove-Item -LiteralPath $ciRoot -Recurse -Force" in entrypoint
    assert "Invoke-Expression" not in entrypoint
    assert "cmd.exe" not in entrypoint


def test_runtime_source_has_no_process_api_relay() -> None:
    source = "\n".join(
        path.read_text(encoding="utf-8")
        for path in sorted((ROOT / "src").rglob("*.py"))
    )

    assert "subprocess" not in source
    assert "os.exec" not in source
    assert not (ROOT / "src" / "ait_python" / "cli.py").exists()
    assert not (ROOT / "src" / "ait_python" / "bundle.py").exists()


def test_removed_task_publish_operation_is_not_exported() -> None:
    assert hasattr(ait_py, "task_workflow_task_land")
    assert not hasattr(ait_py, "task_workflow_task_land_apply_direct")
    assert not hasattr(ait_py, "task_workflow_task_publish")
