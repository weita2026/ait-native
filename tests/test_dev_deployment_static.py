from __future__ import annotations

import subprocess
from pathlib import Path

from ait.plan_graph import load_task_graph
from ait.repo_paths import RepoContext

WORKSPACE_ROOT = Path(__file__).resolve().parent.parent
AUTHORED_ROOT = RepoContext.discover(WORKSPACE_ROOT).repo_root
AUTHORED_DOCS = AUTHORED_ROOT / "docs"


def _read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def test_in_repo_operator_assets_are_removed_and_docs_redirect_to_sibling_operator_repo() -> None:
    unexpected = [
        WORKSPACE_ROOT / ".dockerignore",
        WORKSPACE_ROOT / "deploy" / "dev",
        WORKSPACE_ROOT / "deploy" / "site" / ".env.example",
        WORKSPACE_ROOT / "deploy" / "site" / "docker-compose.yml",
        WORKSPACE_ROOT / "deploy" / "site" / "Caddyfile",
    ]
    for path in unexpected:
        assert not path.exists(), path

    local_dev = _read(AUTHORED_ROOT / "LOCAL_DEVELOPMENT.md")
    self_hosted = _read(AUTHORED_ROOT / "SELF_HOSTED_TEAM_DEPLOYMENT.md")
    site_readme = _read(AUTHORED_ROOT / "deploy" / "site" / "README.md")

    assert "../ait_docker" in local_dev
    assert "../ait_docker" in self_hosted
    assert "../ait_docker" in site_readme
    assert "deploy/dev/README.md" not in local_dev
    assert "./ait.sh docker" not in self_hosted
    assert "docker compose" not in self_hosted
    assert "docker-compose.yml" not in site_readme


