#!/usr/bin/env python3
"""Validate and execute the live TG-1 required pytest suite from PostgreSQL."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
import time
import xml.etree.ElementTree as ET
from contextlib import closing
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from ait.repo_paths import RepoContext, configured_repo_discovery_start, resolve_bound_repo_root


DEFAULT_TEST_GROUP_ID = "TG-1"
DEFAULT_MINIMUM_COUNT = 24
DEFAULT_MEMBERSHIP_SQL_PATH = Path("sql") / "tg1_required_live_pytest_node_ids.sql"
DEFAULT_CONTRACT_DOC_PATH = Path("docs") / "sprints" / "tg1_sprint_planning_workflow_contract_group.md"
FORMAL_MEMBER_PATTERN = re.compile(r"^\d+\. `([^`]+)`", flags=re.MULTILINE)


def _now_utc() -> str:
    return datetime.now(timezone.utc).isoformat()


def _json_dump(payload: Any) -> str:
    return json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False)


def _truncate_text(value: str, *, limit: int = 4000) -> str:
    text = str(value or "")
    if len(text) <= limit:
        return text
    return text[: limit - 15] + "\n...<truncated>"


def _normalize_schema_name(value: str, *, field: str) -> str:
    text = str(value or "").strip()
    if not text:
        raise ValueError(f"{field} is required")
    if not text.replace("_", "a").isalnum() or not (text[0].isalpha() or text[0] == "_"):
        raise ValueError(f"{field} must be a valid PostgreSQL schema identifier")
    return text


def _optional_text(value: object) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _load_psycopg():
    import psycopg  # type: ignore

    return psycopg


def _discover_repo_name(repo_root: Path) -> str:
    config_path = repo_root / ".ait" / "config.json"
    payload = json.loads(config_path.read_text(encoding="utf-8"))
    repo_name = str(payload.get("repo_name") or "").strip()
    if not repo_name:
        raise RuntimeError(f"repo_name is missing from {config_path}")
    return repo_name


def _validated_tests_repo_root(candidate: Path, *, repo_name: str) -> Path | None:
    try:
        actual_repo_name = _discover_repo_name(candidate)
    except (FileNotFoundError, OSError, RuntimeError, json.JSONDecodeError):
        return None
    if actual_repo_name != repo_name:
        return None
    return candidate.resolve()


def _resolve_tests_repo_root(host_repo_root: Path, *, repo_name: str, tests_repo_root: Path | None) -> Path:
    explicit_root = tests_repo_root.expanduser().resolve() if tests_repo_root is not None else None
    if explicit_root is not None:
        candidate = explicit_root
        actual_repo_name = _discover_repo_name(candidate)
        if actual_repo_name != repo_name:
            raise RuntimeError(
                f"Resolved tests repo root {candidate} belongs to repo `{actual_repo_name}`, expected `{repo_name}`."
            )
        return candidate

    try:
        canonical_repo_root = RepoContext.discover(host_repo_root).repo_root
    except FileNotFoundError:
        canonical_repo_root = host_repo_root.resolve()
    configured_start = configured_repo_discovery_start()
    configured_repo_root: Path | None = None
    if configured_start is not None:
        try:
            configured_repo_root = RepoContext.discover(configured_start).repo_root
        except FileNotFoundError:
            configured_repo_root = configured_start.expanduser().resolve()

    candidate_roots: list[Path] = []
    seen_candidates: set[Path] = set()

    def _remember(candidate_root: Path) -> None:
        resolved_candidate = candidate_root.expanduser().resolve()
        if resolved_candidate in seen_candidates:
            return
        seen_candidates.add(resolved_candidate)
        candidate_roots.append(resolved_candidate)

    _remember(canonical_repo_root.parent / repo_name)
    if configured_repo_root is not None:
        _remember(configured_repo_root.parent / repo_name)
    _remember(
        resolve_bound_repo_root(
            repo_name,
            preferred_repo_root=canonical_repo_root,
            fallback_root=host_repo_root,
        )
    )
    if configured_repo_root is not None:
        _remember(
            resolve_bound_repo_root(
                repo_name,
                preferred_repo_root=configured_repo_root,
                fallback_root=configured_repo_root,
            )
        )

    for candidate in candidate_roots:
        matched = _validated_tests_repo_root(candidate, repo_name=repo_name)
        if matched is not None:
            return matched

    candidate = candidate_roots[-1]
    actual_repo_name = _discover_repo_name(candidate)
    if actual_repo_name != repo_name:
        raise RuntimeError(
            f"Resolved tests repo root {candidate} belongs to repo `{actual_repo_name}`, expected `{repo_name}`."
        )
    return candidate


def _resolve_membership_sql_path(repo_root: Path, membership_sql_path: Path) -> Path:
    if membership_sql_path.is_absolute():
        return membership_sql_path.resolve()
    try:
        canonical_repo_root = RepoContext.discover(repo_root).repo_root
    except FileNotFoundError:
        canonical_repo_root = repo_root.resolve()
    candidate_paths = [
        (canonical_repo_root / membership_sql_path).resolve(),
        (repo_root / membership_sql_path).resolve(),
    ]
    for candidate in candidate_paths:
        if candidate.is_file():
            return candidate
    return candidate_paths[0]


def _resolve_contract_doc_path(tests_repo_root: Path, contract_doc_path: Path | None) -> Path | None:
    relative_path = contract_doc_path or DEFAULT_CONTRACT_DOC_PATH
    if relative_path.is_absolute():
        resolved_path = relative_path.resolve()
        return resolved_path if resolved_path.is_file() else None
    try:
        canonical_repo_root = RepoContext.discover(tests_repo_root).repo_root
    except FileNotFoundError:
        canonical_repo_root = tests_repo_root.resolve()
    candidate_paths = [
        (canonical_repo_root / relative_path).resolve(),
        (tests_repo_root / relative_path).resolve(),
    ]
    for candidate in candidate_paths:
        if candidate.is_file():
            return candidate
    return None


def _set_session_read_only(cursor: Any) -> None:
    cursor.execute("set session characteristics as transaction read only")


def _resolve_repo_id(cursor: Any, *, repo_id: str | None, repo_name: str, content_schema: str) -> str:
    normalized_repo_id = str(repo_id or "").strip()
    if normalized_repo_id:
        return normalized_repo_id
    cursor.execute(
        f"""
        select repo_id
        from {content_schema}.repositories
        where repo_name = %s
        order by repo_id asc
        limit 2
        """,
        (repo_name,),
    )
    rows = list(cursor.fetchall())
    if not rows:
        raise RuntimeError(f"No repositories row found for repo `{repo_name}`.")
    if len(rows) > 1:
        raise RuntimeError(f"Multiple repo_id values found for repo `{repo_name}`; pass --repo-id explicitly.")
    return str(rows[0]["repo_id"])


def _test_groups_has_expected_case_count(cursor: Any, *, control_schema: str) -> bool:
    cursor.execute(
        """
        select 1
        from information_schema.columns
        where table_schema = %s
          and table_name = 'test_groups'
          and column_name = 'expected_case_count'
        limit 1
        """,
        (control_schema,),
    )
    return cursor.fetchone() is not None


def _fetch_group_metadata(cursor: Any, *, repo_id: str, control_schema: str, test_group_id: str) -> dict[str, Any]:
    if _test_groups_has_expected_case_count(cursor, control_schema=control_schema):
        cursor.execute(
            f"""
            select test_group_id, group_name, description, expected_case_count
            from {control_schema}.test_groups
            where repo_id = %s and test_group_id = %s
            """,
            (repo_id, test_group_id),
        )
    else:
        cursor.execute(
            f"""
            select test_group_id, group_name, description
            from {control_schema}.test_groups
            where repo_id = %s and test_group_id = %s
            """,
            (repo_id, test_group_id),
        )
    row = cursor.fetchone()
    if not row:
        raise RuntimeError(f"Test group `{test_group_id}` does not exist for repo `{repo_id}`.")
    expected_case_count = row.get("expected_case_count") if hasattr(row, "get") else None
    return {
        "test_group_id": str(row["test_group_id"]),
        "group_name": str(row["group_name"]),
        "description": str(row["description"]),
        "expected_case_count": int(expected_case_count) if expected_case_count is not None else None,
    }


def _load_sql_template(path: Path) -> str:
    template = path.read_text(encoding="utf-8").strip()
    if not template:
        raise RuntimeError(f"SQL template at {path} is empty.")
    return template


def _fetch_live_pytest_node_ids(
    cursor: Any,
    *,
    repo_id: str,
    control_schema: str,
    test_group_id: str,
    membership_sql_path: Path,
) -> list[str]:
    sql = _load_sql_template(membership_sql_path).format(control_schema=control_schema)
    cursor.execute(sql, (repo_id, test_group_id))
    rows = list(cursor.fetchall())
    return [str(row["pytest_node_id"]).strip() for row in rows if str(row["pytest_node_id"]).strip()]


def _load_contract_member_count(contract_doc_path: Path) -> int:
    members = FORMAL_MEMBER_PATTERN.findall(contract_doc_path.read_text(encoding="utf-8"))
    if not members:
        raise RuntimeError(f"Contract doc at {contract_doc_path} does not declare any formal TG-1 members.")
    if len(set(members)) != len(members):
        raise RuntimeError(f"Contract doc at {contract_doc_path} declares duplicate TG-1 members.")
    return len(members)


def _resolve_minimum_count(
    *,
    explicit_minimum_count: int | None,
    metadata_expected_count: int | None,
    contract_member_count: int | None,
) -> tuple[int, str]:
    if explicit_minimum_count is not None:
        return max(int(explicit_minimum_count), DEFAULT_MINIMUM_COUNT), "cli"
    if metadata_expected_count is not None:
        return max(int(metadata_expected_count), DEFAULT_MINIMUM_COUNT), "test_groups.expected_case_count"
    if contract_member_count is not None:
        return max(int(contract_member_count), DEFAULT_MINIMUM_COUNT), "contract_doc"
    return DEFAULT_MINIMUM_COUNT, "legacy_default"


def _validate_live_membership(*, live_nodes: list[str], minimum_count: int) -> dict[str, Any]:
    floor_count = max(int(minimum_count), DEFAULT_MINIMUM_COUNT)
    failures: list[str] = []
    if len(live_nodes) < floor_count:
        failures.append(f"Live TG-1 membership has {len(live_nodes)} case(s); expected at least {floor_count}.")
    return {
        "status": "pass" if not failures else "fail",
        "floor_count": floor_count,
        "failures": failures,
    }


def _write_tests_namespace_shim(repo_root: Path, shim_root: Path) -> Path | None:
    shim_tests_dir = shim_root / "tests"
    created = False

    def _write_module(relative_path: str, source_module: str, *, source_path: Path) -> None:
        nonlocal created
        if not source_path.is_file():
            return
        target = shim_tests_dir / relative_path
        target.parent.mkdir(parents=True, exist_ok=True)
        if not (shim_tests_dir / "__init__.py").exists():
            shim_tests_dir.mkdir(parents=True, exist_ok=True)
            (shim_tests_dir / "__init__.py").write_text("", encoding="utf-8")
        if target.parent != shim_tests_dir:
            parent = target.parent
            while parent != shim_root and parent != shim_tests_dir:
                init_path = parent / "__init__.py"
                if not init_path.exists():
                    init_path.write_text("", encoding="utf-8")
                parent = parent.parent
        target.write_text(f"from {source_module} import *\n", encoding="utf-8")
        created = True

    _write_module("postgres_fake.py", "postgres_fake", source_path=repo_root / "postgres_fake.py")
    _write_module("postgres_live.py", "postgres_live", source_path=repo_root / "postgres_live.py")
    _write_module("ait_web/helpers.py", "ait_web.helpers", source_path=repo_root / "ait_web" / "helpers.py")

    return shim_root if created else None


def _build_pytest_env(
    *,
    source_repo_root: Path,
    tests_repo_root: Path,
    extra_python_paths: list[Path] | None = None,
) -> dict[str, str]:
    env = os.environ.copy()
    python_paths = [str(source_repo_root / "src"), str(tests_repo_root / "src")]
    for candidate in extra_python_paths or []:
        python_paths.insert(0, str(candidate))
    existing = str(env.get("PYTHONPATH") or "").strip()
    if existing:
        python_paths.append(existing)
    env["PYTHONPATH"] = os.pathsep.join(python_paths)
    return env


def _run_pytest(
    *,
    source_repo_root: Path,
    tests_repo_root: Path,
    pytest_node_ids: list[str],
    junit_xml: Path | None = None,
    pytest_workers: int | None = None,
    pytest_dist: str | None = None,
) -> dict[str, Any]:
    command = [sys.executable, "-m", "pytest", "-q"]
    resolved_workers = int(pytest_workers) if pytest_workers is not None else None
    if resolved_workers is not None:
        if resolved_workers < 1:
            raise ValueError("pytest_workers must be at least 1 when provided.")
        if resolved_workers > 1:
            command.extend(["-n", str(resolved_workers)])
            if pytest_dist:
                command.extend(["--dist", str(pytest_dist)])
    command.extend(pytest_node_ids)
    if junit_xml is not None:
        junit_xml.parent.mkdir(parents=True, exist_ok=True)
        command.extend(["--junitxml", str(junit_xml)])
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="tg1-pytest-shim-") as shim_dir_text:
        shim_dir = Path(shim_dir_text)
        shim_root = _write_tests_namespace_shim(tests_repo_root, shim_dir)
        completed = subprocess.run(
            command,
            cwd=tests_repo_root,
            env=_build_pytest_env(
                source_repo_root=source_repo_root,
                tests_repo_root=tests_repo_root,
                extra_python_paths=[shim_root] if shim_root is not None else None,
            ),
            text=True,
            capture_output=True,
        )
    return {
        "command": " ".join(command),
        "exit_code": int(completed.returncode),
        "status": "pass" if completed.returncode == 0 else "fail",
        "duration_seconds": round(time.monotonic() - started, 3),
        "stdout": _truncate_text(completed.stdout),
        "stderr": _truncate_text(completed.stderr),
    }


def run_tg1_required_cases(
    *,
    dsn: str,
    repo_root: Path,
    content_schema: str,
    control_schema: str,
    tests_repo_root: Path | None = None,
    membership_sql_path: Path = DEFAULT_MEMBERSHIP_SQL_PATH,
    contract_doc_path: Path | None = None,
    repo_id: str | None = None,
    repo_name: str | None = None,
    test_group_id: str = DEFAULT_TEST_GROUP_ID,
    minimum_count: int | None = None,
    junit_xml: Path | None = None,
    pytest_workers: int | None = None,
    pytest_dist: str | None = None,
) -> dict[str, Any]:
    psycopg = _load_psycopg()
    normalized_content_schema = _normalize_schema_name(content_schema, field="content_schema")
    normalized_control_schema = _normalize_schema_name(control_schema, field="control_schema")
    resolved_repo_name = str(repo_name or "").strip() or _discover_repo_name(repo_root)
    resolved_tests_repo_root = _resolve_tests_repo_root(
        repo_root,
        repo_name=resolved_repo_name,
        tests_repo_root=tests_repo_root,
    )
    resolved_membership_sql_path = _resolve_membership_sql_path(repo_root, membership_sql_path)
    resolved_contract_doc_path = _resolve_contract_doc_path(resolved_tests_repo_root, contract_doc_path)

    with closing(psycopg.connect(dsn, row_factory=psycopg.rows.dict_row)) as connection:
        with connection.cursor() as cursor:
            _set_session_read_only(cursor)
            resolved_repo_id = _resolve_repo_id(
                cursor,
                repo_id=repo_id,
                repo_name=resolved_repo_name,
                content_schema=normalized_content_schema,
            )
            metadata = _fetch_group_metadata(
                cursor,
                repo_id=resolved_repo_id,
                control_schema=normalized_control_schema,
                test_group_id=test_group_id,
            )
            live_nodes = _fetch_live_pytest_node_ids(
                cursor,
                repo_id=resolved_repo_id,
                control_schema=normalized_control_schema,
                test_group_id=test_group_id,
                membership_sql_path=resolved_membership_sql_path,
            )

    contract_member_count = (
        _load_contract_member_count(resolved_contract_doc_path) if resolved_contract_doc_path is not None else None
    )
    resolved_minimum_count, minimum_count_source = _resolve_minimum_count(
        explicit_minimum_count=minimum_count,
        metadata_expected_count=metadata["expected_case_count"],
        contract_member_count=contract_member_count,
    )
    validation = _validate_live_membership(live_nodes=live_nodes, minimum_count=resolved_minimum_count)
    payload: dict[str, Any] = {
        "generated_at": _now_utc(),
        "repo_name": resolved_repo_name,
        "repo_id": resolved_repo_id,
        "content_schema": normalized_content_schema,
        "control_schema": normalized_control_schema,
        "tests_repo_root": str(resolved_tests_repo_root),
        "membership_sql_path": str(resolved_membership_sql_path),
        "test_group_id": test_group_id,
        "group_name": metadata["group_name"],
        "group_description": metadata["description"],
        "contract_doc_path": str(resolved_contract_doc_path) if resolved_contract_doc_path is not None else None,
        "contract_member_count": contract_member_count,
        "minimum_count": validation["floor_count"],
        "minimum_count_source": minimum_count_source,
        "live_pytest_node_ids": live_nodes,
        "live_count": len(live_nodes),
        "pytest_workers": int(pytest_workers) if pytest_workers is not None else None,
        "pytest_dist": str(pytest_dist).strip() if pytest_dist else None,
        "validation_failures": validation["failures"],
        "validation_status": validation["status"],
        "status": validation["status"],
        "pytest": None,
    }
    if validation["status"] != "pass":
        return payload

    pytest_result = _run_pytest(
        source_repo_root=repo_root,
        tests_repo_root=resolved_tests_repo_root,
        pytest_node_ids=live_nodes,
        junit_xml=junit_xml,
        pytest_workers=pytest_workers,
        pytest_dist=pytest_dist,
    )
    payload["pytest"] = pytest_result
    payload["status"] = pytest_result["status"]
    return payload


def _build_markdown(payload: dict[str, Any]) -> str:
    lines = [
        f"# TG-1 Required CI Gate ({payload['repo_name']})",
        "",
        f"- repo_id: `{payload['repo_id']}`",
        f"- test_group_id: `{payload['test_group_id']}`",
        f"- tests_repo_root: `{payload['tests_repo_root']}`",
        f"- membership_sql_path: `{payload['membership_sql_path']}`",
        f"- contract_doc_path: `{payload['contract_doc_path']}`" if payload.get("contract_doc_path") else "- contract_doc_path: none",
        (
            f"- contract_member_count: {payload['contract_member_count']}"
            if payload.get("contract_member_count") is not None
            else "- contract_member_count: none"
        ),
        f"- live_count: {payload['live_count']}",
        f"- minimum_count: {payload['minimum_count']}",
        f"- minimum_count_source: `{payload['minimum_count_source']}`",
        (
            f"- pytest_workers: {payload['pytest_workers']}"
            if payload.get("pytest_workers") is not None
            else "- pytest_workers: default"
        ),
        (f"- pytest_dist: `{payload['pytest_dist']}`" if payload.get("pytest_dist") else "- pytest_dist: default"),
        f"- validation_status: `{payload['validation_status']}`",
        f"- status: `{payload['status']}`",
        "",
        "## Live TG-1 nodes",
        "",
    ]
    live_nodes = list(payload.get("live_pytest_node_ids") or [])
    if not live_nodes:
        lines.append("- none")
    else:
        lines.extend(f"- `{node}`" for node in live_nodes)
    pytest_result = payload.get("pytest") or {}
    if pytest_result:
        lines.extend(
            [
                "",
                "## Pytest",
                "",
                f"- status: `{pytest_result['status']}`",
                f"- exit_code: {pytest_result['exit_code']}",
                f"- duration_seconds: {pytest_result['duration_seconds']}",
            ]
        )
    return "\n".join(lines).rstrip() + "\n"


def _write_failure_junit(path: Path, *, message: str, details: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    suite = ET.Element("testsuite", name="tg1_required", tests="1", failures="1", errors="0", skipped="0")
    case = ET.SubElement(suite, "testcase", classname="ci.tg1_required", name="tg1_contract")
    failure = ET.SubElement(case, "failure", message=message)
    failure.text = details
    tree = ET.ElementTree(suite)
    tree.write(path, encoding="utf-8", xml_declaration=True)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dsn", default=os.environ.get("AIT_NATIVE_SERVER_POSTGRES_DSN"), help="PostgreSQL DSN.")
    parser.add_argument("--repo-root", default=".", help="Repository root.")
    parser.add_argument(
        "--content-schema",
        default=os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", "ait_native_content"),
        help="PostgreSQL content schema.",
    )
    parser.add_argument(
        "--control-schema",
        default=os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", "ait_native_control"),
        help="PostgreSQL control schema.",
    )
    parser.add_argument("--tests-repo-root", help="Optional repo root where the TG-1 pytest nodes should be executed.")
    parser.add_argument(
        "--membership-sql",
        default=str(DEFAULT_MEMBERSHIP_SQL_PATH),
        help="SQL template used to resolve the live TG-1 pytest membership.",
    )
    parser.add_argument(
        "--contract-doc",
        help=(
            "Optional TG-1 contract doc path used to derive the expected live-member floor when --minimum-count is omitted. "
            "Defaults to docs/sprints/tg1_sprint_planning_workflow_contract_group.md in the tests repo."
        ),
    )
    parser.add_argument("--repo-id", help="Explicit repo_id.")
    parser.add_argument("--repo-name", help="Explicit repo_name.")
    parser.add_argument("--test-group-id", default=DEFAULT_TEST_GROUP_ID, help="Test group id to validate.")
    parser.add_argument(
        "--minimum-count",
        type=int,
        default=None,
        help=(
            "Optional live TG-1 size override. When omitted, the gate prefers test_groups.expected_case_count, "
            "then the sibling contract doc, then the legacy 24-case floor."
        ),
    )
    parser.add_argument(
        "--pytest-workers",
        type=int,
        default=None,
        help="Optional pytest-xdist worker count for the live TG-1 execution. Omit to keep serial pytest.",
    )
    parser.add_argument(
        "--pytest-dist",
        default=None,
        help="Optional pytest --dist mode paired with --pytest-workers when worker count exceeds 1.",
    )
    parser.add_argument("--output", help="Optional JSON output path.")
    parser.add_argument("--markdown", help="Optional Markdown summary path.")
    parser.add_argument("--junit-xml", help="Optional JUnit XML output path.")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    dsn = str(args.dsn or "").strip()
    if not dsn:
        parser.error("Pass --dsn or set AIT_NATIVE_SERVER_POSTGRES_DSN.")
    repo_root = Path(args.repo_root).expanduser().resolve()
    output_path = Path(args.output).expanduser() if args.output else None
    markdown_path = Path(args.markdown).expanduser() if args.markdown else None
    junit_path = Path(args.junit_xml).expanduser() if args.junit_xml else None
    try:
        payload = run_tg1_required_cases(
            dsn=dsn,
            repo_root=repo_root,
            content_schema=str(args.content_schema),
            control_schema=str(args.control_schema),
            tests_repo_root=Path(args.tests_repo_root).expanduser().resolve() if args.tests_repo_root else None,
            membership_sql_path=Path(str(args.membership_sql)),
            contract_doc_path=Path(str(args.contract_doc)) if args.contract_doc else None,
            repo_id=_optional_text(args.repo_id),
            repo_name=_optional_text(args.repo_name),
            test_group_id=str(args.test_group_id or DEFAULT_TEST_GROUP_ID).strip() or DEFAULT_TEST_GROUP_ID,
            minimum_count=int(args.minimum_count) if args.minimum_count is not None else None,
            junit_xml=junit_path.resolve() if junit_path is not None else None,
            pytest_workers=int(args.pytest_workers) if args.pytest_workers is not None else None,
            pytest_dist=_optional_text(args.pytest_dist),
        )
        text = _json_dump(payload)
    except Exception as exc:
        payload = {
            "generated_at": _now_utc(),
            "status": "fail",
            "error": str(exc),
        }
        text = _json_dump(payload)
        if junit_path is not None and not junit_path.exists():
            _write_failure_junit(junit_path, message="TG-1 required gate failed", details=str(exc))
        if output_path is not None:
            output_path.parent.mkdir(parents=True, exist_ok=True)
            output_path.write_text(text + ("\n" if not text.endswith("\n") else ""), encoding="utf-8")
        if markdown_path is not None:
            markdown_path.parent.mkdir(parents=True, exist_ok=True)
            markdown_path.write_text(f"# TG-1 Required CI Gate\n\n- status: `fail`\n- error: {exc}\n", encoding="utf-8")
        else:
            print(text)
        return 1

    if output_path is not None:
        output_path.parent.mkdir(parents=True, exist_ok=True)
        output_path.write_text(text + ("\n" if not text.endswith("\n") else ""), encoding="utf-8")
    else:
        print(text)
    if markdown_path is not None:
        markdown_path.parent.mkdir(parents=True, exist_ok=True)
        markdown_path.write_text(_build_markdown(payload), encoding="utf-8")
    if junit_path is not None and payload["status"] != "pass" and not junit_path.exists():
        details = payload.get("error") or "\n".join(payload.get("validation_failures") or []) or "TG-1 validation failed."
        _write_failure_junit(junit_path, message="TG-1 required gate failed", details=str(details))
    return 0 if payload["status"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
