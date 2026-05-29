from __future__ import annotations

import base64
import hashlib
import io
import json
import os
import re
import stat
import sys
import tarfile
import tempfile
import tomllib
import urllib.parse
import zipfile
from contextlib import contextmanager
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from .release_ops import (
    _materialize_bundle,
    _merge_metadata,
    _pyproject_metadata,
    _release_epoch,
    _release_source_bundle,
    _require_profile,
    _with_release_notes_readme,
    _workspace_matches_release_source,
    get_release_candidate,
)
from .repo_paths import RepoContext
from .store_local_releases import update_local_release


@contextmanager
def _build_environment(epoch: int):
    original_source_date_epoch = os.environ.get("SOURCE_DATE_EPOCH")
    original_pythonhashseed = os.environ.get("PYTHONHASHSEED")
    os.environ["SOURCE_DATE_EPOCH"] = str(epoch)
    os.environ["PYTHONHASHSEED"] = "0"
    try:
        yield
    finally:
        if original_source_date_epoch is None:
            os.environ.pop("SOURCE_DATE_EPOCH", None)
        else:
            os.environ["SOURCE_DATE_EPOCH"] = original_source_date_epoch
        if original_pythonhashseed is None:
            os.environ.pop("PYTHONHASHSEED", None)
        else:
            os.environ["PYTHONHASHSEED"] = original_pythonhashseed


def _artifact_kind(path: Path) -> str:
    if path.name.endswith(".tar.gz"):
        return "sdist"
    if path.suffix == ".whl":
        return "wheel"
    if path.name.endswith(".manifest.json"):
        return "manifest"
    if path.name.endswith(".sha256"):
        return "checksum"
    if path.suffix == ".rb":
        return "formula"
    return "artifact"


def _artifact_info(path: Path) -> dict[str, Any]:
    data = path.read_bytes()
    return {
        "kind": _artifact_kind(path),
        "path": path.as_posix(),
        "absolute_path": str(path.resolve()),
        "url": path.resolve().as_uri(),
        "size_bytes": len(data),
        "sha256": hashlib.sha256(data).hexdigest(),
    }


def _replace_artifact(existing: list[dict[str, Any]], artifact: dict[str, Any]) -> list[dict[str, Any]]:
    retained = [row for row in existing if str(row.get("kind") or "") != str(artifact.get("kind") or "")]
    retained.append(artifact)
    retained.sort(key=lambda row: str(row.get("kind") or ""))
    return retained


def _distribution_name(name: str, *, wheel_safe: bool) -> str:
    text = str(name or "").strip()
    if wheel_safe:
        return re.sub(r"[-.]+", "_", text)
    return text


def _zip_timestamp(epoch: int) -> tuple[int, int, int, int, int, int]:
    clamped = max(epoch, 315532800)
    dt = datetime.fromtimestamp(clamped, tz=timezone.utc)
    return (dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second)


def _wheel_source_root(source_dir: Path) -> Path:
    data = tomllib.loads((source_dir / "pyproject.toml").read_text(encoding="utf-8"))
    tool = data.get("tool") if isinstance(data.get("tool"), dict) else {}
    setuptools_cfg = tool.get("setuptools") if isinstance(tool.get("setuptools"), dict) else {}
    package_dir = setuptools_cfg.get("package-dir") if isinstance(setuptools_cfg.get("package-dir"), dict) else {}
    candidate = package_dir.get("") if isinstance(package_dir.get(""), str) else None
    if candidate:
        root = source_dir / candidate
        if root.exists():
            return root
    fallback = source_dir / "src"
    if fallback.exists():
        return fallback
    return source_dir


def _iter_installable_files(source_root: Path) -> list[tuple[Path, str]]:
    entries: list[tuple[Path, str]] = []
    for path in sorted(source_root.rglob("*")):
        if not path.is_file():
            continue
        if "__pycache__" in path.parts:
            continue
        arcname = path.relative_to(source_root).as_posix()
        entries.append((path, arcname))
    return entries


def _write_wheel_entry(zf: zipfile.ZipFile, arcname: str, data: bytes, *, epoch: int, mode: int = 0o644) -> None:
    info = zipfile.ZipInfo(arcname)
    info.date_time = _zip_timestamp(epoch)
    info.compress_type = zipfile.ZIP_DEFLATED
    info.external_attr = (mode & 0xFFFF) << 16
    zf.writestr(info, data)


def _record_digest(data: bytes) -> str:
    return base64.urlsafe_b64encode(hashlib.sha256(data).digest()).decode("ascii").rstrip("=")


