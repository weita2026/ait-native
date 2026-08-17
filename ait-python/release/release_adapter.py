#!/usr/bin/env python3
"""Build and smoke-test exact ait-python release wheels without a shell."""

from __future__ import annotations

import base64
import csv
from dataclasses import dataclass
from datetime import datetime, timezone
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import platform
import shutil
import subprocess
import sys
import tomllib
import venv
import zipfile


ROOT = Path(__file__).resolve().parents[1]
PYPROJECT_PATH = ROOT / "pyproject.toml"
INTERNAL_AIT_PY_MANIFEST = (
    ".ait-external/ait-core/rust/crates/ait-py/Cargo.toml"
)
PUBLIC_AIT_PY_MANIFEST = "../ait-core/rust/crates/ait-py/Cargo.toml"
WHEEL_DIRECTORY = ROOT / "dist" / "wheels"
ABI3_FEATURE = "pyo3/abi3-py311"
ABI3_PYTHON_TAG = "cp311"
ABI3_ABI_TAG = "abi3"
MATURIN_VERSION = "1.13.3"
ZIGLANG_VERSION = "0.15.2"
WHEEL_NORMALIZATION_CONTRACT = "ait.python.wheel-normalization.v1"
CANONICAL_SBOM_SOURCE_ROOT = (
    "path+file:///ait-release-source/.ait-external/ait-core"
)
SBOM_SOURCE_ROOT_MARKERS = (
    "/.ait-external/ait-core",
    "/ait-core",
)
ZIP_MIN_YEAR = 1980
ZIP_MAX_YEAR = 2107


class ReleaseAdapterError(RuntimeError):
    """A bounded release-contract violation."""


@dataclass(frozen=True, slots=True)
class TargetSpec:
    platform_tag: str
    compatibility: str
    system: str
    machine: str
    macos_deployment_target: str | None = None


@dataclass(frozen=True, slots=True)
class ReleaseIdentity:
    target: TargetSpec
    manifest_path: Path
    manifest_path_argument: str


TARGETS = {
    "aarch64-apple-darwin": TargetSpec(
        "macosx_11_0_arm64", "pypi", "darwin", "aarch64", "11.0"
    ),
    "x86_64-apple-darwin": TargetSpec(
        "macosx_10_12_x86_64", "pypi", "darwin", "x86_64", "10.12"
    ),
    "aarch64-unknown-linux-gnu": TargetSpec(
        "manylinux_2_28_aarch64", "manylinux_2_28", "linux", "aarch64"
    ),
    "x86_64-unknown-linux-gnu": TargetSpec(
        "manylinux_2_28_x86_64", "manylinux_2_28", "linux", "x86_64"
    ),
    "aarch64-pc-windows-msvc": TargetSpec(
        "win_arm64", "pypi", "windows", "aarch64"
    ),
    "x86_64-pc-windows-msvc": TargetSpec(
        "win_amd64", "pypi", "windows", "x86_64"
    ),
}


def expected_wheel_name(target: str, version: str) -> str:
    spec = target_spec(target)
    return (
        f"ait_python-{version}-{ABI3_PYTHON_TAG}-{ABI3_ABI_TAG}-"
        f"{spec.platform_tag}.whl"
    )


def expected_wheel_path(target: str, version: str) -> Path:
    return WHEEL_DIRECTORY / expected_wheel_name(target, version)


def target_spec(target: str) -> TargetSpec:
    try:
        return TARGETS[target]
    except KeyError as error:
        accepted = ", ".join(sorted(TARGETS))
        raise ReleaseAdapterError(
            f"unsupported release target {target!r}; expected one of: {accepted}"
        ) from error


def normalized_host() -> tuple[str, str]:
    system = platform.system().strip().lower()
    machine = platform.machine().strip().lower()
    machine_aliases = {
        "amd64": "x86_64",
        "arm64": "aarch64",
    }
    return system, machine_aliases.get(machine, machine)


def require_native_target(target: str) -> TargetSpec:
    spec = target_spec(target)
    host = normalized_host()
    expected = (spec.system, spec.machine)
    if host != expected:
        raise ReleaseAdapterError(
            f"target {target} requires native host {expected[0]}/{expected[1]}, "
            f"got {host[0]}/{host[1]}"
        )
    return spec


def read_project() -> dict[str, object]:
    return tomllib.loads(PYPROJECT_PATH.read_text(encoding="utf-8"))


