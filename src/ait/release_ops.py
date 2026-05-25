from __future__ import annotations

import base64
import fnmatch
import hashlib
import io
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import urllib.parse
import zipfile
from contextlib import contextmanager
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, Iterable

from .store import (
    RepoContext,
    create_local_release,
    current_line,
    export_snapshot_bundle,
    get_line,
    get_local_release,
    get_snapshot,
    load_config,
    repo_status,
    update_local_release,
)

_PUBLIC_PACKAGE_RELEASE_URLS = {
    "Homepage": "https://ait-native.dev",
    "Documentation": "https://ait-native.dev",
    "Source": "https://github.com/weita2026/ait-native",
    "Issues": "https://github.com/weita2026/ait-native/issues",
    "Releases": "https://github.com/weita2026/ait-native/releases",
}

SUPPORTED_RELEASE_PROFILES = {
    "local-cli": {
        "required_scripts": ["ait"],
        "release_docs": [
            "README.md",
            "docs/LOCAL_QUICKSTART.md",
            "docs/PACKAGE_TARGETS.md",
            "docs/COMPATIBILITY_MATRIX.md",
            "docs/legal/public_release_license_summary.md",
        ],
        "license_files": ["LICENSE", "NOTICE", "docs/THIRD_PARTY_NOTICES.md"],
        "contributor_files": ["docs/CONTRIBUTING.md"],
        "quickstart_files": ["docs/LOCAL_QUICKSTART.md"],
    },
    "public-self-hosted-core": {
        "required_scripts": ["ait", "ait-agent", "ait-server", "ait-worker", "aitk"],
        "forbidden_scripts": ["ait-web"],
        "release_docs": [
            "README.md",
            "README.pypi.md",
            "docs/LOCAL_QUICKSTART.md",
            "docs/HOMEBREW_TAP.md",
            "docs/SELF_HOSTED_TEAM_DEPLOYMENT.md",
            "docs/PYPI_PUBLISHING.md",
            "docs/PACKAGE_TARGETS.md",
            "docs/COMPATIBILITY_MATRIX.md",
            "docs/public_package_targets_contract.json",
            "docs/public_compatibility_matrix.json",
            "docs/public_self_hosted_deployment_contract.json",
            "docs/legal/public_package_surface_map.json",
            "docs/legal/public_release_license_summary.md",
        ],
        "license_files": [
            "LICENSE",
            "NOTICE",
            "docs/THIRD_PARTY_NOTICES.md",
            "LICENSES/AGPL-3.0-only.txt",
            "LICENSES/LicenseRef-AIT-Commercial.txt",
            "docs/TRADEMARK_POLICY.md",
        ],
        "contributor_files": ["docs/CONTRIBUTING.md", "docs/LOCAL_DEVELOPMENT.md"],
        "quickstart_files": [
            "docs/LOCAL_QUICKSTART.md",
            "docs/HOMEBREW_TAP.md",
            "docs/SELF_HOSTED_TEAM_DEPLOYMENT.md",
        ],
        "excluded_paths": [
            "docs/benchmarks/**",
            "docs/sprints/**",
            "site/**",
            "deploy/site/**",
            "src/ait_web/**",
            "tests/ait_web/**",
            "src/ait_native/web.py",
        ],
        "setuptools_package_excludes": ["ait_web*"],
        "project_overrides": {
            "description": "Agent-first Markdown workflow CLI with optional self-hosted server and worker surfaces",
            "readme": {
                "file": "README.pypi.md",
                "content-type": "text/markdown",
            },
            "license": "Apache-2.0 AND AGPL-3.0-only",
            "license-files": [
                "LICENSE",
                "NOTICE",
                "LICENSES/AGPL-3.0-only.txt",
            ],
            "keywords": ["ai", "agent", "workflow", "markdown", "cli"],
            "classifiers": [
                "Development Status :: 3 - Alpha",
                "Environment :: Console",
                "Intended Audience :: Developers",
                "Operating System :: OS Independent",
                "Programming Language :: Python :: 3",
                "Programming Language :: Python :: 3.11",
                "Topic :: Software Development :: Build Tools",
                "Topic :: Software Development :: Version Control",
            ],
            "urls": _PUBLIC_PACKAGE_RELEASE_URLS,
        },
        "required_package_metadata": {
            "license": "Apache-2.0 AND AGPL-3.0-only",
            "readme_file": "README.pypi.md",
            "keywords_min_count": 3,
            "classifiers_min_count": 5,
            "project_urls": ["Homepage", "Documentation", "Source", "Issues", "Releases"],
        },
        "publish_support": {
            "files": [
                ".github/workflows/pypi-publish.yml",
                "docs/PYPI_PUBLISHING.md",
            ],
            "workflow_path": ".github/workflows/pypi-publish.yml",
            "workflow_contains": [
                "workflow_dispatch:",
                "pypa/gh-action-pypi-publish@release/v1",
                "id-token: write",
                "name: pypi",
                "https://pypi.org/p/ait-native",
            ],
            "doc_path": "docs/PYPI_PUBLISHING.md",
            "doc_contains": [
                "weita2026/ait-native",
                ".github/workflows/pypi-publish.yml",
                "Trusted Publisher",
                "twine upload dist/*",
            ],
        },
    },
}

_MARKDOWN_LINK_RE = re.compile(r"(?<!!)\[[^\]]*\]\(([^)]+)\)")


