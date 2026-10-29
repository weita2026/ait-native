from __future__ import annotations

import base64
import csv
import hashlib
import io
import json
from pathlib import Path
import runpy
import struct
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
    assert manifest["package"]["version"] == "1.2.0"
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
        target: f"dist/wheels/{expected_wheel_name(target, '1.2.0')}"
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
                "components": ["ait"],
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

    for retired in ("ait-agent", "ait-agent-worker"):
        mapping["subtrees"][0]["components"] = ["ait", retired]
        mapping_path.write_text(json.dumps(mapping), encoding="utf-8")
        with pytest.raises(error_type, match="source mapping is invalid"):
            validate_layout(public_path, python_root)
    mapping["subtrees"][0]["components"] = ["ait"]
    mapping_path.write_text(json.dumps(mapping), encoding="utf-8")

    for retired in ("ait-agent", "ait-agent-worker"):
        family["components"].append({
            "id": retired,
            "source_repository": "ait-core",
            "source_snapshot": "SNP-BBBBBBBBBBBB",
        })
        family_path.write_text(json.dumps(family), encoding="utf-8")
        with pytest.raises(error_type, match="retired chat components"):
            validate_layout(public_path, python_root)
        family["components"].pop()

    family["components"][0]["source_snapshot"] = "SNP-DDDDDDDDDDDD"
    family_path.write_text(json.dumps(family), encoding="utf-8")
    with pytest.raises(error_type, match="Snapshots differ"):
        validate_layout(public_path, python_root)
    family["components"][0]["source_snapshot"] = "SNP-BBBBBBBBBBBB"
    family["components"][1]["source_snapshot"] = "SNP-DDDDDDDDDDDD"
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
    dist_info = "ait_python-1.1.4.dist-info"
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
        "ait_py/ait_py.abi3.so": macho_image(),
        "ait_python/cli.py": b"def main():\n    return 0\n",
        f"{dist_info}/entry_points.txt": b"[console_scripts]\nait = ait_python.cli:main\n",
        f"{dist_info}/METADATA": (
            b"Metadata-Version: 2.4\n"
            b"Name: ait-python\n"
            b"Version: 1.1.4\n"
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

    normalize_wheel(wheel_a, "1.1.4", timestamp)
    normalize_wheel(wheel_b, "1.1.4", timestamp)

    assert wheel_a.read_bytes() == wheel_b.read_bytes()
    first_normalized = wheel_a.read_bytes()
    normalize_wheel(wheel_a, "1.1.4", timestamp)
    assert wheel_a.read_bytes() == first_normalized

    dist_info = "ait_python-1.1.4.dist-info"
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
    wheel_name = expected_wheel_name(target, "1.1.4")
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
    normalize_wheel(valid_wheel, "1.1.4", timestamp)
    verify_wheel(valid_wheel, target, "1.1.4")

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
    normalize_wheel(altered_wheel, "1.1.4", timestamp)
    with pytest.raises(error_type, match="NOTICE differs"):
        verify_wheel(altered_wheel, target, "1.1.4")


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
        normalize_wheel(missing_sbom, "1.1.4", timestamp)


PYTHON_LAUNCHER = b"#!python\nimport sys\nfrom ait_python.cli import main\nsys.exit(main())\n"


def macho_image(*, kind: int = 6, initializer: bytes | None = b"_PyInit_ait_py", public: bool = True, trie: bool = False) -> bytes:
    name = initializer or b"unrelated_export"
    if trie:
        root = b"\0\1" + name + b"\0" + bytes([len(name) + 4])
        exports = root + b"\2\0\x10\0"
        header = struct.pack("<8I", 0xfeedfacf, 0x100000c, 0, kind, 1, 16, 0, 0)
        return header + struct.pack("<4I", 0x80000033, 16, 48, len(exports)) + exports
    strings = b"\0" + name + b"\0"
    header = struct.pack("<8I", 0xfeedfacf, 0x100000c, 0, kind, 1, 24, 0, 0)
    symbols = struct.pack("<IBBHQ", 1, 0xf if public else 0x1f, 1, 0, 0x1000)
    return header + struct.pack("<6I", 2, 24, 56, 1, 72, len(strings)) + symbols + strings + b"_PyInit_ait_py\0"


def elf_image(*, kind: int = 3, entry: int = 0, initializer: bytes | None = b"PyInit_ait_py",
              defined: bool = True, public: bool = True, visibility: int = 0) -> bytes:
    strings = b"\0" + (initializer or b"unrelated_export") + b"\0"
    symbols = b"\0" * 24 + struct.pack("<IBBHQQ", 1, 0x12 if public else 2, visibility, 1 if defined else 0, 0x1000, 1)
    symbol_start = 64 + len(strings)
    section_start = symbol_start + len(symbols)
    header = struct.pack("<16sHHIQQQIHHHHHH", b"\x7fELF\x02\x01\x01" + b"\0" * 9,
                         kind, 183, 1, entry, 0, section_start, 0, 64, 0, 0, 64, 3, 0)
    sections = b"\0" * 64
    sections += struct.pack("<IIQQQQIIQQ", 0, 3, 0, 0, 64, len(strings), 0, 0, 1, 0)
    sections += struct.pack("<IIQQQQIIQQ", 0, 11, 0, 0, symbol_start, len(symbols), 1, 0, 8, 24)
    return header + strings + symbols + sections + b"PyInit_ait_py\0"


def pe_image(*, library: bool, initializer: bytes | None = b"PyInit_ait_py", defined: bool = True) -> bytes:
    image = bytearray(1024)
    image[:2] = b"MZ"
    image[60:64] = (64).to_bytes(4, "little")
    image[64:68] = b"PE\0\0"
    struct.pack_into("<HHIIIHH", image, 68, 0x8664, 1, 0, 0, 0, 240, 0x2002 if library else 2)
    struct.pack_into("<H", image, 88, 0x20b)
    struct.pack_into("<I", image, 88 + 60, 512)
    struct.pack_into("<I", image, 88 + 108, 16)
    struct.pack_into("<II", image, 88 + 112, 0x1000, 0xa0)
    struct.pack_into("<8sIIIIIIHHI", image, 328, b".text\0\0\0", 512, 0x1000, 512, 512, 0, 0, 0, 0, 0x60000020)
    struct.pack_into("<IIIII", image, 512 + 20, 1, 1, 0x1040, 0x1044, 0x1048)
    struct.pack_into("<IIH", image, 512 + 64, 0x1100 if defined else 0, 0x1050, 0)
    name = (initializer or b"unrelated_export") + b"\0"
    image[512 + 80:512 + 80 + len(name)] = name
    image[768] = 0xc3
    return bytes(image) + b"PyInit_ait_py\0"


def language_cli_archive(
    payloads: dict[str, bytes] | None = None, *, entrypoint: bytes | None = b"[console_scripts]\nait = ait_python.cli:main\n"
) -> io.BytesIO:
    output = io.BytesIO()
    members = {
        "ait_python/cli.py": b"def main():\n    return 0\n",
        "ait_py/ait_py.abi3.so": macho_image(),
    }
    if entrypoint is not None:
        members["ait_native-1.2.0.dist-info/entry_points.txt"] = entrypoint
    members.update(payloads or {})
    with zipfile.ZipFile(output, "w") as archive:
        for name, data in members.items():
            info = zipfile.ZipInfo(name)
            # Native bindings and Python launchers may legitimately have executable mode.
            info.create_system = 3
            info.external_attr = 0o100755 << 16
            archive.writestr(info, data)
    output.seek(0)
    return output


def elf_pie() -> bytes:
    image = bytearray(120)
    image[:6] = b"\x7fELF\x02\x01"
    image[16:18] = (3).to_bytes(2, "little")
    image[32:40] = (64).to_bytes(8, "little")
    image[54:56] = (56).to_bytes(2, "little")
    image[56:58] = (1).to_bytes(2, "little")
    image[64:68] = (3).to_bytes(4, "little")
    return bytes(image)


def test_wheel_gate_keeps_text_scripts_console_metadata_and_qualified_bindings() -> None:
    validate = release_adapter_namespace()["validate_language_cli_payload"]
    payload = language_cli_archive({
        "ait_native-1.2.0.data/scripts/ait": PYTHON_LAUNCHER,
        "ait_native-1.2.0.data/scripts/ait-script.py": PYTHON_LAUNCHER,
        "ait_py/ait_py.pyd": pe_image(library=True),
    })
    with zipfile.ZipFile(payload) as archive:
        validate(archive, "ait_native-1.2.0.dist-info")


def test_wheel_gate_rejects_embedded_windows_launcher_even_with_python_companion() -> None:
    adapter = release_adapter_namespace()
    windows_launcher = io.BytesIO(pe_image(library=False))
    windows_launcher.seek(0, io.SEEK_END)
    with zipfile.ZipFile(windows_launcher, "a") as launcher:
        launcher.writestr("__main__.py", PYTHON_LAUNCHER)
    with zipfile.ZipFile(language_cli_archive({
        "ait_native-1.2.0.data/scripts/ait.exe": windows_launcher.getvalue(),
        "ait_native-1.2.0.data/scripts/ait-script.py": PYTHON_LAUNCHER,
    })) as archive:
        with pytest.raises(adapter["ReleaseAdapterError"], match="standalone executable"):
            adapter["validate_language_cli_payload"](archive, "ait_native-1.2.0.dist-info")


@pytest.mark.parametrize("payload", [macho_image(), macho_image(kind=8), macho_image(trie=True), elf_image(), pe_image(library=True)])
def test_native_binding_image_requires_a_shared_image_and_real_initializer_export(payload: bytes) -> None:
    release_adapter_namespace()["validate_python_extension_image"](payload)


@pytest.mark.parametrize("payload", [
    macho_image(kind=2), macho_image(initializer=None), macho_image(public=False), macho_image(trie=True, initializer=None),
    elf_image(kind=2), elf_image(entry=0x1000), elf_image(initializer=None), elf_image(defined=False), elf_image(public=False),
    elf_image(visibility=2), pe_image(library=False), pe_image(library=True, initializer=None), pe_image(library=True, defined=False),
    b"native fixture PyInit_ait_py", b"\xcf\xfa\xed\xfe", b"MZ", b"\x7fELF",
])
def test_native_binding_gate_rejects_executables_undefined_or_bait_exports(payload: bytes) -> None:
    adapter = release_adapter_namespace()
    with pytest.raises(adapter["ReleaseAdapterError"]):
        adapter["validate_python_extension_image"](payload)


@pytest.mark.parametrize("member,payload", [
    ("ait_native-1.2.0.data/scripts/ait", b"\xcf\xfa\xed\xfe" + b"\0" * 8 + (2).to_bytes(4, "little")),
    ("ait_native-1.2.0.data/scripts/ait-server", b"native command fixture"),
    ("ait_native-1.2.0.data/scripts/ait-runner.exe", pe_image(library=False)),
    ("ait_python/assets/helper", b"\x7fELF\x02\x01" + b"\0" * 10 + (2).to_bytes(2, "little")),
    ("ait_python/assets/helper.exe", pe_image(library=False)),
    ("ait_python/assets/renamed-library", macho_image()),
    ("ait_native-1.2.0.data/scripts/ait.exe", PYTHON_LAUNCHER),
    ("ait_python/assets/renamed-pie", elf_pie()),
    ("ait_native-1.2.0.data/scripts/ait", b"#!/bin/sh\nexec ait \"$@\"\n"),
    ("ait_python/cli.py", b"import subprocess as child\ndef main(): return child.run(['ait'])\n"),
])
def test_wheel_gate_rejects_native_commands_and_process_relays(member: str, payload: bytes) -> None:
    adapter = release_adapter_namespace()
    with zipfile.ZipFile(language_cli_archive({member: payload})) as archive:
        with pytest.raises(adapter["ReleaseAdapterError"], match="standalone executable|relay"):
            adapter["validate_language_cli_payload"](archive, "ait_native-1.2.0.dist-info")


@pytest.mark.parametrize("member,payload", [
    ("ait_py/ait_py.abi3.so", macho_image(kind=2)),
    ("ait_py/ait_py.pyd", pe_image(library=False)),
    ("ait_py/ait_py.pyd", pe_image(library=True, initializer=None)),
    ("ait_py/ait_py.abi3.so", elf_image(entry=0x1000)),
])
def test_wheel_gate_validates_every_binding_candidate_beyond_its_filename(member: str, payload: bytes) -> None:
    adapter = release_adapter_namespace()
    with zipfile.ZipFile(language_cli_archive({member: payload})) as archive:
        with pytest.raises(adapter["ReleaseAdapterError"], match="shared|PyInit"):
            adapter["validate_language_cli_payload"](archive, "ait_native-1.2.0.dist-info")


@pytest.mark.parametrize("entrypoint", [
    None,
    b"[console_scripts]\n",
    b"[console_scripts]\nait = another.cli:main\n",
    b"[console_scripts]\nait = ait_python.cli:main\nait-server = server:main\n",
])
def test_wheel_gate_rejects_missing_or_replaced_language_entry(entrypoint: bytes | None) -> None:
    adapter = release_adapter_namespace()
    with zipfile.ZipFile(language_cli_archive(entrypoint=entrypoint)) as archive:
        with pytest.raises(adapter["ReleaseAdapterError"], match="console"):
            adapter["validate_language_cli_payload"](archive, "ait_native-1.2.0.dist-info")


@pytest.mark.parametrize("source", [
    b"import subprocess as relay\nrelay.run(['ait'])\n",
    b"from os import execv as relay\nrelay('ait', ['ait'])\n",
    b"runtime.call('run_cli', argv)\n",
    b"runtime.call('current_source_cache_run_cli', argv)\n",
    b"from ait_py import entry_with_args as maintenance\nmaintenance(argv)\n",
    b"extension.currentSourceCacheRunCli(argv)\n",
    b"extension.entry_with_args(argv)\n",
    b"import importlib\nimportlib.import_module('subprocess').run(['ait'])\n",
])
def test_frontend_guard_rejects_known_argv_and_process_backdoors(source: bytes) -> None:
    adapter = release_adapter_namespace()
    with pytest.raises(adapter["ReleaseAdapterError"], match="relay"):
        adapter["validate_python_frontend"](source, "frontend.py")


def test_frontend_guard_checks_calls_rather_than_documentation_words() -> None:
    adapter = release_adapter_namespace()
    adapter["validate_python_frontend"](
        b'"""No subprocess or runCli fallback."""\nruntime.call("task_workflow_status", root)\n',
        "frontend.py",
    )
