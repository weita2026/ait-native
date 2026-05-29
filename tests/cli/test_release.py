from __future__ import annotations

import tarfile
import zipfile

from ait.store import (
    close_local_task,
    create_local_change,
    create_snapshot,
    create_local_release,
    create_local_task,
    export_snapshot_bundle,
    land_local_change,
)

from ._shared import *  # noqa: F401,F403


def _write_release_fixture(repo: Path) -> None:
    (repo / "README.md").write_text(
        "# Fixture Release\n\nStart with the [local quickstart](./docs/LOCAL_QUICKSTART.md).\n",
        encoding="utf-8",
    )
    docs_dir = repo / "docs"
    docs_dir.mkdir(parents=True, exist_ok=True)
    (docs_dir / "LOCAL_QUICKSTART.md").write_text(
        "# Local Quickstart\n\nContinue to the [deployment guide](./SELF_HOSTED_TEAM_DEPLOYMENT.md).\n",
        encoding="utf-8",
    )
    (docs_dir / "HOMEBREW_TAP.md").write_text(
        "# Homebrew Tap\n\nUse `brew tap weita2026/ait-native https://github.com/weita2026/ait-native` and then `brew install weita2026/ait-native/ait-native`.\n",
        encoding="utf-8",
    )
    (docs_dir / "SELF_HOSTED_TEAM_DEPLOYMENT.md").write_text(
        "# Deployment Guide\n\nCheck the [compatibility matrix](./COMPATIBILITY_MATRIX.md).\n",
        encoding="utf-8",
    )
    (docs_dir / "PYPI_PUBLISHING.md").write_text(
        "# PyPI Publishing\n\nPush the clean public release commit and matching `v*` tag to `weita2026/ait-native`, let the workflow `.github/workflows/pypi-publish.yml` run through a `Trusted Publisher`, and keep `twine upload dist/*` only as the manual fallback. For GitHub Releases asset publication, follow [GitHub Release Publishing](./GITHUB_RELEASE_PUBLISHING.md).\n",
        encoding="utf-8",
    )
    (docs_dir / "GITHUB_RELEASE_PUBLISHING.md").write_text(
        "# GitHub Release Publishing\n\nUse `scripts/github_release_publish.sh` to prepare `release-assets-v*`, then let `.github/workflows/github-release-publish.yml` consume that asset ref and publish the real GitHub Release with `GITHUB_TOKEN`. Recovery can use `workflow_dispatch`.\n",
        encoding="utf-8",
    )
    (docs_dir / "PACKAGE_TARGETS.md").write_text(
        "# Package Targets\n\nThis release ships `ait`, `ait-agent`, `ait-server`, `ait-worker`, and `aitk`.\n",
        encoding="utf-8",
    )
    (docs_dir / "COMPATIBILITY_MATRIX.md").write_text(
        "# Compatibility Matrix\n\nSee the [package targets](./PACKAGE_TARGETS.md).\n",
        encoding="utf-8",
    )
    (docs_dir / "CONTRIBUTING.md").write_text("Contribute carefully.\n", encoding="utf-8")
    (docs_dir / "LOCAL_DEVELOPMENT.md").write_text(
        "Use the [local quickstart](./LOCAL_QUICKSTART.md) before editing runtime surfaces.\n",
        encoding="utf-8",
    )
    (repo / "LICENSE").write_text("MIT\n", encoding="utf-8")
    (repo / "NOTICE").write_text("Fixture notice\n", encoding="utf-8")
    (docs_dir / "THIRD_PARTY_NOTICES.md").write_text("No third-party notices.\n", encoding="utf-8")
    (docs_dir / "TRADEMARK_POLICY.md").write_text("No trademark grants.\n", encoding="utf-8")
    (docs_dir / "public_package_targets_contract.json").write_text(
        json.dumps(
            {
                "version": "0.1.0",
                "required_scripts": ["ait", "ait-agent", "ait-server", "ait-worker", "aitk"],
                "excluded_scripts": ["ait-web"],
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    (docs_dir / "public_compatibility_matrix.json").write_text(
        json.dumps(
            {
                "version": "0.1.0",
                "python": [">=3.11"],
                "deployment": ["self-hosted-core"],
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    (docs_dir / "public_self_hosted_deployment_contract.json").write_text(
        json.dumps(
            {
                "version": "0.1.0",
                "guide_path": "docs/SELF_HOSTED_TEAM_DEPLOYMENT.md",
                "readiness_commands": [
                    "curl -fsS https://ait.example/healthz",
                ],
                "related_docs": [
                    "docs/server_backup_restore_dr.md",
                    "docs/server_disaster_recovery_checklist.md",
                ],
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    legal_dir = docs_dir / "legal"
    legal_dir.mkdir(parents=True, exist_ok=True)
    (legal_dir / "public_package_surface_map.json").write_text(
        json.dumps(
            {
                "version": "0.1.0",
                "public_surfaces": ["ait", "ait-agent", "ait-server", "ait-worker", "aitk"],
                "excluded_surfaces": ["ait-web"],
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    (legal_dir / "public_release_license_summary.md").write_text(
        "# License Summary\n\nReview the [package targets](../PACKAGE_TARGETS.md).\n",
        encoding="utf-8",
    )
    licenses_dir = repo / "LICENSES"
    licenses_dir.mkdir(parents=True, exist_ok=True)
    (licenses_dir / "AGPL-3.0-only.txt").write_text("AGPL fixture text.\n", encoding="utf-8")
    (licenses_dir / "LicenseRef-AIT-Commercial.txt").write_text("Commercial fixture text.\n", encoding="utf-8")

    workflows_dir = repo / ".github" / "workflows"
    workflows_dir.mkdir(parents=True, exist_ok=True)
    (workflows_dir / "pypi-publish.yml").write_text(
        """
name: pypi-publish
on:
  workflow_dispatch:
  push:
    tags:
      - "v*"
jobs:
  publish-pypi:
    environment:
      name: pypi
      url: https://pypi.org/p/ait-native
    permissions:
      id-token: write
    steps:
      - uses: pypa/gh-action-pypi-publish@release/v1
""".strip()
        + "\n",
        encoding="utf-8",
    )
    (workflows_dir / "github-release-publish.yml").write_text(
        """
name: github-release-publish
on:
  workflow_dispatch:
  push:
    tags:
      - "v*"
permissions:
  contents: write
jobs:
  publish-release:
    steps:
      - uses: actions/checkout@v4
      - run: echo release-assets-v0.1.0
      - run: gh release create
      - run: gh release upload
""".strip()
        + "\n",
        encoding="utf-8",
    )

    scripts_dir = repo / "scripts"
    scripts_dir.mkdir(parents=True, exist_ok=True)
    (scripts_dir / "github_release_publish.sh").write_text(
        "#!/usr/bin/env bash\nrelease-assets-v0.1.0\n",
        encoding="utf-8",
    )

    src_dir = repo / "src"
    src_dir.mkdir(parents=True, exist_ok=True)
    (src_dir / "fixture_release.py").write_text(
        "def main():\n    print('fixture release')\n",
        encoding="utf-8",
    )
    (src_dir / "ait_agent").mkdir(parents=True, exist_ok=True)
    (src_dir / "ait_agent" / "__init__.py").write_text("", encoding="utf-8")
    (src_dir / "ait_agent" / "cli.py").write_text(
        "def main():\n    print('fixture agent')\n",
        encoding="utf-8",
    )
    (src_dir / "ait_server").mkdir(parents=True, exist_ok=True)
    (src_dir / "ait_server" / "__init__.py").write_text("", encoding="utf-8")
    (src_dir / "ait_server" / "app.py").write_text(
        "def main():\n    print('fixture server')\n",
        encoding="utf-8",
    )
    (src_dir / "ait_server" / "worker.py").write_text(
        "def main():\n    print('fixture worker')\n",
        encoding="utf-8",
    )
    (src_dir / "ait_tk").mkdir(parents=True, exist_ok=True)
    (src_dir / "ait_tk" / "__init__.py").write_text("", encoding="utf-8")
    (src_dir / "ait_tk" / "launcher.py").write_text(
        "def main():\n    print('fixture tk')\n",
        encoding="utf-8",
    )
    (src_dir / "ait_web").mkdir(parents=True, exist_ok=True)
    (src_dir / "ait_web" / "__init__.py").write_text("", encoding="utf-8")
    (src_dir / "ait_web" / "app.py").write_text(
        "def main():\n    print('fixture web')\n",
        encoding="utf-8",
    )
    (src_dir / "ait_native").mkdir(parents=True, exist_ok=True)
    (src_dir / "ait_native" / "__init__.py").write_text("", encoding="utf-8")
    (src_dir / "ait_native" / "web.py").write_text(
        "def main():\n    print('fixture native web shim')\n",
        encoding="utf-8",
    )
    tests_dir = repo / "tests" / "ait_web"
    tests_dir.mkdir(parents=True, exist_ok=True)
    (tests_dir / "test_smoke.py").write_text(
        "def test_fixture_web_smoke():\n    assert True\n",
        encoding="utf-8",
    )
    (repo / "pyproject.toml").write_text(
        """
[build-system]
requires = ["setuptools>=68", "wheel"]
build-backend = "setuptools.build_meta"

[project]
name = "fixture-release"
version = "0.1.0"
description = "fixture release candidate"
readme = "README.md"
requires-python = ">=3.11"
license = "MIT"
dependencies = ["typer>=0.12,<1.0"]

[project.scripts]
ait = "fixture_release:main"
ait-server = "ait_server.app:main"
ait-worker = "ait_server.worker:main"
ait-web = "ait_web.app:main"
ait-agent = "ait_agent.cli:main"
aitk = "ait_tk.launcher:main"

[tool.setuptools]
package-dir = {"" = "src"}

[tool.setuptools.packages.find]
where = ["src"]
""".strip()
        + "\n",
        encoding="utf-8",
    )


def test_release_help_lists_first_slice_commands():
    help_out = runner.invoke(app, ["release", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    for token in ("candidate", "check", "build", "formula", "publish", "show"):
        assert token in help_out.stdout

    candidate_help = runner.invoke(app, ["release", "candidate", "create", "--help"], catch_exceptions=False)
    assert candidate_help.exit_code == 0, candidate_help.stdout
    assert "Create a durable local release candidate record" in candidate_help.stdout


def test_release_candidate_create_rejects_version_mismatch(tmp_path: Path, monkeypatch):
    repo = tmp_path / "fixture-release-version-mismatch"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "fixture-release"], catch_exceptions=False).exit_code == 0
    _write_release_fixture(repo)
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed"], catch_exceptions=False).exit_code == 0

    create_out = runner.invoke(
        app,
        ["release", "candidate", "create", "--version", "0.2.0", "--line", "main", "--profile", "local-cli", "--json"],
        catch_exceptions=False,
    )
    assert create_out.exit_code != 0
    message = create_out.stdout or create_out.output
    assert "Requested release version" in message
    assert "pyproject.toml version" in message


def test_release_candidate_e2e_builds_and_generates_formula(tmp_path: Path, monkeypatch):
    repo = tmp_path / "fixture-release-e2e"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "fixture-release"], catch_exceptions=False).exit_code == 0
    _write_release_fixture(repo)
    snapshot_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert snapshot_out.exit_code == 0, snapshot_out.stdout

    create_out = runner.invoke(
        app,
        ["release", "candidate", "create", "--version", "0.1.0", "--line", "main", "--profile", "local-cli", "--json"],
        catch_exceptions=False,
    )
    assert create_out.exit_code == 0, create_out.stdout
    release_record = json.loads(create_out.stdout)
    assert release_record["status"] == "candidate"
    assert release_record["package"]["name"] == "fixture-release"

    check_out = runner.invoke(
        app,
        ["release", "check", release_record["release_id"], "--skip-tests-reason", "fixture coverage runs elsewhere", "--json"],
        catch_exceptions=False,
    )
    assert check_out.exit_code == 0, check_out.stdout
    checked = json.loads(check_out.stdout)
    assert checked["status"] == "checked"
    assert checked["check_summary"]["decision"] == "pass"
    tests_check = next(row for row in checked["checks"] if row["check_id"] == "tests")
    assert tests_check["status"] == "skipped"

    build_out = runner.invoke(app, ["release", "build", release_record["release_id"], "--json"], catch_exceptions=False)
    assert build_out.exit_code == 0, build_out.stdout
    built = json.loads(build_out.stdout)
    artifact_kinds = {row["kind"] for row in built["artifacts"]}
    assert {"sdist", "wheel", "manifest", "checksum"} <= artifact_kinds
    for row in built["artifacts"]:
        assert (repo / row["path"]).exists(), row
    sdist_row = next(row for row in built["artifacts"] if row["kind"] == "sdist")
    with tarfile.open(repo / sdist_row["path"], "r:gz") as tf:
        readme_name = next(name for name in tf.getnames() if name.endswith("/README.md"))
        readme_text = tf.extractfile(readme_name).read().decode("utf-8")
    assert "## Release Notes" in readme_text
    assert "### v0.1.0" in readme_text
    assert "Initial published release for this profile." in readme_text
    wheel_row = next(row for row in built["artifacts"] if row["kind"] == "wheel")
    with zipfile.ZipFile(repo / wheel_row["path"]) as zf:
        metadata_name = next(name for name in zf.namelist() if name.endswith(".dist-info/METADATA"))
        metadata_text = zf.read(metadata_name).decode("utf-8")
    assert "Requires-Dist: typer>=0.12,<1.0" in metadata_text

    formula_out = runner.invoke(
        app,
        ["release", "formula", release_record["release_id"], "--name", "fixture-release", "--json"],
        catch_exceptions=False,
    )
    assert formula_out.exit_code == 0, formula_out.stdout
    formula = json.loads(formula_out.stdout)
    assert formula["formula"]["name"] == "fixture-release"
    assert formula["formula"]["artifact_kind"] == "wheel"
    assert (repo / formula["formula"]["path"]).exists()

    show_out = runner.invoke(app, ["release", "show", release_record["release_id"], "--json"], catch_exceptions=False)
    assert show_out.exit_code == 0, show_out.stdout
    shown = json.loads(show_out.stdout)
    assert shown["next_action"]["code"] == "publish_remote"


def test_release_candidate_flow_supplements_ignored_pyproject_from_clean_workspace(tmp_path: Path, monkeypatch):
    repo = tmp_path / "fixture-release-ignored-pyproject"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "fixture-release"], catch_exceptions=False).exit_code == 0
    _write_release_fixture(repo)
    aitignore_path = repo / ".aitignore"
    existing_aitignore = aitignore_path.read_text(encoding="utf-8") if aitignore_path.exists() else ""
    aitignore_path.write_text(existing_aitignore + "pyproject.toml\n", encoding="utf-8")

    snapshot_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert snapshot_out.exit_code == 0, snapshot_out.stdout

    create_out = runner.invoke(
        app,
        ["release", "candidate", "create", "--version", "0.1.0", "--line", "main", "--profile", "local-cli", "--json"],
        catch_exceptions=False,
    )
    assert create_out.exit_code == 0, create_out.stdout
    release_record = json.loads(create_out.stdout)
    assert release_record["status"] == "candidate"
    assert release_record["package"]["version"] == "0.1.0"

    check_out = runner.invoke(
        app,
        ["release", "check", release_record["release_id"], "--skip-tests-reason", "fixture coverage runs elsewhere", "--json"],
        catch_exceptions=False,
    )
    assert check_out.exit_code == 0, check_out.stdout
    checked = json.loads(check_out.stdout)
    assert checked["status"] == "checked"
    assert checked["check_summary"]["decision"] == "pass"

    build_out = runner.invoke(app, ["release", "build", release_record["release_id"], "--json"], catch_exceptions=False)
    assert build_out.exit_code == 0, build_out.stdout
    built = json.loads(build_out.stdout)
    artifact_kinds = {row["kind"] for row in built["artifacts"]}
    assert {"sdist", "wheel", "manifest", "checksum"} <= artifact_kinds


def test_release_build_appends_task_based_notes_to_readme(tmp_path: Path, monkeypatch):
    repo = tmp_path / "fixture-release-task-notes"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "fixture-release"], catch_exceptions=False).exit_code == 0
    _write_release_fixture(repo)
    baseline_out = runner.invoke(app, ["snapshot", "create", "--message", "baseline", "--json"], catch_exceptions=False)
    assert baseline_out.exit_code == 0, baseline_out.stdout
    baseline_snapshot_id = json.loads(baseline_out.stdout)["snapshot_id"]

    ctx = RepoContext.discover()
    baseline_bundle = export_snapshot_bundle(ctx, baseline_snapshot_id)
    create_local_release(
        ctx,
        version="0.1.0",
        line_name="main",
        snapshot_id=baseline_snapshot_id,
        manifest_hash=str(baseline_bundle["manifest_hash"]),
        profile="local-cli",
        package_name="fixture-release",
        package_version="0.1.0",
        package_requires_python=">=3.11",
        status="published",
    )

    task = create_local_task(
        ctx,
        title="Add task-based release notes",
        intent="Record landed tasks in release README output.",
        risk_tier="medium",
    )
    change = create_local_change(
        ctx,
        task["task_id"],
        "Add task-based release notes",
        "main",
        "medium",
    )

    pyproject_path = repo / "pyproject.toml"
    pyproject_path.write_text(
        pyproject_path.read_text(encoding="utf-8").replace('version = "0.1.0"', 'version = "0.2.0"', 1),
        encoding="utf-8",
    )
    (repo / "README.md").write_text(
        (repo / "README.md").read_text(encoding="utf-8") + "\nRelease delta body.\n",
        encoding="utf-8",
    )

    delta_snapshot_id = create_snapshot(ctx, "delta")["snapshot_id"]
    land_local_change(
        ctx,
        change["change_id"],
        target_line="main",
        landed_snapshot_id=delta_snapshot_id,
        pre_land_target_snapshot_id=baseline_snapshot_id,
    )
    close_local_task(ctx, task["task_id"], "completed")

    create_out = runner.invoke(
        app,
        ["release", "candidate", "create", "--version", "0.2.0", "--line", "main", "--profile", "local-cli", "--json"],
        catch_exceptions=False,
    )
    assert create_out.exit_code == 0, create_out.stdout
    release_record = json.loads(create_out.stdout)

    build_out = runner.invoke(app, ["release", "build", release_record["release_id"], "--json"], catch_exceptions=False)
    assert build_out.exit_code == 0, build_out.stdout
    built = json.loads(build_out.stdout)
    sdist_row = next(row for row in built["artifacts"] if row["kind"] == "sdist")
    with tarfile.open(repo / sdist_row["path"], "r:gz") as tf:
        readme_name = next(name for name in tf.getnames() if name.endswith("/README.md"))
        readme_text = tf.extractfile(readme_name).read().decode("utf-8")

    assert "## Release Notes" in readme_text
    assert "### v0.2.0" in readme_text
    assert "Tasks landed since `v0.1.0` (1 task):" in readme_text
    assert f"`{task['task_id']}` Add task-based release notes" in readme_text


def test_public_self_hosted_release_candidate_excludes_ait_web_surface(tmp_path: Path, monkeypatch):
    repo = tmp_path / "fixture-release-public-self-hosted"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "fixture-release"], catch_exceptions=False).exit_code == 0
    _write_release_fixture(repo)
    snapshot_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert snapshot_out.exit_code == 0, snapshot_out.stdout

    create_out = runner.invoke(
        app,
        [
            "release",
            "candidate",
            "create",
            "--version",
            "0.1.0",
            "--line",
            "main",
            "--profile",
            "public-self-hosted-core",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert create_out.exit_code == 0, create_out.stdout
    release_record = json.loads(create_out.stdout)

    check_out = runner.invoke(
        app,
        ["release", "check", release_record["release_id"], "--skip-tests-reason", "fixture coverage runs elsewhere", "--json"],
        catch_exceptions=False,
    )
    assert check_out.exit_code == 0, check_out.stdout
    checked = json.loads(check_out.stdout)
    assert checked["check_summary"]["decision"] == "pass"
    package_targets = next(row for row in checked["checks"] if row["check_id"] == "package_targets")
    assert package_targets["status"] == "pass"
    package_metadata = next(row for row in checked["checks"] if row["check_id"] == "package_metadata")
    assert package_metadata["status"] == "pass"
    package_readme_links = next(row for row in checked["checks"] if row["check_id"] == "package_readme_links")
    assert package_readme_links["status"] == "pass"
    publish_automation = next(row for row in checked["checks"] if row["check_id"] == "publish_automation")
    assert publish_automation["status"] == "pass"

    build_out = runner.invoke(app, ["release", "build", release_record["release_id"], "--json"], catch_exceptions=False)
    assert build_out.exit_code == 0, build_out.stdout
    built = json.loads(build_out.stdout)

    wheel_row = next(row for row in built["artifacts"] if row["kind"] == "wheel")
    with zipfile.ZipFile(repo / wheel_row["path"]) as zf:
        wheel_names = set(zf.namelist())
        entry_points_name = next(name for name in wheel_names if name.endswith(".dist-info/entry_points.txt"))
        entry_points_text = zf.read(entry_points_name).decode("utf-8")
        metadata_name = next(name for name in wheel_names if name.endswith(".dist-info/METADATA"))
        metadata_text = zf.read(metadata_name).decode("utf-8")
    assert "ait = fixture_release:main" in entry_points_text
    assert "ait-agent = ait_agent.cli:main" in entry_points_text
    assert "ait-server = ait_server.app:main" in entry_points_text
    assert "ait-worker = ait_server.worker:main" in entry_points_text
    assert "aitk = ait_tk.launcher:main" in entry_points_text
    assert "ait-web =" not in entry_points_text
    assert "License-Expression: Apache-2.0 AND AGPL-3.0-only" in metadata_text
    assert "Project-URL: Homepage, https://ait-native.dev" in metadata_text
    assert "Project-URL: Source, https://github.com/weita2026/ait-native" in metadata_text
    assert "Classifier: Development Status :: 3 - Alpha" in metadata_text
    assert "Description-Content-Type: text/markdown" in metadata_text
    assert "License-File: LICENSE" in metadata_text
    assert "License-File: LICENSES/AGPL-3.0-only.txt" in metadata_text
    assert "## Install" in metadata_text
    assert "ait_web/app.py" not in wheel_names
    assert "ait_web/__init__.py" not in wheel_names
    assert "ait_native/web.py" not in wheel_names
    assert "ait_server/app.py" in wheel_names
    assert any(name.endswith(".dist-info/licenses/LICENSE") for name in wheel_names)
    assert any(name.endswith(".dist-info/licenses/LICENSES/AGPL-3.0-only.txt") for name in wheel_names)

    sdist_row = next(row for row in built["artifacts"] if row["kind"] == "sdist")
    with tarfile.open(repo / sdist_row["path"], "r:gz") as tf:
        sdist_names = set(tf.getnames())
        pkg_info_name = next(name for name in sdist_names if name.endswith("/PKG-INFO"))
        pkg_info_text = tf.extractfile(pkg_info_name).read().decode("utf-8")
        workflow_name = next(name for name in sdist_names if name.endswith("/.github/workflows/pypi-publish.yml"))
        workflow_text = tf.extractfile(workflow_name).read().decode("utf-8")
        github_release_workflow_name = next(
            name for name in sdist_names if name.endswith("/.github/workflows/github-release-publish.yml")
        )
        github_release_workflow_text = tf.extractfile(github_release_workflow_name).read().decode("utf-8")
        publish_doc_name = next(name for name in sdist_names if name.endswith("/docs/PYPI_PUBLISHING.md"))
        publish_doc_text = tf.extractfile(publish_doc_name).read().decode("utf-8")
        github_release_doc_name = next(name for name in sdist_names if name.endswith("/docs/GITHUB_RELEASE_PUBLISHING.md"))
        github_release_doc_text = tf.extractfile(github_release_doc_name).read().decode("utf-8")
    assert not any(name.endswith("/src/ait_web/app.py") for name in sdist_names)
    assert not any(name.endswith("/src/ait_web/__init__.py") for name in sdist_names)
    assert not any(name.endswith("/src/ait_native/web.py") for name in sdist_names)
    assert not any(name.endswith("/tests/ait_web/test_smoke.py") for name in sdist_names)
    assert any(name.endswith("/README.pypi.md") for name in sdist_names)
    assert any(name.endswith("/docs/HOMEBREW_TAP.md") for name in sdist_names)
    assert any(name.endswith("/docs/GITHUB_RELEASE_PUBLISHING.md") for name in sdist_names)
    assert any(name.endswith("/docs/PYPI_PUBLISHING.md") for name in sdist_names)
    assert any(name.endswith("/scripts/github_release_publish.sh") for name in sdist_names)
    assert "Description-Content-Type: text/markdown" in pkg_info_text
    assert "Project-URL: Homepage, https://ait-native.dev" in pkg_info_text
    assert "## Install" in pkg_info_text
    assert "workflow_dispatch:" in workflow_text
    assert "push:" in workflow_text
    assert "tags:" in workflow_text
    assert '"v*"' in workflow_text
    assert "pypa/gh-action-pypi-publish@release/v1" in workflow_text
    assert "id-token: write" in workflow_text
    assert "workflow_dispatch:" in github_release_workflow_text
    assert "push:" in github_release_workflow_text
    assert '"v*"' in github_release_workflow_text
    assert "contents: write" in github_release_workflow_text
    assert "gh release create" in github_release_workflow_text
    assert "gh release upload" in github_release_workflow_text
    assert "release-assets-" in github_release_workflow_text
    assert "matching `v*` tag" in publish_doc_text
    assert "Trusted Publisher" in publish_doc_text
    assert "GITHUB_RELEASE_PUBLISHING.md" in publish_doc_text
    assert "scripts/github_release_publish.sh" in github_release_doc_text
    assert ".github/workflows/github-release-publish.yml" in github_release_doc_text
    assert "release-assets-v*" in github_release_doc_text
    assert "workflow_dispatch" in github_release_doc_text
    assert "GITHUB_TOKEN" in github_release_doc_text

    formula_out = runner.invoke(
        app,
        ["release", "formula", release_record["release_id"], "--name", "fixture-release", "--json"],
        catch_exceptions=False,
    )
    assert formula_out.exit_code == 0, formula_out.stdout
    formula = json.loads(formula_out.stdout)
    formula_text = (repo / formula["formula"]["path"]).read_text(encoding="utf-8")
    assert 'homepage "https://ait-native.dev"' in formula_text
    assert 'license all_of: ["Apache-2.0", "AGPL-3.0-only"]' in formula_text
    assert "preserve_rpath" in formula_text
    assert 'url "file://' in formula_text
    assert 'system Formula["python@' in formula_text
    assert 'system Formula["python@3' in formula_text
    assert '"-m", "venv", libexec' in formula_text
    assert 'cp cached_download, wheel' in formula_text
    assert 'system libexec/"bin/python", "-m", "pip", "install", wheel' in formula_text
    assert "`ait-server` and `ait-worker` still require self-hosted runtime configuration." in formula_text
    assert 'bin.install_symlink libexec/"bin/ait"' in formula_text
    assert 'bin.install_symlink libexec/"bin/ait-server"' in formula_text
    assert 'bin.install_symlink libexec/"bin/ait-worker"' in formula_text
    assert 'bin.install_symlink libexec/"bin/aitk"' in formula_text
    assert 'bin.install_symlink libexec/"bin/ait-web"' not in formula_text
    assert formula["formula"]["artifact_kind"] == "wheel"


def test_release_publish_uploads_built_candidate_to_remote(tmp_path: Path, monkeypatch):
    repo = tmp_path / "fixture-release-remote-publish"
    repo.mkdir()
    monkeypatch.chdir(repo)

    with running_server(tmp_path / "server-data-release-publish") as base_url:
        assert runner.invoke(app, ["init", "--name", "fixture-release"], catch_exceptions=False).exit_code == 0
        _write_release_fixture(repo)
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", "fixture-release", "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0

        snapshot_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
        assert snapshot_out.exit_code == 0, snapshot_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        create_out = runner.invoke(
            app,
            ["release", "candidate", "create", "--version", "0.1.0", "--line", "main", "--profile", "local-cli", "--json"],
            catch_exceptions=False,
        )
        assert create_out.exit_code == 0, create_out.stdout
        release_record = json.loads(create_out.stdout)

        check_out = runner.invoke(
            app,
            ["release", "check", release_record["release_id"], "--skip-tests-reason", "fixture coverage runs elsewhere", "--json"],
            catch_exceptions=False,
        )
        assert check_out.exit_code == 0, check_out.stdout

        build_out = runner.invoke(app, ["release", "build", release_record["release_id"], "--json"], catch_exceptions=False)
        assert build_out.exit_code == 0, build_out.stdout
        built = json.loads(build_out.stdout)

        formula_out = runner.invoke(
            app,
            ["release", "formula", release_record["release_id"], "--name", "fixture-release", "--json"],
            catch_exceptions=False,
        )
        assert formula_out.exit_code == 0, formula_out.stdout

        publish_out = runner.invoke(
            app,
            ["release", "publish", release_record["release_id"], "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert publish_out.exit_code == 0, publish_out.stdout
        published = json.loads(publish_out.stdout)
        assert published["status"] == "published"
        assert published["next_action"]["code"] == "published_remote"
        assert published["metadata"]["remote_publish"]["repo_name"] == "fixture-release"

        remote_show_out = runner.invoke(
            app,
            ["release", "show", release_record["release_id"], "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert remote_show_out.exit_code == 0, remote_show_out.stdout
        remote_release = json.loads(remote_show_out.stdout)
        assert remote_release["status"] == "published"
        assert remote_release["formula"]["url"].startswith(base_url)
        assert remote_release["formula"]["download_url"].startswith(base_url)
        assert remote_release["formula"]["artifact_kind"] == "wheel"

        artifact_map = {row["kind"]: row for row in remote_release["artifacts"]}
        assert {"sdist", "wheel", "manifest", "checksum", "formula"} <= set(artifact_map)
        local_sdist = next(row for row in built["artifacts"] if row["kind"] == "sdist")
        downloaded_sdist = urllib.request.urlopen(artifact_map["sdist"]["download_url"]).read()
        assert downloaded_sdist == Path(local_sdist["path"]).read_bytes()
