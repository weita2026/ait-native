from __future__ import annotations

import os
import shutil
import stat
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_SOURCE = ROOT / "scripts" / "github_release_publish.sh"


def _run(command: list[str], *, cwd: Path, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(command, cwd=cwd, env=env, check=True, capture_output=True, text=True)


def _git(cwd: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return _run(["git", *args], cwd=cwd)


def _configure_git_identity(repo: Path) -> None:
    _git(repo, "config", "user.name", "AIT Script Test")
    _git(repo, "config", "user.email", "ait-script-tests@example.com")


def _copy_script(workspace: Path) -> Path:
    (workspace / "scripts").mkdir(parents=True, exist_ok=True)
    target = workspace / "scripts" / "github_release_publish.sh"
    shutil.copy2(SCRIPT_SOURCE, target)
    mode = os.stat(target).st_mode
    os.chmod(target, mode | stat.S_IXUSR)
    return target


def _write_dist_fixture(workspace: Path, version: str) -> dict[str, Path]:
    dist_dir = workspace / "dist"
    dist_dir.mkdir(parents=True, exist_ok=True)
    wheel = dist_dir / f"ait_native-{version}-py3-none-any.whl"
    sdist = dist_dir / f"ait-native-{version}.tar.gz"
    manifest = dist_dir / f"ait-release-{version}.manifest.json"
    checksum = dist_dir / f"ait-release-{version}.sha256"
    wheel.write_bytes(b"wheel-bytes\n")
    sdist.write_bytes(b"sdist-bytes\n")
    manifest.write_text('{"version":"%s"}\n' % version, encoding="utf-8")
    checksum.write_text("checksums\n", encoding="utf-8")
    return {
        "wheel": wheel,
        "sdist": sdist,
        "manifest": manifest,
        "checksum": checksum,
    }


def _run_script(workspace: Path, *args: str, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    script_path = workspace / "scripts" / "github_release_publish.sh"
    return _run([str(script_path), *args], cwd=workspace, env=env)


def test_github_release_publish_script_metadata_and_formula_rewrite(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    _copy_script(workspace)
    files = _write_dist_fixture(workspace, "1.2.3")

    formula_dir = workspace / "Formula"
    formula_dir.mkdir(parents=True, exist_ok=True)
    formula_path = formula_dir / "ait-native.rb"
    formula_path.write_text(
        '\n'.join(
            [
                'class AitNative < Formula',
                '  url "file:///tmp/ait_native-1.2.3-py3-none-any.whl"',
                '  sha256 "old-hash"',
                "end",
                "",
            ]
        ),
        encoding="utf-8",
    )

    metadata = _run_script(workspace, "metadata", "--version", "1.2.3")
    assert "release_tag=v1.2.3" in metadata.stdout
    assert "asset_ref_tag=release-assets-v1.2.3" in metadata.stdout
    assert "wheel_name=ait_native-1.2.3-py3-none-any.whl" in metadata.stdout
    assert f"wheel_path={files['wheel']}" in metadata.stdout

    rewrite = _run_script(
        workspace,
        "rewrite-formula",
        "--version",
        "1.2.3",
        "--formula",
        str(formula_path),
    )
    formula_text = formula_path.read_text(encoding="utf-8")
    assert "https://github.com/weita2026/ait-native/releases/download/v1.2.3/ait_native-1.2.3-py3-none-any.whl" in formula_text
    assert 'sha256 "' in formula_text
    assert "formula_path=" in rewrite.stdout


def test_github_release_publish_script_pushes_release_assets_ref(tmp_path: Path) -> None:
    seed = tmp_path / "seed"
    seed.mkdir()
    _git(seed, "init", "-b", "main")
    _configure_git_identity(seed)
    (seed / "README.md").write_text("seed\n", encoding="utf-8")
    _git(seed, "add", "README.md")
    _git(seed, "commit", "-m", "seed main")

    origin = tmp_path / "origin.git"
    _git(tmp_path, "clone", "--bare", str(seed), str(origin))

    workspace = tmp_path / "workspace"
    workspace.mkdir()
    _copy_script(workspace)
    _write_dist_fixture(workspace, "2.3.4")
    notes_path = workspace / "release-notes.md"
    notes_path.write_text("# v2.3.4\n\nPrepared notes.\n", encoding="utf-8")

    env = {
        **os.environ,
        "AIT_GITHUB_RELEASE_GIT_NAME": "AIT Release Bot",
        "AIT_GITHUB_RELEASE_GIT_EMAIL": "ait-release-bot@example.com",
    }
    publish = _run_script(
        workspace,
        "publish-assets-ref",
        "--version",
        "2.3.4",
        "--remote-url",
        str(origin),
        "--notes-file",
        str(notes_path),
        "--branch",
        "release-assets-test",
        env=env,
    )
    assert "release_tag=v2.3.4" in publish.stdout
    assert "asset_ref_tag=release-assets-v2.3.4" in publish.stdout
    assert "asset_branch=release-assets-test" in publish.stdout

    branch_listing = _git(tmp_path, f"--git-dir={origin}", "show-ref", "--verify", "refs/heads/release-assets-test")
    assert "refs/heads/release-assets-test" in branch_listing.stdout
    tag_listing = _git(tmp_path, f"--git-dir={origin}", "show-ref", "--verify", "refs/tags/release-assets-v2.3.4")
    assert "refs/tags/release-assets-v2.3.4" in tag_listing.stdout

    verifier = tmp_path / "verifier"
    _git(tmp_path, "clone", "--branch", "release-assets-test", str(origin), str(verifier))
    release_dir = verifier / "releases" / "v2.3.4"
    assert (release_dir / "ait_native-2.3.4-py3-none-any.whl").read_bytes() == b"wheel-bytes\n"
    assert (release_dir / "ait-native-2.3.4.tar.gz").read_bytes() == b"sdist-bytes\n"
    assert (release_dir / "ait-release-2.3.4.manifest.json").read_text(encoding="utf-8") == '{"version":"2.3.4"}\n'
    assert (release_dir / "ait-release-2.3.4.sha256").read_text(encoding="utf-8") == "checksums\n"
    assert (release_dir / "release-notes.md").read_text(encoding="utf-8") == "# v2.3.4\n\nPrepared notes.\n"

    main_clone = tmp_path / "main-clone"
    _git(tmp_path, "clone", str(origin), str(main_clone))
    assert not (main_clone / "releases" / "v2.3.4").exists()