def snapshot_id(value: object) -> str | None:
    if (
        isinstance(value, str)
        and len(value) == 16
        and value.startswith("SNP-")
        and all(character in "0123456789ABCDEF" for character in value[4:])
    ):
        return value
    return None


def read_json_object(path: Path, label: str) -> dict[str, object]:
    if not path.is_file() or path.is_symlink():
        raise ReleaseAdapterError(f"{label} must be a real file")
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ReleaseAdapterError(f"{label} must contain valid UTF-8 JSON") from error
    if not isinstance(value, dict):
        raise ReleaseAdapterError(f"{label} must contain a JSON object")
    return value


def rows_by_repository(mapping: dict[str, object]) -> dict[str, dict[str, object]]:
    subtrees = mapping.get("subtrees")
    if not isinstance(subtrees, list):
        raise ReleaseAdapterError("public source mapping subtrees must be an array")
    rows: dict[str, dict[str, object]] = {}
    for row in subtrees:
        if not isinstance(row, dict) or not isinstance(
            row.get("source_repository"), str
        ):
            raise ReleaseAdapterError("public source mapping subtree is invalid")
        repository = row["source_repository"]
        if repository in rows:
            raise ReleaseAdapterError(
                f"public source mapping repeats repository {repository!r}"
            )
        rows[repository] = row
    return rows


def component_snapshot(
    family: dict[str, object], component_id: str, repository: str
) -> str:
    components = family.get("components")
    if not isinstance(components, list):
        raise ReleaseAdapterError("public family components must be an array")
    matches = [
        row
        for row in components
        if isinstance(row, dict) and row.get("id") == component_id
    ]
    if len(matches) != 1 or matches[0].get("source_repository") != repository:
        raise ReleaseAdapterError(
            f"public family component {component_id!r} has invalid source authority"
        )
    selected = snapshot_id(matches[0].get("source_snapshot"))
    if selected is None:
        raise ReleaseAdapterError(
            f"public family component {component_id!r} has invalid Snapshot authority"
        )
    return selected


def validate_public_monorepo_authority(
    mapping: dict[str, object], family: dict[str, object]
) -> None:
    if (
        mapping.get("schema") != "ait.release.monorepo-source/v1"
        or mapping.get("public_source_identity") != "weita2026/ait-native"
        or mapping.get("public_publish") is not False
        or snapshot_id(mapping.get("coordinator_snapshot")) is None
        or family.get("schema") != "ait.release.family/v3"
    ):
        raise ReleaseAdapterError("public monorepo source identity is invalid")
    rows = rows_by_repository(mapping)
    if set(rows) != {
        "ait-core",
        "ait-server",
        "ait-runner",
        "ait-python",
        "ait-node",
    }:
        raise ReleaseAdapterError("public source mapping repository set is invalid")
    core_row = rows["ait-core"]
    python_row = rows["ait-python"]
    if (
        core_row.get("path") != "ait-core"
        or core_row.get("license") != "Apache-2.0"
        or core_row.get("components")
        != ["ait", "ait-agent", "ait-agent-worker"]
        or core_row.get("transforms") != []
        or python_row.get("path") != "ait-python"
        or python_row.get("license") != "Apache-2.0"
        or python_row.get("components") != ["ait-python"]
        or python_row.get("transforms") != ["python-core-path/v1"]
    ):
        raise ReleaseAdapterError("public Python/core source mapping is invalid")
    core_snapshot = snapshot_id(core_row.get("source_snapshot"))
    python_snapshot = snapshot_id(python_row.get("source_snapshot"))
    if core_snapshot is None or python_snapshot is None:
        raise ReleaseAdapterError("public Python/core Snapshot mapping is invalid")
    public_source = family.get("public_source")
    if not isinstance(public_source, dict) or (
        public_source.get("model") != "release-monorepo"
        or public_source.get("identity") != "weita2026/ait-native"
    ):
        raise ReleaseAdapterError("public family source identity is invalid")
    transforms = public_source.get("transforms")
    expected_transform = {
        "id": "python-core-path/v1",
        "source_repository": "ait-python",
        "path": "pyproject.toml",
        "from": INTERNAL_AIT_PY_MANIFEST,
        "to": PUBLIC_AIT_PY_MANIFEST,
    }
    if not isinstance(transforms, list) or [
        row
        for row in transforms
        if isinstance(row, dict) and row.get("id") == "python-core-path/v1"
    ] != [expected_transform]:
        raise ReleaseAdapterError("public Python core-path transform is invalid")
    if (
        component_snapshot(family, "ait", "ait-core") != core_snapshot
        or component_snapshot(family, "ait-agent", "ait-core") != core_snapshot
        or component_snapshot(family, "ait-agent-worker", "ait-core") != core_snapshot
        or component_snapshot(family, "ait-python", "ait-python")
        != python_snapshot
    ):
        raise ReleaseAdapterError(
            "public family component Snapshots differ from the source mapping"
        )


