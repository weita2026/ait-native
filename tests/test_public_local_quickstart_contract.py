from __future__ import annotations

import json
import tomllib
from pathlib import Path


def test_public_local_quickstart_contract_shape() -> None:
    payload = json.loads(Path("docs/public_local_quickstart_contract.json").read_text(encoding="utf-8"))

    assert payload["schema_version"] == 1
    assert payload["guide_path"] == "docs/LOCAL_QUICKSTART.md"
    assert payload["mode"] == "local_only"
    assert payload["workflow_steps"][1]["command"] == "ait plan sync <file-or-dir>"
    assert payload["workflow_steps"][2]["command"].startswith("ait task start --local")
    assert payload["workflow_steps"][4]["command"] == "ait workflow land-local <change-id>"
    assert any(item["issue"] == "forgot_local_flag" for item in payload["troubleshooting"])


def test_public_local_quickstart_test_install_matches_repo_parallel_pytest_default() -> None:
    payload = json.loads(Path("docs/public_local_quickstart_contract.json").read_text(encoding="utf-8"))
    pyproject = tomllib.loads(Path("pyproject.toml").read_text(encoding="utf-8"))

    assert "python3 -m pip install -e .[test]" in payload["install_commands"]
    test_deps = pyproject["project"]["optional-dependencies"]["test"]
    assert "pytest-xdist>=3,<4" in test_deps
    addopts = pyproject["tool"]["pytest"]["ini_options"]["addopts"]
    assert "-n 3" in addopts
    assert "--dist loadfile" in addopts
