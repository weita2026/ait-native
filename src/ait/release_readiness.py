from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, Iterable

from .release_ops import (
    _MARKDOWN_LINK_RE,
    _bundle_entry_bytes,
    _bundle_entry_text,
    _bundle_file_map,
    _materialize_bundle,
    _merge_metadata,
    _pyproject_metadata,
    _readme_declared_file,
    _release_source_bundle,
    _require_profile,
    _workspace_candidate_roots,
    _workspace_matches_release_source,
    get_release_candidate,
)
from .repo_paths import RepoContext
from .store_local_releases import update_local_release
from .store_repo_reads import repo_status
from .store_worktree_runtime import current_line


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
        workflow_checks = (
            publish_support.get("workflow_checks")
            if isinstance(publish_support.get("workflow_checks"), list)
            else []
        )
        if not workflow_checks:
            legacy_workflow_path = str(publish_support.get("workflow_path") or "").strip()
            if legacy_workflow_path:
                workflow_checks = [
                    {
                        "path": legacy_workflow_path,
                        "contains": publish_support.get("workflow_contains") or [],
                    }
                ]
        doc_checks = (
            publish_support.get("doc_checks")
            if isinstance(publish_support.get("doc_checks"), list)
            else []
        )
        if not doc_checks:
            legacy_doc_path = str(publish_support.get("doc_path") or "").strip()
            if legacy_doc_path:
                doc_checks = [
                    {
                        "path": legacy_doc_path,
                        "contains": publish_support.get("doc_contains") or [],
                    }
                ]
        expected_files = [
            str(path).strip()
            for path in publish_support.get("files", [])
            if str(path).strip()
        ]
        missing_publish_files = [path for path in expected_files if path not in file_map]
        publish_failures: list[str] = []
        if missing_publish_files:
            publish_failures.append(f"missing files: {', '.join(missing_publish_files)}")
        for workflow_check in workflow_checks:
            if not isinstance(workflow_check, dict):
                continue
            workflow_path = str(workflow_check.get("path") or "").strip()
            if not workflow_path:
                continue
            workflow_entry = file_map.get(workflow_path)
            if workflow_entry is None:
                publish_failures.append(f"missing workflow: {workflow_path}")
                continue
            workflow_text = _bundle_entry_text(workflow_entry)
            workflow_missing = [
                str(fragment)
                for fragment in workflow_check.get("contains", [])
                if str(fragment).strip() and str(fragment) not in workflow_text
            ]
            if workflow_missing:
                publish_failures.append(f"{workflow_path} missing fragments: {', '.join(workflow_missing)}")
        for doc_check in doc_checks:
            if not isinstance(doc_check, dict):
                continue
            doc_path = str(doc_check.get("path") or "").strip()
            if not doc_path:
                continue
            doc_entry = file_map.get(doc_path)
            if doc_entry is None:
                publish_failures.append(f"missing publish doc: {doc_path}")
                continue
            doc_text = _bundle_entry_text(doc_entry)
            doc_missing = [
                str(fragment)
                for fragment in doc_check.get("contains", [])
                if str(fragment).strip() and str(fragment) not in doc_text
            ]
            if doc_missing:
                publish_failures.append(f"{doc_path} missing fragments: {', '.join(doc_missing)}")
        checks.append(
            _check_result(
                "publish_automation",
                "GitHub Release plus PyPI publication workflows and operator handoff are present",
                status="pass" if not publish_failures else "fail",
                details=(
                    "Public release workflows, asset helper script, and operator docs are bundled for the clean public repo."
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
    update_local_release(
        ctx,
        release_id,
        status=status_value,
        checks=checks,
        metadata=metadata,
        event_type="release.checked",
    )
    return get_release_candidate(ctx, release_id)