def _require_profile(profile: str) -> dict[str, Any]:
    normalized = str(profile or "").strip().lower()
    if normalized not in SUPPORTED_RELEASE_PROFILES:
        supported = ", ".join(sorted(SUPPORTED_RELEASE_PROFILES))
        raise ValueError(f"Unsupported release profile {profile!r}. Supported profiles: {supported}.")
    return SUPPORTED_RELEASE_PROFILES[normalized]


def _bundle_file_map(bundle: dict[str, Any]) -> dict[str, dict[str, Any]]:
    return {
        str(entry.get("path") or ""): entry
        for entry in bundle.get("files", [])
        if isinstance(entry, dict) and str(entry.get("path") or "").strip()
    }


def _bundle_entry_bytes(entry: dict[str, Any]) -> bytes:
    return base64.b64decode(entry["content_b64"])


def _bundle_entry_text(entry: dict[str, Any]) -> str:
    return _bundle_entry_bytes(entry).decode("utf-8")


def _bundle_text(file_map: dict[str, dict[str, Any]], path: str) -> str:
    entry = file_map.get(path)
    if entry is None:
        raise ValueError(f"Release source snapshot is missing required file: {path}")
    return _bundle_entry_text(entry)


def _bundle_entry_with_bytes(entry: dict[str, Any], data: bytes) -> dict[str, Any]:
    updated = dict(entry)
    updated["content_b64"] = base64.b64encode(data).decode("ascii")
    updated["size_bytes"] = len(data)
    return updated


def _bundle_entry_with_text(entry: dict[str, Any], text: str) -> dict[str, Any]:
    return _bundle_entry_with_bytes(entry, text.encode("utf-8"))


def _public_pypi_readme() -> str:
    return """# ait-native

`ait-native` is the first public Python distribution for the `ait` workflow family.

It packages these command surfaces today:

- `ait` for the local trust-layer CLI
- `ait-agent` for transport/runtime helper flows
- `ait-server` for the shared workflow control plane
- `ait-worker` for the async shared-control-plane worker
- `aitk` for the local read-only history companion

## Install

```bash
pip install ait-native
```

If you plan to run the shared self-hosted control plane, install the PostgreSQL extra:

```bash
pip install "ait-native[postgres]"
```

## Quick start

- Local-first path: https://ait-native.dev
- Self-hosted guide: https://ait-native.dev
- Source: https://github.com/weita2026/ait-native

## Important license boundary

`ait-native` is a combined public distribution with multiple release-facing license surfaces.

- Local CLI and local companion surfaces remain Apache-2.0.
- Public self-hosted `ait-server` / `ait-worker` surfaces follow AGPL-3.0-only.

Read the release-facing summary before relying on a broader grant:

- https://ait-native.dev
- https://github.com/weita2026/ait-native
"""


def _workspace_candidate_roots(ctx: RepoContext) -> list[Path]:
    roots: list[Path] = []
    seen: set[Path] = set()

    def _append(candidate: Path | str | None) -> None:
        if candidate is None:
            return
        path = Path(candidate).expanduser()
        try:
            resolved = path.resolve()
        except OSError:
            resolved = path
        if resolved in seen:
            return
        seen.add(resolved)
        roots.append(resolved)

    _append(ctx.root)
    _append(ctx.repo_root)
    if ctx.worktree_config_path is not None and ctx.worktree_config_path.exists():
        try:
            payload = json.loads(ctx.worktree_config_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            payload = {}
        if isinstance(payload, dict):
            _append(payload.get("repo_root"))
            _append(payload.get("workspace_root"))
    return roots


def _workspace_file_entry(ctx: RepoContext, path: str) -> dict[str, Any] | None:
    for root in _workspace_candidate_roots(ctx):
        target = root / path
        if not target.exists() or not target.is_file():
            continue
        data = target.read_bytes()
        mode = oct(target.stat().st_mode & 0o777)
        return {
            "path": path,
            "mode": mode,
            "size_bytes": len(data),
            "content_b64": base64.b64encode(data).decode("ascii"),
        }
    return None


def _supplement_workspace_release_files(
    ctx: RepoContext,
    file_map: dict[str, dict[str, Any]],
    *,
    allowed_paths: Iterable[str],
    workspace_matches_release_source: bool,
) -> dict[str, dict[str, Any]]:
    if not workspace_matches_release_source:
        return file_map
    combined = dict(file_map)
    for raw_path in allowed_paths:
        path = str(raw_path or "").strip()
        if not path or path in combined:
            continue
        entry = _workspace_file_entry(ctx, path)
        if entry is not None:
            combined[path] = entry
    return combined


def _workspace_matches_release_source(ctx: RepoContext, *, line_name: str, snapshot_id: str) -> tuple[bool, bool]:
    status = repo_status(ctx)
    workspace_clean = bool(status.get("workspace_dirty")) is False
    workspace_matches_line = (
        current_line(ctx) == line_name and str(status.get("head_snapshot_id") or "").strip() == snapshot_id
    )
    return workspace_clean, workspace_matches_line


def _supplement_workspace_release_bundle(
    ctx: RepoContext,
    bundle: dict[str, Any],
    *,
    allowed_paths: Iterable[str],
    workspace_matches_release_source: bool,
) -> dict[str, Any]:
    file_map = _supplement_workspace_release_files(
        ctx,
        _bundle_file_map(bundle),
        allowed_paths=allowed_paths,
        workspace_matches_release_source=workspace_matches_release_source,
    )
    ordered_paths = [
        str(entry.get("path") or "")
        for entry in bundle.get("files", [])
        if isinstance(entry, dict) and str(entry.get("path") or "").strip()
    ]
    files: list[dict[str, Any]] = []
    seen: set[str] = set()
    for path in ordered_paths:
        entry = file_map.get(path)
        if entry is None:
            continue
        files.append(entry)
        seen.add(path)
    for path in sorted(file_map):
        if path in seen:
            continue
        files.append(file_map[path])
    updated = dict(bundle)
    updated["files"] = files
    return updated


def _path_matches_any(path: str, patterns: Iterable[str]) -> bool:
    normalized = path.replace("\\", "/")
    return any(fnmatch.fnmatchcase(normalized, str(pattern).replace("\\", "/")) for pattern in patterns)


def _toml_key(key: str) -> str:
    if re.fullmatch(r"[A-Za-z0-9_-]+", key):
        return key
    return json.dumps(key)


def _toml_scalar(value: Any) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, int):
        return str(value)
    return json.dumps(str(value))