def _infer_readme_content_type(path: str) -> str | None:
    suffix = Path(path).suffix.lower()
    if suffix == ".md":
        return "text/markdown"
    if suffix == ".rst":
        return "text/x-rst"
    if suffix == ".txt":
        return "text/plain"
    return None


def _readme_payload(source_dir: Path, readme_value: str | dict[str, Any] | None) -> tuple[str | None, str | None]:
    if isinstance(readme_value, str):
        target = source_dir / readme_value
        if not target.exists():
            return None, _infer_readme_content_type(readme_value)
        return target.read_text(encoding="utf-8"), _infer_readme_content_type(readme_value)
    if isinstance(readme_value, dict):
        content_type = str(readme_value.get("content-type") or "").strip() or None
        inline_text = readme_value.get("text")
        if isinstance(inline_text, str):
            return inline_text, content_type or "text/markdown"
        file_path = str(readme_value.get("file") or "").strip()
        if file_path:
            target = source_dir / file_path
            if not target.exists():
                return None, content_type or _infer_readme_content_type(file_path)
            return target.read_text(encoding="utf-8"), content_type or _infer_readme_content_type(file_path)
    return None, None


def _distribution_metadata_bytes(
    source_dir: Path,
    *,
    package_name: str,
    version: str,
    description: str | None,
    requires_python: str | None,
    license_value: Any,
    license_files: list[str],
    dependencies: list[str],
    scripts: dict[str, Any],
    urls: dict[str, str],
    classifiers: list[str],
    keywords: list[str],
    readme_value: str | dict[str, Any] | None,
) -> tuple[bytes, bytes, list[tuple[str, bytes]]]:
    readme_body, readme_content_type = _readme_payload(source_dir, readme_value)
    metadata_lines = [
        "Metadata-Version: 2.4",
        f"Name: {package_name}",
        f"Version: {version}",
    ]
    if description:
        metadata_lines.append(f"Summary: {description}")
    if requires_python:
        metadata_lines.append(f"Requires-Python: {requires_python}")
    if license_value:
        metadata_lines.append(f"License-Expression: {license_value}")
    if keywords:
        metadata_lines.append(f"Keywords: {', '.join(keywords)}")
    for label, url in sorted(urls.items()):
        metadata_lines.append(f"Project-URL: {label}, {url}")
    for classifier in classifiers:
        metadata_lines.append(f"Classifier: {classifier}")
    for requirement in dependencies:
        metadata_lines.append(f"Requires-Dist: {requirement}")
    if readme_content_type:
        metadata_lines.append(f"Description-Content-Type: {readme_content_type}")
    for license_file in license_files:
        metadata_lines.append(f"License-File: {license_file}")
    metadata_text = "\n".join(metadata_lines) + "\n"
    if readme_body:
        metadata_text += "\n" + readme_body.rstrip() + "\n"

    entry_lines = []
    if scripts:
        entry_lines.append("[console_scripts]")
        for name in sorted(scripts):
            entry_lines.append(f"{name} = {scripts[name]}")
    entry_points_bytes = ("\n".join(entry_lines) + "\n").encode("utf-8") if entry_lines else b""

    license_entries: list[tuple[str, bytes]] = []
    for license_file in license_files:
        target = source_dir / license_file
        if not target.exists() or not target.is_file():
            continue
        license_entries.append((license_file, target.read_bytes()))
    return metadata_text.encode("utf-8"), entry_points_bytes, license_entries


def _build_sdist(
    source_dir: Path,
    dist_dir: Path,
    *,
    package_name: str,
    version: str,
    description: str | None,
    requires_python: str | None,
    license_value: Any,
    license_files: list[str],
    dependencies: list[str],
    scripts: dict[str, Any],
    urls: dict[str, str],
    classifiers: list[str],
    keywords: list[str],
    readme_value: str | dict[str, Any] | None,
    epoch: int,
) -> Path:
    filename = f"{_distribution_name(package_name, wheel_safe=False)}-{version}.tar.gz"
    target = dist_dir / filename
    root_name = f"{_distribution_name(package_name, wheel_safe=False)}-{version}"
    metadata_bytes, _, _ = _distribution_metadata_bytes(
        source_dir,
        package_name=package_name,
        version=version,
        description=description,
        requires_python=requires_python,
        license_value=license_value,
        license_files=license_files,
        dependencies=dependencies,
        scripts=scripts,
        urls=urls,
        classifiers=classifiers,
        keywords=keywords,
        readme_value=readme_value,
    )
    with tarfile.open(target, "w:gz") as tar:
        info = tarfile.TarInfo(name=f"{root_name}/PKG-INFO")
        info.size = len(metadata_bytes)
        info.mtime = epoch
        info.mode = 0o644
        info.uid = 0
        info.gid = 0
        info.uname = "root"
        info.gname = "root"
        tar.addfile(info, io.BytesIO(metadata_bytes))
        for path in sorted(source_dir.rglob("*")):
            if not path.is_file():
                continue
            if "__pycache__" in path.parts:
                continue
            data = path.read_bytes()
            info = tarfile.TarInfo(name=f"{root_name}/{path.relative_to(source_dir).as_posix()}")
            info.size = len(data)
            info.mtime = epoch
            info.mode = path.stat().st_mode & 0o777
            info.uid = 0
            info.gid = 0
            info.uname = "root"
            info.gname = "root"
            tar.addfile(info, io.BytesIO(data))
    return target


