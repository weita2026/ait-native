from __future__ import annotations

import json
from pathlib import Path


def test_public_contributor_workflow_contract_shape() -> None:
    payload = json.loads(Path("docs/public_contributor_workflow_contract.json").read_text(encoding="utf-8"))

    assert payload["schema_version"] == 1
    assert payload["guides"]["contributing"] == "docs/CONTRIBUTING.md"
    assert payload["guides"]["local_development"] == "docs/LOCAL_DEVELOPMENT.md"
    assert payload["workflow_contract"]["task_bootstrap_command"] == "ait task start"
    assert payload["mode_boundaries"]["solo_remote"]["mode"] == "solo_remote"
    assert ".[test]" in payload["development_expectations"]["editable_install_examples"][0]
