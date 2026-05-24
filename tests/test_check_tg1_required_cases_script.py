from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SCRIPT_PATH = ROOT / "scripts" / "check_tg1_required_cases.py"
SPEC = importlib.util.spec_from_file_location("check_tg1_required_cases", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
tg1_script = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = tg1_script
SPEC.loader.exec_module(tg1_script)


def test_resolve_membership_sql_path_prefers_canonical_repo_root_for_task_worktree(tmp_path: Path) -> None:
    repo_root = tmp_path / "ait"
    (repo_root / ".ait").mkdir(parents=True)
    (repo_root / ".ait" / "config.json").write_text(json.dumps({"repo_name": "ait"}), encoding="utf-8")
    sql_path = repo_root / "sql" / "tg1_required_live_pytest_node_ids.sql"
    sql_path.parent.mkdir(parents=True)
    sql_path.write_text("select 1;\n", encoding="utf-8")

    worktree_root = tmp_path / "t-1080"
    worktree_root.mkdir()
    (worktree_root / ".ait").symlink_to(repo_root / ".ait", target_is_directory=True)
    (worktree_root / ".ait-worktree.json").write_text(
        json.dumps({"repo_root": str(repo_root), "workspace_root": str(worktree_root)}),
        encoding="utf-8",
    )

    resolved = tg1_script._resolve_membership_sql_path(
        worktree_root,
        Path("sql") / "tg1_required_live_pytest_node_ids.sql",
    )

    assert resolved == sql_path.resolve()


def test_resolve_contract_doc_path_prefers_canonical_repo_root_for_task_worktree(tmp_path: Path) -> None:
    repo_root = tmp_path / "ait_test"
    (repo_root / ".ait").mkdir(parents=True)
    (repo_root / ".ait" / "config.json").write_text(json.dumps({"repo_name": "ait_test"}), encoding="utf-8")
    contract_doc = repo_root / "docs" / "sprints" / "tg1_sprint_planning_workflow_contract_group.md"
    contract_doc.parent.mkdir(parents=True)
    contract_doc.write_text("# TG-1\n\n1. `tests/test_alpha.py::test_alpha`\n", encoding="utf-8")

    worktree_root = tmp_path / "atxt-2000"
    worktree_root.mkdir()
    (worktree_root / ".ait").symlink_to(repo_root / ".ait", target_is_directory=True)
    (worktree_root / ".ait-worktree.json").write_text(
        json.dumps({"repo_root": str(repo_root), "workspace_root": str(worktree_root)}),
        encoding="utf-8",
    )

    resolved = tg1_script._resolve_contract_doc_path(worktree_root, None)

    assert resolved == contract_doc.resolve()


def test_validate_live_membership_enforces_24_case_floor() -> None:
    passing_nodes = [f"tests/test_case_{index}.py::test_case" for index in range(24)]
    failing_nodes = passing_nodes[:-1]

    passing = tg1_script._validate_live_membership(live_nodes=passing_nodes, minimum_count=24)
    failing = tg1_script._validate_live_membership(live_nodes=failing_nodes, minimum_count=24)

    assert passing == {
        "status": "pass",
        "floor_count": 24,
        "failures": [],
    }
    assert failing["status"] == "fail"
    assert failing["floor_count"] == 24
    assert failing["failures"] == ["Live TG-1 membership has 23 case(s); expected at least 24."]


def test_load_contract_member_count_counts_formal_members_and_rejects_duplicates(tmp_path: Path) -> None:
    contract_doc = tmp_path / "tg1_contract.md"
    contract_doc.write_text(
        "\n".join(
            [
                "# TG-1 Contract",
                "",
                "1. `tests/test_alpha.py::test_alpha`",
                "2. `tests/test_beta.py::test_beta`",
            ]
        )
        + "\n",
        encoding="utf-8",
    )

    assert tg1_script._load_contract_member_count(contract_doc) == 2

    duplicate_doc = tmp_path / "tg1_contract_duplicate.md"
    duplicate_doc.write_text(
        "\n".join(
            [
                "# TG-1 Contract",
                "",
                "1. `tests/test_alpha.py::test_alpha`",
                "2. `tests/test_alpha.py::test_alpha`",
            ]
        )
        + "\n",
        encoding="utf-8",
    )

    try:
        tg1_script._load_contract_member_count(duplicate_doc)
    except RuntimeError as exc:
        assert "duplicate TG-1 members" in str(exc)
    else:
        raise AssertionError("Expected duplicate contract members to raise RuntimeError")


def test_resolve_minimum_count_prefers_explicit_then_metadata_then_contract_doc() -> None:
    assert tg1_script._resolve_minimum_count(
        explicit_minimum_count=26,
        metadata_expected_count=25,
        contract_member_count=24,
    ) == (26, "cli")
    assert tg1_script._resolve_minimum_count(
        explicit_minimum_count=None,
        metadata_expected_count=25,
        contract_member_count=24,
    ) == (25, "test_groups.expected_case_count")
    assert tg1_script._resolve_minimum_count(
        explicit_minimum_count=None,
        metadata_expected_count=None,
        contract_member_count=25,
    ) == (25, "contract_doc")
    assert tg1_script._resolve_minimum_count(
        explicit_minimum_count=None,
        metadata_expected_count=None,
        contract_member_count=None,
    ) == (24, "legacy_default")


def test_build_parser_defaults_to_auto_minimum_count_and_no_baseline_contract() -> None:
    parser = tg1_script.build_parser()
    args = parser.parse_args([])

    assert args.minimum_count is None
    assert args.pytest_workers is None
    assert args.pytest_dist is None
    assert all(action.dest != "baseline_contract" for action in parser._actions)


def test_run_pytest_enables_xdist_when_workers_exceed_one(tmp_path: Path, monkeypatch) -> None:
    source_repo_root = tmp_path / "ait"
    tests_repo_root = tmp_path / "ait_test"
    source_repo_root.mkdir()
    (source_repo_root / "src").mkdir()
    tests_repo_root.mkdir()
    (tests_repo_root / "src").mkdir()
    (tests_repo_root / "postgres_fake.py").write_text("", encoding="utf-8")
    (tests_repo_root / "postgres_live.py").write_text("", encoding="utf-8")
    (tests_repo_root / "ait_web").mkdir()
    (tests_repo_root / "ait_web" / "helpers.py").write_text("", encoding="utf-8")

    recorded: dict[str, object] = {}

    def fake_run(command, **kwargs):
        recorded["command"] = command
        recorded["cwd"] = kwargs.get("cwd")
        recorded["env"] = kwargs.get("env")
        return subprocess.CompletedProcess(command, 0, stdout="ok\n", stderr="")

    monkeypatch.setattr(tg1_script.subprocess, "run", fake_run)

    result = tg1_script._run_pytest(
        source_repo_root=source_repo_root,
        tests_repo_root=tests_repo_root,
        pytest_node_ids=["cli/test_alpha.py::test_alpha", "cli/test_beta.py::test_beta"],
        pytest_workers=6,
        pytest_dist="loadfile",
    )

    assert recorded["command"] == [
        sys.executable,
        "-m",
        "pytest",
        "-q",
        "-n",
        "6",
        "--dist",
        "loadfile",
        "cli/test_alpha.py::test_alpha",
        "cli/test_beta.py::test_beta",
    ]
    assert recorded["cwd"] == tests_repo_root
    assert result["status"] == "pass"
    assert "-n 6 --dist loadfile" in result["command"]


def test_build_markdown_reports_live_membership_only() -> None:
    payload = {
        "repo_name": "ait_test",
        "repo_id": "REPO-123",
        "test_group_id": "TG-1",
        "tests_repo_root": "/tmp/ait_test",
        "membership_sql_path": "/tmp/sql/tg1_required_live_pytest_node_ids.sql",
        "contract_doc_path": "/tmp/ait_test/docs/sprints/tg1_sprint_planning_workflow_contract_group.md",
        "contract_member_count": 25,
        "live_count": 24,
        "minimum_count": 24,
        "minimum_count_source": "contract_doc",
        "pytest_workers": 6,
        "pytest_dist": "loadfile",
        "validation_status": "pass",
        "status": "pass",
        "live_pytest_node_ids": ["tests/test_contract.py::test_tg1_contract"],
        "pytest": {
            "status": "pass",
            "exit_code": 0,
            "duration_seconds": 1.25,
        },
    }

    markdown = tg1_script._build_markdown(payload)

    assert "baseline_contract" not in markdown
    assert "Missing baseline nodes" not in markdown
    assert "- live_count: 24" in markdown
    assert "- minimum_count_source: `contract_doc`" in markdown
    assert "- pytest_workers: 6" in markdown
    assert "- pytest_dist: `loadfile`" in markdown
    assert "## Live TG-1 nodes" in markdown
