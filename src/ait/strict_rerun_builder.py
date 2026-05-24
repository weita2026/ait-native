from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
from pathlib import Path
from typing import Any, Optional

from ait_protocol.common import utc_now

from .local_content_workspace import IGNORED_DIRS, IGNORED_FILES, workspace_path_is_ignored
from .store import init_repo

STRICT_RERUN_PROTOCOL_PATH = "docs/benchmarks/task_dag_token_savings_fixture_protocol.md"
DEFAULT_STRICT_RERUN_SEED_MANIFEST = Path("docs/benchmarks/task_dag_token_savings_measured_20260421.json")
DEFAULT_STRICT_RERUN_BASELINE_MODE = "git_linear"
DEFAULT_STRICT_RERUN_CANDIDATE_MODES = ("ait_linear", "ait_dag")
DEFAULT_STRICT_RERUN_AGGREGATE_CANDIDATE_MODE = "ait_dag"
DEFAULT_STRICT_RERUN_BOOTSTRAP_PROFILE = "steady_state_task_cost"
DEFAULT_GIT_USER_NAME = "Benchmark Fixture"
DEFAULT_GIT_USER_EMAIL = "benchmark@example.invalid"
SUPPORTED_STRICT_RERUN_MODES = frozenset(
    {
        "git_linear",
        "ait_linear",
        "ait_dag",
        "ait_dag_local_first_final_land_packet",
        "ait_dag_local_first_final_land_e2e",
    }
)


def load_strict_rerun_seed_manifest(seed_manifest_path: Path) -> dict[str, Any]:
    seed_manifest_path = Path(seed_manifest_path)
    try:
        manifest = json.loads(seed_manifest_path.read_text(encoding="utf-8"))
    except FileNotFoundError as exc:
        raise ValueError(f"Strict rerun seed manifest not found: {seed_manifest_path}") from exc
    except json.JSONDecodeError as exc:
        raise ValueError(f"Strict rerun seed manifest is not valid JSON: {seed_manifest_path}: {exc}") from exc
    if not isinstance(manifest, dict):
        raise ValueError("Strict rerun seed manifest must be a JSON object.")
    return manifest


def strict_rerun_workloads_from_manifest(seed_manifest: dict[str, Any]) -> list[dict[str, str]]:
    workloads: list[dict[str, str]] = []
    for row in seed_manifest.get("workloads") or []:
        if not isinstance(row, dict):
            continue
        workload_id = str(row.get("workload_id") or row.get("id") or "").strip()
        if not workload_id:
            continue
        workloads.append(
            {
                "workload_id": workload_id,
                "title": str(row.get("title") or workload_id).strip(),
                "category": str(row.get("category") or "long").strip() or "long",
                "acceptance": str(row.get("acceptance") or "").strip(),
            }
        )
    if not workloads:
        raise ValueError("Strict rerun seed manifest does not define any workloads.")
    return workloads


