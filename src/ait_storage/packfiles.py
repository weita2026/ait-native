from __future__ import annotations

import difflib
import hashlib
import json
from pathlib import Path
from typing import Any, Callable, Iterable
import zipfile

PACK_FORMAT_V2 = "ait-pack-v2"
PACK_INDEX_ENTRY_NAME = "pack-index.json"
PACK_DELTA_TEXT_V1 = "text-line-v1"
PACK_DELTA_GIT_BINARY_V1 = "git-binary-v1"
DEFAULT_MAX_DELTA_CHAIN_DEPTH = 4
MIN_DELTA_BLOB_BYTES = 32
MAX_DELTA_BLOB_BYTES = 131072
MIN_DELTA_SAVINGS_BYTES = 16


def _pack_entries_by_name(pack_index: dict[str, Any]) -> dict[str, dict[str, Any]]:
    entries = pack_index.get("entries")
    if not isinstance(entries, list):
        raise ValueError("Invalid pack index: missing entries list")
    out: dict[str, dict[str, Any]] = {}
    for entry in entries:
        if not isinstance(entry, dict):
            raise ValueError("Invalid pack index: malformed entry")
        entry_name = entry.get("entry_name")
        if not isinstance(entry_name, str) or not entry_name:
            raise ValueError("Invalid pack index: entry missing entry_name")
        if entry_name in out:
            if _pack_entry_equivalent(out[entry_name], entry):
                continue
            raise ValueError(f"Invalid pack index: duplicate entry_name {entry_name}")
        out[entry_name] = entry
    return out


def _pack_entry_equivalent(left: dict[str, Any], right: dict[str, Any]) -> bool:
    keys = (
        "entry_name",
        "blob_id",
        "entry_type",
        "byte_length",
        "uncompressed_byte_length",
        "base_blob_id",
        "chain_depth",
        "checksum",
        "delta_algorithm",
    )
    return all(left.get(key) == right.get(key) for key in keys)


def _entry_metadata(
    entry_name: str,
    blob_id: str,
    data: bytes,
    *,
    logical_data: bytes | None = None,
    entry_type: str = "full",
    base_blob_id: str | None = None,
    chain_depth: int = 0,
    delta_algorithm: str | None = None,
) -> dict[str, Any]:
    resolved_data = logical_data if logical_data is not None else data
    metadata = {
        "entry_name": entry_name,
        "blob_id": blob_id,
        "entry_type": entry_type,
        "byte_length": len(data),
        "uncompressed_byte_length": len(resolved_data),
        "base_blob_id": base_blob_id,
        "chain_depth": chain_depth,
        "checksum": hashlib.sha256(resolved_data).hexdigest(),
    }
    if delta_algorithm:
        metadata["delta_algorithm"] = delta_algorithm
    return metadata


def build_pack_index(pack_id: str, created_at: str, members: Iterable[dict[str, Any]]) -> dict[str, Any]:
    entries = [
        _entry_metadata(
            member["entry_name"],
            member["blob_id"],
            member["data"],
            logical_data=member.get("logical_data"),
            entry_type=member.get("entry_type", "full"),
            base_blob_id=member.get("base_blob_id"),
            chain_depth=int(member.get("chain_depth", 0)),
            delta_algorithm=member.get("delta_algorithm"),
        )
        for member in members
    ]
    total_bytes = sum(int(entry["byte_length"]) for entry in entries)
    return {
        "pack_format": PACK_FORMAT_V2,
        "pack_id": pack_id,
        "created_at": created_at,
        "index_entry_name": PACK_INDEX_ENTRY_NAME,
        "member_count": len(entries),
        "total_bytes": total_bytes,
        "entries": entries,
    }


def _split_text_lines(data: bytes) -> list[str]:
    return data.decode("utf-8").splitlines(keepends=True)


