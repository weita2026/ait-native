from __future__ import annotations

import json
import os
import re
import tempfile
from pathlib import Path

from typer.testing import CliRunner

from ait.cli.app import app


REPO_ROOT = Path(__file__).resolve().parents[1]
runner = CliRunner()
PLAN_SOURCE_TOKEN_FORBIDDEN: dict[Path, tuple[str, ...]] = {
    REPO_ROOT / "src" / "ait" / "cli" / "commands" / "plan.py": (
        "line_sync",
        "root_main_sync",
        "remote_main_sync",
        "--default-line",
    ),
    REPO_ROOT / "src" / "ait" / "cli" / "app.py": (
        "line_sync",
        "root_main_sync",
        "remote_main_sync",
    ),
}
PLAN_SOURCE_REGEX_FORBIDDEN: dict[Path, tuple[str, ...]] = {
    REPO_ROOT / "src" / "ait" / "cli" / "app.py": (
        r"(?is)plan sync.{0,400}--default-line",
        r"(?is)--default-line.{0,400}plan sync",
    ),
}


def _invoke_ok(*argv: str):
    result = runner.invoke(app, list(argv), catch_exceptions=False)
    assert result.exit_code == 0, result.output
    return result


def _help_inventory(*argv: str) -> tuple[list[str], list[str]]:
    help_out = _invoke_ok(*argv, "--help")
    options: list[str] = []
    commands: list[str] = []
    for raw_line in help_out.stdout.splitlines():
        line = raw_line.rstrip()
        option_match = re.match(r"│\s*(?:\*\s+)?(--[a-z0-9-]+)\b", line)
        if option_match:
            options.append(option_match.group(1))
            continue
        command_match = re.match(r"│\s*([a-z][a-z0-9-]*)\s{2,}", line)
        if command_match:
            commands.append(command_match.group(1))
    return options, commands


def _assert_public_plan_contract() -> None:
    plan_help = _invoke_ok("plan", "--help").stdout
    _, commands = _help_inventory("plan")
    sync_options, _ = _help_inventory("plan", "sync")
    assert "create" not in commands
    assert "revise" not in commands
    assert "--default-line" not in sync_options
    for forbidden in ("line_sync", "root_main_sync", "remote_main_sync", "--default-line"):
        assert forbidden not in plan_help
    for subcommand in ("create", "revise"):
        result = runner.invoke(app, ["plan", subcommand], catch_exceptions=False)
        assert result.exit_code != 0
        assert "No such command" in (result.stdout or result.stderr or result.output)
    default_line = runner.invoke(app, ["plan", "sync", "README.md", "--default-line", "main"], catch_exceptions=False)
    assert default_line.exit_code != 0
    assert "No such option" in (default_line.stdout or default_line.stderr or default_line.output)


def _assert_plan_sync_stays_lineage_only() -> None:
    with tempfile.TemporaryDirectory(prefix="ait-plan-cli-contract-") as tmp_dir:
        repo = Path(tmp_dir) / "repo"
        repo.mkdir()
        (repo / "README.md").write_text("base\n", encoding="utf-8")
        plan_file = repo / "docs" / "sprints" / "contract.md"
        plan_file.parent.mkdir(parents=True, exist_ok=True)
        plan_file.write_text(
            "# Contract\n\n"
            "## Keep Sync Lineage Only [plan-ref: contract/root]\n\n"
            "- [ ] Keep sync lineage only [ref: contract/lineage-only]\n",
            encoding="utf-8",
        )

        previous_cwd = Path.cwd()
        os.chdir(repo)
        try:
            _invoke_ok("init", "--name", "contract")
            _invoke_ok("snapshot", "create", "--message", "seed")
            line_before = json.loads(_invoke_ok("line", "show", "main", "--json").stdout)
            sync_help = _invoke_ok("plan", "sync", "--help").stdout
            sync_payload = json.loads(_invoke_ok("plan", "sync", str(plan_file), "--json").stdout)
            line_after = json.loads(_invoke_ok("line", "show", "main", "--json").stdout)
        finally:
            os.chdir(previous_cwd)

        for forbidden in ("line_sync", "root_main_sync", "remote_main_sync", "--default-line"):
            assert forbidden not in sync_help
        for key in ("line_sync", "root_main_sync", "remote_main_sync"):
            assert key not in sync_payload
        assert line_after["head_snapshot_id"] == line_before["head_snapshot_id"]