def build_strict_rerun_fixture_bundle(
    *,
    benchmark_id: str,
    output_dir: Path,
    source_root: Path,
    workloads: list[dict[str, str]],
    source_snapshot_id: str | None = None,
    seed_manifest_path: Path | None = None,
    description: str | None = None,
    baseline_mode: str = DEFAULT_STRICT_RERUN_BASELINE_MODE,
    candidate_modes: list[str] | None = None,
    aggregate_candidate_mode: str | None = None,
    minimum_comparable_long_workloads: int | None = None,
    bootstrap_profile: str = DEFAULT_STRICT_RERUN_BOOTSTRAP_PROFILE,
    ait_policy_profile: str = "prototype",
    default_line: str = "main",
    default_author_mode: str = "ai_with_human_review",
    default_model: str | None = None,
    git_user_name: str = DEFAULT_GIT_USER_NAME,
    git_user_email: str = DEFAULT_GIT_USER_EMAIL,
    force: bool = False,
) -> dict[str, Any]:
    benchmark_id = str(benchmark_id).strip()
    if not benchmark_id:
        raise ValueError("benchmark_id is required.")

    source_root = Path(source_root).resolve()
    if not source_root.is_dir():
        raise ValueError(f"Source root does not exist: {source_root}")

    output_dir = Path(output_dir).resolve()
    if output_dir == source_root:
        raise ValueError("output_dir must not be the same directory as source_root.")
    if output_dir.exists():
        if not force and any(output_dir.iterdir()):
            raise ValueError(f"Output directory is not empty: {output_dir}; pass force=True to replace it.")
        shutil.rmtree(output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    normalized_workloads = _normalize_workloads(workloads)
    normalized_candidate_modes = _normalize_candidate_modes(candidate_modes)
    normalized_aggregate_candidate_mode = _normalize_aggregate_candidate_mode(
        aggregate_candidate_mode,
        normalized_candidate_modes,
    )
    long_workload_count = sum(1 for row in normalized_workloads if row["category"] == "long")
    minimum_long = minimum_comparable_long_workloads if minimum_comparable_long_workloads is not None else long_workload_count
    if minimum_long <= 0:
        raise ValueError("minimum_comparable_long_workloads must be at least 1.")

    extra_skip_prefixes = _extra_skip_prefixes(source_root, output_dir)
    source_tree = _scan_source_tree(source_root, extra_skip_prefixes=extra_skip_prefixes)
    if not source_tree["entries"]:
        raise ValueError(f"Source root has no copyable files after ignores: {source_root}")

    prepared_at = utc_now()
    description_text = (
        str(description).strip()
        if description is not None
        else f"Strict fresh-fixture rerun scaffold for {benchmark_id} built from {source_root.name}."
    )

    fixtures_dir = output_dir / "fixtures"
    evidence_dir = output_dir / "evidence"
    fixtures_dir.mkdir(parents=True, exist_ok=True)
    evidence_dir.mkdir(parents=True, exist_ok=True)

    fixture_rows: list[dict[str, Any]] = []
    manifest_workloads: list[dict[str, Any]] = []
    for workload in normalized_workloads:
        workload_id = workload["workload_id"]
        title = workload["title"]
        category = workload["category"]
        acceptance = workload["acceptance"]
        workload_fixture_modes: dict[str, Any] = {}
        runs: list[dict[str, Any]] = []
        for mode in [baseline_mode, *normalized_candidate_modes]:
            fixture_root = fixtures_dir / workload_id / mode
            _materialize_source_tree(source_root, fixture_root, source_tree["entries"])
            fixture_payload = _prepare_fixture_root(
                fixture_root,
                mode=mode,
                benchmark_id=benchmark_id,
                workload_id=workload_id,
                git_user_name=git_user_name,
                git_user_email=git_user_email,
                ait_policy_profile=ait_policy_profile,
                default_line=default_line,
                default_author_mode=default_author_mode,
                default_model=default_model,
            )
            evidence_file = evidence_dir / f"{workload_id}__{mode}.json"
            evidence_file.write_text(json.dumps(fixture_payload, indent=2, sort_keys=False) + "\n", encoding="utf-8")
            fixture_row = {
                **fixture_payload,
                "workload_id": workload_id,
                "title": title,
                "category": category,
                "acceptance": acceptance,
                "fixture_root": str(fixture_root),
                "fixture_root_relative": fixture_root.relative_to(output_dir).as_posix(),
                "fixture_evidence_file": str(evidence_file),
                "fixture_evidence_file_relative": evidence_file.relative_to(output_dir).as_posix(),
                "source_tree_digest": source_tree["tree_digest"],
                "source_file_count": source_tree["file_count"],
                "source_total_bytes": source_tree["total_bytes"],
            }
            fixture_rows.append(fixture_row)
            workload_fixture_modes[mode] = {
                "fixture_root": fixture_row["fixture_root_relative"],
                "fixture_evidence_file": fixture_row["fixture_evidence_file_relative"],
                "initial_commit_id": fixture_payload["git"]["initial_commit_id"],
                "prepared_head_commit_id": fixture_payload["git"]["prepared_head_commit_id"],
                "clean_status_before_measured_run": fixture_payload["git"]["clean_status_before_measured_run"],
            }
            run_id = f"{workload_id}-{mode.replace('_', '-')}-strict-01"
            notes = _run_notes_for_mode(mode)
            runs.append(
                {
                    "run_id": run_id,
                    "mode": mode,
                    "usage_kind": "measured",
                    "quality": "pending",
                    "completion_status": "pending",
                    "equivalent_completion": None,
                    "usage": {
                        "prompt_tokens": None,
                        "completion_tokens": None,
                        "total_tokens": None,
                        "cached_input_tokens": None,
                        "reasoning_output_tokens": None,
                    },
                    "fixture_root": fixture_row["fixture_root_relative"],
                    "fixture_evidence_file": fixture_row["fixture_evidence_file_relative"],
                    "notes": notes,
                }
            )

        manifest_workloads.append(
            {
                "workload_id": workload_id,
                "title": title,
                "category": category,
                "acceptance": acceptance,
                "strict_fixture": {
                    "source_tree_digest": source_tree["tree_digest"],
                    "source_file_count": source_tree["file_count"],
                    "source_total_bytes": source_tree["total_bytes"],
                    "modes": workload_fixture_modes,
                },
                "runs": runs,
            }
        )

    manifest = {
        "benchmark_id": benchmark_id,
        "description": description_text,
        "baseline_mode": baseline_mode,
        "candidate_modes": normalized_candidate_modes,
        "aggregate_candidate_mode": normalized_aggregate_candidate_mode,
        "minimum_comparable_long_workloads": minimum_long,
        "require_equivalent_completion": True,
        "strict_rerun_protocol": {
            "protocol_file": STRICT_RERUN_PROTOCOL_PATH,
            "prepared_at": prepared_at,
            "source_root": str(source_root),
            "source_snapshot_id": source_snapshot_id,
            "source_tree_digest": source_tree["tree_digest"],
            "source_file_count": source_tree["file_count"],
            "source_total_bytes": source_tree["total_bytes"],
            "bootstrap_profile": bootstrap_profile,
            "ait_policy_profile": ait_policy_profile,
            "seed_manifest_path": str(seed_manifest_path) if seed_manifest_path is not None else None,
            "fixture_bundle_dir": "fixtures",
            "evidence_dir": "evidence",
        },
        "workloads": manifest_workloads,
    }

    fixture_bundle = {
        "benchmark_id": benchmark_id,
        "prepared_at": prepared_at,
        "protocol_file": STRICT_RERUN_PROTOCOL_PATH,
        "source_root": str(source_root),
        "source_snapshot_id": source_snapshot_id,
        "source_tree_digest": source_tree["tree_digest"],
        "source_file_count": source_tree["file_count"],
        "source_total_bytes": source_tree["total_bytes"],
        "baseline_mode": baseline_mode,
        "candidate_modes": normalized_candidate_modes,
        "aggregate_candidate_mode": normalized_aggregate_candidate_mode,
        "bootstrap_profile": bootstrap_profile,
        "ait_policy_profile": ait_policy_profile,
        "seed_manifest_path": str(seed_manifest_path) if seed_manifest_path is not None else None,
        "fixtures": fixture_rows,
    }

    manifest_path = output_dir / f"{benchmark_id}.json"
    fixture_bundle_path = output_dir / f"{benchmark_id}_fixtures.json"
    readme_path = output_dir / "README.md"
    manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=False) + "\n", encoding="utf-8")
    fixture_bundle_path.write_text(json.dumps(fixture_bundle, indent=2, sort_keys=False) + "\n", encoding="utf-8")
    readme_path.write_text(
        render_strict_rerun_bundle_markdown(
            {
                "benchmark_id": benchmark_id,
                "prepared_at": prepared_at,
                "source_root": str(source_root),
                "source_snapshot_id": source_snapshot_id,
                "source_tree_digest": source_tree["tree_digest"],
                "bootstrap_profile": bootstrap_profile,
                "ait_policy_profile": ait_policy_profile,
                "manifest_path": manifest_path.name,
                "fixture_bundle_path": fixture_bundle_path.name,
                "seed_manifest_path": str(seed_manifest_path) if seed_manifest_path is not None else None,
                "workloads": manifest_workloads,
            }
        ),
        encoding="utf-8",
    )

    return {
        "benchmark_id": benchmark_id,
        "prepared_at": prepared_at,
        "source_root": str(source_root),
        "source_snapshot_id": source_snapshot_id,
        "source_tree_digest": source_tree["tree_digest"],
        "source_file_count": source_tree["file_count"],
        "source_total_bytes": source_tree["total_bytes"],
        "baseline_mode": baseline_mode,
        "candidate_modes": normalized_candidate_modes,
        "aggregate_candidate_mode": normalized_aggregate_candidate_mode,
        "minimum_comparable_long_workloads": minimum_long,
        "bootstrap_profile": bootstrap_profile,
        "ait_policy_profile": ait_policy_profile,
        "seed_manifest_path": str(seed_manifest_path) if seed_manifest_path is not None else None,
        "output_dir": str(output_dir),
        "manifest_path": str(manifest_path),
        "fixture_bundle_path": str(fixture_bundle_path),
        "readme_path": str(readme_path),
        "fixture_count": len(fixture_rows),
        "workload_count": len(manifest_workloads),
        "fixtures": fixture_rows,
    }