def build_text_delta(base_data: bytes, target_data: bytes) -> bytes:
    base_lines = _split_text_lines(base_data)
    target_lines = _split_text_lines(target_data)
    matcher = difflib.SequenceMatcher(a=base_lines, b=target_lines, autojunk=False)
    ops: list[dict[str, Any]] = []
    for tag, i1, i2, j1, j2 in matcher.get_opcodes():
        if tag == "equal":
            ops.append({"op": "copy", "start": i1, "end": i2})
        elif tag in {"insert", "replace"}:
            ops.append({"op": "data", "lines": target_lines[j1:j2]})
        elif tag == "delete":
            continue
        else:
            raise ValueError(f"Unsupported delta opcode: {tag}")
    payload = {
        "algorithm": PACK_DELTA_TEXT_V1,
        "encoding": "utf-8",
        "ops": ops,
    }
    return json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")


def build_text_delta_member(
    *,
    entry_name: str,
    blob_id: str,
    base_blob_id: str,
    base_data: bytes,
    target_data: bytes,
    chain_depth: int,
) -> dict[str, Any]:
    return {
        "entry_name": entry_name,
        "blob_id": blob_id,
        "data": build_text_delta(base_data, target_data),
        "logical_data": target_data,
        "entry_type": "delta",
        "base_blob_id": base_blob_id,
        "chain_depth": chain_depth,
        "delta_algorithm": PACK_DELTA_TEXT_V1,
    }


def _encode_size_varint(value: int) -> bytes:
    if value < 0:
        raise ValueError("Pack delta sizes must be non-negative")
    out = bytearray()
    remaining = value
    while True:
        byte = remaining & 0x7F
        remaining >>= 7
        if remaining:
            out.append(byte | 0x80)
        else:
            out.append(byte)
            return bytes(out)


def _decode_size_varint(data: bytes, offset: int = 0) -> tuple[int, int]:
    value = 0
    shift = 0
    cursor = offset
    while True:
        if cursor >= len(data):
            raise ValueError("Invalid pack delta payload: truncated size header")
        byte = data[cursor]
        cursor += 1
        value |= (byte & 0x7F) << shift
        if not (byte & 0x80):
            return value, cursor
        shift += 7
        if shift > 63:
            raise ValueError("Invalid pack delta payload: size header is too large")


def _encode_copy_instruction(offset: int, size: int) -> bytes:
    if offset < 0 or size <= 0:
        raise ValueError("Pack delta copy instructions require non-negative offsets and positive sizes")
    command = 0x80
    payload = bytearray()
    for bit, shift in enumerate((0, 8, 16, 24)):
        byte = (offset >> shift) & 0xFF
        if byte:
            command |= 1 << bit
            payload.append(byte)
    for bit, shift in enumerate((0, 8, 16), start=4):
        byte = (size >> shift) & 0xFF
        if byte:
            command |= 1 << bit
            payload.append(byte)
    return bytes([command]) + payload


def _append_copy_instructions(out: bytearray, offset: int, size: int) -> None:
    remaining = size
    current_offset = offset
    while remaining > 0:
        chunk_size = min(remaining, 0xFFFF)
        out.extend(_encode_copy_instruction(current_offset, chunk_size))
        current_offset += chunk_size
        remaining -= chunk_size


def _append_insert_instructions(out: bytearray, data: bytes) -> None:
    cursor = 0
    while cursor < len(data):
        chunk = data[cursor : cursor + 0x7F]
        out.append(len(chunk))
        out.extend(chunk)
        cursor += len(chunk)


def _delta_match_granularity(base_data: bytes, target_data: bytes) -> int:
    max_size = max(len(base_data), len(target_data))
    if max_size < 8 * 1024:
        return 1
    if max_size < 64 * 1024:
        return 8
    return 32


def _chunk_bytes(data: bytes, unit: int) -> bytes | tuple[bytes, ...]:
    if unit == 1:
        return data
    return tuple(data[index : index + unit] for index in range(0, len(data), unit))