def _toml_list(values: Iterable[Any]) -> str:
    return "[" + ", ".join(_toml_scalar(value) for value in values) + "]"


def _toml_inline_table(mapping: dict[str, Any]) -> str:
    parts = [f"{_toml_key(str(key))} = {_toml_value(value)}" for key, value in mapping.items()]
    return "{ " + ", ".join(parts) + " }"


def _toml_value(value: Any) -> str:
    if isinstance(value, dict):
        return _toml_inline_table(value)
    if isinstance(value, list):
        return _toml_list(value)
    return _toml_scalar(value)


def _toml_section(lines: list[str], header: str, mapping: dict[str, Any]) -> None:
    if not mapping:
        return
    if lines:
        lines.append("")
    lines.append(f"[{header}]")
    for key, value in mapping.items():
        lines.append(f"{_toml_key(str(key))} = {_toml_value(value)}")


def _serialize_pyproject(data: dict[str, Any]) -> str:
    lines: list[str] = []
    build_system = data.get("build-system") if isinstance(data.get("build-system"), dict) else {}
    _toml_section(lines, "build-system", build_system)

    project = data.get("project") if isinstance(data.get("project"), dict) else {}
    project_body = {
        key: value
        for key, value in project.items()
        if key not in {"scripts", "optional-dependencies"} and value is not None
    }
    _toml_section(lines, "project", project_body)
    scripts = project.get("scripts") if isinstance(project.get("scripts"), dict) else {}
    _toml_section(lines, "project.scripts", scripts)
    optional_dependencies = (
        project.get("optional-dependencies")
        if isinstance(project.get("optional-dependencies"), dict)
        else {}
    )
    _toml_section(lines, "project.optional-dependencies", optional_dependencies)

    tool = data.get("tool") if isinstance(data.get("tool"), dict) else {}
    setuptools_cfg = tool.get("setuptools") if isinstance(tool.get("setuptools"), dict) else {}
    setuptools_body = dict(setuptools_cfg)
    packages_cfg = setuptools_body.pop("packages", None)
    _toml_section(lines, "tool.setuptools", setuptools_body)
    packages_find = (
        packages_cfg.get("find")
        if isinstance(packages_cfg, dict) and isinstance(packages_cfg.get("find"), dict)
        else {}
    )
    _toml_section(lines, "tool.setuptools.packages.find", packages_find)

    pytest_cfg = tool.get("pytest") if isinstance(tool.get("pytest"), dict) else {}
    pytest_ini = (
        pytest_cfg.get("ini_options")
        if isinstance(pytest_cfg.get("ini_options"), dict)
        else {}
    )
    _toml_section(lines, "tool.pytest.ini_options", pytest_ini)
    return "\n".join(lines) + "\n"


def _shape_release_pyproject(pyproject_text: str, profile_settings: dict[str, Any]) -> str:
    data = tomllib.loads(pyproject_text)
    project = data.get("project")
    if not isinstance(project, dict):
        return pyproject_text
    project_overrides = (
        profile_settings.get("project_overrides")
        if isinstance(profile_settings.get("project_overrides"), dict)
        else {}
    )
    for key, value in project_overrides.items():
        project[key] = value
    scripts = dict(project.get("scripts") or {})
    forbidden_scripts = [str(name) for name in profile_settings.get("forbidden_scripts") or []]
    for script_name in forbidden_scripts:
        scripts.pop(script_name, None)
    project["scripts"] = scripts

    package_excludes = list(profile_settings.get("setuptools_package_excludes") or [])
    if package_excludes:
        tool = data.setdefault("tool", {})
        if isinstance(tool, dict):
            setuptools_cfg = tool.get("setuptools")
            if isinstance(setuptools_cfg, dict):
                packages_cfg = setuptools_cfg.get("packages")
                if isinstance(packages_cfg, dict):
                    find_cfg = packages_cfg.get("find")
                    if isinstance(find_cfg, dict):
                        existing = [
                            str(value).strip()
                            for value in find_cfg.get("exclude", [])
                            if str(value).strip()
                        ] if isinstance(find_cfg.get("exclude"), list) else []
                        merged: list[str] = []
                        for pattern in [*existing, *package_excludes]:
                            if pattern not in merged:
                                merged.append(pattern)
                        if merged:
                            find_cfg["exclude"] = merged
    return _serialize_pyproject(data)