def _build_wheel(
    source_dir: Path,
    dist_dir: Path,
    *,
    package_name: str,
    version: str,
    description: str | None,
    requires_python: str | None,
    license_value: Any,
    license_files: list[str],
    dependencies: list[str],
    scripts: dict[str, Any],
    urls: dict[str, str],
    classifiers: list[str],
    keywords: list[str],
    readme_value: str | dict[str, Any] | None,
    epoch: int,
) -> Path:
    dist_name = _distribution_name(package_name, wheel_safe=True)
    filename = f"{dist_name}-{version}-py3-none-any.whl"
    target = dist_dir / filename
    source_root = _wheel_source_root(source_dir)
    metadata_dir = f"{dist_name}-{version}.dist-info"
    metadata_bytes, entry_points_bytes, license_entries = _distribution_metadata_bytes(
        source_dir,
        package_name=package_name,
        version=version,
        description=description,
        requires_python=requires_python,
        license_value=license_value,
        license_files=license_files,
        dependencies=dependencies,
        scripts=scripts,
        urls=urls,
        classifiers=classifiers,
        keywords=keywords,
        readme_value=readme_value,
    )
    wheel_bytes = b"Wheel-Version: 1.0\nGenerator: ait release\nRoot-Is-Purelib: true\nTag: py3-none-any\n"

    records: list[tuple[str, bytes]] = []
    with zipfile.ZipFile(target, "w") as zf:
        for path, arcname in _iter_installable_files(source_root):
            data = path.read_bytes()
            mode = path.stat().st_mode & 0o777
            _write_wheel_entry(zf, arcname, data, epoch=epoch, mode=mode)
            records.append((arcname, data))
        generated_entries = [
            (f"{metadata_dir}/METADATA", metadata_bytes),
            (f"{metadata_dir}/WHEEL", wheel_bytes),
        ]
        if entry_points_bytes:
            generated_entries.append((f"{metadata_dir}/entry_points.txt", entry_points_bytes))
        for relative_path, data in license_entries:
            generated_entries.append((f"{metadata_dir}/licenses/{relative_path}", data))
        for arcname, data in generated_entries:
            _write_wheel_entry(zf, arcname, data, epoch=epoch)
            records.append((arcname, data))

        record_lines = [
            f"{arcname},sha256={_record_digest(data)},{len(data)}"
            for arcname, data in records
        ]
        record_lines.append(f"{metadata_dir}/RECORD,,")
        record_bytes = ("\n".join(record_lines) + "\n").encode("utf-8")
        _write_wheel_entry(zf, f"{metadata_dir}/RECORD", record_bytes, epoch=epoch)
    return target