def build_git_binary_delta(base_data: bytes, target_data: bytes) -> bytes:
    unit = _delta_match_granularity(base_data, target_data)
    matcher = difflib.SequenceMatcher(a=_chunk_bytes(base_data, unit), b=_chunk_bytes(target_data, unit), autojunk=False)
    out = bytearray()
    out.extend(_encode_size_varint(len(base_data)))
    out.extend(_encode_size_varint(len(target_data)))
    for tag, i1, i2, j1, j2 in matcher.get_opcodes():
        base_start = i1 * unit
        base_end = min(i2 * unit, len(base_data))
        target_start = j1 * unit
        target_end = min(j2 * unit, len(target_data))
        if tag == "equal":
            _append_copy_instructions(out, base_start, base_end - base_start)
            continue
        if tag in {"insert", "replace"}:
            _append_insert_instructions(out, target_data[target_start:target_end])
            continue
        if tag == "delete":
            continue
        raise ValueError(f"Unsupported delta opcode: {tag}")
    return bytes(out)


def build_git_binary_delta_member(
    *,
    entry_name: str,
    blob_id: str,
    base_blob_id: str,
    base_data: bytes,
    target_data: bytes,
    chain_depth: int,
) -> dict[str, Any]:
    return {
        "entry_name": entry_name,
        "blob_id": blob_id,
        "data": build_git_binary_delta(base_data, target_data),
        "logical_data": target_data,
        "entry_type": "delta",
        "base_blob_id": base_blob_id,
        "chain_depth": chain_depth,
        "delta_algorithm": PACK_DELTA_GIT_BINARY_V1,
    }


def build_pack_members(
    blob_items: Iterable[dict[str, Any]],
    *,
    max_delta_chain_depth: int = DEFAULT_MAX_DELTA_CHAIN_DEPTH,
    initial_by_path: dict[str, dict[str, Any]] | None = None,
) -> list[dict[str, Any]]:
    members: list[dict[str, Any]] = []
    latest_by_path: dict[str, dict[str, Any]] = {
        str(path_hint): {
            "blob_id": candidate["blob_id"],
            "data": candidate["data"],
            "chain_depth": int(candidate.get("chain_depth", 0)),
        }
        for path_hint, candidate in (initial_by_path or {}).items()
        if path_hint
    }
    for item in blob_items:
        entry_name = item["entry_name"]
        blob_id = item["blob_id"]
        data = item["data"]
        path_hint = str(item.get("path_hint") or "")
        member: dict[str, Any] = {
            "entry_name": entry_name,
            "blob_id": blob_id,
            "data": data,
            "entry_type": "full",
            "chain_depth": 0,
        }
        base_candidate = latest_by_path.get(path_hint) if path_hint else None
        if (
            base_candidate is not None
            and base_candidate["blob_id"] != blob_id
            and base_candidate["chain_depth"] < max_delta_chain_depth
            and MIN_DELTA_BLOB_BYTES <= len(data) <= MAX_DELTA_BLOB_BYTES
            and MIN_DELTA_BLOB_BYTES <= len(base_candidate["data"]) <= MAX_DELTA_BLOB_BYTES
        ):
            delta_data = build_git_binary_delta(base_candidate["data"], data)
            if delta_data is not None and len(delta_data) + MIN_DELTA_SAVINGS_BYTES < len(data):
                member = build_git_binary_delta_member(
                    entry_name=entry_name,
                    blob_id=blob_id,
                    base_blob_id=base_candidate["blob_id"],
                    base_data=base_candidate["data"],
                    target_data=data,
                    chain_depth=base_candidate["chain_depth"] + 1,
                )
        members.append(member)
        if path_hint:
            latest_by_path[path_hint] = {
                "blob_id": blob_id,
                "data": data,
                "chain_depth": int(member.get("chain_depth", 0)),
            }
    return members


