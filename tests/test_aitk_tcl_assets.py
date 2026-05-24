from pathlib import Path
import shutil
import subprocess

import pytest


ROOT = Path(__file__).resolve().parents[1]
AITSCRIPT = ROOT / "src/ait_tk/aitk.tcl"


def test_aitk_tcl_script_exists():
    assert AITSCRIPT.exists(), "aitk.tcl should be created for the Tcl/Tk shell"


def test_aitk_tcl_script_is_syntactically_complete():
    tclsh = shutil.which("tclsh")
    if tclsh is None:
        pytest.skip("tclsh is not installed")

    probe = """
set path [lindex $argv 0]
set handle [open $path r]
set script [read $handle]
close $handle
if {![info complete $script]} {
    puts stderr "aitk.tcl is not a complete Tcl script"
    exit 1
}
"""
    subprocess.run([tclsh, "-", str(AITSCRIPT)], input=probe, text=True, check=True)


def test_aitk_markdown_heading_slug_handles_hyphen_collapse():
    tclsh = shutil.which("tclsh")
    if tclsh is None:
        pytest.skip("tclsh is not installed")

    text = AITSCRIPT.read_text(encoding="utf-8")
    start_marker = "proc Aitk::markdown_heading_slug {text} {"
    end_marker = "\n}\n\nproc Aitk::markdown_link_parts"
    start = text.index(start_marker)
    end = text.index(end_marker, start) + 3
    proc_text = text[start:end]
    probe = f"""
namespace eval Aitk {{}}
{proc_text}
puts [Aitk::markdown_heading_slug {{ait native PostgreSQL runtime delivery notes}}]
"""

    result = subprocess.run([tclsh], input=probe, text=True, capture_output=True, check=True)

    assert result.stdout.strip() == "ait-native-postgresql-runtime-delivery-notes"


def test_aitk_markdown_table_helpers_parse_alignment_and_widths():
    tclsh = shutil.which("tclsh")
    if tclsh is None:
        pytest.skip("tclsh is not installed")

    text = AITSCRIPT.read_text(encoding="utf-8")
    start_marker = "proc Aitk::markdown_table_candidate {line} {"
    end_marker = "\n}\n\nproc Aitk::insert_markdown_table_cell"
    start = text.index(start_marker)
    end = text.index(end_marker, start) + 3
    proc_text = text[start:end]
    probe = f"""
namespace eval Aitk {{}}
{proc_text}
puts [Aitk::markdown_table_candidate {{ | Name | Count | }}]
puts [join [Aitk::markdown_table_cells {{ | Name | Count | }}] ","]
set separators [Aitk::markdown_table_cells {{ | :--- | ---: | :---: | }}]
puts [Aitk::markdown_table_separator_cells $separators]
puts [join [Aitk::markdown_table_alignments $separators] ","]
set rows [list \
    [Aitk::markdown_table_cells {{| **Name** | [Task](docs/task.md) |}}] \
    [Aitk::markdown_table_cells {{| alpha | done |}}]]
puts [join [Aitk::markdown_table_widths $rows] ","]
"""

    result = subprocess.run([tclsh], input=probe, text=True, capture_output=True, check=True)

    assert result.stdout.splitlines() == [
        "1",
        "Name,Count",
        "1",
        "left,right,center",
        "5,4",
    ]


