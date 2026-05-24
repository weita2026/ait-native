from __future__ import annotations

import json
import re
from pathlib import Path


def test_public_compatibility_matrix_matches_project_version() -> None:
    payload = json.loads(Path("docs/public_compatibility_matrix.json").read_text(encoding="utf-8"))
    package_init = Path("src/ait_native/__init__.py").read_text(encoding="utf-8")
    version_match = re.search(r'^__version__\s*=\s*"([^"]+)"', package_init, re.MULTILINE)

    assert version_match is not None
    assert payload["schema_version"] == 1
    assert payload["guide_path"] == "docs/COMPATIBILITY_MATRIX.md"
    assert payload["distribution"]["name"] == "ait-native"
    assert payload["distribution"]["version"] == version_match.group(1)
    assert payload["support_policy"]["shared_surfaces"] == "same_release_family_only"


def test_public_compatibility_matrix_profiles_and_unsupported_mixes() -> None:
    payload = json.loads(Path("docs/public_compatibility_matrix.json").read_text(encoding="utf-8"))

    profiles = {profile["profile_id"]: profile for profile in payload["profiles"]}
    assert profiles["local_only_first_loop"]["required_surfaces"] == ["ait"]
    assert profiles["self_hosted_shared_control_plane_core"]["required_surfaces"] == [
        "postgres",
        "ait-server",
        "ait-worker",
    ]
    assert "self_hosted_shared_control_plane_with_web" not in profiles

    unsupported = {entry["mix_id"] for entry in payload["unsupported_mixes"]}
    assert unsupported == {
        "shared_sqlite_deployment",
        "worker_without_matching_server",
        "mixed_release_family_shared_stack",
    }
    assert payload["required_checks_after_upgrade"] == [
        "ait repo readiness --json",
        "ait repo jobs --diagnostics --json",
        "ait doctor postgres --connect --json",
    ]