def _shape_release_bundle(bundle: dict[str, Any], profile_settings: dict[str, Any]) -> dict[str, Any]:
    excluded_paths = [str(path).strip() for path in profile_settings.get("excluded_paths") or [] if str(path).strip()]
    files: list[dict[str, Any]] = []
    generated_entries: dict[str, dict[str, Any]] = {}
    for entry in bundle.get("files", []):
        if not isinstance(entry, dict):
            continue
        path = str(entry.get("path") or "").strip()
        if not path:
            continue
        if excluded_paths and _path_matches_any(path, excluded_paths):
            continue
        if path == "pyproject.toml":
            shaped_text = _shape_release_pyproject(_bundle_entry_text(entry), profile_settings)
            files.append(_bundle_entry_with_text(entry, shaped_text))
            continue
        files.append(entry)
    project_overrides = (
        profile_settings.get("project_overrides")
        if isinstance(profile_settings.get("project_overrides"), dict)
        else {}
    )
    readme_override = project_overrides.get("readme")
    if isinstance(readme_override, dict) and str(readme_override.get("file") or "").strip() == "README.pypi.md":
        generated_entries["README.pypi.md"] = {
            "path": "README.pypi.md",
            "mode": "0644",
            "size_bytes": 0,
            "content_b64": "",
        }
        generated_entries["README.pypi.md"] = _bundle_entry_with_text(generated_entries["README.pypi.md"], _public_pypi_readme())
    existing_paths = {str(entry.get("path") or "") for entry in files if isinstance(entry, dict)}
    for path in sorted(generated_entries):
        if path in existing_paths:
            continue
        files.append(generated_entries[path])
    updated = dict(bundle)
    updated["files"] = files
    return updated


def _pyproject_readme_value(project: dict[str, Any]) -> str | dict[str, Any] | None:
    readme = project.get("readme")
    if isinstance(readme, str):
        value = readme.strip()
        return value or None
    if isinstance(readme, dict):
        return {
            str(key): value
            for key, value in readme.items()
            if value is not None and str(key).strip()
        }
    return None


def _readme_declared_file(readme_value: str | dict[str, Any] | None) -> str | None:
    if isinstance(readme_value, str):
        return readme_value
    if isinstance(readme_value, dict):
        file_path = str(readme_value.get("file") or "").strip()
        return file_path or None
    return None


def _release_source_bundle(
    ctx: RepoContext,
    *,
    snapshot_id: str,
    profile_settings: dict[str, Any],
    allowed_paths: Iterable[str],
    workspace_matches_release_source: bool,
) -> dict[str, Any]:
    bundle = _supplement_workspace_release_bundle(
        ctx,
        export_snapshot_bundle(ctx, snapshot_id),
        allowed_paths=allowed_paths,
        workspace_matches_release_source=workspace_matches_release_source,
    )
    return _shape_release_bundle(bundle, profile_settings)


def _pyproject_metadata(bundle: dict[str, Any]) -> dict[str, Any]:
    file_map = _bundle_file_map(bundle)
    data = tomllib.loads(_bundle_text(file_map, "pyproject.toml"))
    project = data.get("project")
    if not isinstance(project, dict):
        raise ValueError("pyproject.toml is missing a [project] table.")
    name = str(project.get("name") or "").strip()
    version = str(project.get("version") or "").strip()
    if not name or not version:
        raise ValueError("pyproject.toml must define project.name and project.version for release candidates.")
    scripts = project.get("scripts") if isinstance(project.get("scripts"), dict) else {}
    dependencies = (
        [str(item).strip() for item in project.get("dependencies", []) if str(item).strip()]
        if isinstance(project.get("dependencies"), list)
        else []
    )
    urls = (
        {
            str(key).strip(): str(value).strip()
            for key, value in project.get("urls", {}).items()
            if str(key).strip() and str(value).strip()
        }
        if isinstance(project.get("urls"), dict)
        else {}
    )
    classifiers = (
        [str(item).strip() for item in project.get("classifiers", []) if str(item).strip()]
        if isinstance(project.get("classifiers"), list)
        else []
    )
    keywords = (
        [str(item).strip() for item in project.get("keywords", []) if str(item).strip()]
        if isinstance(project.get("keywords"), list)
        else []
    )
    license_files = (
        [str(item).strip() for item in project.get("license-files", []) if str(item).strip()]
        if isinstance(project.get("license-files"), list)
        else []
    )
    return {
        "name": name,
        "version": version,
        "requires_python": str(project.get("requires-python") or "").strip() or None,
        "description": str(project.get("description") or "").strip() or None,
        "readme": _pyproject_readme_value(project),
        "license": project.get("license"),
        "license_files": license_files,
        "dependencies": dependencies,
        "scripts": scripts,
        "urls": urls,
        "classifiers": classifiers,
        "keywords": keywords,
    }


def _release_next_action(record: dict[str, Any]) -> dict[str, str]:
    release_id = str(record["release_id"])
    status_value = str(record.get("status") or "").strip()
    checks = record.get("checks") if isinstance(record.get("checks"), list) else []
    blocking = [row for row in checks if isinstance(row, dict) and bool(row.get("blocking"))]
    artifacts = record.get("artifacts") if isinstance(record.get("artifacts"), list) else []
    formula = record.get("formula") if isinstance(record.get("formula"), dict) else {}
    package = record.get("package") if isinstance(record.get("package"), dict) else {}
    metadata = record.get("metadata") if isinstance(record.get("metadata"), dict) else {}
    remote_publish = metadata.get("remote_publish") if isinstance(metadata.get("remote_publish"), dict) else {}
    artifact_kinds = {str(row.get("kind") or "") for row in artifacts if isinstance(row, dict)}
    if status_value == "published" or remote_publish:
        remote_label = str(remote_publish.get("remote_name") or remote_publish.get("repo_name") or "ait-server")
        return {
            "code": "published_remote",
            "detail": f"Release is already published to {remote_label}. Reuse `ait release show {release_id} --remote <name>` to inspect the shared record.",
        }
    if not checks:
        return {
            "code": "run_checks",
            "detail": f"Run `ait release check {release_id}` to record the first structured readiness checks.",
        }
    if blocking:
        return {
            "code": "resolve_checks",
            "detail": f"Resolve the blocking release checks, then rerun `ait release check {release_id}`.",
        }
    if not {"sdist", "wheel"}.issubset(artifact_kinds):
        return {
            "code": "build_candidate",
            "detail": f"Run `ait release build {release_id}` to produce deterministic release artifacts.",
        }
    if not formula.get("path"):
        formula_name = str(package.get("name") or "ait").strip() or "ait"
        return {
            "code": "generate_formula",
            "detail": f"Run `ait release formula {release_id} --name {formula_name}` to draft the Homebrew formula surface.",
        }
    return {
        "code": "publish_remote",
        "detail": f"Run `ait release publish {release_id} --remote <name>` to publish this private release candidate to ait-server.",
    }