def apply_text_delta(base_data: bytes, delta_data: bytes) -> bytes:
    payload = json.loads(delta_data.decode("utf-8"))
    if payload.get("algorithm") != PACK_DELTA_TEXT_V1:
        raise ValueError(f"Unsupported pack delta algorithm: {payload.get('algorithm')!r}")
    if payload.get("encoding") != "utf-8":
        raise ValueError(f"Unsupported pack delta encoding: {payload.get('encoding')!r}")
    base_lines = _split_text_lines(base_data)
    out: list[str] = []
    ops = payload.get("ops")
    if not isinstance(ops, list):
        raise ValueError("Invalid pack delta payload: missing ops")
    for op in ops:
        if not isinstance(op, dict):
            raise ValueError("Invalid pack delta payload: malformed op")
        kind = op.get("op")
        if kind == "copy":
            start = int(op.get("start", 0))
            end = int(op.get("end", start))
            if start < 0 or end < start or end > len(base_lines):
                raise ValueError("Invalid pack delta payload: copy range out of bounds")
            out.extend(base_lines[start:end])
            continue
        if kind == "data":
            lines = op.get("lines")
            if not isinstance(lines, list) or any(not isinstance(line, str) for line in lines):
                raise ValueError("Invalid pack delta payload: malformed data lines")
            out.extend(lines)
            continue
        raise ValueError(f"Invalid pack delta payload: unsupported op {kind!r}")
    return "".join(out).encode("utf-8")


def apply_git_binary_delta(base_data: bytes, delta_data: bytes) -> bytes:
    expected_base_size, cursor = _decode_size_varint(delta_data)
    if expected_base_size != len(base_data):
        raise ValueError("Invalid pack delta payload: base size mismatch")
    expected_target_size, cursor = _decode_size_varint(delta_data, cursor)
    out = bytearray()
    while cursor < len(delta_data):
        command = delta_data[cursor]
        cursor += 1
        if command & 0x80:
            offset = 0
            size = 0
            if command & 0x01:
                offset |= delta_data[cursor]
                cursor += 1
            if command & 0x02:
                offset |= delta_data[cursor] << 8
                cursor += 1
            if command & 0x04:
                offset |= delta_data[cursor] << 16
                cursor += 1
            if command & 0x08:
                offset |= delta_data[cursor] << 24
                cursor += 1
            if command & 0x10:
                size |= delta_data[cursor]
                cursor += 1
            if command & 0x20:
                size |= delta_data[cursor] << 8
                cursor += 1
            if command & 0x40:
                size |= delta_data[cursor] << 16
                cursor += 1
            if size == 0:
                size = 0x10000
            end = offset + size
            if offset < 0 or end > len(base_data):
                raise ValueError("Invalid pack delta payload: copy range out of bounds")
            out.extend(base_data[offset:end])
            continue
        if command == 0:
            raise ValueError("Invalid pack delta payload: zero instruction is reserved")
        end = cursor + command
        if end > len(delta_data):
            raise ValueError("Invalid pack delta payload: insert data truncated")
        out.extend(delta_data[cursor:end])
        cursor = end
    if len(out) != expected_target_size:
        raise ValueError("Invalid pack delta payload: target size mismatch")
    return bytes(out)


def apply_pack_delta(base_data: bytes, delta_data: bytes, *, algorithm: str) -> bytes:
    if algorithm == PACK_DELTA_GIT_BINARY_V1:
        return apply_git_binary_delta(base_data, delta_data)
    if algorithm == PACK_DELTA_TEXT_V1:
        return apply_text_delta(base_data, delta_data)
    raise ValueError(f"Unsupported pack delta algorithm: {algorithm!r}")