def validate_external_materialization(root: Path, manifest_path: Path) -> None:
    lock_path = root / "ait-external.lock"
    marker_path = (
        root / ".ait-external" / "ait-core" / ".ait-external-marker.json"
    )
    lockfile = tomllib.loads(lock_path.read_text(encoding="utf-8"))
    nodes = lockfile.get("node")
    if not isinstance(nodes, list) or len(nodes) != 1:
        raise ReleaseAdapterError("ait-external.lock must contain exactly one node")
    marker = read_json_object(marker_path, "external materialization marker")
    node = nodes[0]
    if not isinstance(node, dict):
        raise ReleaseAdapterError("external lock node must be an object")
    for field in (
        "name",
        "repo_name",
        "repository_index",
        "snapshot",
        "materialize_to",
    ):
        if marker.get(field) != node.get(field):
            raise ReleaseAdapterError(
                f"external marker field {field!r} does not match ait-external.lock"
            )
    if not manifest_path.is_file() or manifest_path.is_symlink():
        raise ReleaseAdapterError("locked ait-py Cargo.toml is missing")


def validate_public_monorepo_materialization(
    root: Path, manifest_path: Path
) -> None:
    if root.name != "ait-python":
        raise ReleaseAdapterError("public Python source subtree name is invalid")
    public_root = root.parent
    mapping = read_json_object(
        public_root / "ait-monorepo-source.json", "public source mapping"
    )
    family = read_json_object(
        public_root / "ait-release-family.json", "public family manifest"
    )
    validate_public_monorepo_authority(mapping, family)
    expected_manifest = public_root / "ait-core" / "rust" / "crates" / "ait-py" / "Cargo.toml"
    if not expected_manifest.is_file() or expected_manifest.is_symlink():
        raise ReleaseAdapterError("public monorepo ait-py Cargo.toml is missing")
    try:
        selected = manifest_path.resolve(strict=True)
        expected = expected_manifest.resolve(strict=True)
    except OSError as error:
        raise ReleaseAdapterError(
            "public monorepo ait-py Cargo.toml cannot be resolved"
        ) from error
    if selected != expected:
        raise ReleaseAdapterError(
            "public monorepo manifest path does not select the mapped ait-core"
        )


def validate_core_source_layout(declared_path: object, root: Path) -> Path:
    if declared_path == INTERNAL_AIT_PY_MANIFEST:
        manifest_path = root / INTERNAL_AIT_PY_MANIFEST
        validate_external_materialization(root, manifest_path)
        return manifest_path
    if declared_path == PUBLIC_AIT_PY_MANIFEST:
        manifest_path = root / PUBLIC_AIT_PY_MANIFEST
        validate_public_monorepo_materialization(root, manifest_path)
        return manifest_path
    raise ReleaseAdapterError("tool.maturin.manifest-path is not locked to ait-core")


def validate_release_identity(target: str, version: str) -> ReleaseIdentity:
    spec = require_native_target(target)
    pyproject = read_project()
    build_system = pyproject.get("build-system")
    project = pyproject.get("project")
    maturin = pyproject.get("tool", {}).get("maturin")
    if (
        not isinstance(build_system, dict)
        or not isinstance(project, dict)
        or not isinstance(maturin, dict)
    ):
        raise ReleaseAdapterError(
            "pyproject.toml lacks build-system, project, or tool.maturin authority"
        )
    if build_system.get("requires") != [f"maturin=={MATURIN_VERSION}"]:
        raise ReleaseAdapterError(
            f"build-system.requires must pin maturin=={MATURIN_VERSION}"
        )
    if sys.version_info < (3, 11):
        raise ReleaseAdapterError("release adapter requires Python 3.11 or newer")
    expected_project = {
        "name": "ait-python",
        "version": version,
        "license": "Apache-2.0",
        "requires-python": ">=3.11",
    }
    for field, expected in expected_project.items():
        if project.get(field) != expected:
            raise ReleaseAdapterError(
                f"pyproject project.{field} must be {expected!r}, "
                f"got {project.get(field)!r}"
            )
    if maturin.get("features") != [ABI3_FEATURE]:
        raise ReleaseAdapterError(
            f"tool.maturin.features must be exactly [{ABI3_FEATURE!r}]"
        )
    declared_manifest = maturin.get("manifest-path")
    manifest_path = validate_core_source_layout(declared_manifest, ROOT)
    return ReleaseIdentity(spec, manifest_path, str(declared_manifest))