def _hydrate_release(record: dict[str, Any]) -> dict[str, Any]:
    payload = dict(record)
    payload["next_action"] = _release_next_action(payload)
    checks = payload.get("checks") if isinstance(payload.get("checks"), list) else []
    passed = sum(1 for row in checks if isinstance(row, dict) and str(row.get("status") or "") == "pass")
    warned = sum(1 for row in checks if isinstance(row, dict) and str(row.get("status") or "") == "warn")
    failed = sum(1 for row in checks if isinstance(row, dict) and str(row.get("status") or "") == "fail")
    skipped = sum(1 for row in checks if isinstance(row, dict) and str(row.get("status") or "") == "skipped")
    payload["check_summary"] = {
        "total": len(checks),
        "passed": passed,
        "warned": warned,
        "failed": failed,
        "skipped": skipped,
        "blocking": sum(1 for row in checks if isinstance(row, dict) and bool(row.get("blocking"))),
        "decision": "fail" if failed else ("warn" if warned else "pass"),
    }
    return payload


def get_release_candidate(ctx: RepoContext, release_id: str) -> dict[str, Any]:
    return _hydrate_release(get_local_release(ctx, release_id))


def create_release_candidate(ctx: RepoContext, *, version: str, line_name: str, profile: str) -> dict[str, Any]:
    profile_settings = _require_profile(profile)
    line = get_line(ctx, line_name)
    snapshot_id = str(line.get("head_snapshot_id") or "").strip()
    if not snapshot_id:
        raise ValueError(f"Line {line_name} does not have a head snapshot yet.")
    workspace_clean, workspace_matches_line = _workspace_matches_release_source(
        ctx,
        line_name=line_name,
        snapshot_id=snapshot_id,
    )
    bundle = _release_source_bundle(
        ctx,
        snapshot_id=snapshot_id,
        profile_settings=profile_settings,
        allowed_paths=["pyproject.toml"],
        workspace_matches_release_source=workspace_clean and workspace_matches_line,
    )
    pyproject = _pyproject_metadata(bundle)
    if version.strip() != pyproject["version"]:
        raise ValueError(
            f"Requested release version {version!r} does not match pyproject.toml version {pyproject['version']!r}."
        )
    metadata = {
        "package": pyproject,
        "profile": str(profile).strip().lower(),
        "profile_settings": profile_settings,
        "source_snapshot_created_at": bundle.get("created_at"),
    }
    record = create_local_release(
        ctx,
        version=version.strip(),
        line_name=line_name,
        snapshot_id=snapshot_id,
        manifest_hash=str(bundle["manifest_hash"]),
        profile=str(profile).strip().lower(),
        package_name=pyproject["name"],
        package_version=pyproject["version"],
        package_requires_python=pyproject["requires_python"],
        metadata=metadata,
    )
    return _hydrate_release(record)


def _release_epoch(bundle: dict[str, Any]) -> int:
    text = str(bundle.get("created_at") or "")
    normalized = text[:-1] + "+00:00" if text.endswith("Z") else text
    parsed = datetime.fromisoformat(normalized) if normalized else datetime.now(timezone.utc)
    aware = parsed if parsed.tzinfo else parsed.replace(tzinfo=timezone.utc)
    return int(aware.timestamp())


def _apply_mode(target: Path, raw_mode: str | None) -> None:
    if raw_mode is None:
        return
    try:
        mode_value = int(str(raw_mode), 8)
    except ValueError:
        return
    if mode_value & stat.S_IXUSR:
        target.chmod(target.stat().st_mode | stat.S_IXUSR)


def _materialize_bundle(bundle: dict[str, Any], destination: Path) -> None:
    epoch = _release_epoch(bundle)
    file_map = _bundle_file_map(bundle)
    for relative_path, entry in file_map.items():
        target = destination / relative_path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(_bundle_entry_bytes(entry))
        os.utime(target, (epoch, epoch))
        _apply_mode(target, entry.get("mode"))


def _workspace_path_exists(ctx: RepoContext, path: str) -> bool:
    return any((root / path).exists() for root in _workspace_candidate_roots(ctx))


def _markdown_link_audit(
    file_map: dict[str, dict[str, Any]],
    paths: Iterable[str],
    *,
    path_exists: Callable[[str], bool] | None = None,
) -> tuple[list[str], list[str]]:
    missing_docs: list[str] = []
    broken_links: list[str] = []
    for path in paths:
        entry = file_map.get(path)
        if entry is None:
            missing_docs.append(path)
            continue
        source_text = _bundle_entry_text(entry)
        source_dir = Path(path).parent
        for match in _MARKDOWN_LINK_RE.finditer(source_text):
            target = match.group(1).strip()
            if not target or target.startswith(("#", "http://", "https://", "mailto:")):
                continue
            target_path = target.split("#", 1)[0].strip()
            if not target_path:
                continue
            normalized = os.path.normpath((source_dir / target_path).as_posix()).replace("\\", "/")
            if normalized not in file_map and not (path_exists(normalized) if path_exists else False):
                broken_links.append(f"{path} -> {target}")
    return missing_docs, broken_links