def write_pack_archive(pack_path: Path, pack_id: str, created_at: str, members: Iterable[dict[str, Any]]) -> dict[str, Any]:
    pack_path.parent.mkdir(parents=True, exist_ok=True)
    prepared_members = list(members)
    pack_index = build_pack_index(pack_id, created_at, prepared_members)
    index_bytes = json.dumps(pack_index, indent=2, sort_keys=True).encode("utf-8")
    index_checksum = hashlib.sha256(index_bytes).hexdigest()
    with zipfile.ZipFile(pack_path, mode="w", compression=zipfile.ZIP_DEFLATED) as zf:
        for member in prepared_members:
            zf.writestr(member["entry_name"], member["data"])
        zf.writestr(PACK_INDEX_ENTRY_NAME, index_bytes)
    return {
        "member_count": pack_index["member_count"],
        "total_bytes": pack_index["total_bytes"],
        "archive_bytes": pack_path.stat().st_size,
        "pack_format": PACK_FORMAT_V2,
        "pack_index_entry_name": PACK_INDEX_ENTRY_NAME,
        "pack_index_checksum": index_checksum,
        "pack_index": pack_index,
    }


def read_pack_index(pack_path: Path) -> dict[str, Any]:
    with zipfile.ZipFile(pack_path, mode="r") as zf:
        if PACK_INDEX_ENTRY_NAME not in zf.namelist():
            raise ValueError(f"Invalid pack archive: missing {PACK_INDEX_ENTRY_NAME}")
        raw = zf.read(PACK_INDEX_ENTRY_NAME)
    pack_index = json.loads(raw.decode("utf-8"))
    if pack_index.get("pack_format") != PACK_FORMAT_V2:
        raise ValueError(f"Invalid pack index: unsupported pack_format {pack_index.get('pack_format')!r}")
    if pack_index.get("index_entry_name") != PACK_INDEX_ENTRY_NAME:
        raise ValueError("Invalid pack index: incorrect index_entry_name")
    _pack_entries_by_name(pack_index)
    return pack_index


def summarize_pack_archives(pack_root: Path, pack_rows: Iterable[dict[str, Any]]) -> dict[str, int]:
    summary = {
        "pack_archive_bytes": 0,
        "indexed_pack_count": 0,
        "index_error_count": 0,
        "pack_indexed_blob_count": 0,
        "pack_member_bytes": 0,
        "pack_full_member_bytes": 0,
        "pack_delta_member_bytes": 0,
        "pack_member_logical_bytes": 0,
        "pack_full_logical_bytes": 0,
        "pack_delta_logical_bytes": 0,
    }
    for row in pack_rows:
        pack_path = row.get("pack_path")
        if not pack_path:
            continue
        pack_abs = pack_root / str(pack_path)
        if not pack_abs.exists():
            continue
        summary["pack_archive_bytes"] += pack_abs.stat().st_size
        try:
            pack_index = read_pack_index(pack_abs)
        except Exception:
            summary["index_error_count"] += 1
            continue
        summary["indexed_pack_count"] += 1
        entries = pack_index.get("entries", [])
        if not isinstance(entries, list):
            summary["index_error_count"] += 1
            continue
        summary["pack_indexed_blob_count"] += len(entries)
        for entry in entries:
            if not isinstance(entry, dict):
                continue
            member_bytes = int(entry.get("byte_length", 0))
            logical_bytes = int(entry.get("uncompressed_byte_length", member_bytes))
            entry_type = str(entry.get("entry_type", "full"))
            summary["pack_member_bytes"] += member_bytes
            summary["pack_member_logical_bytes"] += logical_bytes
            if entry_type == "delta":
                summary["pack_delta_member_bytes"] += member_bytes
                summary["pack_delta_logical_bytes"] += logical_bytes
            else:
                summary["pack_full_member_bytes"] += member_bytes
                summary["pack_full_logical_bytes"] += logical_bytes
    return summary


