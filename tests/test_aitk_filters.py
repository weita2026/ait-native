from __future__ import annotations

from ait.aitk_filters import filter_history_rows, filter_line_health


def test_filter_line_health_query_matches_multiple_row_fields():
    rows = [
        {
            "snapshot_id": "SNP-001",
            "line_name": "feature/abc",
            "message": "refactor parser",
            "head_lines": ["feature/abc"],
            "is_contained": False,
            "age_days": 6,
        },
        {
            "snapshot_id": "SNP-002",
            "line_name": "main",
            "message": "bootstrap baseline",
            "head_labels": ["main"],
            "contained_in_main": True,
            "age_days": 1,
        },
    ]

    assert filter_line_health(rows, query="abc") == [rows[0]]
    assert filter_line_health(rows, query="SNP-002") == [rows[1]]
    assert filter_line_health(rows, query="bootstrap") == [rows[1]]
    assert filter_line_health(rows, query="feature/") == [rows[0]]


def test_filter_line_health_stale_threshold_uses_age_days_or_stale_age_days():
    rows = [
        {
            "snapshot_id": "SNP-A",
            "line_name": "feature/old",
            "age_days": 10,
            "is_contained": False,
        },
        {
            "snapshot_id": "SNP-B",
            "line_name": "feature/recent",
            "stale_age_days": 1,
            "contained_in_mainline": False,
        },
        {
            "snapshot_id": "SNP-C",
            "line_name": "main",
            "is_contained": True,
            "stale_age_days": 30,
        },
    ]

    assert filter_line_health(rows, stale_days=5) == [rows[0], rows[2]]


def test_filter_line_health_contained_and_uncontained_filters_are_supported():
    rows = [
        {
            "snapshot_id": "SNP-A",
            "line_name": "feature/a",
            "is_contained_in_main": True,
            "age_days": 7,
        },
        {
            "snapshot_id": "SNP-B",
            "line_name": "feature/b",
            "is_contained": False,
            "age_days": 7,
        },
    ]

    assert filter_line_health(rows, contained=True) == [rows[0]]
    assert filter_line_health(rows, uncontained=True) == [rows[1]]
    assert filter_line_health(rows, contained=True, uncontained=True) == rows


def test_filter_line_health_dirty_related_and_text_are_combinable():
    rows = [
        {
            "snapshot_id": "SNP-A",
            "line_name": "feature/dirty-a",
            "message": "first",
            "contained": False,
            "dirty_related": True,
            "stale_age_days": 8,
        },
        {
            "snapshot_id": "SNP-B",
            "line_name": "feature/dirty-b",
            "message": "second",
            "contained": False,
            "dirty_related": False,
            "stale_age_days": 8,
        },
    ]

    assert filter_line_health(rows, dirty_related=True, query="dirty-a") == [rows[0]]
    assert filter_line_health(rows, dirty_related=False, contained=True) == []
    assert filter_line_health(rows, dirty_related=True, stale_days=5) == [rows[0]]


def test_filter_history_rows_search_by_query_line_and_path_and_combination():
    rows = [
        {
            "snapshot_id": "SNP-100",
            "line_name": "feature/workspace",
            "message": "add support for logs",
            "changed_files": [{"path": "src/logger.py"}, {"path": "README.md"}],
            "head_lines": ["feature/workspace"],
        },
        {
            "snapshot_id": "SNP-101",
            "line_name": "feature/db",
            "title": "tweak schema",
            "changed_paths": ["src/db/schema.sql", "src/db/migrations.py"],
        },
        {
            "snapshot_id": "SNP-102",
            "line": "main",
            "message": "chore baseline",
            "paths": ["docs/plan.md"],
        },
    ]

    assert filter_history_rows(rows, query="log") == [rows[0]]
    assert filter_history_rows(rows, line="feature") == [rows[0], rows[1]]
    assert filter_history_rows(rows, path="schema.sql") == [rows[1]]
    assert filter_history_rows(rows, query="SNP-10", line="main") == [rows[2]]
    assert filter_history_rows(rows, query="chore", path="plan.md") == [rows[2]]


def test_filter_history_rows_no_criteria_returns_original_order_for_valid_rows():
    rows = [
        {"snapshot_id": "SNP-001", "line_name": "main", "message": "m1"},
        {"snapshot_id": "SNP-002", "line_name": "feature/a", "message": "m2"},
    ]

    assert filter_history_rows(rows) == rows