def _scan_private_surface(ctx: RepoContext, file_map: dict[str, dict[str, Any]], paths: Iterable[str]) -> list[str]:
    home = str(Path.home())
    roots = _workspace_candidate_roots(ctx)
    repo_root = str(roots[-1] if roots else ctx.repo_root)
    patterns = [
        ("home_path", home),
        ("repo_root_path", repo_root),
        ("mac_user_path", "/Users/"),
        ("loopback_runtime", "127.0.0.1:8088"),
        ("localhost_runtime", "localhost:8088"),
    ]
    findings: list[str] = []
    for path in paths:
        entry = file_map.get(path)
        if entry is None:
            continue
        text = _bundle_entry_bytes(entry).decode("utf-8", errors="ignore")
        for label, needle in patterns:
            if needle and needle in text:
                findings.append(f"{path}: {label} ({needle})")
    return findings


def _check_result(
    check_id: str,
    label: str,
    *,
    status: str,
    details: str,
    blocking: bool,
) -> dict[str, Any]:
    return {
        "check_id": check_id,
        "label": label,
        "status": status,
        "details": details,
        "blocking": blocking,
    }


def _merge_metadata(record: dict[str, Any], **updates: Any) -> dict[str, Any]:
    metadata = dict(record.get("metadata") or {})
    metadata.update(updates)
    return metadata