def render_strict_rerun_bundle_markdown(payload: dict[str, Any]) -> str:
    lines = [
        f"# {payload.get('benchmark_id') or 'Strict rerun bundle'}",
        "",
        "Generated strict fresh-fixture benchmark scaffold.",
        "",
        f"- Prepared at: `{payload.get('prepared_at') or ''}`",
        f"- Source root: `{payload.get('source_root') or ''}`",
        f"- Source snapshot id: `{payload.get('source_snapshot_id') or 'not recorded'}`",
        f"- Source tree digest: `{payload.get('source_tree_digest') or ''}`",
        f"- Bootstrap profile: `{payload.get('bootstrap_profile') or ''}`",
        f"- `ait` policy profile: `{payload.get('ait_policy_profile') or ''}`",
        f"- Seed manifest: `{payload.get('seed_manifest_path') or 'none'}`",
        f"- Benchmark manifest: `{payload.get('manifest_path') or ''}`",
        f"- Fixture bundle: `{payload.get('fixture_bundle_path') or ''}`",
        "",
        "## Workloads",
        "",
        "| Workload | Category | Fixture roots |",
        "| --- | --- | --- |",
    ]
    for workload in payload.get("workloads") or []:
        if not isinstance(workload, dict):
            continue
        strict_fixture = workload.get("strict_fixture") if isinstance(workload.get("strict_fixture"), dict) else {}
        modes = strict_fixture.get("modes") if isinstance(strict_fixture.get("modes"), dict) else {}
        fixture_text = ", ".join(
            f"`{mode}` → `{details.get('fixture_root')}`"
            for mode, details in modes.items()
            if isinstance(details, dict) and details.get("fixture_root")
        )
        lines.append(
            f"| `{workload.get('workload_id') or ''}` | `{workload.get('category') or ''}` | {fixture_text or 'n/a'} |"
        )
    lines.extend(
        [
            "",
            "## Next steps",
            "",
            "1. Start one fresh measured AI session from each fixture root.",
            "2. Capture provider-measured session JSONL paths for every `run_id` in the manifest.",
            "3. Fill usage with `ait benchmark codex-fill-usage --manifest <manifest> --run-session RUN_ID=SESSION_JSONL --output-manifest <filled-manifest>`.",
            "4. Set `completion_status` / `equivalent_completion` for each run before treating it as comparable evidence.",
            "5. Inspect readiness with `ait benchmark token-savings-status --manifest <filled-manifest>`.",
            "6. Generate the report with `ait benchmark token-savings --manifest <filled-manifest>`.",
            "",
            "## Strict evidence reminders",
            "",
            "- `git_linear` must show explicit Git discovery in the captured run log.",
            "- `ait_linear` and `ait_dag` must show explicit `ait` workflow usage from the prepared fixtures.",
            "- Keep bootstrap-inclusive and steady-state profiles separate.",
        ]
    )
    return "\n".join(lines) + "\n"


