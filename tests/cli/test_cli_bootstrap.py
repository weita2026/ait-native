from __future__ import annotations

import importlib
import os
import subprocess
import sys
from pathlib import Path


def test_package_app_export_matches_cli_app_object() -> None:
    import ait.cli as cli_package

    cli_app_module = importlib.import_module("ait.cli.app")
    assert cli_package.app is cli_app_module.app


def test_console_import_prefers_worktree_src_without_pythonpath(tmp_path: Path):
    repo_root = Path(__file__).resolve().parents[2]
    cli_init = repo_root / "src" / "ait" / "cli" / "__init__.py"

    installed_src = tmp_path / "installed-src"
    installed_cli = installed_src / "ait" / "cli"
    installed_cli.mkdir(parents=True)
    (installed_src / "ait" / "__init__.py").write_text("", encoding="utf-8")
    installed_cli.joinpath("__init__.py").write_text(cli_init.read_text(encoding="utf-8"), encoding="utf-8")
    installed_cli.joinpath("app.py").write_text("app = 'repo-root-source'\n", encoding="utf-8")

    worktree = tmp_path / "worktree"
    worktree_cli = worktree / "src" / "ait" / "cli"
    worktree_cli.mkdir(parents=True)
    (worktree / ".ait-worktree.json").write_text('{"worktree_name": "t-probe"}\n', encoding="utf-8")
    (worktree / "src" / "ait" / "__init__.py").write_text("", encoding="utf-8")
    worktree_cli.joinpath("__init__.py").write_text("", encoding="utf-8")
    worktree_cli.joinpath("app.py").write_text("app = 'worktree-source'\n", encoding="utf-8")

    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    console_script = bin_dir / "ait"
    console_script.write_text(
        "import importlib\n"
        "from ait.cli import app\n"
        "loaded = importlib.import_module('ait.cli.app')\n"
        "print(app)\n"
        "print(loaded.__file__)\n",
        encoding="utf-8",
    )

    env = os.environ.copy()
    env["PYTHONPATH"] = str(installed_src)
    completed = subprocess.run(
        [sys.executable, str(console_script)],
        cwd=worktree,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )

    assert completed.returncode == 0, completed.stderr
    lines = completed.stdout.splitlines()
    assert lines[0] == "worktree-source"
    assert lines[1] == str(worktree_cli / "app.py")
