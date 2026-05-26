from __future__ import annotations

import json
from pathlib import Path


def test_public_self_hosted_deployment_contract_shape() -> None:
    payload = json.loads(Path("docs/public_self_hosted_deployment_contract.json").read_text(encoding="utf-8"))

    assert payload["schema_version"] == 1
    assert payload["guide_path"] == "docs/SELF_HOSTED_TEAM_DEPLOYMENT.md"
    assert payload["deployment_mode"] == "self_hosted_team"
    assert payload["topology"]["required_services"] == ["postgres", "ait-server", "ait-worker"]
    assert payload["boot_order"][0] == "postgres"
    assert payload["topology"]["optional_services"] == ["ait-agent"]
    assert any(command.endswith("ait repo readiness --json") for command in payload["readiness_commands"])


def test_public_self_hosted_deployment_contract_routes_bootstrap_through_direct_binary_operator_contract() -> None:
    payload = json.loads(Path("docs/public_self_hosted_deployment_contract.json").read_text(encoding="utf-8"))

    assert payload["bootstrap_steps"][:3] == [
        {
            "id": "runtime_root",
            "command": "mkdir -p /srv/ait/server-data /srv/ait/retire-exports && export AIT_NATIVE_SERVER_DATA=/srv/ait/server-data && export AIT_SERVER_RETIRE_EXPORT_ROOT=/srv/ait/retire-exports",
        },
        {
            "id": "runtime_backend",
            "command": "export AIT_NATIVE_SERVER_DB_BACKEND=postgres",
        },
        {
            "id": "postgres_dsn",
            "command": "export AIT_NATIVE_SERVER_POSTGRES_DSN='postgresql://<user>:<password>@<host>:5432/ait_native'",
        },
    ]
    assert payload["bootstrap_steps"][3]["commands"][0] == 'pg_isready -d "$AIT_NATIVE_SERVER_POSTGRES_DSN"'
    assert 'AIT_SERVER_RETIRE_EXPORT_ROOT="$AIT_SERVER_RETIRE_EXPORT_ROOT"' in payload["bootstrap_steps"][3]["commands"][1]
    assert payload["bootstrap_steps"][3]["commands"][1].endswith("ait-server")
    assert 'AIT_SERVER_RETIRE_EXPORT_ROOT="$AIT_SERVER_RETIRE_EXPORT_ROOT"' in payload["bootstrap_steps"][3]["commands"][2]
    assert payload["bootstrap_steps"][3]["commands"][2].endswith("ait-worker run --worker-id worker-1")
    assert payload["readiness_commands"] == [
        "curl -fsS http://<ait-server-host>:<port>/healthz",
        'ait doctor runtime-root --server-data "$AIT_NATIVE_SERVER_DATA" --json',
        'test -d "$AIT_SERVER_RETIRE_EXPORT_ROOT" && test -w "$AIT_SERVER_RETIRE_EXPORT_ROOT"',
        'ait doctor postgres --server-data "$AIT_NATIVE_SERVER_DATA" --dsn "$AIT_NATIVE_SERVER_POSTGRES_DSN" --content-schema "${AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA:-ait_native_content}" --control-schema "${AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA:-ait_native_control}" --connect --json',
        "ait repo readiness --json",
        "ait repo jobs --diagnostics --json",
    ]
    assert "retire export root" in payload["backup_and_recovery"]["must_backup"]
    assert payload["backup_and_recovery"]["daily_helper"].startswith(
        'python3 scripts/runtime_backup.py --runtime-root "$AIT_NATIVE_SERVER_DATA"'
    )
    assert payload["backup_and_recovery"]["related_docs"] == [
        "docs/server_backup_restore_dr.md",
        "docs/server_disaster_recovery_checklist.md",
    ]
    assert payload["decision_boundary"][0]["guide"] == "docs/LOCAL_QUICKSTART.md"
