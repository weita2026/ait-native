#!/usr/bin/env python3
"""Discover repository test cases and upsert them into PostgreSQL."""

from __future__ import annotations

import argparse
import ast
import hashlib
import importlib
import inspect
import json
import os
import re
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from contextlib import closing
from typing import Any

from ait.repo_paths import RepoContext, configured_repo_name

_SCHEMA_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
TEST_CASE_TABLE = "test_case_inventory"


@dataclass(frozen=True)
class DiscoveredTestCase:
    pytest_node_id: str
    test_file_path: str
    class_name: str | None
    function_name: str
    description: str
    source_line: int


def _now_utc() -> datetime:
    return datetime.now(timezone.utc)


def _json_dump(payload: Any) -> str:
    return json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False)


def _load_psycopg():
    try:
        return importlib.import_module("psycopg")
    except ModuleNotFoundError as exc:
        raise RuntimeError(
            "Test case inventory requires psycopg. Install with: pip install 'ait-native[postgres]'"
        ) from exc


def _normalize_nonempty_text(value: str | None, *, field: str) -> str:
    text = str(value or "").strip()
    if not text:
        raise ValueError(f"{field} is required")
    return text


def _normalize_schema_name(value: str | None, *, field: str) -> str:
    text = _normalize_nonempty_text(value, field=field)
    if not _SCHEMA_RE.match(text):
        raise ValueError(f"{field} must be a valid PostgreSQL schema identifier")
    return text


def _infer_repo_name(repo_root: Path) -> str:
    try:
        ctx = RepoContext.discover(repo_root)
    except FileNotFoundError:
        return repo_root.resolve().name
    return configured_repo_name(ctx) or ctx.root.resolve().name


def _humanize_test_name(function_name: str) -> str:
    text = function_name[5:] if function_name.startswith("test_") else function_name
    text = " ".join(segment for segment in text.replace("_", " ").split() if segment)
    if not text:
        return function_name
    return text[0].upper() + text[1:]


def _normalize_description(
    docstring: str | None,
    *,
    pytest_node_id: str,
    function_name: str,
    class_name: str | None,
) -> str:
    cleaned = inspect.cleandoc(docstring or "").strip()
    if cleaned:
        return " ".join(line.strip() for line in cleaned.splitlines() if line.strip())
    humanized = _humanize_test_name(function_name)
    if class_name:
        return f"Auto-generated test description for {pytest_node_id}: verifies {humanized} in {class_name}."
    return f"Auto-generated test description for {pytest_node_id}: verifies {humanized}."


class _TestCollector(ast.NodeVisitor):
    def __init__(self, *, relative_path: Path):
        self.relative_path = relative_path.as_posix()
        self.class_stack: list[str] = []
        self.test_cases: list[DiscoveredTestCase] = []

    def visit_ClassDef(self, node: ast.ClassDef) -> Any:
        self.class_stack.append(node.name)
        self.generic_visit(node)
        self.class_stack.pop()
        return None

    def visit_FunctionDef(self, node: ast.FunctionDef) -> Any:
        self._maybe_record(node)
        if self.class_stack:
            return None
        self.generic_visit(node)
        return None

    def visit_AsyncFunctionDef(self, node: ast.AsyncFunctionDef) -> Any:
        self._maybe_record(node)
        if self.class_stack:
            return None
        self.generic_visit(node)
        return None

    def _maybe_record(self, node: ast.FunctionDef | ast.AsyncFunctionDef) -> None:
        if not node.name.startswith("test_"):
            return
        if self.class_stack and not self.class_stack[0].startswith("Test"):
            return
        class_name = ".".join(self.class_stack) if self.class_stack else None
        pytest_node_id = f"{self.relative_path}::{node.name}"
        if class_name:
            pytest_node_id = f"{self.relative_path}::{class_name}::{node.name}"
        self.test_cases.append(
            DiscoveredTestCase(
                pytest_node_id=pytest_node_id,
                test_file_path=self.relative_path,
                class_name=class_name,
                function_name=node.name,
                description=_normalize_description(
                    ast.get_docstring(node, clean=True),
                    pytest_node_id=pytest_node_id,
                    function_name=node.name,
                    class_name=class_name,
                ),
                source_line=int(getattr(node, "lineno", 0) or 0),
            )
        )