def _run_command(command: list[str], *, cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(command, cwd=str(cwd), text=True, capture_output=True)


def run_release_checks(
    ctx: RepoContext,
    release_id: str,
    *,
    tests_command: str | None = None,
    skip_tests_reason: str | None = None,
) -> dict[str, Any]:
    if tests_command and skip_tests_reason:
        raise ValueError("Use either `--tests-command` or `--skip-tests-reason`, not both.")
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
    file_map = _bundle_file_map(bundle)
    pyproject = _pyproject_metadata(bundle)
    checks: list[dict[str, Any]] = []

    checks.append(
        _check_result(
            "workspace_clean",
            "Workspace is clean against the selected line head",
            status="pass" if workspace_clean and workspace_matches_line else "fail",
            details=(
                f"Workspace is clean on line {record['line']} at snapshot {record['snapshot_id']}."
                if workspace_clean and workspace_matches_line
                else (
                    f"Current workspace is on line {current_line(ctx)} with snapshot {repo_status(ctx).get('head_snapshot_id')}; "
                    f"release source is {record['line']} at {record['snapshot_id']}."
                )
            ),
            blocking=not (workspace_clean and workspace_matches_line),
        )
    )
    version_match = pyproject["version"] == record["version"]
    checks.append(
        _check_result(
            "version_matches_pyproject",
            "Release version matches pyproject.toml",
            status="pass" if version_match else "fail",
            details=f"pyproject.toml version is {pyproject['version']!r}.",
            blocking=not version_match,
        )
    )

    try:
        with tempfile.TemporaryDirectory(prefix="ait-release-check-") as temp_dir:
            export_dir = Path(temp_dir) / "source"
            export_dir.mkdir(parents=True, exist_ok=True)
            _materialize_bundle(bundle, export_dir)
            restore_ok = (export_dir / "pyproject.toml").exists()
    except Exception as exc:
        restore_ok = False
        restore_detail = f"Snapshot export failed: {exc}"
    else:
        restore_detail = f"Snapshot exported to an isolated source tree with {(export_dir / 'pyproject.toml').as_posix()} present."
    checks.append(
        _check_result(
            "snapshot_export",
            "Selected snapshot can be exported into an isolated source tree",
            status="pass" if restore_ok else "fail",
            details=restore_detail,
            blocking=not restore_ok,
        )
    )

    missing_docs, broken_links = _markdown_link_audit(
        file_map,
        profile_settings["release_docs"],
        path_exists=lambda path: workspace_clean and workspace_matches_line and _workspace_path_exists(ctx, path),
    )
    docs_ok = not missing_docs and not broken_links
    doc_problems = []
    if missing_docs:
        doc_problems.append(f"missing docs: {', '.join(missing_docs)}")
    if broken_links:
        doc_problems.append(f"broken links: {', '.join(broken_links[:5])}")
    checks.append(
        _check_result(
            "release_docs_links",
            "Release-facing Markdown docs have valid local links",
            status="pass" if docs_ok else "fail",
            details="Release-facing Markdown links resolved cleanly." if docs_ok else "; ".join(doc_problems),
            blocking=not docs_ok,
        )
    )

    private_findings = _scan_private_surface(ctx, file_map, profile_settings["release_docs"])
    checks.append(
        _check_result(
            "public_surface_private_paths",
            "Release-facing docs do not expose private machine paths or local runtime defaults",
            status="pass" if not private_findings else "fail",
            details="No private path or loopback runtime strings were detected in the release-facing docs."
            if not private_findings
            else "; ".join(private_findings[:5]),
            blocking=bool(private_findings),
        )
    )

    with tempfile.TemporaryDirectory(prefix="ait-release-compileall-") as temp_dir:
        export_dir = Path(temp_dir) / "source"
        export_dir.mkdir(parents=True, exist_ok=True)
        _materialize_bundle(bundle, export_dir)
        compile_targets = [path for path in ("src", "tests") if (export_dir / path).exists()]
        if not compile_targets:
            compile_ok = False
            compile_detail = "No `src/` or `tests/` tree was present in the release snapshot."
        else:
            completed = _run_command([sys.executable, "-m", "compileall", *compile_targets], cwd=export_dir)
            compile_ok = completed.returncode == 0
            compile_detail = completed.stdout.strip() or completed.stderr.strip() or "compileall completed."
    checks.append(
        _check_result(
            "compileall",
            "`compileall` passes for the exported release source",
            status="pass" if compile_ok else "fail",
            details=compile_detail,
            blocking=not compile_ok,
        )
    )

    with tempfile.TemporaryDirectory(prefix="ait-release-tests-") as temp_dir:
        export_dir = Path(temp_dir) / "source"
        export_dir.mkdir(parents=True, exist_ok=True)
        _materialize_bundle(bundle, export_dir)
        if tests_command:
            completed = subprocess.run(tests_command, cwd=str(export_dir), text=True, shell=True, capture_output=True)
            tests_ok = completed.returncode == 0
            checks.append(
                _check_result(
                    "tests",
                    "Release test status is explicitly recorded",
                    status="pass" if tests_ok else "fail",
                    details=(completed.stdout.strip() or completed.stderr.strip() or tests_command),
                    blocking=not tests_ok,
                )
            )
        elif skip_tests_reason:
            checks.append(
                _check_result(
                    "tests",
                    "Release test status is explicitly recorded",
                    status="skipped",
                    details=skip_tests_reason.strip(),
                    blocking=False,
                )
            )
        else:
            checks.append(
                _check_result(
                    "tests",
                    "Release test status is explicitly recorded",
                    status="fail",
                    details="No `--tests-command` or `--skip-tests-reason` was supplied.",
                    blocking=True,
                )
            )

    license_files = [path for path in profile_settings["license_files"] if path in file_map]
    checks.append(
        _check_result(
            "license_readiness",
            "License and notice artifacts are present for the selected profile",
            status="pass" if len(license_files) == len(profile_settings["license_files"]) else "fail",
            details=(
                f"Found {', '.join(license_files)}."
                if len(license_files) == len(profile_settings["license_files"])
                else f"Missing: {', '.join(path for path in profile_settings['license_files'] if path not in license_files)}."
            ),
            blocking=len(license_files) != len(profile_settings["license_files"]),
        )
    )

    contributor_files = [path for path in profile_settings["contributor_files"] if path in file_map]
    checks.append(
        _check_result(
            "contributor_readiness",
            "Contributor guidance exists for the selected profile",
            status="pass" if contributor_files else "fail",
            details=f"Found {', '.join(contributor_files)}." if contributor_files else "Missing CONTRIBUTING.md.",
            blocking=not contributor_files,
        )
    )

    quickstart_files = [path for path in profile_settings["quickstart_files"] if path in file_map]
    checks.append(
        _check_result(
            "quickstart_readiness",
            "Quickstart guidance exists for the selected profile",
            status="pass" if quickstart_files else "fail",
            details=f"Found {', '.join(quickstart_files)}." if quickstart_files else "Missing release-facing quickstart docs.",
            blocking=not quickstart_files,
        )
    )

    scripts = pyproject["scripts"] if isinstance(pyproject.get("scripts"), dict) else {}
    missing_scripts = [name for name in profile_settings["required_scripts"] if name not in scripts]
    checks.append(
        _check_result(
            "package_targets",
            "Package targets required by the selected profile are present",
            status="pass"
            if not missing_scripts and not [name for name in profile_settings.get("forbidden_scripts", []) if name in scripts]
            else "fail",
            details=(
                "All required console scripts are declared in pyproject.toml and forbidden scripts are absent."
                if not missing_scripts and not [name for name in profile_settings.get("forbidden_scripts", []) if name in scripts]
                else "; ".join(
                    detail
                    for detail in (
                        f"Missing scripts: {', '.join(missing_scripts)}." if missing_scripts else "",
                        f"Forbidden scripts still present: {', '.join(name for name in profile_settings.get('forbidden_scripts', []) if name in scripts)}."
                        if [name for name in profile_settings.get("forbidden_scripts", []) if name in scripts]
                        else "",
                    )
                    if detail
                )
            ),
            blocking=bool(missing_scripts or [name for name in profile_settings.get("forbidden_scripts", []) if name in scripts]),
        )
    )

    required_package_metadata = (
        profile_settings.get("required_package_metadata")
        if isinstance(profile_settings.get("required_package_metadata"), dict)
        else {}
    )
    if required_package_metadata:
        package_urls = pyproject.get("urls") if isinstance(pyproject.get("urls"), dict) else {}
        package_keywords = pyproject.get("keywords") if isinstance(pyproject.get("keywords"), list) else []
        package_classifiers = pyproject.get("classifiers") if isinstance(pyproject.get("classifiers"), list) else []
        readme_file = _readme_declared_file(pyproject.get("readme"))
        required_urls = [
            str(label).strip()
            for label in required_package_metadata.get("project_urls", [])
            if str(label).strip()
        ]
        missing_url_labels = [label for label in required_urls if label not in package_urls]
        expected_license = str(required_package_metadata.get("license") or "").strip()
        keywords_min_count = int(required_package_metadata.get("keywords_min_count") or 0)
        classifiers_min_count = int(required_package_metadata.get("classifiers_min_count") or 0)
        expected_readme_file = str(required_package_metadata.get("readme_file") or "").strip() or None
        metadata_failures: list[str] = []
        if expected_license and str(pyproject.get("license") or "").strip() != expected_license:
            metadata_failures.append(f"license should be {expected_license!r}")
        if expected_readme_file and readme_file != expected_readme_file:
            metadata_failures.append(f"readme should target {expected_readme_file}")
        if missing_url_labels:
            metadata_failures.append(f"missing project URLs: {', '.join(missing_url_labels)}")
        if len(package_keywords) < keywords_min_count:
            metadata_failures.append(f"keywords count {len(package_keywords)} is below {keywords_min_count}")
        if len(package_classifiers) < classifiers_min_count:
            metadata_failures.append(f"classifier count {len(package_classifiers)} is below {classifiers_min_count}")
        checks.append(
            _check_result(
                "package_metadata",
                "Public package metadata is ready for PyPI-facing publication",
                status="pass" if not metadata_failures else "fail",
                details=(
                    "Project URLs, readme target, keywords, classifiers, and license expression are present."
                    if not metadata_failures
                    else "; ".join(metadata_failures)
                ),
                blocking=bool(metadata_failures),
            )
        )
        if expected_readme_file:
            readme_entry = file_map.get(expected_readme_file)
            readme_findings: list[str] = []
            if readme_entry is None:
                readme_findings.append(f"missing readme file: {expected_readme_file}")
            else:
                for match in _MARKDOWN_LINK_RE.finditer(_bundle_entry_text(readme_entry)):
                    target = match.group(1).strip()
                    if target and not target.startswith(("#", "http://", "https://", "mailto:")):
                        readme_findings.append(target)
            checks.append(
                _check_result(
                    "package_readme_links",
                    "PyPI-facing package readme avoids local relative links",
                    status="pass" if not readme_findings else "fail",
                    details=(
                        "The public package readme uses only absolute or fragment links."
                        if not readme_findings
                        else f"Relative or missing readme targets: {', '.join(readme_findings[:5])}"
                    ),
                    blocking=bool(readme_findings),
                )
            )

    publish_support = (
        profile_settings.get("publish_support")
        if isinstance(profile_settings.get("publish_support"), dict)
        else {}
    )
    if publish_support:
        expected_files = [
            str(path).strip()
            for path in publish_support.get("files", [])
            if str(path).strip()
        ]
        missing_publish_files = [path for path in expected_files if path not in file_map]
        publish_failures: list[str] = []
        if missing_publish_files:
            publish_failures.append(f"missing files: {', '.join(missing_publish_files)}")
        workflow_path = str(publish_support.get("workflow_path") or "").strip()
        if workflow_path:
            workflow_entry = file_map.get(workflow_path)
            if workflow_entry is None:
                publish_failures.append(f"missing workflow: {workflow_path}")
            else:
                workflow_text = _bundle_entry_text(workflow_entry)
                workflow_missing = [
                    str(fragment)
                    for fragment in publish_support.get("workflow_contains", [])
                    if str(fragment).strip() and str(fragment) not in workflow_text
                ]
                if workflow_missing:
                    publish_failures.append(f"workflow missing fragments: {', '.join(workflow_missing)}")
        doc_path = str(publish_support.get("doc_path") or "").strip()
        if doc_path:
            doc_entry = file_map.get(doc_path)
            if doc_entry is None:
                publish_failures.append(f"missing publish doc: {doc_path}")
            else:
                doc_text = _bundle_entry_text(doc_entry)
                doc_missing = [
                    str(fragment)
                    for fragment in publish_support.get("doc_contains", [])
                    if str(fragment).strip() and str(fragment) not in doc_text
                ]
                if doc_missing:
                    publish_failures.append(f"publish doc missing fragments: {', '.join(doc_missing)}")
        checks.append(
            _check_result(
                "publish_automation",
                "PyPI trusted-publishing workflow and operator handoff are present",
                status="pass" if not publish_failures else "fail",
                details=(
                    "Trusted publishing workflow and operator doc are bundled for the clean public repo."
                    if not publish_failures
                    else "; ".join(publish_failures)
                ),
                blocking=bool(publish_failures),
            )
        )

    failed = [row for row in checks if row["status"] == "fail"]
    decision = "fail" if failed else "pass"
    metadata = _merge_metadata(
        record,
        package=pyproject,
        check_summary={
            "decision": decision,
            "failed": len(failed),
            "blocking": sum(1 for row in checks if row["blocking"]),
            "checked_at": datetime.now(timezone.utc).isoformat(),
        },
    )
    status_value = "checked" if decision == "pass" else record["status"]
    updated = update_local_release(
        ctx,
        release_id,
        status=status_value,
        checks=checks,
        metadata=metadata,
        event_type="release.checked",
    )
    return _hydrate_release(updated)


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
        final_artifacts: list[dict[str, Any]] = []
        for path in built_paths:
            final_artifacts.append(_artifact_info(path))

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

    artifacts = []
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
    )
    updated = update_local_release(
        ctx,
        release_id,
        status="built",
        artifacts=artifacts,
        formula={},
        metadata=metadata,
        event_type="release.built",
    )
    return _hydrate_release(updated)


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
    script_names = sorted(name for name in scripts if str(name).strip())
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
    metadata = _merge_metadata(
        record,
        formula=formula,
    )
    updated = update_local_release(
        ctx,
        release_id,
        artifacts=updated_artifacts,
        formula=formula,
        metadata=metadata,
        event_type="release.formula_generated",
    )
    return _hydrate_release(updated)