def build_release_candidate(ctx: RepoContext, release_id: str) -> dict[str, Any]:
    record = get_release_candidate(ctx, release_id)
    profile_settings = _require_profile(record["profile"])
    release_surface_paths = [
        *profile_settings["release_docs"],
        *profile_settings["license_files"],
        *profile_settings["contributor_files"],
        *profile_settings["quickstart_files"],
    ]
    workspace_clean, workspace_matches_line = _workspace_matches_release_source(
        ctx,
        line_name=str(record["line"]),
        snapshot_id=str(record["snapshot_id"]),
    )
    bundle = _release_source_bundle(
        ctx,
        snapshot_id=str(record["snapshot_id"]),
        profile_settings=profile_settings,
        allowed_paths=["pyproject.toml", *release_surface_paths],
        workspace_matches_release_source=workspace_clean and workspace_matches_line,
    )
    bundle, release_notes = _with_release_notes_readme(ctx, record, bundle)
    pyproject = _pyproject_metadata(bundle)
    dist_dir = ctx.root / "dist"
    dist_dir.mkdir(parents=True, exist_ok=True)
    epoch = _release_epoch(bundle)

    with tempfile.TemporaryDirectory(prefix=f"{release_id.lower()}-build-") as temp_dir:
        source_dir = Path(temp_dir) / "source"
        source_dir.mkdir(parents=True, exist_ok=True)
        _materialize_bundle(bundle, source_dir)
        with _build_environment(epoch):
            package_name = str(pyproject["name"])
            sdist_path = _build_sdist(
                source_dir,
                dist_dir,
                package_name=package_name,
                version=str(record["version"]),
                description=pyproject.get("description"),
                requires_python=pyproject.get("requires_python"),
                license_value=pyproject.get("license"),
                license_files=pyproject.get("license_files") if isinstance(pyproject.get("license_files"), list) else [],
                dependencies=pyproject.get("dependencies") if isinstance(pyproject.get("dependencies"), list) else [],
                scripts=pyproject.get("scripts") if isinstance(pyproject.get("scripts"), dict) else {},
                urls=pyproject.get("urls") if isinstance(pyproject.get("urls"), dict) else {},
                classifiers=pyproject.get("classifiers") if isinstance(pyproject.get("classifiers"), list) else [],
                keywords=pyproject.get("keywords") if isinstance(pyproject.get("keywords"), list) else [],
                readme_value=pyproject.get("readme"),
                epoch=epoch,
            )
            wheel_path = _build_wheel(
                source_dir,
                dist_dir,
                package_name=package_name,
                version=str(record["version"]),
                description=pyproject.get("description"),
                requires_python=pyproject.get("requires_python"),
                license_value=pyproject.get("license"),
                license_files=pyproject.get("license_files") if isinstance(pyproject.get("license_files"), list) else [],
                dependencies=pyproject.get("dependencies") if isinstance(pyproject.get("dependencies"), list) else [],
                scripts=pyproject.get("scripts") if isinstance(pyproject.get("scripts"), dict) else {},
                urls=pyproject.get("urls") if isinstance(pyproject.get("urls"), dict) else {},
                classifiers=pyproject.get("classifiers") if isinstance(pyproject.get("classifiers"), list) else [],
                keywords=pyproject.get("keywords") if isinstance(pyproject.get("keywords"), list) else [],
                readme_value=pyproject.get("readme"),
                epoch=epoch,
            )
        built_paths = [sdist_path, wheel_path]
        final_artifacts = [_artifact_info(path) for path in built_paths]

    manifest_path = dist_dir / f"ait-release-{record['version']}.manifest.json"
    manifest_payload = {
        "release_id": record["release_id"],
        "repo_name": record["repo_name"],
        "version": record["version"],
        "line": record["line"],
        "snapshot_id": record["snapshot_id"],
        "manifest_hash": record["manifest_hash"],
        "profile": record["profile"],
        "package": pyproject,
        "built_at": datetime.now(timezone.utc).isoformat(),
        "source_date_epoch": epoch,
        "artifacts": final_artifacts,
    }
    manifest_path.write_text(json.dumps(manifest_payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    manifest_artifact = _artifact_info(manifest_path)

    checksum_path = dist_dir / f"ait-release-{record['version']}.sha256"
    checksum_lines = [
        f"{artifact['sha256']}  {Path(artifact['path']).name}"
        for artifact in [*final_artifacts, manifest_artifact]
    ]
    checksum_path.write_text("\n".join(checksum_lines) + "\n", encoding="utf-8")
    checksum_artifact = _artifact_info(checksum_path)

    artifacts: list[dict[str, Any]] = []
    for artifact in [*final_artifacts, manifest_artifact, checksum_artifact]:
        artifacts = _replace_artifact(artifacts, artifact)

    metadata = _merge_metadata(
        record,
        package=pyproject,
        build={
            "dist_dir": dist_dir.as_posix(),
            "manifest_path": manifest_path.as_posix(),
            "checksum_path": checksum_path.as_posix(),
            "built_at": datetime.now(timezone.utc).isoformat(),
            "source_date_epoch": epoch,
            "builder": "ait_internal_sdist_and_wheel",
        },
        release_notes=release_notes,
    )
    update_local_release(
        ctx,
        release_id,
        status="built",
        artifacts=artifacts,
        formula={},
        metadata=metadata,
        event_type="release.built",
    )
    return get_release_candidate(ctx, release_id)


def _formula_class_name(name: str) -> str:
    tokens = re.split(r"[^A-Za-z0-9]+", name)
    return "".join(token[:1].upper() + token[1:] for token in tokens if token) or "Ait"


def _homebrew_license_literal(license_value: str | None) -> str:
    value = str(license_value or "").strip()
    if not value:
        return ":cannot_represent"
    if " AND " in value and "(" not in value and ")" not in value:
        parts = [part.strip() for part in value.split(" AND ") if part.strip()]
        return "all_of: [" + ", ".join(json.dumps(part) for part in parts) + "]"
    if " OR " in value and "(" not in value and ")" not in value:
        parts = [part.strip() for part in value.split(" OR ") if part.strip()]
        return "any_of: [" + ", ".join(json.dumps(part) for part in parts) + "]"
    return json.dumps(value)


def _package_homepage(package: dict[str, Any], repo_name: str) -> str:
    urls = package.get("urls") if isinstance(package.get("urls"), dict) else {}
    for label in ("Homepage", "Documentation", "Source"):
        value = str(urls.get(label) or "").strip()
        if value:
            return value
    return f"https://example.invalid/{repo_name}"


def _artifact_download_name(artifact: dict[str, Any]) -> str:
    explicit_path = str(artifact.get("path") or "").strip()
    if explicit_path:
        name = Path(explicit_path).name.strip()
        if name:
            return name
    explicit_url = str(artifact.get("url") or "").strip()
    if explicit_url:
        parsed = urllib.parse.urlparse(explicit_url)
        name = Path(parsed.path).name.strip()
        if name:
            return name
    raise ValueError("Release artifact is missing a usable download filename.")


def generate_release_formula(ctx: RepoContext, release_id: str, *, name: str) -> dict[str, Any]:
    record = get_release_candidate(ctx, release_id)
    artifacts = [row for row in record.get("artifacts") or [] if isinstance(row, dict)]
    wheel = next((row for row in artifacts if str(row.get("kind") or "") == "wheel"), None)
    if wheel is None:
        raise ValueError(f"Release {release_id} does not have a wheel artifact yet. Run `ait release build {release_id}` first.")
    package = record.get("package") if isinstance(record.get("package"), dict) else {}
    scripts = package.get("scripts") if isinstance(package.get("scripts"), dict) else {}
    script_names = sorted(script_name for script_name in scripts if str(script_name).strip())
    python_formula = f"python@{sys.version_info.major}.{sys.version_info.minor}"
    formula_path = ctx.root / "dist" / f"{name}.rb"
    formula_path.parent.mkdir(parents=True, exist_ok=True)
    homepage = _package_homepage(package, str(record["repo_name"]))
    license_literal = _homebrew_license_literal(str(package.get("license") or ""))
    wheel_filename = _artifact_download_name(wheel)
    symlink_lines = "\n".join(
        f'    bin.install_symlink libexec/"bin/{script_name}"' for script_name in script_names
    )
    if not symlink_lines:
        symlink_lines = '    odie "Formula generated without any console scripts to link."'
    formula_text = f"""class {_formula_class_name(name)} < Formula
  preserve_rpath

  desc {json.dumps(str(package.get("description") or f"{record['repo_name']} release candidate"))}
  homepage {json.dumps(homepage)}
  url {json.dumps(str(wheel["url"]))}
  sha256 {json.dumps(str(wheel["sha256"]))}
  license {license_literal}

  depends_on "{python_formula}"

  def install
    system Formula["{python_formula}"].opt_bin/"python3", "-m", "venv", libexec
    wheel = buildpath/{json.dumps(wheel_filename)}
    cp cached_download, wheel
    system libexec/"bin/python", "-m", "pip", "install", wheel
{symlink_lines}
  end

  def caveats
    <<~EOS
      Homebrew tap formula generated by `ait release formula`.
      Expected console scripts from this package: {", ".join(script_names) or "none"}.
      This draft intentionally avoids auto-starting services.
      `ait-server` and `ait-worker` still require self-hosted runtime configuration.
    EOS
  end
end
"""
    formula_path.write_text(formula_text, encoding="utf-8")
    formula_artifact = _artifact_info(formula_path)
    updated_artifacts = list(artifacts)
    updated_artifacts = _replace_artifact(updated_artifacts, formula_artifact)
    formula = {
        "name": name,
        "class_name": _formula_class_name(name),
        "path": formula_path.as_posix(),
        "artifact_kind": "wheel",
        "homepage": homepage,
        "license": str(package.get("license") or ""),
        "url": wheel["url"],
        "sha256": wheel["sha256"],
        "wheel_filename": wheel_filename,
    }
    metadata = _merge_metadata(record, formula=formula)
    update_local_release(
        ctx,
        release_id,
        artifacts=updated_artifacts,
        formula=formula,
        metadata=metadata,
        event_type="release.formula_generated",
    )
    return get_release_candidate(ctx, release_id)