def discover_test_cases(*, repo_root: Path, tests_root: Path) -> list[DiscoveredTestCase]:
    resolved_repo_root = repo_root.resolve()
    resolved_tests_root = tests_root.resolve()
    test_cases: list[DiscoveredTestCase] = []
    for path in sorted(resolved_tests_root.rglob("*.py")):
        relative_path = path.resolve().relative_to(resolved_repo_root)
        module = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
        collector = _TestCollector(relative_path=relative_path)
        collector.visit(module)
        test_cases.extend(collector.test_cases)
    return sorted(test_cases, key=lambda row: (row.test_file_path, row.class_name or "", row.function_name))


def deduplicate_test_cases(test_cases: list[DiscoveredTestCase]) -> tuple[list[DiscoveredTestCase], list[str]]:
    by_node_id: dict[str, DiscoveredTestCase] = {}
    duplicate_node_ids: list[str] = []
    for case in test_cases:
        if case.pytest_node_id in by_node_id and case.pytest_node_id not in duplicate_node_ids:
            duplicate_node_ids.append(case.pytest_node_id)
        by_node_id[case.pytest_node_id] = case
    unique_cases = sorted(
        by_node_id.values(),
        key=lambda row: (row.test_file_path, row.class_name or "", row.function_name, row.source_line),
    )
    return unique_cases, sorted(duplicate_node_ids)


def build_test_case_id(*, repo_id: str, pytest_node_id: str) -> str:
    digest = hashlib.blake2b(
        f"{repo_id}:{pytest_node_id}".encode("utf-8"),
        digest_size=10,
        person=b"ait-test-case",
    ).hexdigest()
    return f"TC-{digest.upper()}"


def build_inventory_rows(
    test_cases: list[DiscoveredTestCase],
    *,
    repo_name: str,
    repo_id: str,
    captured_at: datetime | None = None,
) -> list[dict[str, Any]]:
    normalized_repo_name = _normalize_nonempty_text(repo_name, field="repo_name")
    normalized_repo_id = _normalize_nonempty_text(repo_id, field="repo_id")
    now = captured_at or _now_utc()
    rows: list[dict[str, Any]] = []
    for case in test_cases:
        rows.append(
            {
                "test_case_id": build_test_case_id(repo_id=normalized_repo_id, pytest_node_id=case.pytest_node_id),
                "repo_name": normalized_repo_name,
                "repo_id": normalized_repo_id,
                "pytest_node_id": case.pytest_node_id,
                "test_file_path": case.test_file_path,
                "class_name": case.class_name,
                "function_name": case.function_name,
                "description": case.description,
                "source_line": case.source_line,
                "discovered_at": now.isoformat(),
                "updated_at": now.isoformat(),
            }
        )
    return rows


def _set_search_path(cursor: Any, schema: str) -> None:
    cursor.execute(f'create schema if not exists "{schema}"')
    cursor.execute(f'set search_path to "{schema}", public')


def _ensure_inventory_table(cursor: Any, *, control_schema: str) -> None:
    _set_search_path(cursor, control_schema)
    cursor.execute(
        f"""
        create table if not exists {TEST_CASE_TABLE} (
            repo_name text not null,
            repo_id text not null,
            test_case_id text not null,
            pytest_node_id text not null,
            test_file_path text not null,
            class_name text,
            function_name text not null,
            description text not null,
            source_line integer not null,
            discovered_at timestamptz not null,
            updated_at timestamptz not null,
            primary key (repo_id, test_case_id)
        )
        """
    )
    cursor.execute(
        f"""
        create unique index if not exists uq_test_case_inventory_repo_id_test_case_id
        on {TEST_CASE_TABLE}(repo_id, test_case_id)
        """
    )
    cursor.execute(
        f"""
        create unique index if not exists uq_test_case_inventory_repo_id_node
        on {TEST_CASE_TABLE}(repo_id, pytest_node_id)
        """
    )
    cursor.execute(
        f"""
        create index if not exists idx_test_case_inventory_repo_id_path
        on {TEST_CASE_TABLE}(repo_id, test_file_path, source_line)
        """
    )
    cursor.execute(
        f"""
        create index if not exists idx_test_case_inventory_repo_name_path
        on {TEST_CASE_TABLE}(repo_name, test_file_path, source_line)
        """
    )