def build_storage_validation_summary(
    *,
    packed_blob_count: int,
    packed_full_blob_count: int,
    packed_delta_blob_count: int,
    pack_count: int,
    pack_index_error_count: int,
    tree_pack_index_error_count: int = 0,
    storage_savings_ratio: float,
    unreferenced_blob_count: int = 0,
    unreferenced_tree_count: int = 0,
    signals_summary: dict[str, Any] | None = None,
) -> dict[str, Any]:
    issues: list[str] = []
    reasons: list[str] = []
    next_actions: list[str] = []
    recommended_action = "none"
    state = "delta_optimized"
    needs_attention = False

    drift_count = int((signals_summary or {}).get("drift_count", 0))
    repairable_drift_count = int((signals_summary or {}).get("repairable_drift_count", 0))
    nonrepairable_drift_count = max(drift_count - repairable_drift_count, 0)
    if pack_index_error_count > 0:
        issues.append("pack_index_errors")
    if tree_pack_index_error_count > 0:
        issues.append("tree_pack_index_errors")
    if drift_count > 0:
        issues.append("storage_drift")
    if issues:
        state = "attention_required"
        needs_attention = True
        if repairable_drift_count > 0 and pack_index_error_count == 0 and tree_pack_index_error_count == 0 and nonrepairable_drift_count == 0:
            recommended_action = "optimize"
            next_actions.append("optimize")
            reasons.append("repairable storage drift is present")
        else:
            recommended_action = "inspect"
            next_actions.append("inspect")
            reasons.append("storage metadata requires operator attention")
        if pack_index_error_count > 0:
            reasons.append("one or more pack indexes could not be read")
        if tree_pack_index_error_count > 0:
            reasons.append("one or more tree pack indexes could not be read")
        return {
            "state": state,
            "recommended_action": recommended_action,
            "next_actions": next_actions,
            "issues": issues,
            "reasons": reasons,
            "needs_attention": needs_attention,
            "has_pack_optimization": packed_blob_count > 0,
            "has_delta_optimization": packed_delta_blob_count > 0,
            "storage_savings_ratio": storage_savings_ratio,
        }

    if packed_blob_count == 0:
        state = "unoptimized"
        reasons.append("no packed blob payloads are currently tracked")
    elif packed_delta_blob_count == 0 and packed_full_blob_count > 0:
        state = "packed_full_only"
        next_actions.append("repack")
        reasons.append("pack data exists but no delta entries have been produced yet")
        if unreferenced_tree_count > 0:
            if "gc" not in next_actions:
                next_actions.append("gc")
            reasons.append("unreachable tree metadata remains to be cleaned up")
    elif pack_count > 1 or unreferenced_blob_count > 0 or unreferenced_tree_count > 0:
        state = "partially_optimized"
        if pack_count > 1:
            next_actions.append("repack")
            reasons.append("multiple live packs remain and repack can still consolidate layout")
            next_actions.append("gc")
            reasons.append("old pack archives should be cleaned after repack")
        else:
            next_actions.append("gc")
        if unreferenced_blob_count > 0:
            if "gc" not in next_actions:
                next_actions.append("gc")
            reasons.append("unreferenced blob payloads remain to be cleaned up")
        if unreferenced_tree_count > 0:
            if "gc" not in next_actions:
                next_actions.append("gc")
            reasons.append("unreachable tree metadata remains to be cleaned up")
    else:
        state = "delta_optimized"
        reasons.append("delta-capable packing is active and no cleanup signals remain")

    if next_actions:
        recommended_action = next_actions[0]

    return {
        "state": state,
        "recommended_action": recommended_action,
        "next_actions": next_actions,
        "issues": issues,
        "reasons": reasons,
        "needs_attention": needs_attention,
        "has_pack_optimization": packed_blob_count > 0,
        "has_delta_optimization": packed_delta_blob_count > 0,
        "storage_savings_ratio": storage_savings_ratio,
    }


