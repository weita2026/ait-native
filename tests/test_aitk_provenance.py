from __future__ import annotations

from ait.aitk_provenance import build_snapshot_provenance_overlay


def test_build_snapshot_provenance_overlay_maps_common_snapshot_fields():
    overlay = build_snapshot_provenance_overlay(
        ["S1", "S2", "S3"],
        changes=[{"change_id": "C-1", "revision_snapshot_id": "S1", "base_snapshot_id": "S2", "title": "change"}],
        patchsets=[{"patchset_id": "P-1", "revision_snapshot_id": "S1", "base_snapshot_id": "S2"}],
        lands=[{"submission_id": "LAND-1", "target_snapshot_id": "S3", "status": "landed"}],
        sessions=[{"session_id": "SESS-1", "head_snapshot_id": "S1", "summary": "session"}],
    )

    assert overlay["S1"]["badges"] == ["change", "patchset", "session"]
    assert overlay["S2"]["badges"] == ["change", "patchset"]
    assert overlay["S3"]["badges"] == ["land"]
    assert {item["snapshot_role"] for item in overlay["S1"]["items"]} == {"revision", "head"}
    assert "/changes/C-1" in overlay["S1"]["links"]
    assert "land:LAND-1" in overlay["S3"]["links"]


def test_build_snapshot_provenance_overlay_ignores_unknown_or_missing_fields():
    overlay = build_snapshot_provenance_overlay(
        ["S1"],
        tasks=[{"task_id": "T-1", "title": "no snapshot"}],
        changes=[{"change_id": "C-1", "revision_snapshot_id": "S2"}],
        patchsets=[{"revision_snapshot_id": "S1"}],
        lands=[None, {"land_id": "LAND-2", "snapshot_id": "S1"}],  # type: ignore[list-item]
    )

    assert overlay["S1"]["badges"] == ["land"]
    assert overlay["S1"]["items"] == [
        {"kind": "land", "id": "LAND-2", "snapshot_id": "S1", "snapshot_role": "snapshot"}
    ]