def _normalize_workloads(workloads: list[dict[str, str]]) -> list[dict[str, str]]:
    normalized: list[dict[str, str]] = []
    seen: set[str] = set()
    for row in workloads:
        if not isinstance(row, dict):
            raise ValueError("workloads must be dictionaries.")
        workload_id = str(row.get("workload_id") or row.get("id") or "").strip()
        if not workload_id:
            raise ValueError("Each workload requires workload_id.")
        if workload_id in seen:
            raise ValueError(f"Duplicate workload_id: {workload_id}")
        seen.add(workload_id)
        normalized.append(
            {
                "workload_id": workload_id,
                "title": str(row.get("title") or workload_id).strip(),
                "category": str(row.get("category") or "long").strip() or "long",
                "acceptance": str(row.get("acceptance") or "").strip(),
            }
        )
    if not normalized:
        raise ValueError("At least one workload is required.")
    return normalized


def _normalize_candidate_modes(candidate_modes: list[str] | None) -> list[str]:
    normalized = [str(mode).strip() for mode in (candidate_modes or list(DEFAULT_STRICT_RERUN_CANDIDATE_MODES)) if str(mode).strip()]
    if not normalized:
        raise ValueError("At least one candidate mode is required.")
    deduped: list[str] = []
    for mode in normalized:
        if mode not in SUPPORTED_STRICT_RERUN_MODES:
            raise ValueError(f"Unsupported strict rerun mode: {mode}")
        if mode == DEFAULT_STRICT_RERUN_BASELINE_MODE:
            raise ValueError("candidate_modes must not include the baseline mode git_linear.")
        if mode not in deduped:
            deduped.append(mode)
    return deduped