def _lookup_repo_id(cursor: Any, *, repo_name: str, content_schema: str) -> str | None:
    _set_search_path(cursor, content_schema)
    cursor.execute(
        "select repo_id from repositories where repo_name = %s",
        (repo_name,),
    )
    row = cursor.fetchone()
    if not row:
        return None
    if isinstance(row, dict):
        value = row.get("repo_id")
    else:
        value = row[0]
    text = str(value or "").strip()
    return text or None


def resolve_repo_scope(
    cursor: Any,
    *,
    repo_root: Path,
    repo_name: str | None,
    repo_id: str | None,
    content_schema: str,
) -> tuple[str, str]:
    normalized_repo_name = str(repo_name or "").strip() or _infer_repo_name(repo_root)
    normalized_repo_id = str(repo_id or "").strip()
    if not normalized_repo_id:
        resolved = _lookup_repo_id(cursor, repo_name=normalized_repo_name, content_schema=content_schema)
        if not resolved:
            raise RuntimeError(
                f"Could not resolve repo_id for repo_name {normalized_repo_name!r}. "
                "Pass --repo-id or initialize the content-plane repositories table first."
            )
        normalized_repo_id = resolved
    return normalized_repo_name, normalized_repo_id


def _existing_pytest_node_ids(cursor: Any, *, repo_id: str, control_schema: str) -> set[str]:
    _set_search_path(cursor, control_schema)
    cursor.execute(
        f"select pytest_node_id from {TEST_CASE_TABLE} where repo_id = %s",
        (repo_id,),
    )
    rows = cursor.fetchall()
    existing: set[str] = set()
    for row in rows:
        if isinstance(row, dict):
            value = row.get("pytest_node_id")
        else:
            value = row[0]
        text = str(value or "").strip()
        if text:
            existing.add(text)
    return existing


def upsert_inventory_rows(
    cursor: Any,
    rows: list[dict[str, Any]],
    *,
    control_schema: str,
) -> dict[str, int]:
    if not rows:
        return {"inserted": 0, "updated": 0}
    repo_id = str(rows[0]["repo_id"])
    existing = _existing_pytest_node_ids(cursor, repo_id=repo_id, control_schema=control_schema)
    inserted = 0
    updated = 0
    _set_search_path(cursor, control_schema)
    for row in rows:
        if row["pytest_node_id"] in existing:
            updated += 1
        else:
            inserted += 1
        cursor.execute(
            f"""
            insert into {TEST_CASE_TABLE} (
                test_case_id,
                repo_name,
                repo_id,
                pytest_node_id,
                test_file_path,
                class_name,
                function_name,
                description,
                source_line,
                discovered_at,
                updated_at
            )
            values (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s)
            on conflict (repo_id, pytest_node_id) do update set
                test_case_id = excluded.test_case_id,
                repo_name = excluded.repo_name,
                test_file_path = excluded.test_file_path,
                class_name = excluded.class_name,
                function_name = excluded.function_name,
                description = excluded.description,
                source_line = excluded.source_line,
                updated_at = excluded.updated_at
            """,
            (
                row["test_case_id"],
                row["repo_name"],
                row["repo_id"],
                row["pytest_node_id"],
                row["test_file_path"],
                row["class_name"],
                row["function_name"],
                row["description"],
                row["source_line"],
                row["discovered_at"],
                row["updated_at"],
            ),
        )
    return {"inserted": inserted, "updated": updated}


