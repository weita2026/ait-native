from __future__ import annotations

import os
import shutil
import stat
import subprocess
from pathlib import Path
from typing import Iterable


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_SOURCE = ROOT / "scripts" / "github_publish.sh"
AITIGNORE_SOURCE = ROOT / ".aitignore"
GITIGNORE_SOURCE = ROOT / ".gitignore"
GITHUB_PUBLISH_IGNORE_SOURCE = ROOT / ".ait-github-publish-ignore"


def _run(command: list[str], *, cwd: Path, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(command, cwd=cwd, env=env, check=True, capture_output=True, text=True)


def _git(cwd: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return _run(["git", *args], cwd=cwd)


def _configure_git_identity(repo: Path) -> None:
    _git(repo, "config", "user.name", "AIT Test")
    _git(repo, "config", "user.email", "ait-tests@example.com")


def _copy_publish_workspace(source_repo: Path, target_repo: Path) -> None:
    target_repo.mkdir()
    (target_repo / ".ait").mkdir()
    (target_repo / "scripts").mkdir()
    shutil.copy2(source_repo / "README.md", target_repo / "README.md")
    if (source_repo / ".aitignore").exists():
        shutil.copy2(source_repo / ".aitignore", target_repo / ".aitignore")
    if (source_repo / ".ait-github-publish-ignore").exists():
        shutil.copy2(source_repo / ".ait-github-publish-ignore", target_repo / ".ait-github-publish-ignore")
    shutil.copy2(GITIGNORE_SOURCE, target_repo / ".gitignore")
    shutil.copy2(SCRIPT_SOURCE, target_repo / "scripts" / "github_publish.sh")
    mode = os.stat(target_repo / "scripts" / "github_publish.sh").st_mode
    os.chmod(target_repo / "scripts" / "github_publish.sh", mode | stat.S_IXUSR)


def _copy_if_exists(source_repo: Path, target_repo: Path, relative_path: str) -> None:
    source = source_repo / relative_path
    if not source.exists():
        return
    target = target_repo / relative_path
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, target)


def _write_release_fixture(
    repo: Path,
    *,
    version: str,
    extra_files: Iterable[str] = (),
) -> None:
    (repo / "README.md").write_text("release docs\n", encoding="utf-8")
    (repo / ".gitignore").write_text(GITIGNORE_SOURCE.read_text(encoding="utf-8"), encoding="utf-8")
    (repo / ".aitignore").write_text(AITIGNORE_SOURCE.read_text(encoding="utf-8"), encoding="utf-8")
    (repo / ".ait-github-publish-ignore").write_text(
        GITHUB_PUBLISH_IGNORE_SOURCE.read_text(encoding="utf-8"), encoding="utf-8"
    )
    (repo / "pyproject.toml").write_text(
        "\n".join(
            [
                "[project]",
                'name = "ait-native"',
                f'version = "{version}"',
                'readme = "README.md"',
                'requires-python = ">=3.11"',
                "",
            ]
        ),
        encoding="utf-8",
    )
    (repo / "src" / "ait").mkdir(parents=True)
    (repo / "src" / "ait" / "__init__.py").write_text(f'__version__ = "{version}"\n', encoding="utf-8")
    (repo / "sql").mkdir(parents=True, exist_ok=True)
    (repo / "sql" / "bootstrap.sql").write_text("select 1;\n", encoding="utf-8")
    (repo / "docs").mkdir(parents=True, exist_ok=True)
    (repo / "docs" / "release.md").write_text("release docs\n", encoding="utf-8")
    (repo / "LICENSE").write_text("license\n", encoding="utf-8")
    (repo / "NOTICE").write_text("notice\n", encoding="utf-8")
    for relative_path in extra_files:
        target = repo / relative_path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(f"{relative_path}\n", encoding="utf-8")


def _run_publish_script(repo: Path, *args: str, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    script_path = repo / "scripts" / "github_publish.sh"
    return _run([str(script_path), *args], cwd=repo, env=env)


def _run_publish_script_no_check(repo: Path, *args: str, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    script_path = repo / "scripts" / "github_publish.sh"
    return subprocess.run([str(script_path), *args], cwd=repo, env=env, check=False, capture_output=True, text=True)


def _staged_status(workspace: Path) -> str:
    return _git(
        workspace,
        f"--git-dir={workspace / '.ait' / 'publisher-git'}",
        f"--work-tree={workspace}",
        "status",
        "--short",
    ).stdout


def test_github_publish_script_bootstrap_fetch_rebase_add_and_push_origin(tmp_path: Path):
    source = tmp_path / "source"
    source.mkdir()
    _git(source, "init", "-b", "main")
    _configure_git_identity(source)
    (source / "README.md").write_text("base\n", encoding="utf-8")
    (source / ".gitignore").write_text(GITIGNORE_SOURCE.read_text(encoding="utf-8"), encoding="utf-8")
    (source / "scripts").mkdir()
    shutil.copy2(SCRIPT_SOURCE, source / "scripts" / "github_publish.sh")
    _git(source, "add", "README.md", ".gitignore", "scripts/github_publish.sh")
    _git(source, "commit", "-m", "seed publish helper")

    origin = tmp_path / "origin.git"
    _git(tmp_path, "clone", "--bare", str(source), str(origin))

    workspace = tmp_path / "workspace"
    _copy_publish_workspace(source, workspace)
    bootstrap = _run_publish_script(workspace, "bootstrap", "--remote-url", str(origin))
    assert f"git_dir={workspace / '.ait' / 'publisher-git'}" in bootstrap.stdout
    hidden_status = _git(
        workspace,
        f"--git-dir={workspace / '.ait' / 'publisher-git'}",
        f"--work-tree={workspace}",
        "status",
        "--short",
    )
    assert hidden_status.stdout.strip() == ""
    _run_publish_script(workspace, "push-origin", "--dry-run")

    (workspace / "LOCAL.txt").write_text("local\n", encoding="utf-8")
    _run_publish_script(workspace, "add", "LOCAL.txt")
    _run_publish_script(workspace, "git", "config", "user.name", "AIT Publish Test")
    _run_publish_script(workspace, "git", "config", "user.email", "ait-publish-tests@example.com")
    _run_publish_script(workspace, "commit", "-m", "local publish change")

    updater = tmp_path / "updater"
    _git(tmp_path, "clone", str(origin), str(updater))
    _configure_git_identity(updater)
    (updater / "REMOTE.txt").write_text("remote\n", encoding="utf-8")
    _git(updater, "add", "REMOTE.txt")
    _git(updater, "commit", "-m", "remote upstream change")
    _git(updater, "push", "origin", "HEAD:main")

    _run_publish_script(workspace, "fetch")
    _run_publish_script(workspace, "rebase", "origin/main")

    (workspace / "PUBLISH.txt").write_text("publish\n", encoding="utf-8")
    _run_publish_script(workspace, "add", "PUBLISH.txt")
    _run_publish_script(workspace, "commit", "-m", "publish helper push")
    _run_publish_script(workspace, "push-origin", "--dry-run")
    _run_publish_script(workspace, "push-origin")

    verifier = tmp_path / "verifier"
    _git(tmp_path, "clone", str(origin), str(verifier))
    assert (verifier / "REMOTE.txt").read_text(encoding="utf-8") == "remote\n"
    assert (verifier / "LOCAL.txt").read_text(encoding="utf-8") == "local\n"
    assert (verifier / "PUBLISH.txt").read_text(encoding="utf-8") == "publish\n"


def test_github_publish_script_uses_canonical_repo_root_from_real_ait_symlink(tmp_path: Path):
    canonical = tmp_path / "canonical"
    canonical.mkdir()
    (canonical / ".ait").mkdir()
    (canonical / "docs").mkdir()
    (canonical / "scripts").mkdir()
    _git(canonical, "init", "-b", "main")
    _configure_git_identity(canonical)
    (canonical / "README.md").write_text("base\n", encoding="utf-8")
    (canonical / ".gitignore").write_text(GITIGNORE_SOURCE.read_text(encoding="utf-8"), encoding="utf-8")
    (canonical / "docs" / "guide.md").write_text("guide\n", encoding="utf-8")
    shutil.copy2(SCRIPT_SOURCE, canonical / "scripts" / "github_publish.sh")
    mode = os.stat(canonical / "scripts" / "github_publish.sh").st_mode
    os.chmod(canonical / "scripts" / "github_publish.sh", mode | stat.S_IXUSR)
    _git(canonical, "add", "README.md", ".gitignore", "docs/guide.md", "scripts/github_publish.sh")
    _git(canonical, "commit", "-m", "seed canonical publish helper")

    origin = tmp_path / "origin.git"
    _git(tmp_path, "clone", "--bare", str(canonical), str(origin))

    task_workspace = tmp_path / "task-workspace"
    task_workspace.mkdir()
    (task_workspace / "scripts").mkdir()
    shutil.copy2(SCRIPT_SOURCE, task_workspace / "scripts" / "github_publish.sh")
    mode = os.stat(task_workspace / "scripts" / "github_publish.sh").st_mode
    os.chmod(task_workspace / "scripts" / "github_publish.sh", mode | stat.S_IXUSR)
    (task_workspace / ".ait").symlink_to(canonical / ".ait", target_is_directory=True)

    bootstrap = _run_publish_script(task_workspace, "bootstrap", "--remote-url", str(origin))
    assert f"repo_root={canonical}" in bootstrap.stdout
    assert f"workspace_root={task_workspace}" in bootstrap.stdout

    status = _run_publish_script(task_workspace, "status")
    assert status.stdout.strip() == ""


def test_github_publish_script_rebase_requires_clean_tracked_publish_work_tree(tmp_path: Path):
    source = tmp_path / "source"
    source.mkdir()
    _git(source, "init", "-b", "main")
    _configure_git_identity(source)
    (source / "README.md").write_text("base\n", encoding="utf-8")
    (source / ".gitignore").write_text(GITIGNORE_SOURCE.read_text(encoding="utf-8"), encoding="utf-8")
    (source / "scripts").mkdir()
    shutil.copy2(SCRIPT_SOURCE, source / "scripts" / "github_publish.sh")
    _git(source, "add", "README.md", ".gitignore", "scripts/github_publish.sh")
    _git(source, "commit", "-m", "seed publish helper")

    origin = tmp_path / "origin.git"
    _git(tmp_path, "clone", "--bare", str(source), str(origin))

    workspace = tmp_path / "workspace"
    _copy_publish_workspace(source, workspace)
    _run_publish_script(workspace, "bootstrap", "--remote-url", str(origin))
    (workspace / "README.md").write_text("dirty\n", encoding="utf-8")

    result = _run_publish_script_no_check(workspace, "rebase", "origin/main")
    assert result.returncode == 1
    assert "Publish rebase requires a clean tracked publish work tree" in result.stderr


def test_github_publish_script_bootstrap_uses_current_default_remote_url(tmp_path: Path):
    source = tmp_path / "source"
    source.mkdir()
    _git(source, "init", "-b", "main")
    _configure_git_identity(source)
    (source / "README.md").write_text("base\n", encoding="utf-8")
    (source / ".gitignore").write_text(GITIGNORE_SOURCE.read_text(encoding="utf-8"), encoding="utf-8")
    (source / "scripts").mkdir()
    shutil.copy2(SCRIPT_SOURCE, source / "scripts" / "github_publish.sh")
    _git(source, "add", "README.md", ".gitignore", "scripts/github_publish.sh")
    _git(source, "commit", "-m", "seed publish helper")

    workspace = tmp_path / "workspace"
    _copy_publish_workspace(source, workspace)
    bootstrap = _run_publish_script(workspace, "bootstrap")
    assert "remote=origin" in bootstrap.stdout

    remote_url = _run_publish_script(workspace, "git", "remote", "get-url", "origin")
    assert remote_url.stdout.strip() == "git@github.com:weita2026/ait.git"


def test_github_publish_script_python_release_reports_metadata(tmp_path: Path):
    source = tmp_path / "source"
    source.mkdir()
    _git(source, "init", "-b", "main")
    _configure_git_identity(source)
    (source / "README.md").write_text("release docs\n", encoding="utf-8")
    (source / ".gitignore").write_text(GITIGNORE_SOURCE.read_text(encoding="utf-8"), encoding="utf-8")
    (source / ".aitignore").write_text(AITIGNORE_SOURCE.read_text(encoding="utf-8"), encoding="utf-8")
    (source / "pyproject.toml").write_text(
        "\n".join(
            [
                "[project]",
                'name = "ait-native"',
                'version = "9.9.9"',
                'readme = "README.md"',
                'requires-python = ">=3.11"',
                "",
            ]
        ),
        encoding="utf-8",
    )
    (source / "src" / "ait").mkdir(parents=True)
    (source / "src" / "ait" / "__init__.py").write_text('__version__ = "9.9.9"\n', encoding="utf-8")
    (source / "scripts").mkdir()
    shutil.copy2(SCRIPT_SOURCE, source / "scripts" / "github_publish.sh")
    _git(source, "add", "README.md", ".aitignore", ".gitignore", "pyproject.toml", "scripts/github_publish.sh", "src/ait/__init__.py")
    _git(source, "commit", "-m", "seed python release helper")

    workspace = tmp_path / "workspace"
    _copy_publish_workspace(source, workspace)
    shutil.copy2(source / "pyproject.toml", workspace / "pyproject.toml")
    (workspace / "src" / "ait").mkdir(parents=True)
    shutil.copy2(source / "src" / "ait" / "__init__.py", workspace / "src" / "ait" / "__init__.py")

    release = _run_publish_script(workspace, "python-release")
    assert "package_name=ait-native" in release.stdout
    assert "package_version=9.9.9" in release.stdout
    assert "requires_python=>=3.11" in release.stdout
    assert "release_path=pyproject.toml" in release.stdout
    assert "release_path=README.md" in release.stdout
    assert "release_path=src" in release.stdout


def test_github_publish_script_add_and_add_python_release_stage_pyproject(tmp_path: Path):
    source = tmp_path / "source"
    source.mkdir()
    _git(source, "init", "-b", "main")
    _configure_git_identity(source)
    (source / "README.md").write_text("release docs\n", encoding="utf-8")
    (source / ".gitignore").write_text(GITIGNORE_SOURCE.read_text(encoding="utf-8"), encoding="utf-8")
    (source / ".aitignore").write_text(AITIGNORE_SOURCE.read_text(encoding="utf-8"), encoding="utf-8")
    (source / "pyproject.toml").write_text(
        "\n".join(
            [
                "[project]",
                'name = "ait-native"',
                'version = "1.2.3"',
                'readme = "README.md"',
                'requires-python = ">=3.11"',
                "",
            ]
        ),
        encoding="utf-8",
    )
    (source / "src" / "ait").mkdir(parents=True)
    (source / "src" / "ait" / "__init__.py").write_text('__version__ = "1.2.3"\n', encoding="utf-8")
    (source / "scripts").mkdir()
    shutil.copy2(SCRIPT_SOURCE, source / "scripts" / "github_publish.sh")
    _git(source, "add", "README.md", ".aitignore", ".gitignore", "pyproject.toml", "scripts/github_publish.sh", "src/ait/__init__.py")
    _git(source, "commit", "-m", "seed python release helper")

    origin = tmp_path / "origin.git"
    _git(tmp_path, "clone", "--bare", str(source), str(origin))

    workspace = tmp_path / "workspace"
    _copy_publish_workspace(source, workspace)
    shutil.copy2(source / "pyproject.toml", workspace / "pyproject.toml")
    (workspace / "src" / "ait").mkdir(parents=True)
    shutil.copy2(source / "src" / "ait" / "__init__.py", workspace / "src" / "ait" / "__init__.py")
    _run_publish_script(workspace, "bootstrap", "--remote-url", str(origin))

    (workspace / "README.md").write_text("release docs updated\n", encoding="utf-8")
    (workspace / "pyproject.toml").write_text(
        "\n".join(
            [
                "[project]",
                'name = "ait-native"',
                'version = "1.2.4"',
                'readme = "README.md"',
                'requires-python = ">=3.11"',
                "",
            ]
        ),
        encoding="utf-8",
    )
    (workspace / "src" / "ait" / "__init__.py").write_text('__version__ = "1.2.4"\n', encoding="utf-8")

    _run_publish_script(workspace, "add")
    status = _git(
        workspace,
        f"--git-dir={workspace / '.ait' / 'publisher-git'}",
        f"--work-tree={workspace}",
        "status",
        "--short",
    )
    assert "M  README.md" in status.stdout
    assert "M  src/ait/__init__.py" in status.stdout
    assert "M  pyproject.toml" in status.stdout

    add_release = _run_publish_script(workspace, "add-python-release")
    assert "package_name=ait-native" in add_release.stdout
    assert "package_version=1.2.4" in add_release.stdout
    staged_status = _git(
        workspace,
        f"--git-dir={workspace / '.ait' / 'publisher-git'}",
        f"--work-tree={workspace}",
        "status",
        "--short",
    )
    assert "M  pyproject.toml" in staged_status.stdout


def test_github_publish_script_add_without_paths_respects_github_publish_ignore(tmp_path: Path):
    source = tmp_path / "source"
    source.mkdir()
    _git(source, "init", "-b", "main")
    _configure_git_identity(source)
    (source / "README.md").write_text("publish me\n", encoding="utf-8")
    (source / ".gitignore").write_text(GITIGNORE_SOURCE.read_text(encoding="utf-8"), encoding="utf-8")
    (source / ".aitignore").write_text(AITIGNORE_SOURCE.read_text(encoding="utf-8"), encoding="utf-8")
    (source / ".ait-github-publish-ignore").write_text(
        "\n".join(
            [
                "docs/plan.md",
                "src/ait_web/**",
                "site/**",
                "deploy/site/**",
                "",
            ]
        ),
        encoding="utf-8",
    )
    (source / "docs").mkdir(parents=True)
    (source / "docs" / "plan.md").write_text("exclude me\n", encoding="utf-8")
    (source / "src" / "ait_web").mkdir(parents=True)
    (source / "src" / "ait_web" / "app.py").write_text("print('exclude')\n", encoding="utf-8")
    (source / "site").mkdir(parents=True)
    (source / "site" / "index.html").write_text("<html></html>\n", encoding="utf-8")
    (source / "deploy" / "site").mkdir(parents=True)
    (source / "deploy" / "site" / "README.md").write_text("exclude deploy site\n", encoding="utf-8")
    (source / "scripts").mkdir()
    shutil.copy2(SCRIPT_SOURCE, source / "scripts" / "github_publish.sh")
    _git(
        source,
        "add",
        "README.md",
        ".aitignore",
        ".ait-github-publish-ignore",
        ".gitignore",
        "docs/plan.md",
        "src/ait_web/app.py",
        "site/index.html",
        "deploy/site/README.md",
        "scripts/github_publish.sh",
    )
    _git(source, "commit", "-m", "seed github publish ignore fixture")

    origin = tmp_path / "origin.git"
    _git(tmp_path, "clone", "--bare", str(source), str(origin))

    workspace = tmp_path / "workspace"
    _copy_publish_workspace(source, workspace)
    _copy_if_exists(source, workspace, "docs/plan.md")
    _copy_if_exists(source, workspace, "src/ait_web/app.py")
    _copy_if_exists(source, workspace, "site/index.html")
    _copy_if_exists(source, workspace, "deploy/site/README.md")
    _run_publish_script(workspace, "bootstrap", "--remote-url", str(origin))

    (workspace / "README.md").write_text("publish me too\n", encoding="utf-8")
    (workspace / "docs" / "plan.md").write_text("still exclude me\n", encoding="utf-8")
    (workspace / "src" / "ait_web" / "app.py").write_text("print('still exclude')\n", encoding="utf-8")
    (workspace / "site" / "index.html").write_text("<html>still exclude</html>\n", encoding="utf-8")
    (workspace / "deploy" / "site" / "README.md").write_text("still exclude deploy site\n", encoding="utf-8")

    _run_publish_script(workspace, "add")
    status = _staged_status(workspace)
    assert "M  README.md" in status
    for relative_path in (
        "docs/plan.md",
        "src/ait_web/app.py",
        "site/index.html",
        "deploy/site/README.md",
    ):
        assert f"M  {relative_path}" not in status
        assert f" M {relative_path}" in status


def test_github_publish_script_usage_omits_legacy_container_release_commands(tmp_path: Path):
    source = tmp_path / "source"
    source.mkdir()
    _write_release_fixture(source, version="2.3.4")
    (source / "scripts").mkdir()
    shutil.copy2(SCRIPT_SOURCE, source / "scripts" / "github_publish.sh")

    workspace = tmp_path / "workspace"
    _copy_publish_workspace(source, workspace)
    for relative_path in ("pyproject.toml", "src/ait/__init__.py", "sql/bootstrap.sql", "docs/release.md"):
        _copy_if_exists(source, workspace, relative_path)

    usage = _run_publish_script_no_check(workspace)
    combined = usage.stdout + usage.stderr
    assert "docker-release" not in combined
    assert "add-docker-release" not in combined
    assert "docker-build" not in combined
    assert "docker-push" not in combined
    assert "python-release" in combined


def test_github_publish_script_rejects_legacy_container_release_aliases(tmp_path: Path):
    source = tmp_path / "source"
    source.mkdir()
    _write_release_fixture(source, version="3.4.5")
    (source / "scripts").mkdir()
    shutil.copy2(SCRIPT_SOURCE, source / "scripts" / "github_publish.sh")

    workspace = tmp_path / "workspace"
    _copy_publish_workspace(source, workspace)
    for relative_path in ("pyproject.toml", "src/ait/__init__.py", "sql/bootstrap.sql", "docs/release.md"):
        _copy_if_exists(source, workspace, relative_path)

    for command in ("docker-release", "add-docker-release", "docker-build", "docker-push"):
        result = _run_publish_script_no_check(workspace, command)
        assert result.returncode != 0
        combined = result.stdout + result.stderr
        assert f"Unknown command: {command}" in combined
