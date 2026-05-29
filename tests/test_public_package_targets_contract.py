from __future__ import annotations

import json
import re
from pathlib import Path

WORKSPACE_ROOT = Path(__file__).resolve().parents[1]


def test_public_package_targets_contract_matches_current_distribution_anchor() -> None:
    payload = json.loads((WORKSPACE_ROOT / "docs/public_package_targets_contract.json").read_text(encoding="utf-8"))
    package_init = (WORKSPACE_ROOT / "src/ait_native/__init__.py").read_text(encoding="utf-8")
    version_match = re.search(r'^__version__\s*=\s*"([^"]+)"', package_init, re.MULTILINE)

    assert version_match is not None
    assert payload["schema_version"] == 1
    assert payload["guide_path"] == "docs/PACKAGE_TARGETS.md"
    assert payload["distribution"]["name"] == "ait-native"
    assert payload["distribution"]["version"] == version_match.group(1)
    assert payload["distribution"]["artifact_model"] == "single_distribution_multi_command"
    assert payload["target_rules"]["separate_public_wheels_promised"] is False


def test_public_package_targets_contract_aligns_with_release_surface_map() -> None:
    payload = json.loads((WORKSPACE_ROOT / "docs/public_package_targets_contract.json").read_text(encoding="utf-8"))
    surface_map = json.loads((WORKSPACE_ROOT / "docs/legal/public_package_surface_map.json").read_text(encoding="utf-8"))

    contract_targets = {item["target_id"]: item for item in payload["package_targets"]}
    mapped_targets = {item["surface_id"]: item for item in surface_map["surfaces"]}

    assert set(contract_targets).issubset(set(mapped_targets))
    assert set(mapped_targets) == set(contract_targets) | {"ait-native-site"}
    assert contract_targets["ait-worker"]["bundled_with"] == "ait-server"
    assert contract_targets["ait-worker"]["extraction_state"] == "bundled_with_server"

    for target_id, contract_target in contract_targets.items():
        assert contract_target["public_boundary"] == mapped_targets[target_id]["release_posture"]

    website_surface = mapped_targets["ait-native-site"]
    assert website_surface["console_scripts"] == []
    assert website_surface["install_surface"] == "official_public_website"
    assert website_surface["release_posture"] == "component_specific_release_facing_surface"

    profiles = {profile["profile_id"]: profile for profile in payload["operator_profiles"]}
    assert profiles["local_only_first_loop"]["required_targets"] == ["ait"]
    assert profiles["self_hosted_shared_control_plane_core"]["required_targets"] == [
        "ait-server",
        "ait-worker",
    ]
    assert "self_hosted_shared_control_plane_with_web" not in profiles
