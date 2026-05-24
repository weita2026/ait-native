#!/usr/bin/env python3
"""Create timestamped runtime backup sets and prune old copies."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


DEFAULT_KEEP_COUNT = 8
DEFAULT_CONTENT_SCHEMA = "ait_native_content"
DEFAULT_CONTROL_SCHEMA = "ait_native_control"
BACKUP_PREFIX = "ait-backup-"


def _utc_now(now: datetime | None = None) -> datetime:
    if now is None:
        return datetime.now(timezone.utc)
    if now.tzinfo is None:
        return now.replace(tzinfo=timezone.utc)
    return now.astimezone(timezone.utc)


def _timestamp_slug(now: datetime | None = None) -> str:
    return _utc_now(now).strftime("%Y%m%dT%H%M%SZ")


def _clean_label(label: str | None) -> str | None:
    text = str(label or "").strip()
    if not text:
        return None
    normalized = "".join(ch.lower() if ch.isalnum() else "-" for ch in text)
    collapsed = "-".join(part for part in normalized.split("-") if part)
    return collapsed or None


def _backup_id(*, now: datetime | None = None, label: str | None = None) -> str:
    slug = _timestamp_slug(now)
    cleaned = _clean_label(label)
    return f"{BACKUP_PREFIX}{slug}" if cleaned is None else f"{BACKUP_PREFIX}{slug}-{cleaned}"


def _is_relative_to(path: Path, parent: Path) -> bool:
    try:
        path.relative_to(parent)
        return True
    except ValueError:
        return False


def resolve_backend(backend: str | None = None, *, postgres_dsn: str | None = None) -> str:
    selected = str(backend or os.environ.get("AIT_NATIVE_SERVER_DB_BACKEND") or "auto").strip().lower()
    if selected in {"postgres", "postgresql"}:
        return "postgres"
    if selected == "sqlite":
        return "sqlite"
    if selected != "auto":
        raise ValueError(f"Unsupported backend: {backend}")
    return "postgres" if str(postgres_dsn or os.environ.get("AIT_NATIVE_SERVER_POSTGRES_DSN") or "").strip() else "sqlite"


def _list_backup_dirs(output_dir: Path) -> list[Path]:
    if not output_dir.exists():
        return []
    rows = [
        path
        for path in output_dir.iterdir()
        if path.is_dir() and path.name.startswith(BACKUP_PREFIX) and not path.name.endswith(".partial")
    ]
    return sorted(rows, key=lambda path: path.name, reverse=True)


def prune_backup_sets(output_dir: Path, *, keep: int = DEFAULT_KEEP_COUNT) -> list[Path]:
    if keep < 1:
        raise ValueError("keep must be at least 1")
    pruned: list[Path] = []
    for path in _list_backup_dirs(output_dir)[keep:]:
        shutil.rmtree(path)
        pruned.append(path)
    return pruned


def _write_json(path: Path, payload: dict[str, Any]) -> None:
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def _dump_postgres_schema(
    *,
    dsn: str,
    schema: str,
    output_path: Path,
    pg_dump_bin: str = "pg_dump",
) -> None:
    command = [
        pg_dump_bin,
        f"--dbname={dsn}",
        "--schema",
        schema,
        "--file",
        str(output_path),
        "--format",
        "plain",
        "--no-owner",
        "--no-privileges",
    ]
    try:
        subprocess.run(command, check=True, capture_output=True, text=True)
    except FileNotFoundError as exc:
        raise RuntimeError(f"Could not find `{pg_dump_bin}` while dumping PostgreSQL schema `{schema}`.") from exc
    except subprocess.CalledProcessError as exc:
        detail = str(exc.stderr or exc.stdout or exc).strip() or "unknown pg_dump failure"
        raise RuntimeError(f"pg_dump failed for PostgreSQL schema `{schema}`: {detail}") from exc


def create_backup_set(
    runtime_root: Path,
    output_dir: Path,
    *,
    keep: int = DEFAULT_KEEP_COUNT,
    backend: str | None = None,
    postgres_dsn: str | None = None,
    content_schema: str = DEFAULT_CONTENT_SCHEMA,
    control_schema: str = DEFAULT_CONTROL_SCHEMA,
    pg_dump_bin: str = "pg_dump",
    now: datetime | None = None,
    label: str | None = None,
) -> dict[str, Any]:
    if keep < 1:
        raise ValueError("keep must be at least 1")

    runtime_root = runtime_root.expanduser().resolve()
    output_dir = output_dir.expanduser().resolve()
    if not runtime_root.exists() or not runtime_root.is_dir():
        raise FileNotFoundError(f"Runtime root does not exist or is not a directory: {runtime_root}")
    if _is_relative_to(output_dir, runtime_root):
        raise ValueError("Output directory must not live inside the runtime root.")

    output_dir.mkdir(parents=True, exist_ok=True)
    resolved_backend = resolve_backend(backend, postgres_dsn=postgres_dsn)
    backup_id = _backup_id(now=now, label=label)
    final_dir = output_dir / backup_id
    partial_dir = output_dir / f"{backup_id}.partial"
    if final_dir.exists():
        raise FileExistsError(f"Backup set already exists: {final_dir}")
    if partial_dir.exists():
        shutil.rmtree(partial_dir)

    created_at = _utc_now(now)
    postgres_dump_rows: list[dict[str, str]] = []
    try:
        partial_dir.mkdir(parents=True, exist_ok=False)
        runtime_copy_dir = partial_dir / "runtime-root"
        shutil.copytree(runtime_root, runtime_copy_dir, symlinks=True)

        if resolved_backend == "postgres":
            dsn = str(postgres_dsn or os.environ.get("AIT_NATIVE_SERVER_POSTGRES_DSN") or "").strip()
            if not dsn:
                raise ValueError("PostgreSQL backup requested but no DSN was provided.")
            postgres_dir = partial_dir / "postgres"
            postgres_dir.mkdir(parents=True, exist_ok=True)
            for schema in (content_schema, control_schema):
                dump_path = postgres_dir / f"{schema}.sql"
                _dump_postgres_schema(dsn=dsn, schema=schema, output_path=dump_path, pg_dump_bin=pg_dump_bin)
                postgres_dump_rows.append({"schema": schema, "path": dump_path.relative_to(partial_dir).as_posix()})

        manifest = {
            "backup_id": backup_id,
            "created_at": created_at.isoformat(),
            "backend": resolved_backend,
            "runtime_root": str(runtime_root),
            "runtime_copy_path": "runtime-root",
            "keep_count": keep,
            "postgres_dumps": postgres_dump_rows,
        }
        _write_json(partial_dir / "manifest.json", manifest)
        partial_dir.rename(final_dir)
    except Exception:
        if partial_dir.exists():
            shutil.rmtree(partial_dir, ignore_errors=True)
        raise

    pruned = prune_backup_sets(output_dir, keep=keep)

    return {
        "backup_id": backup_id,
        "backup_path": str(final_dir),
        "manifest_path": str(final_dir / "manifest.json"),
        "backend": resolved_backend,
        "runtime_root": str(runtime_root),
        "keep_count": keep,
        "postgres_dumps": [
            {"schema": row["schema"], "path": str(final_dir / Path(row["path"]))}
            for row in postgres_dump_rows
        ],
        "pruned_backup_paths": [str(path) for path in pruned],
    }


def _default_runtime_root() -> str | None:
    value = str(os.environ.get("AIT_NATIVE_SERVER_DATA") or "").strip()
    return value or None


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--runtime-root",
        default=_default_runtime_root(),
        help="Server runtime root to capture. Defaults to AIT_NATIVE_SERVER_DATA when set.",
    )
    parser.add_argument("--output-dir", required=True, help="Directory where timestamped backup sets are stored.")
    parser.add_argument("--keep", type=int, default=DEFAULT_KEEP_COUNT, help=f"How many backup sets to keep. Default: {DEFAULT_KEEP_COUNT}.")
    parser.add_argument("--backend", default="auto", help="sqlite, postgres, or auto. Default: auto.")
    parser.add_argument("--postgres-dsn", default=os.environ.get("AIT_NATIVE_SERVER_POSTGRES_DSN"), help="PostgreSQL DSN override.")
    parser.add_argument(
        "--content-schema",
        default=os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", DEFAULT_CONTENT_SCHEMA),
        help=f"PostgreSQL content schema name. Default: {DEFAULT_CONTENT_SCHEMA}.",
    )
    parser.add_argument(
        "--control-schema",
        default=os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", DEFAULT_CONTROL_SCHEMA),
        help=f"PostgreSQL control schema name. Default: {DEFAULT_CONTROL_SCHEMA}.",
    )
    parser.add_argument("--pg-dump-bin", default="pg_dump", help="pg_dump binary path. Default: pg_dump.")
    parser.add_argument("--label", help="Optional backup label appended to the backup set name.")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    runtime_root = str(args.runtime_root or "").strip()
    if not runtime_root:
        parser.error("Pass --runtime-root or set AIT_NATIVE_SERVER_DATA.")
    try:
        payload = create_backup_set(
            Path(runtime_root),
            Path(args.output_dir),
            keep=int(args.keep),
            backend=args.backend,
            postgres_dsn=args.postgres_dsn,
            content_schema=args.content_schema,
            control_schema=args.control_schema,
            pg_dump_bin=args.pg_dump_bin,
            label=args.label,
        )
    except Exception as exc:
        print(json.dumps({"ok": False, "error": str(exc)}, indent=2, sort_keys=True), file=sys.stderr)
        return 1

    print(json.dumps({"ok": True, **payload}, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