def release_environment(spec: TargetSpec) -> dict[str, str]:
    environment = os.environ.copy()
    environment["PYTHONDONTWRITEBYTECODE"] = "1"
    environment["PIP_DISABLE_PIP_VERSION_CHECK"] = "1"
    if uses_zig_manylinux(spec):
        environment.pop("CARGO_ZIGBUILD_ZIG_PATH", None)
        environment["CARGO_ZIGBUILD_PYTHON_PATH"] = sys.executable
    if spec.macos_deployment_target is not None:
        environment["MACOSX_DEPLOYMENT_TARGET"] = spec.macos_deployment_target
    return environment


def require_tool(name: str) -> None:
    if shutil.which(name) is None:
        raise ReleaseAdapterError(f"required release tool {name!r} is unavailable")


def require_maturin() -> None:
    require_tool("maturin")
    completed = subprocess.run(
        ["maturin", "--version"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    actual = completed.stdout.strip()
    expected = f"maturin {MATURIN_VERSION}"
    if actual != expected:
        raise ReleaseAdapterError(
            f"release requires {expected!r}, got {actual!r}"
        )


def uses_zig_manylinux(spec: TargetSpec) -> bool:
    return spec.system == "linux" and spec.compatibility.startswith("manylinux_")


def require_ziglang() -> None:
    completed = subprocess.run(
        [sys.executable, "-m", "ziglang", "version"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    actual = completed.stdout.strip()
    if actual != ZIGLANG_VERSION:
        raise ReleaseAdapterError(
            f"release requires ziglang {ZIGLANG_VERSION!r}, got {actual!r}"
        )


def maturin_build_command(
    identity: ReleaseIdentity, target: str
) -> list[str]:
    command = [
        "maturin",
        "build",
        "--release",
        "--locked",
        "--target",
        target,
        "--target-dir",
        ".ait/cargo-target",
        "--features",
        ABI3_FEATURE,
        "--out",
        str(WHEEL_DIRECTORY.relative_to(ROOT)),
        "--compatibility",
        identity.target.compatibility,
    ]
    if uses_zig_manylinux(identity.target):
        command.append("--zig")
    command.extend(["--interpreter", sys.executable])
    return command


def run_command(argv: list[str], environment: dict[str, str]) -> None:
    subprocess.run(argv, cwd=ROOT, env=environment, check=True)


def parse_source_date_epoch(value: str | None) -> tuple[int, datetime]:
    if value is None or not value.isascii() or not value.isdigit():
        raise ReleaseAdapterError(
            "SOURCE_DATE_EPOCH must be a non-negative decimal Unix-seconds integer"
        )
    epoch = int(value)
    try:
        timestamp = datetime.fromtimestamp(epoch, timezone.utc)
    except (OSError, OverflowError, ValueError) as error:
        raise ReleaseAdapterError(
            "SOURCE_DATE_EPOCH is outside the supported UTC datetime range"
        ) from error
    if not ZIP_MIN_YEAR <= timestamp.year <= ZIP_MAX_YEAR:
        raise ReleaseAdapterError(
            "SOURCE_DATE_EPOCH must resolve to a ZIP timestamp from "
            f"{ZIP_MIN_YEAR} through {ZIP_MAX_YEAR}"
        )
    return epoch, timestamp


def require_source_date_epoch() -> tuple[int, datetime]:
    return parse_source_date_epoch(os.environ.get("SOURCE_DATE_EPOCH"))


def canonical_sbom_timestamp(timestamp: datetime) -> str:
    return timestamp.strftime("%Y-%m-%dT%H:%M:%S.000000000Z")


def sbom_source_suffix(value: str) -> str | None:
    normalized = value.replace("\\", "/")
    for marker in SBOM_SOURCE_ROOT_MARKERS:
        marker_index = normalized.find(marker)
        if marker_index < 0:
            continue
        suffix_index = marker_index + len(marker)
        if suffix_index == len(normalized) or normalized[suffix_index] == "/":
            return normalized[suffix_index:]
    return None


def canonical_sbom_reference(value: str) -> tuple[str, bool]:
    if not value.startswith("path+file://"):
        return value, False
    suffix = sbom_source_suffix(value)
    if suffix is None:
        return value, False
    return f"{CANONICAL_SBOM_SOURCE_ROOT}{suffix}", True


def normalize_sbom_value(value: object) -> tuple[object, int]:
    if isinstance(value, dict):
        normalized: dict[str, object] = {}
        matched = 0
        for key, child in value.items():
            normalized_child, child_matched = normalize_sbom_value(child)
            normalized[key] = normalized_child
            matched += child_matched
        return normalized, matched
    if isinstance(value, list):
        normalized_items = []
        matched = 0
        for child in value:
            normalized_child, child_matched = normalize_sbom_value(child)
            normalized_items.append(normalized_child)
            matched += child_matched
        return normalized_items, matched
    if isinstance(value, str):
        normalized, matched = canonical_sbom_reference(value)
        return normalized, int(matched)
    return value, 0


def iter_json_strings(value: object):
    if isinstance(value, dict):
        for child in value.values():
            yield from iter_json_strings(child)
    elif isinstance(value, list):
        for child in value:
            yield from iter_json_strings(child)
    elif isinstance(value, str):
        yield value


def validate_canonical_sbom(sbom: object) -> None:
    if not isinstance(sbom, dict):
        raise ReleaseAdapterError("wheel CycloneDX SBOM must be a JSON object")
    metadata = sbom.get("metadata")
    if not isinstance(metadata, dict) or not isinstance(
        metadata.get("timestamp"), str
    ):
        raise ReleaseAdapterError("wheel CycloneDX SBOM lacks metadata.timestamp")
    local_references = 0
    for value in iter_json_strings(sbom):
        if not value.startswith("path+file://"):
            continue
        if sbom_source_suffix(value) is None:
            continue
        local_references += 1
        if not value.startswith(CANONICAL_SBOM_SOURCE_ROOT):
            raise ReleaseAdapterError(
                "wheel CycloneDX SBOM exposes a non-canonical local source URI"
            )
    if local_references == 0:
        raise ReleaseAdapterError(
            "wheel CycloneDX SBOM lacks the expected ait-core path references"
        )


def normalize_sbom_bytes(data: bytes, timestamp: datetime) -> bytes:
    try:
        sbom = json.loads(data.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ReleaseAdapterError(
            "wheel CycloneDX SBOM must contain valid UTF-8 JSON"
        ) from error
    normalized, matched = normalize_sbom_value(sbom)
    if not isinstance(normalized, dict) or matched == 0:
        raise ReleaseAdapterError(
            "wheel CycloneDX SBOM lacks normalizable ait-core path references"
        )
    metadata = normalized.get("metadata")
    if not isinstance(metadata, dict):
        raise ReleaseAdapterError("wheel CycloneDX SBOM lacks metadata")
    metadata["timestamp"] = canonical_sbom_timestamp(timestamp)
    validate_canonical_sbom(normalized)
    return (
        json.dumps(
            normalized,
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
        ).encode("utf-8")
        + b"\n"
    )


def record_digest(data: bytes) -> str:
    encoded = base64.urlsafe_b64encode(hashlib.sha256(data).digest())
    return "sha256=" + encoded.rstrip(b"=").decode("ascii")


def wheel_record_bytes(
    names: list[str], payloads: dict[str, bytes], record_path: str
) -> bytes:
    output = io.StringIO(newline="")
    writer = csv.writer(output, lineterminator="\n")
    for name in names:
        if name == record_path:
            writer.writerow((name, "", ""))
        else:
            data = payloads[name]
            writer.writerow((name, record_digest(data), str(len(data))))
    return output.getvalue().encode("utf-8")


def validate_wheel_record(
    archive: zipfile.ZipFile, names: list[str], record_path: str
) -> None:
    try:
        record_text = archive.read(record_path).decode("utf-8")
    except UnicodeDecodeError as error:
        raise ReleaseAdapterError("wheel RECORD must be UTF-8") from error
    rows: dict[str, tuple[str, str]] = {}
    for row in csv.reader(io.StringIO(record_text, newline="")):
        if len(row) != 3 or not row[0] or row[0] in rows:
            raise ReleaseAdapterError("wheel RECORD contains a malformed row")
        rows[row[0]] = (row[1], row[2])
    if set(rows) != set(names):
        raise ReleaseAdapterError("wheel RECORD inventory does not match ZIP members")
    for name in names:
        digest, size = rows[name]
        if name == record_path:
            if digest or size:
                raise ReleaseAdapterError(
                    "wheel RECORD must leave its own digest and size empty"
                )
            continue
        data = archive.read(name)
        if digest != record_digest(data) or size != str(len(data)):
            raise ReleaseAdapterError(
                f"wheel RECORD digest or size mismatch for {name!r}"
            )


def validated_wheel_payloads(
    wheel: Path, version: str
) -> tuple[list[zipfile.ZipInfo], dict[str, bytes], str, str]:
    if wheel.is_symlink() or not wheel.is_file():
        raise ReleaseAdapterError(f"wheel must be a regular file: {wheel}")
    dist_info = f"ait_python-{version}.dist-info"
    record_path = f"{dist_info}/RECORD"
    sbom_path = f"{dist_info}/sboms/ait-py.cyclonedx.json"
    with zipfile.ZipFile(wheel) as archive:
        if archive.comment:
            raise ReleaseAdapterError("wheel ZIP archive comment is not admitted")
        infos = archive.infolist()
        names = [info.filename for info in infos]
        if len(names) != len(set(names)):
            raise ReleaseAdapterError("wheel contains duplicate member names")
        if record_path not in names or sbom_path not in names:
            raise ReleaseAdapterError(
                "wheel lacks its exact RECORD or CycloneDX SBOM control member"
            )
        for signature_path in (
            f"{record_path}.jws",
            f"{record_path}.p7s",
        ):
            if signature_path in names:
                raise ReleaseAdapterError(
                    "signed wheel RECORD cannot be rewritten during normalization"
                )
        for info in infos:
            member = PurePosixPath(info.filename)
            if member.is_absolute() or ".." in member.parts:
                raise ReleaseAdapterError(f"unsafe wheel member {info.filename!r}")
            member_type = (info.external_attr >> 16) & 0o170000
            if member_type == 0o120000:
                raise ReleaseAdapterError(
                    f"wheel member {info.filename!r} is a symlink"
                )
            if info.flag_bits & 0x1:
                raise ReleaseAdapterError(
                    f"wheel member {info.filename!r} is encrypted"
                )
            if info.compress_type not in {
                zipfile.ZIP_STORED,
                zipfile.ZIP_DEFLATED,
                zipfile.ZIP_BZIP2,
                zipfile.ZIP_LZMA,
            }:
                raise ReleaseAdapterError(
                    f"wheel member {info.filename!r} uses unsupported compression"
                )
        if archive.testzip() is not None:
            raise ReleaseAdapterError("wheel CRC verification failed")
        payloads = {info.filename: archive.read(info) for info in infos}
    return infos, payloads, record_path, sbom_path


def normalized_zip_info(
    source: zipfile.ZipInfo, timestamp: datetime
) -> zipfile.ZipInfo:
    info = zipfile.ZipInfo(
        filename=source.filename,
        date_time=(
            timestamp.year,
            timestamp.month,
            timestamp.day,
            timestamp.hour,
            timestamp.minute,
            timestamp.second,
        ),
    )
    info.compress_type = source.compress_type
    info.create_system = source.create_system
    info.create_version = source.create_version
    info.extract_version = source.extract_version
    info.internal_attr = source.internal_attr
    info.external_attr = source.external_attr
    info.flag_bits = source.flag_bits & 0x800
    info.extra = b""
    info.comment = b""
    return info


def normalize_wheel(wheel: Path, version: str, timestamp: datetime) -> None:
    infos, payloads, record_path, sbom_path = validated_wheel_payloads(
        wheel, version
    )
    names = [info.filename for info in infos]
    payloads[sbom_path] = normalize_sbom_bytes(payloads[sbom_path], timestamp)
    payloads[record_path] = wheel_record_bytes(names, payloads, record_path)
    temporary = wheel.with_name(f".{wheel.name}.normalize.tmp")
    if temporary.exists() or temporary.is_symlink():
        raise ReleaseAdapterError(
            f"wheel normalization temporary path already exists: {temporary}"
        )
    source_mode = wheel.stat().st_mode & 0o777
    try:
        with zipfile.ZipFile(temporary, mode="x", allowZip64=True) as archive:
            for source in infos:
                info = normalized_zip_info(source, timestamp)
                compresslevel = (
                    9
                    if source.compress_type
                    in {zipfile.ZIP_DEFLATED, zipfile.ZIP_BZIP2}
                    else None
                )
                archive.writestr(
                    info,
                    payloads[source.filename],
                    compress_type=source.compress_type,
                    compresslevel=compresslevel,
                )
        os.chmod(temporary, source_mode)
        os.replace(temporary, wheel)
    finally:
        if temporary.exists() or temporary.is_symlink():
            temporary.unlink()


def check_release(target: str, version: str) -> dict[str, object]:
    identity = validate_release_identity(target, version)
    spec = identity.target
    require_tool("cargo")
    require_maturin()
    run_command(
        [
            "cargo",
            "check",
            "--manifest-path",
            identity.manifest_path_argument,
            "--locked",
            "--release",
            "--target",
            target,
            "--target-dir",
            ".ait/cargo-target",
            "--features",
            ABI3_FEATURE,
            "--lib",
        ],
        release_environment(spec),
    )
    return {
        "action": "check",
        "abi": f"{ABI3_PYTHON_TAG}-{ABI3_ABI_TAG}",
        "status": "pass",
        "target": target,
        "version": version,
    }


def build_release(target: str, version: str) -> dict[str, object]:
    identity = validate_release_identity(target, version)
    spec = identity.target
    source_date_epoch, release_timestamp = require_source_date_epoch()
    require_maturin()
    if uses_zig_manylinux(spec):
        require_ziglang()
    WHEEL_DIRECTORY.mkdir(parents=True, exist_ok=True)
    existing_wheels = sorted(WHEEL_DIRECTORY.glob("*.whl"))
    if existing_wheels:
        raise ReleaseAdapterError(
            "wheel output directory must start empty: "
            + ", ".join(path.name for path in existing_wheels)
        )
    environment = release_environment(spec)
    environment["SOURCE_DATE_EPOCH"] = str(source_date_epoch)
    run_command(maturin_build_command(identity, target), environment)
    wheel = expected_wheel_path(target, version)
    produced = sorted(WHEEL_DIRECTORY.glob("*.whl"))
    if produced != [wheel]:
        raise ReleaseAdapterError(
            f"Maturin produced {[path.name for path in produced]!r}; "
            f"expected only {wheel.name!r}"
        )
    normalize_wheel(wheel, version, release_timestamp)
    verify_wheel(wheel, target, version)
    return {
        "action": "build",
        "normalization_contract": WHEEL_NORMALIZATION_CONTRACT,
        "sha256": sha256_file(wheel),
        "size_bytes": wheel.stat().st_size,
        "source_date_epoch": source_date_epoch,
        "status": "pass",
        "target": target,
        "version": version,
        "wheel": str(wheel.relative_to(ROOT)),
        "ziglang_version": ZIGLANG_VERSION if uses_zig_manylinux(spec) else None,
    }


def verify_wheel(wheel: Path, target: str, version: str) -> None:
    spec = target_spec(target)
    if wheel.name != expected_wheel_name(target, version) or not wheel.is_file():
        raise ReleaseAdapterError(f"expected wheel is missing: {wheel}")
    dist_info = f"ait_python-{version}.dist-info"
    record_path = f"{dist_info}/RECORD"
    sbom_path = f"{dist_info}/sboms/ait-py.cyclonedx.json"
    with zipfile.ZipFile(wheel) as archive:
        infos = archive.infolist()
        names = [info.filename for info in infos]
        if len(names) != len(set(names)):
            raise ReleaseAdapterError("wheel contains duplicate member names")
        for info in infos:
            member = PurePosixPath(info.filename)
            if member.is_absolute() or ".." in member.parts:
                raise ReleaseAdapterError(f"unsafe wheel member {info.filename!r}")
            member_type = (info.external_attr >> 16) & 0o170000
            if member_type == 0o120000:
                raise ReleaseAdapterError(f"wheel member {info.filename!r} is a symlink")
        if archive.testzip() is not None:
            raise ReleaseAdapterError("wheel CRC verification failed")
        if archive.comment or any(info.extra or info.comment for info in infos):
            raise ReleaseAdapterError(
                "normalized wheel must not contain ZIP comments or extra fields"
            )
        if len({info.date_time for info in infos}) != 1:
            raise ReleaseAdapterError(
                "normalized wheel members do not share one release timestamp"
            )
        if any(
            "__pycache__" in name or name.endswith((".pyc", ".pyo"))
            for name in names
        ):
            raise ReleaseAdapterError("wheel contains interpreter-specific bytecode")
        required_members = {
            "ait_py/__init__.py",
            f"{dist_info}/METADATA",
            f"{dist_info}/WHEEL",
            f"{dist_info}/licenses/LICENSE",
            f"{dist_info}/licenses/NOTICE",
            record_path,
            sbom_path,
        }
        missing = sorted(required_members.difference(names))
        if missing:
            raise ReleaseAdapterError(f"wheel lacks required members: {missing}")
        for legal_name in ("LICENSE", "NOTICE"):
            member = f"{dist_info}/licenses/{legal_name}"
            if archive.read(member) != (ROOT / legal_name).read_bytes():
                raise ReleaseAdapterError(
                    f"wheel {legal_name} differs from the repository authority"
                )
        extension_suffix = ".pyd" if spec.system == "windows" else ".so"
        if not any(
            name.startswith("ait_py/ait_py") and name.endswith(extension_suffix)
            for name in names
        ):
            raise ReleaseAdapterError("wheel lacks the native ait_py extension")
        metadata = archive.read(f"{dist_info}/METADATA").decode("utf-8")
        wheel_metadata = archive.read(f"{dist_info}/WHEEL").decode("utf-8")
        validate_wheel_record(archive, names, record_path)
        try:
            sbom = json.loads(archive.read(sbom_path).decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise ReleaseAdapterError(
                "wheel CycloneDX SBOM must contain valid UTF-8 JSON"
            ) from error
        validate_canonical_sbom(sbom)
    required_metadata = (
        "Name: ait-python",
        f"Version: {version}",
        "License-Expression: Apache-2.0",
        "License-File: LICENSE",
        "License-File: NOTICE",
        "Requires-Python: >=3.11",
    )
    for row in required_metadata:
        if row not in metadata.splitlines():
            raise ReleaseAdapterError(f"wheel METADATA lacks exact row {row!r}")
    expected_tag = f"Tag: {ABI3_PYTHON_TAG}-{ABI3_ABI_TAG}-{spec.platform_tag}"
    if expected_tag not in wheel_metadata.splitlines():
        raise ReleaseAdapterError(f"wheel metadata lacks exact tag {expected_tag!r}")


def smoke_release(target: str, version: str) -> dict[str, object]:
    identity = validate_release_identity(target, version)
    spec = identity.target
    wheel = expected_wheel_path(target, version)
    verify_wheel(wheel, target, version)
    smoke_root = ROOT / ".ait" / "release-smoke" / target
    if smoke_root.exists():
        raise ReleaseAdapterError(f"smoke environment already exists: {smoke_root}")
    venv.EnvBuilder(with_pip=True).create(smoke_root)
    python = smoke_root / (
        "Scripts/python.exe" if spec.system == "windows" else "bin/python"
    )
    environment = release_environment(spec)
    run_command(
        [
            str(python),
            "-m",
            "pip",
            "install",
            "--no-cache-dir",
            "--no-index",
            "--no-deps",
            str(wheel),
        ],
        environment,
    )
    smoke_code = (
        "from importlib.metadata import version; "
        "import ait_py, ait_python; "
        f"assert version('ait-python') == {version!r}; "
        f"assert ait_python.__version__ == {version!r}; "
        "assert ait_python.NativeRuntime().binding_info()['runtime_authority'] == 'rust'"
    )
    run_command([str(python), "-c", smoke_code], environment)
    run_command([str(python), "-m", "pip", "check"], environment)
    return {
        "action": "smoke",
        "installed_version": version,
        "status": "pass",
        "target": target,
        "wheel": str(wheel.relative_to(ROOT)),
    }


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main(argv: list[str]) -> int:
    if len(argv) != 4 or argv[1] not in {"check", "build", "smoke"}:
        print(
            "usage: release_adapter.py {check|build|smoke} <target> <version>",
            file=sys.stderr,
        )
        return 64
    action, target, version = argv[1:]
    actions = {
        "check": check_release,
        "build": build_release,
        "smoke": smoke_release,
    }
    try:
        result = actions[action](target, version)
    except (
        OSError,
        ReleaseAdapterError,
        subprocess.CalledProcessError,
        zipfile.BadZipFile,
    ) as error:
        print(f"ait-python release adapter failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