def sync_test_case_inventory(
    *,
    dsn: str,
    repo_root: Path,
    tests_root: Path,
    repo_name: str | None,
    repo_id: str | None,
    content_schema: str,
    control_schema: str,
) -> dict[str, Any]:
    psycopg = _load_psycopg()
    normalized_content_schema = _normalize_schema_name(content_schema, field="content_schema")
    normalized_control_schema = _normalize_schema_name(control_schema, field="control_schema")
    discovered = discover_test_cases(repo_root=repo_root, tests_root=tests_root)
    unique_cases, duplicate_node_ids = deduplicate_test_cases(discovered)
    with closing(psycopg.connect(dsn)) as connection:
        with connection.cursor() as cursor:
            resolved_repo_name, resolved_repo_id = resolve_repo_scope(
                cursor,
                repo_root=repo_root,
                repo_name=repo_name,
                repo_id=repo_id,
                content_schema=normalized_content_schema,
            )
            _ensure_inventory_table(cursor, control_schema=normalized_control_schema)
            rows = build_inventory_rows(
                unique_cases,
                repo_name=resolved_repo_name,
                repo_id=resolved_repo_id,
            )
            counts = upsert_inventory_rows(cursor, rows, control_schema=normalized_control_schema)
        connection.commit()
    return {
        "ok": True,
        "repo_name": resolved_repo_name,
        "repo_id": resolved_repo_id,
        "table": f"{normalized_control_schema}.{TEST_CASE_TABLE}",
        "test_case_count": len(unique_cases),
        "raw_discovered_test_case_count": len(discovered),
        "duplicate_pytest_node_id_count": len(duplicate_node_ids),
        "duplicate_pytest_node_ids": duplicate_node_ids,
        "inserted": counts["inserted"],
        "updated": counts["updated"],
        "generated_at": _now_utc().isoformat(),
    }


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--dsn",
        default=os.environ.get("AIT_NATIVE_SERVER_POSTGRES_DSN"),
        help="PostgreSQL DSN. Defaults to AIT_NATIVE_SERVER_POSTGRES_DSN when set.",
    )
    parser.add_argument(
        "--repo-root",
        default=".",
        help="Repository root to scan. Defaults to the current directory.",
    )
    parser.add_argument(
        "--tests-root",
        default="tests",
        help="Tests directory relative to repo-root. Defaults to tests.",
    )
    parser.add_argument("--repo-name", help="Optional repository name override.")
    parser.add_argument("--repo-id", help="Optional repository id override. Recommended when the repositories table is not available.")
    parser.add_argument(
        "--content-schema",
        default=os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", "ait_native_content"),
        help="Content schema used to resolve repo_id when --repo-id is omitted.",
    )
    parser.add_argument(
        "--control-schema",
        default=os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", "ait_native_control"),
        help="Control schema that stores test_case_inventory.",
    )
    parser.add_argument("--json", action="store_true", help="Emit JSON instead of a text summary.")
    parser.add_argument("--output", help="Optional output path. Defaults to stdout.")
    return parser


def format_summary(payload: dict[str, Any]) -> str:
    return "\n".join(
        [
            f"Repository: {payload['repo_name']} ({payload['repo_id']})",
            f"Target table: {payload['table']}",
            f"Discovered test cases: {payload['test_case_count']}",
            f"Inserted: {payload['inserted']}",
            f"Updated: {payload['updated']}",
            f"Generated at: {payload['generated_at']}",
        ]
    )


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    dsn = str(args.dsn or "").strip()
    if not dsn:
        parser.error("Pass --dsn or set AIT_NATIVE_SERVER_POSTGRES_DSN.")
    repo_root = Path(args.repo_root).expanduser().resolve()
    tests_root = (repo_root / str(args.tests_root)).resolve()
    if not tests_root.exists():
        parser.error(f"Tests root does not exist: {tests_root}")
    try:
        payload = sync_test_case_inventory(
            dsn=dsn,
            repo_root=repo_root,
            tests_root=tests_root,
            repo_name=args.repo_name,
            repo_id=args.repo_id,
            content_schema=args.content_schema,
            control_schema=args.control_schema,
        )
    except Exception as exc:
        print(_json_dump({"ok": False, "error": str(exc)}), file=sys.stderr)
        return 1
    text = _json_dump(payload) if args.json else format_summary(payload)
    if args.output:
        output_path = Path(args.output).expanduser()
        output_path.parent.mkdir(parents=True, exist_ok=True)
        output_path.write_text(text + ("" if text.endswith("\n") else "\n"), encoding="utf-8")
    else:
        print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
