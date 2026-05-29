from __future__ import annotations

import base64
import fnmatch
import json
import os
import re
import shutil
import stat
import tomllib
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable

from .store_repo_reads import (
    get_line,
    get_snapshot,
    repo_status,
)
from .release_notes import (
    apply_release_notes_to_readme,
    collect_release_note_tasks,
)
from .store_local_releases import (
    create_local_release,
    get_local_release,
    update_local_release,
)
from .store_local_tasks import (
    get_local_task,
    list_local_tasks,
)
from .store_local_changes import (
    get_local_change,
    list_local_changes,
)
from .store_content_ops import (
    export_snapshot_bundle,
)
from .repo_paths import (
    RepoContext,
)
from .store_worktree_runtime import (
    current_line,
)
from .store_repo_config import (
    load_config,
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
            "docs/GITHUB_RELEASE_PUBLISHING.md",
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
                ".github/workflows/github-release-publish.yml",
                "docs/PYPI_PUBLISHING.md",
                "docs/GITHUB_RELEASE_PUBLISHING.md",
                "scripts/github_release_publish.sh",
            ],
            "workflow_checks": [
                {
                    "path": ".github/workflows/pypi-publish.yml",
                    "contains": [
                        "workflow_dispatch:",
                        "push:",
                        "tags:",
                        '"v*"',
                        "pypa/gh-action-pypi-publish@release/v1",
                        "id-token: write",
                        "name: pypi",
                        "https://pypi.org/p/ait-native",
                    ],
                },
                {
                    "path": ".github/workflows/github-release-publish.yml",
                    "contains": [
                        "workflow_dispatch:",
                        "push:",
                        "tags:",
                        '"v*"',
                        "contents: write",
                        "gh release create",
                        "gh release upload",
                        "release-assets-",
                    ],
                },
            ],
            "doc_checks": [
                {
                    "path": "docs/PYPI_PUBLISHING.md",
                    "contains": [
                        "weita2026/ait-native",
                        ".github/workflows/pypi-publish.yml",
                        "matching `v*` tag",
                        "Trusted Publisher",
                        "twine upload dist/*",
                        "GITHUB_RELEASE_PUBLISHING.md",
                    ],
                },
                {
                    "path": "docs/GITHUB_RELEASE_PUBLISHING.md",
                    "contains": [
                        "scripts/github_release_publish.sh",
                        ".github/workflows/github-release-publish.yml",
                        "release-assets-v*",
                        "workflow_dispatch",
                        "GITHUB_TOKEN",
                    ],
                },
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
    updated = dict(bundle)
    updated["files"] = _ordered_bundle_files(bundle, file_map)
    return updated


def _ordered_bundle_files(bundle: dict[str, Any], file_map: dict[str, dict[str, Any]]) -> list[dict[str, Any]]:
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
    return files


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


def _with_release_notes_readme(
    ctx: RepoContext,
    record: dict[str, Any],
    bundle: dict[str, Any],
) -> tuple[dict[str, Any], dict[str, Any] | None]:
    file_map = _bundle_file_map(bundle)
    readme_entry = file_map.get("README.md")
    if readme_entry is None:
        return bundle, None

    notes = collect_release_note_tasks(ctx, record)
    updated_text = apply_release_notes_to_readme(
        _bundle_text(file_map, "README.md"),
        record=record,
        notes=notes,
    )
    file_map["README.md"] = _bundle_entry_with_text(readme_entry, updated_text)

    updated = dict(bundle)
    updated["files"] = _ordered_bundle_files(bundle, file_map)
    return updated, notes


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


def _merge_metadata(record: dict[str, Any], **updates: Any) -> dict[str, Any]:
    metadata = dict(record.get("metadata") or {})
    metadata.update(updates)
    return metadata


def run_release_checks(
    ctx: RepoContext,
    release_id: str,
    *,
    tests_command: str | None = None,
    skip_tests_reason: str | None = None,
) -> dict[str, Any]:
    from .release_readiness import run_release_checks as _impl

    return _impl(
        ctx,
        release_id,
        tests_command=tests_command,
        skip_tests_reason=skip_tests_reason,
    )


def build_release_candidate(ctx: RepoContext, release_id: str) -> dict[str, Any]:
    from .release_artifact_builder import build_release_candidate as _impl

    return _impl(ctx, release_id)


def generate_release_formula(ctx: RepoContext, release_id: str, *, name: str) -> dict[str, Any]:
    from .release_artifact_builder import generate_release_formula as _impl

    return _impl(ctx, release_id, name=name)