def _normalize_aggregate_candidate_mode(aggregate_candidate_mode: str | None, candidate_modes: list[str]) -> str:
    if aggregate_candidate_mode is None or not str(aggregate_candidate_mode).strip():
        if DEFAULT_STRICT_RERUN_AGGREGATE_CANDIDATE_MODE in candidate_modes:
            return DEFAULT_STRICT_RERUN_AGGREGATE_CANDIDATE_MODE
        return candidate_modes[0]
    normalized = str(aggregate_candidate_mode).strip()
    if normalized not in candidate_modes:
        raise ValueError("aggregate_candidate_mode must be one of the candidate modes.")
    return normalized


def _extra_skip_prefixes(source_root: Path, output_dir: Path) -> list[Path]:
    try:
        relative = output_dir.relative_to(source_root)
    except ValueError:
        return []
    return [relative]


def _scan_source_tree(source_root: Path, *, extra_skip_prefixes: list[Path]) -> dict[str, Any]:
    entries: list[dict[str, Any]] = []
    digest = hashlib.sha256()
    total_bytes = 0
    for path in sorted(source_root.rglob("*")):
        if path == source_root:
            continue
        rel_path = path.relative_to(source_root)
        if _should_skip_relpath(source_root, rel_path, extra_skip_prefixes=extra_skip_prefixes):
            continue
        if path.is_dir():
            continue
        if path.is_symlink():
            link_target = os.readlink(path)
            digest.update(rel_path.as_posix().encode("utf-8"))
            digest.update(b"\0symlink\0")
            digest.update(str(link_target).encode("utf-8"))
            entries.append({"rel_path": rel_path.as_posix(), "kind": "symlink", "target": str(link_target)})
            continue
        data = path.read_bytes()
        total_bytes += len(data)
        digest.update(rel_path.as_posix().encode("utf-8"))
        digest.update(b"\0file\0")
        digest.update(data)
        entries.append({"rel_path": rel_path.as_posix(), "kind": "file"})
    return {
        "entries": entries,
        "file_count": sum(1 for row in entries if row.get("kind") == "file"),
        "entry_count": len(entries),
        "total_bytes": total_bytes,
        "tree_digest": digest.hexdigest(),
    }


