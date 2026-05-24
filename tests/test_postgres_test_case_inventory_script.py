from __future__ import annotations

import importlib.util
import json
import sqlite3
import sys
import textwrap
from pathlib import Path

from tests.postgres_fake import FakePsycopg, fake_postgres_dsn, fake_postgres_schema_db_path


ROOT = Path(__file__).resolve().parent.parent
SCRIPT_PATH = ROOT / "scripts" / "postgres_test_case_inventory.py"
SPEC = importlib.util.spec_from_file_location("postgres_test_case_inventory", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
inventory_script = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = inventory_script
SPEC.loader.exec_module(inventory_script)


def _write_test_file(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(textwrap.dedent(text).strip() + "\n", encoding="utf-8")


def test_discover_test_cases_collects_module_and_class_cases(tmp_path: Path) -> None:
    repo_root = tmp_path / "repo"
    _write_test_file(
        repo_root / "tests" / "test_sample.py",
        """
        def test_plain_case():
            \"\"\"Exercise the plain path.\"\"\"
            assert True


        class TestWidgetFlow:
            def test_handles_widget_state(self):
                assert True


        class Helper:
            def test_not_collected(self):
                assert True
        """,
    )

    cases = inventory_script.discover_test_cases(
        repo_root=repo_root,
        tests_root=repo_root / "tests",
    )

    assert [case.pytest_node_id for case in cases] == [
        "tests/test_sample.py::test_plain_case",
        "tests/test_sample.py::TestWidgetFlow::test_handles_widget_state",
    ]
    assert cases[0].description == "Exercise the plain path."
    assert "verifies Handles widget state in TestWidgetFlow" in cases[1].description


def test_build_inventory_rows_include_repo_id_in_stable_ids() -> None:
    cases = [
        inventory_script.DiscoveredTestCase(
            pytest_node_id="tests/test_sample.py::test_plain_case",
            test_file_path="tests/test_sample.py",
            class_name=None,
            function_name="test_plain_case",
            description="Exercise the plain path.",
            source_line=1,
        )
    ]

    first = inventory_script.build_inventory_rows(cases, repo_name="ait", repo_id="REPO-123")
    second = inventory_script.build_inventory_rows(cases, repo_name="ait", repo_id="REPO-123")
    other_repo = inventory_script.build_inventory_rows(cases, repo_name="ait", repo_id="REPO-456")

    assert first[0]["repo_id"] == "REPO-123"
    assert first[0]["repo_name"] == "ait"
    assert first[0]["test_case_id"] == second[0]["test_case_id"]
    assert first[0]["test_case_id"] != other_repo[0]["test_case_id"]


def test_deduplicate_test_cases_keeps_last_duplicate_node_id() -> None:
    first = inventory_script.DiscoveredTestCase(
        pytest_node_id="tests/test_dup.py::test_same_name",
        test_file_path="tests/test_dup.py",
        class_name=None,
        function_name="test_same_name",
        description="first",
        source_line=10,
    )
    second = inventory_script.DiscoveredTestCase(
        pytest_node_id="tests/test_dup.py::test_same_name",
        test_file_path="tests/test_dup.py",
        class_name=None,
        function_name="test_same_name",
        description="second",
        source_line=20,
    )

    unique_cases, duplicate_node_ids = inventory_script.deduplicate_test_cases([first, second])

    assert duplicate_node_ids == ["tests/test_dup.py::test_same_name"]
    assert len(unique_cases) == 1
    assert unique_cases[0].description == "second"
    assert unique_cases[0].source_line == 20


def test_main_writes_json_and_persists_inventory_rows(tmp_path: Path, monkeypatch) -> None:
    repo_root = tmp_path / "repo"
    _write_test_file(
        repo_root / "tests" / "test_sample.py",
        """
        def test_plain_case():
            assert True
        """,
    )
    fake_runtime = tmp_path / "fake-runtime"
    dsn = fake_postgres_dsn(fake_runtime)
    monkeypatch.setattr(inventory_script, "_load_psycopg", lambda: FakePsycopg())

    output_path = tmp_path / "inventory.json"
    exit_code = inventory_script.main(
        [
            "--dsn",
            dsn,
            "--repo-root",
            str(repo_root),
            "--repo-name",
            "ait",
            "--repo-id",
            "AITR-001",
            "--json",
            "--output",
            str(output_path),
        ]
    )

    assert exit_code == 0
    payload = json.loads(output_path.read_text(encoding="utf-8"))
    assert payload["repo_id"] == "AITR-001"
    assert payload["repo_name"] == "ait"
    assert payload["test_case_count"] == 1
    assert payload["raw_discovered_test_case_count"] == 1
    assert payload["duplicate_pytest_node_id_count"] == 0
    assert payload["inserted"] == 1
    assert payload["updated"] == 0

    control_db = fake_postgres_schema_db_path(fake_runtime, "ait_native_control")
    conn = sqlite3.connect(control_db)
    row = conn.execute(
        """
        select repo_name, repo_id, pytest_node_id, function_name, description
        from test_case_inventory
        """
    ).fetchone()
    conn.close()
    assert row == (
        "ait",
        "AITR-001",
        "tests/test_sample.py::test_plain_case",
        "test_plain_case",
        "Auto-generated test description for tests/test_sample.py::test_plain_case: verifies Plain case.",
    )


def test_main_can_lookup_repo_id_from_content_schema(tmp_path: Path, monkeypatch) -> None:
    repo_root = tmp_path / "repo"
    _write_test_file(
        repo_root / "tests" / "test_sample.py",
        """
        def test_plain_case():
            assert True
        """,
    )
    fake_runtime = tmp_path / "fake-runtime"
    dsn = fake_postgres_dsn(fake_runtime)
    fake_psycopg = FakePsycopg()
    monkeypatch.setattr(inventory_script, "_load_psycopg", lambda: fake_psycopg)

    connection = fake_psycopg.connect(dsn)
    try:
        with connection.cursor() as cursor:
            cursor.execute('create schema if not exists "ait_native_content"')
            cursor.execute('set search_path to "ait_native_content", public')
            cursor.execute(
                """
                create table if not exists repositories (
                    repo_name text primary key,
                    repo_id text not null unique
                )
                """
            )
            cursor.execute(
                "insert into repositories(repo_name, repo_id) values (%s, %s)",
                ("ait", "AITR-LOOKUP"),
            )
        connection.commit()
    finally:
        connection.close()

    output_path = tmp_path / "inventory.json"
    exit_code = inventory_script.main(
        [
            "--dsn",
            dsn,
            "--repo-root",
            str(repo_root),
            "--repo-name",
            "ait",
            "--json",
            "--output",
            str(output_path),
        ]
    )

    assert exit_code == 0
    payload = json.loads(output_path.read_text(encoding="utf-8"))
    assert payload["repo_id"] == "AITR-LOOKUP"
    assert payload["duplicate_pytest_node_id_count"] == 0
    assert payload["inserted"] == 1
