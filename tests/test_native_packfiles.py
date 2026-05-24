from __future__ import annotations

import json
from pathlib import Path
import zipfile

import pytest

from ait_native.packfiles import (
    DEFAULT_MAX_DELTA_CHAIN_DEPTH,
    PACK_DELTA_GIT_BINARY_V1,
    PACK_INDEX_ENTRY_NAME,
    build_git_binary_delta_member,
    build_text_delta_member,
    pack_has_entry,
    read_pack_entry,
    read_pack_index,
    write_pack_archive,
)


def test_pack_index_drives_entry_lookup_and_reads(tmp_path: Path):
    pack_path = tmp_path / "test-pack.zip"
    archive = write_pack_archive(
        pack_path,
        "PCK-TEST",
        "2026-04-12T00:00:00+00:00",
        [
            {"entry_name": "blobs/BLB-1", "blob_id": "BLB-1", "data": b"hello\n", "entry_type": "full", "chain_depth": 0},
            {"entry_name": "blobs/BLB-2", "blob_id": "BLB-2", "data": b"world\n", "entry_type": "full", "chain_depth": 0},
        ],
    )

    pack_index = read_pack_index(pack_path)
    assert archive["pack_index_checksum"]
    assert pack_index["pack_id"] == "PCK-TEST"
    assert pack_index["member_count"] == 2
    assert pack_has_entry(pack_path, "blobs/BLB-1") is True
    assert pack_has_entry(pack_path, "blobs/BLB-missing") is False
    assert read_pack_entry(pack_path, "blobs/BLB-2") == b"world\n"


def test_read_pack_entry_rejects_checksum_mismatch(tmp_path: Path):
    pack_path = tmp_path / "corrupt-pack.zip"
    write_pack_archive(
        pack_path,
        "PCK-CORRUPT",
        "2026-04-12T00:00:00+00:00",
        [{"entry_name": "blobs/BLB-1", "blob_id": "BLB-1", "data": b"hello\n", "entry_type": "full", "chain_depth": 0}],
    )

    pack_index = read_pack_index(pack_path)
    pack_index["entries"][0]["checksum"] = "deadbeef"
    with zipfile.ZipFile(pack_path, mode="w", compression=zipfile.ZIP_DEFLATED) as zf:
        zf.writestr("blobs/BLB-1", b"hello\n")
        zf.writestr(PACK_INDEX_ENTRY_NAME, json.dumps(pack_index, indent=2, sort_keys=True).encode("utf-8"))

    with pytest.raises(ValueError, match="checksum mismatch"):
        read_pack_entry(pack_path, "blobs/BLB-1")


def test_pack_has_entry_returns_false_when_index_is_missing(tmp_path: Path):
    pack_path = tmp_path / "missing-index.zip"
    with zipfile.ZipFile(pack_path, mode="w", compression=zipfile.ZIP_DEFLATED) as zf:
        zf.writestr("blobs/BLB-1", b"hello\n")

    assert pack_has_entry(pack_path, "blobs/BLB-1") is False
    with pytest.raises(ValueError, match="missing pack-index.json"):
        read_pack_index(pack_path)


def test_read_pack_index_tolerates_duplicate_identical_entries(tmp_path: Path):
    pack_path = tmp_path / "duplicate-identical-index.zip"
    duplicate_entry = {
        "entry_name": "blobs/BLB-1",
        "blob_id": "BLB-1",
        "entry_type": "full",
        "byte_length": 6,
        "uncompressed_byte_length": 6,
        "base_blob_id": None,
        "chain_depth": 0,
        "checksum": "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03",
    }
    pack_index = {
        "pack_format": "ait-pack-v2",
        "pack_id": "PCK-DUP",
        "created_at": "2026-04-25T00:00:00+00:00",
        "index_entry_name": PACK_INDEX_ENTRY_NAME,
        "member_count": 2,
        "total_bytes": 12,
        "entries": [duplicate_entry, dict(duplicate_entry)],
    }
    with zipfile.ZipFile(pack_path, mode="w", compression=zipfile.ZIP_DEFLATED) as zf:
        zf.writestr("blobs/BLB-1", b"hello\n")
        zf.writestr(PACK_INDEX_ENTRY_NAME, json.dumps(pack_index, indent=2, sort_keys=True).encode("utf-8"))

    parsed = read_pack_index(pack_path)
    assert parsed["member_count"] == 2
    assert read_pack_entry(pack_path, "blobs/BLB-1") == b"hello\n"