def test_ait_sh_no_longer_exposes_operator_lifecycle_stubs() -> None:
    usage = subprocess.run(
        ["bash", "ait.sh"],
        cwd=WORKSPACE_ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    combined_usage = usage.stdout + usage.stderr
    assert "./ait.sh docker" not in combined_usage

    removed = subprocess.run(
        ["bash", "ait.sh", "docker"],
        cwd=WORKSPACE_ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    assert removed.returncode != 0
    combined = removed.stdout + removed.stderr
    assert "../ait_docker" not in combined
    assert "./ait.sh docker" not in combined


def test_m4b_graph_artifact_is_valid_and_bound_to_schema_migration_plan():
    graph = load_task_graph(WORKSPACE_ROOT / "docs" / "sprints" / "m4b_postgres_schema_migrations.task_graph.json")
    assert graph["repo_name"] == "ait"
    assert graph["graph_id"] == "m4b-postgres-schema-migrations/version-checks"
    assert graph["source_plan"]["artifact_path"] == "docs/sprints/m4b_postgres_schema_migrations.md"
    assert [node["plan_item_ref"] for node in graph["nodes"]] == [
        "m4b-schema-migrations/contract",
        "m4b-schema-migrations/version-table",
        "m4b-schema-migrations/doctor-checks",
        "m4b-schema-migrations/validation-land",
    ]


def test_m4c_graph_artifact_is_valid_and_bound_to_async_queue_plan():
    graph = load_task_graph(WORKSPACE_ROOT / "docs" / "sprints" / "m4c_async_queue_jobs.task_graph.json")
    assert graph["repo_name"] == "ait"
    assert graph["graph_id"] == "m4c-async-queue-jobs/job-contract"
    assert graph["source_plan"]["artifact_path"] == "docs/sprints/m4c_async_queue_jobs.md"
    assert [node["plan_item_ref"] for node in graph["nodes"]] == [
        "m4c-async-jobs/contract",
        "m4c-async-jobs/job-registry",
        "m4c-async-jobs/worker-dispatch",
        "m4c-async-jobs/validation-land",
    ]


def test_m4d_graph_artifact_is_valid_and_bound_to_job_recovery_plan():
    graph = load_task_graph(WORKSPACE_ROOT / "docs" / "sprints" / "m4d_job_recovery_diagnostics.task_graph.json")
    assert graph["repo_name"] == "ait"
    assert graph["graph_id"] == "m4d-job-recovery-diagnostics/operator-visibility"
    assert graph["source_plan"]["artifact_path"] == "docs/sprints/m4d_job_recovery_diagnostics.md"
    assert [node["plan_item_ref"] for node in graph["nodes"]] == [
        "m4d-job-recovery/contract",
        "m4d-job-recovery/diagnostics",
        "m4d-job-recovery/operator-surface",
        "m4d-job-recovery/validation-land",
    ]


def test_m4e_graph_artifact_is_valid_and_bound_to_runtime_root_plan():
    graph = load_task_graph(WORKSPACE_ROOT / "docs" / "sprints" / "m4e_runtime_data_root_hygiene.task_graph.json")
    assert graph["repo_name"] == "ait"
    assert graph["graph_id"] == "m4e-runtime-data-root-hygiene/operator-safety"
    assert graph["source_plan"]["artifact_path"] == "docs/sprints/m4e_runtime_data_root_hygiene.md"
    assert [node["plan_item_ref"] for node in graph["nodes"]] == [
        "m4e-runtime-root/contract",
        "m4e-runtime-root/snapshot-status-guard",
        "m4e-runtime-root/doctor-tests",
        "m4e-runtime-root/validation-land",
    ]

def test_m4f_graph_artifact_is_valid_and_bound_to_backup_restore_plan():
    graph = load_task_graph(WORKSPACE_ROOT / "docs" / "sprints" / "m4f_backup_restore_dr.task_graph.json")
    assert graph["repo_name"] == "ait"
    assert graph["graph_id"] == "m4f-backup-restore-dr/operator-runbook"
    assert graph["source_plan"]["artifact_path"] == "docs/sprints/m4f_backup_restore_dr.md"
    assert [node["plan_item_ref"] for node in graph["nodes"]] == [
        "m4f-backup-restore/contract",
        "m4f-backup-restore/backup-inventory",
        "m4f-backup-restore/restore-dr",
        "m4f-backup-restore/validation-land",
    ]


def test_m4f_backup_restore_docs_cover_runtime_database_and_dr_guidance():
    runbook = _read(AUTHORED_DOCS / "server_backup_restore_dr.md")
    checklist = _read(AUTHORED_DOCS / "server_disaster_recovery_checklist.md")
    runtime_ops = _read(AUTHORED_DOCS / "ait_native_runtime_operations.md")

    for term in (
        "content.db",
        "control.db",
        "objects/",
        "refs/",
        "telegram-sync.json",
        "ait doctor runtime-root --json",
        "ait doctor postgres --connect --json",
        "scripts/runtime_backup.py",
        "--keep 8",
        "0 2 * * *",
        "Do not rely on local project snapshots as server backups",
    ):
        assert term in runbook

    for term in (
        "RPO",
        "RTO",
        "ait repo jobs --diagnostics --json",
        "ait repo storage --json",
        "Restore `telegram-sync.json`",
    ):
        assert term in checklist

    assert "server_backup_restore_dr.md" in runtime_ops
    assert "server_disaster_recovery_checklist.md" in runtime_ops
    assert "scripts/runtime_backup.py" in runtime_ops

def test_m4g_graph_artifact_is_valid_and_bound_to_server_metrics_plan():
    graph = load_task_graph(WORKSPACE_ROOT / "docs" / "sprints" / "m4g_server_operator_metrics.task_graph.json")
    assert graph["repo_name"] == "ait"
    assert graph["graph_id"] == "m4g-server-operator-metrics/rollup"
    assert graph["source_plan"]["artifact_path"] == "docs/sprints/m4g_server_operator_metrics.md"
    assert [node["plan_item_ref"] for node in graph["nodes"]] == [
        "m4g-operator-metrics/contract",
        "m4g-operator-metrics/read-api-cli",
        "m4g-operator-metrics/tests",
        "m4g-operator-metrics/validation-land",
    ]


def test_m4g_runtime_operations_document_server_operator_metrics_surface():
    runtime_ops = _read(AUTHORED_DOCS / "ait_native_runtime_operations.md")
    for term in (
        "ait repo metrics --json",
        "GET /v1/native/admin/metrics",
        "multi-repo storage totals",
        "worker status inferred from running job locks",
        "job outcome counts by state and type",
        "live_turn_pressure",
        "in_flight_turns",
        "queued_turns",
        "cache_state",
        "cache_age_seconds",
        "cache_ttl_seconds",
    ):
        assert term in runtime_ops


def test_m4h_graph_artifact_is_valid_and_bound_to_server_readiness_plan():
    graph = load_task_graph(WORKSPACE_ROOT / "docs" / "sprints" / "m4h_server_readiness_preflight.task_graph.json")
    assert graph["repo_name"] == "ait"
    assert graph["graph_id"] == "m4h-server-readiness-preflight/operator-preflight"
    assert graph["source_plan"]["artifact_path"] == "docs/sprints/m4h_server_readiness_preflight.md"
    assert [node["plan_item_ref"] for node in graph["nodes"]] == [
        "m4h-readiness/contract",
        "m4h-readiness/read-api-cli",
        "m4h-readiness/tests",
        "m4h-readiness/validation-land",
    ]


def test_m4h_runtime_operations_and_engineering_plan_document_readiness_surface():
    runtime_ops = _read(AUTHORED_DOCS / "ait_native_runtime_operations.md")
    engineering = _read(AUTHORED_DOCS / "engineering_plan.md")
    for term in (
        "ait repo readiness --json",
        "GET /v1/native/admin/readiness",
        "runtime backend details",
        "PostgreSQL schema readiness",
        "SQLite remains local to the standalone `ait` CLI metadata store only",
        "long Telegram/Codex turns are active",
    ):
        assert term in runtime_ops
    assert "M4H added read-only server readiness preflight" in engineering
    assert "[x] Keep SQLite as the default local CLI metadata path" in engineering


def test_server_postgres_cutover_graph_artifact_is_valid_and_parallelized():
    graph = load_task_graph(WORKSPACE_ROOT / "docs" / "sprints" / "server_postgres_cutover.task_graph.json")
    assert graph["repo_name"] == "ait"
    assert graph["graph_id"] == "server-postgres-cutover/parallel-dispatch"
    assert graph["source_plan"]["artifact_path"] == "docs/sprints/server_postgres_cutover.md"
    assert [node["plan_item_ref"] for node in graph["nodes"]] == [
        "server-postgres-cutover/governance-freeze",
        "server-postgres-cutover/inventory",
        "server-postgres-cutover/migration-tooling",
        "server-postgres-cutover/parity-validation",
        "server-postgres-cutover/cutover-runbook",
        "server-postgres-cutover/startup-enforcement",
        "server-postgres-cutover/retirement",
    ]
    assert [group["group_id"] for group in graph["parallel_groups"]] == [
        "foundation-lanes",
        "cutover-prep-lanes",
    ]


def test_server_postgres_cutover_docs_state_governance_freeze_and_dag():
    cutover = _read(AUTHORED_DOCS / "sprints" / "server_postgres_cutover.md")
    engineering = _read(AUTHORED_DOCS / "engineering_plan.md")
    assert "DAG graph JSON: [server_postgres_cutover.task_graph.json]" in cutover
    assert "- [x] Freeze the governance change" in cutover
    assert "- [x] Add first-party SQLite→PostgreSQL export/import tooling" in cutover
    assert "- [x] Publish a maintenance-window cutover runbook" in cutover
    assert "Parallel lane shape" in cutover
    assert "governance freeze completed" in engineering
    assert "legacy server SQLite inventory, cutover, and parity commands were retired" in engineering


def test_server_postgres_cutover_runbook_documents_retired_sqlite_surfaces():
    runbook = _read(AUTHORED_DOCS / "server_postgres_cutover_runbook.md")
    for term in (
        "Status: retired operator runbook.",
        "ait-server runtime state is PostgreSQL-only",
        "Current restore validation uses",
        "do not recreate `content.db` or `control.db`",
    ):
        assert term in runbook


def test_ait_server_v2_runtime_operations_documents_fast_operator_health_surface():
    runtime_ops = _read(AUTHORED_DOCS / "ait_native_runtime_operations.md")
    for term in (
        "GET /healthz",
        "existing `GET /healthz` surface",
        "Use it for process-level health checks",
        "oldest_in_flight_turn_age_seconds",
        "oldest_queued_turn_age_seconds",
        "cache_age_seconds",
        "cache_ttl_seconds",
    ):
        assert term in runtime_ops
