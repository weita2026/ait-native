from __future__ import annotations

import json
import time
from typing import Any, Mapping

from ait_protocol.common import utc_now


def land_request_payload(row) -> dict:
    out = dict(row)
    out.pop("priority", None)
    out["result"] = json.loads(out["result_json"])
    return out


def elapsed_ms(start: float, end: float | None = None) -> float:
    finished = time.perf_counter() if end is None else end
    return round((finished - start) * 1000.0, 3)


def phase_timings_from_result(result: Mapping[str, Any] | None) -> dict[str, Any]:
    if not isinstance(result, Mapping):
        return {}
    raw = result.get("phase_timings_ms")
    if not isinstance(raw, Mapping):
        return {}
    return dict(raw)


def land_freshness_result(
    target_line: str,
    patchset: Mapping[str, Any],
    *,
    target_line_head: str | None,
    alignment: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    normalized_target = str(target_line_head or "").strip() or None
    expected_base_snapshot_id = str(patchset.get("base_snapshot_id") or "").strip() or None
    revision_snapshot_id = str(patchset.get("revision_snapshot_id") or "").strip() or None
    alignment_map = dict(alignment) if isinstance(alignment, Mapping) else {}
    target_matches_revision_snapshot = bool(
        alignment_map.get("target_matches_revision_snapshot")
        if alignment_map
        else normalized_target and revision_snapshot_id and normalized_target == revision_snapshot_id
    )
    target_matches_revision_tree = bool(
        alignment_map.get("target_matches_revision_tree")
        if alignment_map
        else target_matches_revision_snapshot
    )
    return {
        "checked_at": utc_now(),
        "target_line": target_line,
        "target_line_head": normalized_target,
        "expected_base_snapshot_id": expected_base_snapshot_id,
        "revision_snapshot_id": revision_snapshot_id,
        "base_is_fresh": bool(
            normalized_target and expected_base_snapshot_id and normalized_target == expected_base_snapshot_id
        ),
        "target_matches_revision_snapshot": target_matches_revision_snapshot,
        "target_matches_revision_tree": target_matches_revision_tree,
        "already_aligned_equivalent": target_matches_revision_tree,
    }