def test_read_pack_entry_reconstructs_git_binary_delta_from_same_pack_base(tmp_path: Path):
    pack_path = tmp_path / "delta-pack.zip"
    base_data = b"alpha\nbeta\ngamma\n"
    target_data = b"alpha\nbeta changed\ngamma\ndelta\n"
    write_pack_archive(
        pack_path,
        "PCK-DELTA",
        "2026-04-13T00:00:00+00:00",
        [
            {"entry_name": "blobs/BLB-BASE", "blob_id": "BLB-BASE", "data": base_data, "entry_type": "full", "chain_depth": 0},
            build_git_binary_delta_member(
                entry_name="blobs/BLB-TARGET",
                blob_id="BLB-TARGET",
                base_blob_id="BLB-BASE",
                base_data=base_data,
                target_data=target_data,
                chain_depth=1,
            ),
        ],
    )

    pack_index = read_pack_index(pack_path)
    assert pack_index["entries"][1]["delta_algorithm"] == PACK_DELTA_GIT_BINARY_V1
    assert read_pack_entry(pack_path, "blobs/BLB-TARGET") == target_data


def test_read_pack_entry_reconstructs_git_binary_delta_via_external_base_resolver(tmp_path: Path):
    pack_path = tmp_path / "external-base-pack.zip"
    base_data = b"one\ntwo\nthree\n"
    target_data = b"zero\none\ntwo\nthree\n"
    write_pack_archive(
        pack_path,
        "PCK-DELTA-EXT",
        "2026-04-13T00:00:00+00:00",
        [
            build_git_binary_delta_member(
                entry_name="blobs/BLB-TARGET",
                blob_id="BLB-TARGET",
                base_blob_id="BLB-BASE",
                base_data=base_data,
                target_data=target_data,
                chain_depth=1,
            ),
        ],
    )

    assert read_pack_entry(
        pack_path,
        "blobs/BLB-TARGET",
        resolve_base_blob=lambda blob_id: base_data if blob_id == "BLB-BASE" else b"",
    ) == target_data


def test_read_pack_entry_reconstructs_git_binary_delta_for_arbitrary_bytes(tmp_path: Path):
    pack_path = tmp_path / "binary-delta-pack.zip"
    base_data = bytes(range(256)) * 2
    target_data = base_data[:160] + b"\x00\xffPATCH\x80\x81" + base_data[192:] + b"\x10\x11TAIL"
    write_pack_archive(
        pack_path,
        "PCK-DELTA-BINARY",
        "2026-04-13T00:00:00+00:00",
        [
            {"entry_name": "blobs/BLB-BASE", "blob_id": "BLB-BASE", "data": base_data, "entry_type": "full", "chain_depth": 0},
            build_git_binary_delta_member(
                entry_name="blobs/BLB-TARGET",
                blob_id="BLB-TARGET",
                base_blob_id="BLB-BASE",
                base_data=base_data,
                target_data=target_data,
                chain_depth=1,
            ),
        ],
    )

    assert read_pack_entry(pack_path, "blobs/BLB-TARGET") == target_data


def test_read_pack_entry_can_still_reconstruct_legacy_text_delta(tmp_path: Path):
    pack_path = tmp_path / "legacy-text-delta-pack.zip"
    base_data = b"alpha\nbeta\ngamma\n"
    target_data = b"alpha\nbeta changed\ngamma\ndelta\n"
    write_pack_archive(
        pack_path,
        "PCK-DELTA-LEGACY",
        "2026-04-13T00:00:00+00:00",
        [
            {"entry_name": "blobs/BLB-BASE", "blob_id": "BLB-BASE", "data": base_data, "entry_type": "full", "chain_depth": 0},
            build_text_delta_member(
                entry_name="blobs/BLB-TARGET",
                blob_id="BLB-TARGET",
                base_blob_id="BLB-BASE",
                base_data=base_data,
                target_data=target_data,
                chain_depth=1,
            ),
        ],
    )

    assert read_pack_entry(pack_path, "blobs/BLB-TARGET") == target_data


def test_read_pack_entry_rejects_delta_chain_depth_exceeded(tmp_path: Path):
    pack_path = tmp_path / "deep-delta-pack.zip"
    base_data = b"zero\n"
    first_data = b"zero\none\n"
    second_data = b"zero\none\ntwo\n"
    write_pack_archive(
        pack_path,
        "PCK-DELTA-DEEP",
        "2026-04-13T00:00:00+00:00",
        [
            {"entry_name": "blobs/BLB-BASE", "blob_id": "BLB-BASE", "data": base_data, "entry_type": "full", "chain_depth": 0},
            build_git_binary_delta_member(
                entry_name="blobs/BLB-1",
                blob_id="BLB-1",
                base_blob_id="BLB-BASE",
                base_data=base_data,
                target_data=first_data,
                chain_depth=1,
            ),
            build_git_binary_delta_member(
                entry_name="blobs/BLB-2",
                blob_id="BLB-2",
                base_blob_id="BLB-1",
                base_data=first_data,
                target_data=second_data,
                chain_depth=2,
            ),
        ],
    )

    with pytest.raises(ValueError, match="chain depth exceeded"):
        read_pack_entry(pack_path, "blobs/BLB-2", max_chain_depth=1)

    assert read_pack_entry(pack_path, "blobs/BLB-2", max_chain_depth=DEFAULT_MAX_DELTA_CHAIN_DEPTH) == second_data
