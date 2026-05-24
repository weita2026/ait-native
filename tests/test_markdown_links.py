from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

from ait.repo_paths import RepoContext

WORKSPACE_ROOT = Path(__file__).resolve().parents[1]
AUTHORED_ROOT = RepoContext.discover(WORKSPACE_ROOT).repo_root
SCRIPT_PATH = WORKSPACE_ROOT / "scripts" / "check_markdown_links.py"
SPEC = importlib.util.spec_from_file_location("check_markdown_links", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
check_markdown_links = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = check_markdown_links
SPEC.loader.exec_module(check_markdown_links)


def test_markdown_link_checker_detects_missing_local_links(tmp_path: Path) -> None:
    docs = tmp_path / "docs"
    docs.mkdir()
    (docs / "source.md").write_text("[missing](./missing.md)\n", encoding="utf-8")

    issues = check_markdown_links.find_broken_links(tmp_path)

    assert len(issues) == 1
    assert issues[0].path == Path("docs/source.md")
    assert issues[0].line_number == 1


def test_markdown_link_checker_ignores_external_anchors_and_fences(tmp_path: Path) -> None:
    docs = tmp_path / "docs"
    docs.mkdir()
    (docs / "target.md").write_text("# Target\n", encoding="utf-8")
    (docs / "source.md").write_text(
        "\n".join(
            [
                "[target](./target.md)",
                "[anchor](#local-anchor)",
                "[external](https://example.com/path)",
                "```",
                "[fenced](./missing.md)",
                "```",
            ]
        ),
        encoding="utf-8",
    )

    assert check_markdown_links.find_broken_links(tmp_path) == []


def test_markdown_link_checker_skips_missing_sprint_targets_when_sprint_surface_is_absent(
    tmp_path: Path,
) -> None:
    docs = tmp_path / "docs"
    docs.mkdir()
    (docs / "index.md").write_text("[card](./sprints/card.md)\n", encoding="utf-8")

    assert check_markdown_links.find_broken_links(tmp_path) == []


def test_markdown_link_checker_still_reports_missing_sprint_targets_when_surface_exists(
    tmp_path: Path,
) -> None:
    docs = tmp_path / "docs"
    sprints = docs / "sprints"
    docs.mkdir()
    sprints.mkdir()
    (docs / "index.md").write_text("[card](./sprints/card.md)\n", encoding="utf-8")

    issues = check_markdown_links.find_broken_links(tmp_path)

    assert len(issues) == 1
    assert issues[0].path == Path("docs/index.md")
    assert issues[0].line_number == 1


def test_markdown_link_checker_ignores_dot_tmp_tool_dirs(tmp_path: Path) -> None:
    temp_env = tmp_path / ".tmp-release-tools"
    temp_docs = temp_env / "lib" / "python3.14" / "site-packages" / "demo"
    temp_docs.mkdir(parents=True)
    (temp_docs / "README.md").write_text("[missing](./missing.md)\n", encoding="utf-8")

    assert check_markdown_links.find_broken_links(tmp_path) == []


def test_repository_markdown_links_are_valid() -> None:
    assert check_markdown_links.find_broken_links(AUTHORED_ROOT) == []
