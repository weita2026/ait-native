from __future__ import annotations

import base64
import csv
import hashlib
import io
import json
from pathlib import Path
import runpy
import subprocess
import sys
import tomllib
from types import SimpleNamespace
import zipfile

import pytest


ROOT = Path(__file__).parents[1]
ADAPTER_PATH = ROOT / "release" / "release_adapter.py"


def release_adapter_namespace() -> dict[str, object]:
    return runpy.run_path(str(ADAPTER_PATH))


def test_release_adapter_maps_all_six_targets_to_exact_abi3_wheels() -> None:
    adapter = release_adapter_namespace()
    expected_wheel_name = adapter["expected_wheel_name"]
    targets = adapter["TARGETS"]
    manifest = json.loads((ROOT / "ait-release.json").read_text(encoding="utf-8"))
    component = manifest["components"][0]

    assert manifest["package"]["name"] == component["id"] == "ait-python"
    assert manifest["package"]["version"] == "1.0.0rc6"
    assert manifest["package"]["license_files"] == [
        {"path": "LICENSE", "role": "license"},
        {"path": "NOTICE", "role": "notice"},
    ]
    assert component["ecosystem"] == "python"
    assert len(targets) == len(component["artifacts"]) == 6
    assert {
        artifact["target"]: artifact["path"]
        for artifact in component["artifacts"]
    } == {
        target: f"dist/wheels/{expected_wheel_name(target, '1.0.0rc6')}"
        for target in targets
    }
    assert {artifact["kind"] for artifact in component["artifacts"]} == {
        "python-wheel"
    }


def test_release_commands_are_direct_and_pyproject_enables_abi3() -> None:
    manifest = json.loads((ROOT / "ait-release.json").read_text(encoding="utf-8"))
    commands = manifest["components"][0]["commands"]
    pyproject = tomllib.loads((ROOT / "pyproject.toml").read_text(encoding="utf-8"))

    for phase in ("test", "build", "smoke"):
        action = "check" if phase == "test" else phase
        assert commands[phase] == [
            [
                "python",
                "release/release_adapter.py",
                action,
                "$AIT_RELEASE_TARGET",
                "$AIT_RELEASE_VERSION",
            ]
        ]
    assert pyproject["tool"]["maturin"]["features"] == [
        "pyo3/abi3-py311"
    ]
    assert pyproject["build-system"]["requires"] == ["maturin==1.13.3"]
    assert "shell=True" not in ADAPTER_PATH.read_text(encoding="utf-8")


