from __future__ import annotations

import importlib.util
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SCRIPT_PATH = ROOT / "scripts" / "postgres_remote_task_test_case_links.py"
SPEC = importlib.util.spec_from_file_location("postgres_remote_task_test_case_links", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
link_script = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = link_script
SPEC.loader.exec_module(link_script)


def _case(test_case_id: str, path: str, function_name: str, *, line: int = 10) -> dict[str, object]:
    return {
        "repo_name": "ait",
        "repo_id": "REPO-1",
        "test_case_id": test_case_id,
        "pytest_node_id": f"{path}::{function_name}",
        "test_file_path": path,
        "class_name": None,
        "function_name": function_name,
        "description": f"Exercise {function_name}.",
        "source_line": line,
    }


def test_extract_changed_test_files_filters_to_python_tests() -> None:
    diff_stats = {
        "paths": {
            "added": ["tests/test_alpha.py", "docs/plan.md"],
            "modified": ["src/app.py", "tests/test_beta.py"],
            "deleted": ["tests/test_gamma.py", "README.md"],
        }
    }

    assert link_script.extract_changed_test_files(diff_stats) == [
        "tests/test_alpha.py",
        "tests/test_beta.py",
        "tests/test_gamma.py",
    ]


def test_build_task_verification_matches_when_reverse_lookup_resolves_same_cases() -> None:
    task = {"task_id": "T-1", "title": "Example", "status": "completed", "repo_id": "REPO-1", "repo_name": "ait"}
    change = {"change_id": "C-1", "selected_patchset_id": "P-1", "current_patchset_id": "P-1"}
    patchset = {"patchset_id": "P-1", "diff_stats": {"paths": {"modified": ["tests/test_alpha.py", "tests/test_beta.py"]}}}
    alpha = _case("TC-1", "tests/test_alpha.py", "test_alpha")
    beta = _case("TC-2", "tests/test_beta.py", "test_beta")

    verification = link_script.build_task_verification(
        task,
        changes=[change],
        patchsets_by_id={"P-1": patchset},
        inventory_by_path={
            "tests/test_alpha.py": [alpha],
            "tests/test_beta.py": [beta],
        },
        inventory_by_function={
            "test_alpha": [alpha],
            "test_beta": [beta],
        },
    )

    assert verification["counts_match"] is True
    assert verification["snapshot_test_case_count"] == 2
    assert verification["reviewer_test_case_count"] == 2
    assert [row["test_case_id"] for row in verification["link_rows"]] == ["TC-1", "TC-2"]


def test_build_task_verification_flags_mismatch_when_function_lookup_expands_to_other_files() -> None:
    task = {"task_id": "T-2", "title": "Example", "status": "completed", "repo_id": "REPO-1", "repo_name": "ait"}
    change = {"change_id": "C-2", "selected_patchset_id": "P-2", "current_patchset_id": "P-2"}
    patchset = {"patchset_id": "P-2", "diff_stats": {"paths": {"modified": ["tests/test_alpha.py"]}}}
    alpha = _case("TC-1", "tests/test_alpha.py", "test_same_name")
    other = _case("TC-99", "tests/test_other.py", "test_same_name", line=25)

    verification = link_script.build_task_verification(
        task,
        changes=[change],
        patchsets_by_id={"P-2": patchset},
        inventory_by_path={"tests/test_alpha.py": [alpha]},
        inventory_by_function={"test_same_name": [alpha, other]},
    )

    assert verification["counts_match"] is False
    assert verification["snapshot_test_case_count"] == 1
    assert verification["reviewer_test_case_count"] == 2
    assert verification["link_rows"] == []