def _should_skip_relpath(source_root: Path, rel_path: Path, *, extra_skip_prefixes: list[Path]) -> bool:
    parts = rel_path.parts
    if not parts:
        return False
    if any(part in IGNORED_DIRS for part in parts[:-1]):
        return True
    if parts[-1] in IGNORED_FILES:
        return True
    for prefix in extra_skip_prefixes:
        prefix_parts = prefix.parts
        if prefix_parts and parts[: len(prefix_parts)] == prefix_parts:
            return True
    return workspace_path_is_ignored(source_root, rel_path)


def _materialize_source_tree(source_root: Path, fixture_root: Path, entries: list[dict[str, Any]]) -> None:
    if fixture_root.exists():
        shutil.rmtree(fixture_root)
    fixture_root.mkdir(parents=True, exist_ok=True)
    for row in entries:
        rel_path = Path(str(row.get("rel_path") or ""))
        if not rel_path.parts:
            continue
        destination = fixture_root / rel_path
        destination.parent.mkdir(parents=True, exist_ok=True)
        kind = row.get("kind")
        source_path = source_root / rel_path
        if kind == "symlink":
            os.symlink(str(row.get("target") or ""), destination)
            continue
        shutil.copy2(source_path, destination)


def _prepare_fixture_root(
    fixture_root: Path,
    *,
    mode: str,
    benchmark_id: str,
    workload_id: str,
    git_user_name: str,
    git_user_email: str,
    ait_policy_profile: str,
    default_line: str,
    default_author_mode: str,
    default_model: str | None,
) -> dict[str, Any]:
    if mode not in SUPPORTED_STRICT_RERUN_MODES:
        raise ValueError(f"Unsupported strict rerun mode: {mode}")
    bootstrap_commands: list[dict[str, Any]] = []
    _run_command(["git", "init"], cwd=fixture_root, command_log=bootstrap_commands, label="git_init")
    _run_command(["git", "config", "user.name", git_user_name], cwd=fixture_root, command_log=bootstrap_commands, label="git_config_user_name")
    _run_command(["git", "config", "user.email", git_user_email], cwd=fixture_root, command_log=bootstrap_commands, label="git_config_user_email")
    _run_command(["git", "add", "-A"], cwd=fixture_root, command_log=bootstrap_commands, label="git_add_baseline")
    _run_command(
        ["git", "commit", "-m", "benchmark fixture baseline"],
        cwd=fixture_root,
        command_log=bootstrap_commands,
        label="git_commit_baseline",
    )
    initial_commit_id = _run_command(
        ["git", "rev-parse", "HEAD"],
        cwd=fixture_root,
        command_log=bootstrap_commands,
        label="git_rev_parse_initial_head",
    )["stdout"].strip()
    git_top_level = _run_command(
        ["git", "rev-parse", "--show-toplevel"],
        cwd=fixture_root,
        command_log=bootstrap_commands,
        label="git_rev_parse_show_toplevel",
    )["stdout"].strip()

    ait_bootstrap: dict[str, Any] | None = None
    prepared_head_commit_id = initial_commit_id
    if mode != DEFAULT_STRICT_RERUN_BASELINE_MODE:
        repo_name = f"{benchmark_id}-{workload_id}-{mode}".replace("_", "-")
        ait_command = [
            "ait",
            "init",
            "--name",
            repo_name,
            "--default-line",
            default_line,
            "--policy-profile",
            ait_policy_profile,
            "--default-author-mode",
            default_author_mode,
        ]
        if default_model:
            ait_command.extend(["--default-model", default_model])
        init_repo(
            fixture_root,
            repo_name=repo_name,
            default_line=default_line,
            policy_profile_name=ait_policy_profile,
            default_author_mode=default_author_mode,
            default_model=default_model,
        )
        bootstrap_commands.append(
            {
                "label": "ait_init_equivalent",
                "command": ait_command,
                "stdout": "",
                "stderr": "",
                "returncode": 0,
            }
        )
        _run_command(["git", "add", "-A"], cwd=fixture_root, command_log=bootstrap_commands, label="git_add_ait_bootstrap")
        _run_command(
            ["git", "commit", "-m", "benchmark ait bootstrap"],
            cwd=fixture_root,
            command_log=bootstrap_commands,
            label="git_commit_ait_bootstrap",
        )
        prepared_head_commit_id = _run_command(
            ["git", "rev-parse", "HEAD"],
            cwd=fixture_root,
            command_log=bootstrap_commands,
            label="git_rev_parse_prepared_head",
        )["stdout"].strip()
        ait_bootstrap = {
            "command": ait_command,
            "policy_profile": ait_policy_profile,
            "default_line": default_line,
            "default_author_mode": default_author_mode,
            "default_model": default_model,
            "bootstrap_commit_id": prepared_head_commit_id,
        }
    else:
        if (fixture_root / ".ait").exists():
            raise ValueError(f"{mode} fixture must not include .ait metadata: {fixture_root}")

    clean_status = _run_command(
        ["git", "status", "--short", "--ignored"],
        cwd=fixture_root,
        command_log=bootstrap_commands,
        label="git_status_short_ignored",
    )["stdout"]
    if clean_status.strip():
        raise ValueError(f"Fixture root is not clean before measured run: {fixture_root}")

    payload = {
        "mode": mode,
        "protocol_file": STRICT_RERUN_PROTOCOL_PATH,
        "bootstrap_commands": bootstrap_commands,
        "git": {
            "top_level": git_top_level,
            "initial_commit_id": initial_commit_id,
            "prepared_head_commit_id": prepared_head_commit_id,
            "clean_status_before_measured_run": clean_status,
        },
    }
    if ait_bootstrap is not None:
        payload["ait_bootstrap"] = ait_bootstrap
    return payload