def _read_entry_bytes(
    zf: zipfile.ZipFile,
    *,
    entry_name: str,
    entries_by_name: dict[str, dict[str, Any]],
    resolve_base_blob: Callable[[str], bytes] | None,
    max_chain_depth: int,
    visited_blob_ids: set[str],
    depth: int,
) -> bytes:
    if depth > max_chain_depth:
        raise ValueError(f"Pack delta chain depth exceeded for {entry_name}")
    entry = entries_by_name.get(entry_name)
    if entry is None:
        raise KeyError(entry_name)
    try:
        data = zf.read(entry_name)
    except KeyError as exc:
        raise KeyError(entry_name) from exc
    entry_type = entry.get("entry_type") or "full"
    entry_chain_depth = int(entry.get("chain_depth", 0) or 0)
    if entry_chain_depth > max_chain_depth:
        raise ValueError(f"Pack delta chain depth exceeded for {entry_name}")
    if entry_type == "full":
        checksum = hashlib.sha256(data).hexdigest()
        if checksum != entry.get("checksum"):
            raise ValueError(f"Pack entry checksum mismatch for {entry_name}")
        return data
    if entry_type != "delta":
        raise ValueError(f"Unsupported pack entry type: {entry_type!r}")
    base_blob_id = entry.get("base_blob_id")
    if not isinstance(base_blob_id, str) or not base_blob_id:
        raise ValueError(f"Invalid delta entry base blob for {entry_name}")
    blob_id = entry.get("blob_id")
    identity = str(blob_id or entry_name)
    if identity in visited_blob_ids:
        raise ValueError(f"Cyclic pack delta chain detected for {entry_name}")
    next_visited = set(visited_blob_ids)
    next_visited.add(identity)
    base_entry_name = f"blobs/{base_blob_id}"
    if base_entry_name in entries_by_name:
        base_data = _read_entry_bytes(
            zf,
            entry_name=base_entry_name,
            entries_by_name=entries_by_name,
            resolve_base_blob=resolve_base_blob,
            max_chain_depth=max_chain_depth,
            visited_blob_ids=next_visited,
            depth=depth + 1,
        )
    elif resolve_base_blob is not None:
        base_data = resolve_base_blob(base_blob_id)
    else:
        raise ValueError(f"Missing base blob resolver for delta entry {entry_name}")
    delta_algorithm = str(entry.get("delta_algorithm") or PACK_DELTA_TEXT_V1)
    resolved = apply_pack_delta(base_data, data, algorithm=delta_algorithm)
    checksum = hashlib.sha256(resolved).hexdigest()
    if checksum != entry.get("checksum"):
        raise ValueError(f"Pack entry checksum mismatch for {entry_name}")
    if len(resolved) != int(entry.get("uncompressed_byte_length", len(resolved))):
        raise ValueError(f"Pack entry size mismatch for {entry_name}")
    return resolved


def read_pack_entry(
    pack_path: Path,
    entry_name: str,
    *,
    resolve_base_blob: Callable[[str], bytes] | None = None,
    max_chain_depth: int = DEFAULT_MAX_DELTA_CHAIN_DEPTH,
) -> bytes:
    pack_index = read_pack_index(pack_path)
    entries_by_name = _pack_entries_by_name(pack_index)
    with zipfile.ZipFile(pack_path, mode="r") as zf:
        return _read_entry_bytes(
            zf,
            entry_name=entry_name,
            entries_by_name=entries_by_name,
            resolve_base_blob=resolve_base_blob,
            max_chain_depth=max_chain_depth,
            visited_blob_ids=set(),
            depth=0,
        )


def pack_has_entry(pack_path: Path, entry_name: str) -> bool:
    if not pack_path.exists():
        return False
    try:
        pack_index = read_pack_index(pack_path)
        if entry_name not in _pack_entries_by_name(pack_index):
            return False
    except Exception:
        return False
    with zipfile.ZipFile(pack_path, mode="r") as zf:
        try:
            zf.getinfo(entry_name)
            return True
        except KeyError:
            return False