def _assert_plan_sync_bypasses_root_worktree_guard() -> None:
    with tempfile.TemporaryDirectory(prefix="ait-plan-sync-root-guard-") as tmp_dir:
        repo = Path(tmp_dir) / "repo"
        repo.mkdir()
        (repo / "README.md").write_text("base\n", encoding="utf-8")
        plan_file = repo / "docs" / "sprints" / "root_guard.md"
        plan_file.parent.mkdir(parents=True, exist_ok=True)
        plan_file.write_text(
            "# Root Guard\n\n"
            "## Keep Public Plan Sync Available [plan-ref: contract/root-guard]\n\n"
            "- [ ] keep repo-root plan sync unblocked [ref: contract/root-guard-bypass]\n",
            encoding="utf-8",
        )

        previous_cwd = Path.cwd()
        os.chdir(repo)
        try:
            _invoke_ok("init", "--name", "contract-root-guard")
            _invoke_ok("snapshot", "create", "--message", "seed")
            _invoke_ok(
                "config",
                "set",
                "--plan-task-binding-mode",
                "advisory",
                "--json",
            )
            started = json.loads(
                _invoke_ok(
                    "task",
                    "start",
                    "--local",
                    "--title",
                    "Root guard task",
                    "--intent",
                    "pin repo root while plan sync stays public",
                    "--base-line",
                    "main",
                    "--json",
                ).stdout
            )
            blocked_snapshot = runner.invoke(
                app,
                ["snapshot", "create", "--message", "blocked from root"],
                catch_exceptions=False,
            )
            assert blocked_snapshot.exit_code == 2
            blocked_output = blocked_snapshot.output or blocked_snapshot.stdout
            assert "Repo root is pinned to bound worktree" in blocked_output
            assert started["task_id"] in blocked_output
            assert started["worktree"]["name"] in blocked_output

            sync_out = runner.invoke(app, ["plan", "sync", str(plan_file), "--json"], catch_exceptions=False)
            assert sync_out.exit_code == 0, sync_out.output
            sync_output = sync_out.output or sync_out.stdout
            assert "Repo root is pinned to bound worktree" not in sync_output
        finally:
            os.chdir(previous_cwd)


def _assert_plan_source_files_omit_legacy_line_alignment_contract() -> None:
    for path, forbidden_tokens in PLAN_SOURCE_TOKEN_FORBIDDEN.items():
        text = path.read_text(encoding="utf-8")
        for forbidden in forbidden_tokens:
            assert forbidden not in text, f"{path} still contains forbidden plan token: {forbidden}"

    for path, forbidden_patterns in PLAN_SOURCE_REGEX_FORBIDDEN.items():
        text = path.read_text(encoding="utf-8")
        for pattern in forbidden_patterns:
            assert not re.search(pattern, text), f"{path} still matches forbidden plan pattern: {pattern}"


def _assert_init_keeps_sprint_readme_forbidden() -> None:
    with tempfile.TemporaryDirectory(prefix="ait-sprint-readme-bootstrap-") as tmp_dir:
        repo = Path(tmp_dir) / "repo"
        repo.mkdir()

        previous_cwd = Path.cwd()
        os.chdir(repo)
        try:
            init_payload = json.loads(_invoke_ok("init", "--name", "contract-bootstrap", "--json").stdout)
        finally:
            os.chdir(previous_cwd)

        assert init_payload["bootstrap_files"] == [
            {"path": "AGENTS.md"},
            {"path": "ait-native.md"},
            {"path": "docs/plan.md"},
            {"path": "docs/milestone.md"},
        ]
        assert init_payload["bootstrap_guide"] == {"path": "ait-native.md"}
        assert init_payload["forbidden_bootstrap_paths"] == [{"path": "docs/sprints/README.md"}]
        assert not any(row["path"] == "docs/sprints/README.md" for row in init_payload["bootstrap_files"])
        assert not (repo / "docs" / "sprints" / "README.md").exists()


def _assert_sprint_readme_contract() -> None:
    sprint_readme = REPO_ROOT / "docs" / "sprints" / "README.md"
    if not sprint_readme.exists():
        return
    text = sprint_readme.read_text(encoding="utf-8")
    for forbidden in ("ait plan create", "ait plan revise", "plan create|revise"):
        assert forbidden not in text
    assert not re.search(r"^\s{0,3}#+ .*?\[plan-ref:\s*[^`].*$", text, re.MULTILINE)
    assert not re.search(r"^\s*-\s+\[[ xX]\].*?\[ref:\s*[^`].*$", text, re.MULTILINE)
    assert "directory note only" in text
    assert "should not become" in text
    assert "primary entry surface" in text

    routing_text = (REPO_ROOT / "docs" / "sprint_artifact_routing.md").read_text(encoding="utf-8")
    assert "Do not treat `docs/sprints/README.md`" in routing_text
    assert "authority layer" in routing_text

    quickstart_text = (REPO_ROOT / "docs" / "ait_native_quickstart.md").read_text(encoding="utf-8")
    assert "must not create" in quickstart_text
    assert "docs/sprints/README.md" in quickstart_text
    assert "sprint entry surface" in quickstart_text


def main() -> None:
    _assert_public_plan_contract()
    _assert_plan_sync_stays_lineage_only()
    _assert_plan_sync_bypasses_root_worktree_guard()
    _assert_plan_source_files_omit_legacy_line_alignment_contract()
    _assert_init_keeps_sprint_readme_forbidden()
    _assert_sprint_readme_contract()


if __name__ == "__main__":
    main()