def _run_command(
    command: list[str],
    *,
    cwd: Path,
    command_log: list[dict[str, Any]],
    label: str,
) -> dict[str, Any]:
    try:
        completed = subprocess.run(command, cwd=cwd, check=False, capture_output=True, text=True)
    except FileNotFoundError as exc:
        raise ValueError(f"Command not found while preparing strict rerun fixture: {command[0]}") from exc
    payload = {
        "label": label,
        "command": command,
        "stdout": completed.stdout,
        "stderr": completed.stderr,
        "returncode": completed.returncode,
    }
    command_log.append(payload)
    if completed.returncode != 0:
        stderr = (completed.stderr or "").strip()
        stdout = (completed.stdout or "").strip()
        detail = stderr or stdout or f"exit {completed.returncode}"
        raise ValueError(f"Command failed for strict rerun fixture: {' '.join(command)} ({detail})")
    return payload


def _run_notes_for_mode(mode: str) -> str:
    if mode == "git_linear":
        return (
            "Strict fresh-fixture run. Start a fresh measured session from this fixture and ensure the captured log "
            "shows explicit Git discovery such as git status, git diff, git log, git show, or git rev-parse."
        )
    if mode == "ait_linear":
        return (
            "Strict fresh-fixture run. Start a fresh measured session from this fixture and ensure the captured log "
            "shows explicit ait init provenance plus linear ait workflow setup without DAG fan-out."
        )
    if mode == "ait_dag":
        return (
            "Strict fresh-fixture run. Start a fresh measured session from this fixture and ensure the captured log "
            "shows explicit ait workflow usage plus the scoped DAG packet provenance and topology label."
        )
    if mode == "ait_dag_local_first_final_land_packet":
        return (
            "Strict fresh-fixture run. Start one fresh worker-only compact packet session with "
            "--local-first-final-land and no auto-land, then require equivalent completion before counting it."
        )
    if mode == "ait_dag_local_first_final_land_e2e":
        return (
            "Strict fresh-fixture run. Start one fresh local-first-final-land session with auto-land enabled "
            "and require equivalent completion before counting it."
        )
    return "Strict fresh-fixture run."