def test_aitk_tcl_shell_skeleton_contract():
    text = AITSCRIPT.read_text(encoding="utf-8")

    required_procs = [
        "proc Aitk::load_payload",
        "proc Aitk::snapshot_items",
        "proc Aitk::snapshot_matches_filters",
        "proc Aitk::snapshot_state_label",
        "proc Aitk::shorten_text",
        "proc Aitk::diff_line_tag",
        "proc Aitk::insert_diff_text",
        "proc Aitk::lazy_snapshot_diff_enabled",
        "proc Aitk::exec_in_repo",
        "proc Aitk::load_parent_diff_for_snapshot",
        "proc Aitk::refresh_views",
        "proc Aitk::render_plan_context",
        "proc Aitk::selected_snapshot_plan_contexts",
        "proc Aitk::plan_items",
        "proc Aitk::plan_matches_filters",
        "proc Aitk::render_plan_links",
        "proc Aitk::render_plan_canvas",
        "proc Aitk::select_tab",
        "proc Aitk::selected_plan_overview",
        "proc Aitk::select_plan_link",
        "proc Aitk::render_selected_plan_detail",
        "proc Aitk::render_selected_plan_preview",
        "proc Aitk::markdown_docs",
        "proc Aitk::filtered_markdown_docs",
        "proc Aitk::open_markdown_browser",
        "proc Aitk::render_markdown_browser",
        "proc Aitk::render_markdown_content",
        "proc Aitk::insert_markdown_inline",
        "proc Aitk::markdown_table_cells",
        "proc Aitk::markdown_table_separator_cells",
        "proc Aitk::render_markdown_table",
        "proc Aitk::configure_markdown_detail_tags",
        "proc Aitk::register_markdown_link",
        "proc Aitk::open_markdown_link",
        "proc Aitk::resolve_markdown_link_doc_path",
        "proc Aitk::scroll_markdown_anchor",
        "proc Aitk::render_selected_markdown_detail",
        "proc Aitk::render_graph",
        "proc Aitk::select_snapshot",
        "proc Aitk::render_diff",
    ]
    for proc_decl in required_procs:
        assert proc_decl in text, f"missing required proc declaration: {proc_decl}"

    # basic gitk-style layout pieces
    assert "ttk::frame .root.tab_bar" in text
    assert 'ttk::button .root.tab_bar.left.plan_button \\' in text
    assert '-text "計畫"' in text
    assert 'ttk::button .root.tab_bar.left.dispatch_button \\' in text
    assert '-text "下發任務"' in text
    assert 'ttk::button .root.tab_bar.markdown_button \\' in text
    assert '-text "Markdown"' in text
    assert '-command {Aitk::open_markdown_browser}' in text
    assert "pack .root.tab_bar.left -side left -anchor w" in text
    assert "pack .root.tab_bar.markdown_button -side right" in text
    assert "ttk::frame .root.tabs.markdown_tab" in text
    assert "pack .root.tabs.markdown_tab -fill both -expand 1" in text
    assert "Aitk::select_tab markdown" in text
    assert text.index('ttk::button .root.tab_bar.left.plan_button \\') < text.index(
        'ttk::button .root.tab_bar.left.dispatch_button \\'
    )
    assert "ttk::panedwindow" in text
    assert "canvas .root.tabs.dispatch_tab.main_split.top_split.graph_container.scroller.graph_canvas" in text
    assert "plans_right.scroller.plan_text" in text
    assert "canvas .root.tabs.plan_tab.plan_split.top_split.plan_list_container.scroller.plan_canvas" in text
    assert "plan_detail_text" in text
    assert "plan_preview_text" in text
    assert "plan_filter_query" in text
    assert "open_selected_plan_overview_link" in text
    assert "plan_context_by_snapshot" in text
    assert "plan_links" in text
    assert "markdown_docs" in text
    assert "markdown_filter_query" in text
    assert "listbox .root.tabs.markdown_tab.split.list.scroller.docs" in text
    assert "text .root.tabs.markdown_tab.split.detail.scroller.content" in text
    assert "Aitk::render_markdown_content $text_widget $content" in text
    assert "$text_widget insert end $content" not in text
    assert "AitkMarkdownH1" in text
    assert "md_code_block" in text
    assert "md_link" in text
    assert "$text_widget tag bind $tag <Button-1> [list Aitk::open_markdown_link $target]" in text
    assert "AitkMarkdownTableHeader" in text
    assert "Aitk::render_markdown_table $text_widget $rows $alignments" in text
    assert '$text_widget insert end "$line\\n" md_table' not in text
    assert "Aitk::render_markdown_browser $doc_path" in text
    assert "set markdown_filter_query \"\"" in text
    assert "dict set state markdown_anchor_indexes $anchors" in text
    assert "md_heading_anchor_$slug" in text
    assert "regsub -all -- {-+} $slug \"-\" slug" in text
    assert "$text_widget configure -state disabled" in text
    assert "toplevel .markdown_browser" not in text
    assert "wm title .markdown_browser" not in text
    assert "plan_markdown_button" not in text
    assert "row_markdown_button" not in text
    assert "open_selected_plan_link" in text
    assert "diff_text" in text
    assert "parent_diff" in text
    assert "Summary: parent diff" in text
    assert "graph_segments" in text
    assert "graph_column" in text
    assert "label_max_chars" in text
    assert "message_x" in text
    assert "#e8f1ff" in text
    assert "diff_added_row" in text
    assert "diff_removed_row" in text
    assert "diff_loader" in text
    assert "snapshot diff" in text
    assert "Loading selected snapshot diff" in text
    assert "exec {*}$command" in text
    assert "AitkDiffFont" in text
    assert "-spacing1 0 -spacing2 0 -spacing3 0" in text
    assert "-padx 4 -pady 1" in text
    assert "filter_query" in text
    assert "filter_stale_days" in text
    assert "provenance_badges" in text
    assert "gitk-style graph layout placeholder" not in text
    assert "set widgets(" not in text, "widgets should remain a Tcl dict, not an array"
    assert "set state(" not in text, "state should remain a Tcl dict, not an array"


def test_aitk_tcl_scrollbars_are_wired():
    text = AITSCRIPT.read_text(encoding="utf-8")

    assert "graph_vscroll" in text
    assert "graph_hscroll" in text
    assert "plan_vscroll" in text
    assert "plan_hscroll" in text
    assert "preview_vscroll" in text
    assert "preview_hscroll" in text
    assert "diff_vscroll" in text
    assert "diff_hscroll" in text
    assert "-yscrollcommand" in text
    assert "-xscrollcommand" in text
    assert "proc Aitk::sync_graph_scrollregion" in text
    assert "-scrollregion" in text


def test_aitk_tcl_diff_pane_prefers_inline_text_diff():
    text = AITSCRIPT.read_text(encoding="utf-8")

    inline_insert = "Aitk::insert_diff_text $diff_widget $inline_text"
    summary_insert = '$diff_widget insert end "\\nSummary: parent diff'
    metadata_insert = '$diff_widget insert end "Snapshot: $snapshot_id\\n"'

    assert "set rendered_text_diff 0" in text
    assert "proc Aitk::diff_line_tag" in text
    assert 'return diff_added_row' in text
    assert 'return diff_removed_row' in text
    assert "No inline text diff available for this snapshot." in text
    assert text.index(inline_insert) < text.index(summary_insert)
    assert text.index(summary_insert) < text.index(metadata_insert)


def test_aitk_tcl_payload_fallback_guidance_present():
    text = AITSCRIPT.read_text(encoding="utf-8")
    assert "unable to parse payload as JSON" in text
    assert "TSV" in text or "tsv" in text
    assert "load_payload_fallback" in text