def test_linux_release_build_uses_only_the_pinned_zig_manylinux_path(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    adapter = release_adapter_namespace()
    release_identity = adapter["ReleaseIdentity"]
    target_spec = adapter["target_spec"]
    build_command = adapter["maturin_build_command"]
    release_environment = adapter["release_environment"]
    require_ziglang = adapter["require_ziglang"]

    linux_target = "aarch64-unknown-linux-gnu"
    linux_spec = target_spec(linux_target)
    identity = release_identity(linux_spec, Path("unused"), "unused")
    command = build_command(identity, linux_target)
    assert command.count("--zig") == 1
    assert command[command.index("--compatibility") + 1] == "manylinux_2_28"
    monkeypatch.setenv("CARGO_ZIGBUILD_ZIG_PATH", "/ambient/zig")
    environment = release_environment(linux_spec)
    assert environment["CARGO_ZIGBUILD_PYTHON_PATH"] == sys.executable
    assert "CARGO_ZIGBUILD_ZIG_PATH" not in environment

    calls: list[list[str]] = []

    def matching_ziglang(
        argv: list[str], **_kwargs: object
    ) -> SimpleNamespace:
        calls.append(argv)
        return SimpleNamespace(stdout="0.15.2\n")

    monkeypatch.setattr(subprocess, "run", matching_ziglang)
    require_ziglang()
    assert calls == [[sys.executable, "-m", "ziglang", "version"]]


def test_non_linux_release_build_does_not_select_zig() -> None:
    adapter = release_adapter_namespace()
    release_identity = adapter["ReleaseIdentity"]
    target_spec = adapter["target_spec"]
    build_command = adapter["maturin_build_command"]
    release_environment = adapter["release_environment"]

    target = "aarch64-apple-darwin"
    spec = target_spec(target)
    identity = release_identity(spec, Path("unused"), "unused")
    assert "--zig" not in build_command(identity, target)
    environment = release_environment(spec)
    assert "CARGO_ZIGBUILD_PYTHON_PATH" not in environment


def test_linux_release_rejects_an_unpinned_ziglang(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    adapter = release_adapter_namespace()
    error_type = adapter["ReleaseAdapterError"]

    monkeypatch.setattr(
        subprocess,
        "run",
        lambda *_args, **_kwargs: SimpleNamespace(stdout="0.16.0\n"),
    )
    with pytest.raises(error_type, match="requires ziglang '0.15.2'"):
        adapter["require_ziglang"]()


def test_release_adapter_accepts_only_exact_internal_and_public_core_layouts(
    tmp_path: Path,
) -> None:
    adapter = release_adapter_namespace()
    error_type = adapter["ReleaseAdapterError"]
    validate_layout = adapter["validate_core_source_layout"]
    internal_path = adapter["INTERNAL_AIT_PY_MANIFEST"]
    public_path = adapter["PUBLIC_AIT_PY_MANIFEST"]

    internal_root = tmp_path / "internal" / "ait-python"
    internal_manifest = internal_root / internal_path
    internal_manifest.parent.mkdir(parents=True)
    internal_manifest.write_text("[package]\nname = 'ait-py'\n", encoding="utf-8")
    (internal_root / "ait-external.lock").write_text(
        """format = "ait.external.lock"

[[node]]
name = "ait-core"
repo_name = "ait-core"
repository_index = 0
snapshot = "SNP-111111111111"
materialize_to = ".ait-external/ait-core"
""",
        encoding="utf-8",
    )
    marker_path = (
        internal_root
        / ".ait-external"
        / "ait-core"
        / ".ait-external-marker.json"
    )
    marker_path.write_text(
        json.dumps(
            {
                "name": "ait-core",
                "repo_name": "ait-core",
                "repository_index": 0,
                "snapshot": "SNP-111111111111",
                "materialize_to": ".ait-external/ait-core",
            }
        ),
        encoding="utf-8",
    )
    assert validate_layout(internal_path, internal_root) == internal_manifest

    public_root = tmp_path / "public" / "ait-native"
    python_root = public_root / "ait-python"
    python_root.mkdir(parents=True)
    public_manifest = public_root / "ait-core/rust/crates/ait-py/Cargo.toml"
    public_manifest.parent.mkdir(parents=True)
    public_manifest.write_text("[package]\nname = 'ait-py'\n", encoding="utf-8")
    mapping = {
        "schema": "ait.release.monorepo-source/v1",
        "public_source_identity": "weita2026/ait-native",
        "public_publish": False,
        "coordinator_snapshot": "SNP-AAAAAAAAAAAA",
        "subtrees": [
            {
                "source_repository": "ait-core",
                "source_snapshot": "SNP-BBBBBBBBBBBB",
                "path": "ait-core",
                "license": "Apache-2.0",
                "components": ["ait", "ait-agent"],
                "transforms": [],
            },
            {"source_repository": "ait-server"},
            {"source_repository": "ait-runner"},
            {
                "source_repository": "ait-python",
                "source_snapshot": "SNP-CCCCCCCCCCCC",
                "path": "ait-python",
                "license": "Apache-2.0",
                "components": ["ait-python"],
                "transforms": ["python-core-path/v1"],
            },
            {"source_repository": "ait-node"},
        ],
    }
    family = {
        "schema": "ait.release.family/v3",
        "public_source": {
            "model": "release-monorepo",
            "identity": "weita2026/ait-native",
            "transforms": [
                {
                    "id": "python-core-path/v1",
                    "source_repository": "ait-python",
                    "path": "pyproject.toml",
                    "from": internal_path,
                    "to": public_path,
                }
            ],
        },
        "components": [
            {
                "id": "ait",
                "source_repository": "ait-core",
                "source_snapshot": "SNP-BBBBBBBBBBBB",
            },
            {
                "id": "ait-agent",
                "source_repository": "ait-core",
                "source_snapshot": "SNP-BBBBBBBBBBBB",
            },
            {
                "id": "ait-python",
                "source_repository": "ait-python",
                "source_snapshot": "SNP-CCCCCCCCCCCC",
            },
        ],
    }
    mapping_path = public_root / "ait-monorepo-source.json"
    family_path = public_root / "ait-release-family.json"
    mapping_path.write_text(json.dumps(mapping), encoding="utf-8")
    family_path.write_text(json.dumps(family), encoding="utf-8")
    assert validate_layout(public_path, python_root).resolve() == public_manifest

    family["components"][2]["source_snapshot"] = "SNP-DDDDDDDDDDDD"
    family_path.write_text(json.dumps(family), encoding="utf-8")
    with pytest.raises(error_type, match="Snapshots differ"):
        validate_layout(public_path, python_root)
    with pytest.raises(error_type, match="not locked to ait-core"):
        validate_layout("../../unmapped/Cargo.toml", python_root)


def write_fixture_wheel(
    path: Path,
    temporary_root: str,
    sbom_timestamp: str,
    zip_timestamp: tuple[int, int, int, int, int, int],
    *,
    include_sbom: bool = True,
    notice_bytes: bytes | None = None,
    core_layout: str = ".ait-external/ait-core",
) -> None:
    dist_info = "ait_python-1.0.0rc6.dist-info"
    local_component = (
        f"path+file://{temporary_root}/{core_layout}/"
        "rust/crates/ait-py#0.1.0"
    )
    local_dependency = (
        f"path+file://{temporary_root}/{core_layout}/"
        "rust/crates/ait-core#0.1.0"
    )
    sbom = {
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "version": 1,
        "metadata": {
            "timestamp": sbom_timestamp,
            "component": {"bom-ref": local_component, "name": "ait-py"},
        },
        "components": [{"bom-ref": local_dependency, "name": "ait-core"}],
        "dependencies": [
            {"ref": local_component, "dependsOn": [local_dependency]}
        ],
    }
    payloads = {
        "ait_py/__init__.py": b"from .ait_py import *\n",
        "ait_py/ait_py.abi3.so": b"native fixture\x00",
        f"{dist_info}/METADATA": (
            b"Metadata-Version: 2.4\n"
            b"Name: ait-python\n"
            b"Version: 1.0.0rc6\n"
            b"License-Expression: Apache-2.0\n"
            b"License-File: LICENSE\n"
            b"License-File: NOTICE\n"
            b"Requires-Python: >=3.11\n"
        ),
        f"{dist_info}/WHEEL": (
            b"Wheel-Version: 1.0\n"
            b"Root-Is-Purelib: false\n"
            b"Tag: cp311-abi3-macosx_11_0_arm64\n"
        ),
        f"{dist_info}/licenses/LICENSE": (ROOT / "LICENSE").read_bytes(),
        f"{dist_info}/licenses/NOTICE": (
            (ROOT / "NOTICE").read_bytes()
            if notice_bytes is None
            else notice_bytes
        ),
    }
    if include_sbom:
        payloads[f"{dist_info}/sboms/ait-py.cyclonedx.json"] = (
            json.dumps(sbom, indent=2).encode("utf-8") + b"\n"
        )
    payloads[f"{dist_info}/RECORD"] = (
        f"{dist_info}/RECORD,,\n".encode("utf-8")
    )
    with zipfile.ZipFile(path, mode="w") as archive:
        for name, data in payloads.items():
            info = zipfile.ZipInfo(name, date_time=zip_timestamp)
            info.create_system = 3
            info.external_attr = 0o100644 << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(info, data, compresslevel=1)


def test_wheel_normalization_converges_temporary_roots_and_timestamps(
    tmp_path: Path,
) -> None:
    adapter = release_adapter_namespace()
    parse_source_date_epoch = adapter["parse_source_date_epoch"]
    normalize_wheel = adapter["normalize_wheel"]
    wheel_a = tmp_path / "a.whl"
    wheel_b = tmp_path / "b.whl"
    write_fixture_wheel(
        wheel_a,
        "/private/tmp/ait-release-adapter-build-alpha/source",
        "2026-08-03T12:00:01.000000000Z",
        (2026, 8, 3, 12, 0, 0),
    )
    write_fixture_wheel(
        wheel_b,
        "/private/tmp/ait-release-adapter-build-beta/source",
        "2026-08-03T13:59:59.000000000Z",
        (2026, 8, 3, 13, 59, 58),
        core_layout="ait-core",
    )
    _, timestamp = parse_source_date_epoch("1785761092")

    normalize_wheel(wheel_a, "1.0.0rc6", timestamp)
    normalize_wheel(wheel_b, "1.0.0rc6", timestamp)

    assert wheel_a.read_bytes() == wheel_b.read_bytes()
    first_normalized = wheel_a.read_bytes()
    normalize_wheel(wheel_a, "1.0.0rc6", timestamp)
    assert wheel_a.read_bytes() == first_normalized

    dist_info = "ait_python-1.0.0rc6.dist-info"
    record_path = f"{dist_info}/RECORD"
    sbom_path = f"{dist_info}/sboms/ait-py.cyclonedx.json"
    with zipfile.ZipFile(wheel_a) as archive:
        names = archive.namelist()
        assert {info.date_time for info in archive.infolist()} == {
            (2026, 8, 3, 12, 44, 52)
        }
        sbom = json.loads(archive.read(sbom_path))
        assert sbom["metadata"]["timestamp"] == (
            "2026-08-03T12:44:52.000000000Z"
        )
        sbom_text = json.dumps(sbom, sort_keys=True)
        assert "ait-release-adapter-build-alpha" not in sbom_text
        assert "ait-release-adapter-build-beta" not in sbom_text
        assert (
            "path+file:///ait-release-source/.ait-external/ait-core"
            in sbom_text
        )
        rows = {
            row[0]: (row[1], row[2])
            for row in csv.reader(
                io.StringIO(archive.read(record_path).decode("utf-8"))
            )
        }
        assert set(rows) == set(names)
        for name in names:
            digest, size = rows[name]
            if name == record_path:
                assert (digest, size) == ("", "")
                continue
            data = archive.read(name)
            expected_digest = base64.urlsafe_b64encode(
                hashlib.sha256(data).digest()
            ).rstrip(b"=")
            assert digest == f"sha256={expected_digest.decode('ascii')}"
            assert size == str(len(data))


def test_wheel_verification_requires_exact_repository_legal_material(
    tmp_path: Path,
) -> None:
    adapter = release_adapter_namespace()
    error_type = adapter["ReleaseAdapterError"]
    expected_wheel_name = adapter["expected_wheel_name"]
    normalize_wheel = adapter["normalize_wheel"]
    parse_source_date_epoch = adapter["parse_source_date_epoch"]
    verify_wheel = adapter["verify_wheel"]
    target = "aarch64-apple-darwin"
    wheel_name = expected_wheel_name(target, "1.0.0rc6")
    _, timestamp = parse_source_date_epoch("1785761092")

    valid_dir = tmp_path / "valid"
    valid_dir.mkdir()
    valid_wheel = valid_dir / wheel_name
    write_fixture_wheel(
        valid_wheel,
        "/private/tmp/ait-release-adapter-valid/source",
        "2026-08-03T12:00:00.000000000Z",
        (2026, 8, 3, 12, 0, 0),
    )
    normalize_wheel(valid_wheel, "1.0.0rc6", timestamp)
    verify_wheel(valid_wheel, target, "1.0.0rc6")

    altered_dir = tmp_path / "altered"
    altered_dir.mkdir()
    altered_wheel = altered_dir / wheel_name
    write_fixture_wheel(
        altered_wheel,
        "/private/tmp/ait-release-adapter-altered/source",
        "2026-08-03T12:00:00.000000000Z",
        (2026, 8, 3, 12, 0, 0),
        notice_bytes=b"altered notice\n",
    )
    normalize_wheel(altered_wheel, "1.0.0rc6", timestamp)
    with pytest.raises(error_type, match="NOTICE differs"):
        verify_wheel(altered_wheel, target, "1.0.0rc6")


def test_release_epoch_and_wheel_controls_fail_closed(
    tmp_path: Path,
) -> None:
    adapter = release_adapter_namespace()
    error_type = adapter["ReleaseAdapterError"]
    parse_source_date_epoch = adapter["parse_source_date_epoch"]
    normalize_wheel = adapter["normalize_wheel"]

    for invalid in (None, "", "-1", "1.5", "0", str(10**20)):
        with pytest.raises(error_type, match="SOURCE_DATE_EPOCH"):
            parse_source_date_epoch(invalid)

    missing_sbom = tmp_path / "missing-sbom.whl"
    write_fixture_wheel(
        missing_sbom,
        "/private/tmp/ait-release-adapter-build-gamma/source",
        "2026-08-03T12:00:00.000000000Z",
        (2026, 8, 3, 12, 0, 0),
        include_sbom=False,
    )
    _, timestamp = parse_source_date_epoch("1785761092")
    with pytest.raises(error_type, match="RECORD or CycloneDX SBOM"):
        normalize_wheel(missing_sbom, "1.0.0rc6", timestamp)
