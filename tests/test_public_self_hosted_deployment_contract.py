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


def test_public_self_hosted_deployment_contract_routes_bootstrap_through_operator_wrapper() -> None:
    payload = json.loads(Path("docs/public_self_hosted_deployment_contract.json").read_text(encoding="utf-8"))

    assert payload["bootstrap_steps"][0]["command"] == "cd ../ait_docker && cp .env.example .env"
    assert payload["bootstrap_steps"][1]["command"] == "cd ../ait_docker && ./ait-docker.sh stack config"
    assert payload["bootstrap_steps"][2]["commands"] == [
        "cd ../ait_docker && ./ait-docker.sh stack up postgres",
        "cd ../ait_docker && ./ait-docker.sh stack up ait-server",
        "cd ../ait_docker && ./ait-docker.sh stack up ait-worker",
    ]
    assert payload["readiness_commands"] == [
        "curl -fsS https://<ait-server-host>/healthz",
        "cd ../ait_docker && ./ait-docker.sh stack exec ait-server ait doctor postgres --connect --json",
        "cd ../ait_docker && ./ait-docker.sh stack exec ait-server ait repo readiness --json",
        "cd ../ait_docker && ./ait-docker.sh stack exec ait-server ait repo jobs --diagnostics --json",
    ]
    assert payload["backup_and_recovery"]["daily_helper"].startswith(
        'python3 scripts/runtime_backup.py --runtime-root "$AIT_NATIVE_SERVER_DATA"'
    )
    assert payload["backup_and_recovery"]["related_docs"] == [
        "docs/server_backup_restore_dr.md",
        "docs/server_disaster_recovery_checklist.md",
    ]
    assert payload["decision_boundary"][0]["guide"] == "docs/LOCAL_QUICKSTART.md"
