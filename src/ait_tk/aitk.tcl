#!/usr/bin/env wish

# Aitk — Tcl/Tk history browser (MVP skeleton)
# Layout: gitk-like three-pane shell
# - top: graph canvas + selected plan context
# - bottom: selected snapshot diff pane

if {[catch {package require Tk} err]} {
    puts stderr "aitk: Tk package is required but not available: $err"
    exit 1
}

namespace eval Aitk {
    variable payload {}
    variable snapshots {}
    variable summary_lines {}
    variable selected_snapshot ""
    variable state [dict create selected_snapshot ""]
    variable widgets {}
    variable filter_query ""
    variable filter_stale_days ""
    variable plan_filter_query ""
    variable markdown_filter_query ""
}

# --- JSON / payload parsing -------------------------------------------------

proc Aitk::load_payload {path} {
    # Accept argv[0] from wish script: first command argument is payload path.
    if {![file exists $path]} {
        error "payload file does not exist: $path"
    }

    set content [read [open $path r]]
    if {[string trim $content] eq ""} {
        error "payload is empty"
    }

    # Preferred: Tcl json package
    if {![catch {
        package require json
        set parsed [json::json2dict $content]
    } parsed_err]} {
        return $parsed
    }

    # Fallback: TSV/line-oriented payload when json package is unavailable.
    # Expected header form examples:
    #   kind\tid\tline\tmessage\tparent\thealth
    #   snapshot\tSNP-001\tmain\tmsg...\tSNP-000\tgreen
    # or line based kv:
    #   meta\tkey\tvalue
    if {![catch {Aitk::load_payload_fallback $content} fallback_payload]} {
        return $fallback_payload
    }

    error "unable to parse payload as JSON. Install Tcl json package or use TSV payload.\n"

}

proc Aitk::load_payload_fallback {content} {
    set payload [dict create version "tsv" snapshots {} lines {} meta {}]
    set lines [split $content "\n"]
    set parsed_snapshots {}
    set parsed_lines {}
    set meta {}

    set in_payload 1
    foreach line_raw $lines {
        set line [string trim $line_raw]
        if {$line eq "" || [string match "#*" $line]} {
            continue
        }

        if {[string first "\u007b" $line] >= 0 || [string first "\u005b" $line] >= 0} {
            # JSON-like content without parser support -> abort fallback.
            set in_payload 0
            continue
        }

        set parts [split $line "\t"]
        if {[llength $parts] < 2} {
            continue
        }

        set kind [lindex $parts 0]
        switch -- $kind {
            snapshot {
                if {[llength $parts] < 3} {
                    continue
                }
                set id [lindex $parts 1]
                set line_name [lindex $parts 2]
                set message ""
                if {[llength $parts] >= 4} {
                    set message [lindex $parts 3]
                }
                set parent_snapshot_id ""
                if {[llength $parts] >= 5} {
                    set parent_snapshot_id [lindex $parts 4]
                }
                set health ""
                if {[llength $parts] >= 6} {
                    set health [lindex $parts 5]
                }
                lappend parsed_snapshots [dict create \
                    snapshot_id $id \
                    snapshot_line $line_name \
                    message $message \
                    parent_snapshot_id $parent_snapshot_id \
                    health $health
                ]
            }
            line {
                if {[llength $parts] < 3} {
                    continue
                }
                set line_name [lindex $parts 1]
                set head [lindex $parts 2]
                set status ""
                if {[llength $parts] >= 4} {
                    set status [lindex $parts 3]
                }
                lappend parsed_lines [dict create \
                    line_name $line_name \
                    head_snapshot_id $head \
                    status $status
                ]
            }
            meta {
                if {[llength $parts] >= 3} {
                    dict set meta [lindex $parts 1] [lindex $parts 2]
                }
            }
            default {
                # Support simple kv payload rows: key=val
                set row_key $kind
                set row_value [lindex $parts 1]
                if {$in_payload} {
                    dict set meta $row_key $row_value
                }
            }
        }
    }

    dict set payload snapshots $parsed_snapshots
    dict set payload lines $parsed_lines
    dict set payload meta $meta
    return $payload
}

# --- UI helpers --------------------------------------------------------------

proc Aitk::widget_path {name} {
    variable widgets
    if {![dict exists $widgets $name]} {
        return ""
    }
    return [dict get $widgets $name]
}

proc Aitk::dict_get_default {value key default} {
    if {[dict exists $value $key]} {
        return [dict get $value $key]
    }
    return $default
}

proc Aitk::truthy {value} {
    set text [string tolower [string trim $value]]
    return [expr {$text in {"1" "true" "yes" "on"}}]
}

proc Aitk::snapshot_items {} {
    set filtered {}
    foreach item [Aitk::all_snapshot_items] {
        if {[Aitk::snapshot_matches_filters $item]} {
            lappend filtered $item
        }
    }
    return $filtered
}

proc Aitk::all_snapshot_items {} {
    variable payload
    if {[dict exists $payload history_rows]} {
        return [dict get $payload history_rows]
    }
    if {[dict exists $payload snapshots]} {
        return [dict get $payload snapshots]
    }
    return {}
}

proc Aitk::snapshot_matches_filters {snapshot} {
    variable filter_query
    variable filter_stale_days

    set query [string tolower [string trim $filter_query]]
    if {$query ne ""} {
        set haystack [Aitk::snapshot_search_text $snapshot]
        if {[string first $query $haystack] < 0} {
            return 0
        }
    }

    set threshold [string trim $filter_stale_days]
    if {$threshold ne ""} {
        if {![string is double -strict $threshold]} {
            return 1
        }
        set age [Aitk::dict_get_default $snapshot age_days ""]
        if {$age eq "" || ![string is double -strict $age] || $age < $threshold} {
            return 0
        }
    }

    return 1
}

proc Aitk::snapshot_search_text {snapshot} {
    set parts {}
    foreach key {snapshot_id line_name snapshot_line message parent_snapshot_id marker} {
        if {[dict exists $snapshot $key]} {
            lappend parts [dict get $snapshot $key]
        }
    }
    foreach key {head_lines provenance_badges changed_files} {
        if {[dict exists $snapshot $key]} {
            foreach value [dict get $snapshot $key] {
                lappend parts $value
            }
        }
    }
    if {[dict exists $snapshot parent_diff files]} {
        foreach file_row [dict get $snapshot parent_diff files] {
            if {[dict exists $file_row path]} {
                lappend parts [dict get $file_row path]
            }
            if {[dict exists $file_row status]} {
                lappend parts [dict get $file_row status]
            }
        }
    }
    return [string tolower [join $parts " "]]
}

proc Aitk::all_plan_items {} {
    variable payload
    if {[dict exists $payload plan_links]} {
        return [dict get $payload plan_links]
    }
    return {}
}

proc Aitk::plan_items {} {
    set filtered {}
    foreach item [Aitk::all_plan_items] {
        if {[Aitk::plan_matches_filters $item]} {
            lappend filtered $item
        }
    }
    return $filtered
}

proc Aitk::plan_matches_filters {plan} {
    variable plan_filter_query

    set query [string tolower [string trim $plan_filter_query]]
    if {$query eq ""} {
        return 1
    }
    return [expr {[string first $query [Aitk::plan_search_text $plan]] >= 0}]
}

proc Aitk::plan_search_text {plan} {
    set parts {}
    foreach key {kind source plan_id title status head_revision_id artifact_path artifact_selector artifact_heading display_path} {
        if {[dict exists $plan $key]} {
            lappend parts [dict get $plan $key]
        }
    }
    if {[dict exists $plan items]} {
        foreach item [dict get $plan items] {
            foreach key {plan_item_ref text checkbox_state} {
                if {[dict exists $item $key]} {
                    lappend parts [dict get $item $key]
                }
            }
            if {[dict exists $item heading_path]} {
                foreach heading [dict get $item heading_path] {
                    lappend parts $heading
                }
            }
        }
    }
    return [string tolower [join $parts " "]]
}

proc Aitk::plan_link_key {plan} {
    set plan_id [Aitk::dict_get_default $plan plan_id ""]
    if {$plan_id ne ""} {
        return "plan:$plan_id"
    }
    set display [Aitk::plan_display_path $plan]
    if {$display ne ""} {
        return "path:$display"
    }
    set title [Aitk::dict_get_default $plan title ""]
    if {$title ne ""} {
        return "title:$title"
    }
    return "plan:[llength [Aitk::all_plan_items]]"
}

proc Aitk::plan_item_count {plan} {
    if {![dict exists $plan items]} {
        return 0
    }
    return [llength [dict get $plan items]]
}

proc Aitk::markdown_docs {} {
    variable payload
    if {[dict exists $payload markdown_docs]} {
        return [dict get $payload markdown_docs]
    }
    return {}
}

proc Aitk::markdown_doc_key {doc} {
    return [Aitk::dict_get_default $doc path [Aitk::dict_get_default $doc display_path ""]]
}

proc Aitk::markdown_search_text {doc} {
    set parts {}
    foreach key {path display_path title source kind} {
        if {[dict exists $doc $key]} {
            lappend parts [dict get $doc $key]
        }
    }
    return [string tolower [join $parts " "]]
}

proc Aitk::markdown_matches_filters {doc} {
    variable markdown_filter_query
    set query [string tolower [string trim $markdown_filter_query]]
    if {$query eq ""} {
        return 1
    }
    return [expr {[string first $query [Aitk::markdown_search_text $doc]] >= 0}]
}

proc Aitk::filtered_markdown_docs {} {
    set filtered {}
    foreach doc [Aitk::markdown_docs] {
        if {[Aitk::markdown_matches_filters $doc]} {
            lappend filtered $doc
        }
    }
    return $filtered
}

proc Aitk::line_for_snapshot {snapshot} {
    if {[dict exists $snapshot line_name]} {
        return [dict get $snapshot line_name]
    }
    if {[dict exists $snapshot snapshot_line]} {
        return [dict get $snapshot snapshot_line]
    }
    return ""
}

proc Aitk::snapshot_state_label {snapshot} {
    set values {}
    if {[dict exists $snapshot marker]} {
        lappend values [dict get $snapshot marker]
    }
    if {[dict exists $snapshot provenance_badges]} {
        foreach badge [dict get $snapshot provenance_badges] {
            lappend values $badge
        }
    }
    return [join $values ","]
}

proc Aitk::safe_tag {prefix value} {
    return [string map {":" "_" "/" "_" "." "_" "-" "_"} "${prefix}_${value}"]
}

proc Aitk::shorten_text {value max_chars} {
    set text [string trim [string map {"\n" " " "\r" " " "\t" " "} $value]]
    if {$max_chars <= 0} {
        return ""
    }
    if {[string length $text] <= $max_chars} {
        return $text
    }
    if {$max_chars <= 3} {
        return [string range $text 0 [expr {$max_chars - 1}]]
    }
    return "[string range $text 0 [expr {$max_chars - 4}]]..."
}

proc Aitk::diff_line_tag {line} {
    if {[regexp {^(\+\+\+|---)(\s|$)} $line]} {
        return diff_meta_row
    }
    if {[string match "@@*" $line]} {
        return diff_hunk_row
    }
    if {[string match "diff --git*" $line] || [string match "index *" $line]} {
        return diff_meta_row
    }

    set prefix [string index $line 0]
    if {$prefix eq "+"} {
        return diff_added_row
    }
    if {$prefix eq "-"} {
        return diff_removed_row
    }
    return ""
}

proc Aitk::insert_diff_text {diff_widget inline_text} {
    set lines [split $inline_text "\n"]
    set last_index [expr {[llength $lines] - 1}]
    for {set idx 0} {$idx <= $last_index} {incr idx} {
        set line [lindex $lines $idx]
        if {$idx == $last_index && $line eq ""} {
            continue
        }
        set tag [Aitk::diff_line_tag $line]
        if {$tag eq ""} {
            $diff_widget insert end "$line\n"
        } else {
            $diff_widget insert end "$line\n" $tag
        }
    }
}

proc Aitk::lazy_snapshot_diff_enabled {} {
    variable payload
    if {![dict exists $payload diff_loader]} {
        return 0
    }
    set loader [dict get $payload diff_loader]
    return [Aitk::truthy [Aitk::dict_get_default $loader enabled 0]]
}

proc Aitk::diff_error_payload {old_snapshot_id new_snapshot_id message} {
    return [dict create \
        old_snapshot_id $old_snapshot_id \
        new_snapshot_id $new_snapshot_id \
        files {} \
        summary [dict create \
            old_snapshot_id $old_snapshot_id \
            new_snapshot_id $new_snapshot_id \
            files_changed "" \
            insertions 0 \
            deletions 0 \
        ] \
        error $message \
    ]
}

proc Aitk::exec_in_repo {command} {
    variable payload
    set repo_root [Aitk::dict_get_default $payload repo_root ""]
    set old_cwd [pwd]
    set changed_dir 0
    if {$repo_root ne "" && [file isdirectory $repo_root]} {
        cd $repo_root
        set changed_dir 1
    }
    set code [catch {exec {*}$command} output]
    if {$changed_dir} {
        cd $old_cwd
    }
    if {$code != 0} {
        error $output
    }
    return $output
}

proc Aitk::load_parent_diff_for_snapshot {snapshot_id snapshot} {
    variable payload
    if {![Aitk::lazy_snapshot_diff_enabled]} {
        return $snapshot
    }
    if {[dict exists $snapshot parent_diff]} {
        return $snapshot
    }
    if {![dict exists $payload diff_loader]} {
        return $snapshot
    }

    set parent_snapshot_id [Aitk::dict_get_default $snapshot parent_snapshot_id ""]
    set loader [dict get $payload diff_loader]
    set ait_cli [Aitk::dict_get_default $loader ait_cli_path "ait"]
    set include_text [Aitk::truthy [Aitk::dict_get_default $loader include_text 1]]
    set max_bytes [Aitk::dict_get_default $loader max_bytes 128000]

    set command [list $ait_cli snapshot diff $parent_snapshot_id $snapshot_id --json --max-bytes $max_bytes]
    if {$include_text} {
        lappend command --include-text
    }

    if {[catch {set raw [Aitk::exec_in_repo $command]} load_err]} {
        set diff [Aitk::diff_error_payload $parent_snapshot_id $snapshot_id $load_err]
    } elseif {[catch {
        package require json
        set diff [json::json2dict $raw]
    } parse_err]} {
        set diff [Aitk::diff_error_payload $parent_snapshot_id $snapshot_id "unable to parse lazy diff JSON: $parse_err"]
    }

    dict set payload snapshots_index $snapshot_id parent_diff $diff
    set changed_files {}
    if {[dict exists $diff files]} {
        foreach file_row [dict get $diff files] {
            if {[dict exists $file_row path]} {
                lappend changed_files [dict get $file_row path]
            }
        }
    }
    dict set payload snapshots_index $snapshot_id changed_files $changed_files
    return [dict get $payload snapshots_index $snapshot_id]
}

proc Aitk::refresh_views {} {
    Aitk::render_graph
    Aitk::render_plan_context

    set snapshots [Aitk::snapshot_items]
    if {[llength $snapshots] > 0} {
        set first [lindex $snapshots 0]
        if {[dict exists $first snapshot_id]} {
            Aitk::select_snapshot [dict get $first snapshot_id]
        }
    }
}

proc Aitk::sync_graph_scrollregion {canvas_widget} {
    set bbox [$canvas_widget bbox all]
    if {$bbox eq ""} {
        set bbox {0 0 1 1}
    }
    $canvas_widget configure -scrollregion $bbox
}

proc Aitk::select_tab {tab_name} {
    variable state

    if {[winfo exists .root.tabs.plan_tab]} {
        pack forget .root.tabs.plan_tab
    }
    if {[winfo exists .root.tabs.dispatch_tab]} {
        pack forget .root.tabs.dispatch_tab
    }
    if {[winfo exists .root.tabs.markdown_tab]} {
        pack forget .root.tabs.markdown_tab
    }

    if {$tab_name eq "markdown"} {
        pack .root.tabs.markdown_tab -fill both -expand 1
    } elseif {$tab_name eq "dispatch"} {
        pack .root.tabs.dispatch_tab -fill both -expand 1
    } else {
        set tab_name "plan"
        pack .root.tabs.plan_tab -fill both -expand 1
    }
    dict set state current_tab $tab_name

    if {[winfo exists .root.tab_bar.left.plan_button] && [winfo exists .root.tab_bar.left.dispatch_button] && [winfo exists .root.tab_bar.markdown_button]} {
        .root.tab_bar.left.plan_button state !disabled
        .root.tab_bar.left.dispatch_button state !disabled
        .root.tab_bar.markdown_button state !disabled
        if {$tab_name eq "plan"} {
            .root.tab_bar.left.plan_button state disabled
        } elseif {$tab_name eq "markdown"} {
            .root.tab_bar.markdown_button state disabled
        } else {
            .root.tab_bar.left.dispatch_button state disabled
        }
    }
}

proc Aitk::build_ui {} {
    variable widgets

    wm title . "aitk"
    wm geometry . "1200x800"

    ttk::frame .root -padding 6
    pack .root -fill both -expand 1

    ttk::frame .root.tab_bar
    ttk::frame .root.tab_bar.left
    ttk::button .root.tab_bar.left.plan_button \
        -text "計畫" \
        -command {Aitk::select_tab plan}
    ttk::button .root.tab_bar.left.dispatch_button \
        -text "下發任務" \
        -command {Aitk::select_tab dispatch}
    ttk::button .root.tab_bar.markdown_button \
        -text "Markdown" \
        -command {Aitk::open_markdown_browser}
    pack .root.tab_bar.left -side left -anchor w
    pack .root.tab_bar.left.plan_button -side left
    pack .root.tab_bar.left.dispatch_button -side left -padx 4
    pack .root.tab_bar.markdown_button -side right
    pack .root.tab_bar -side top -fill x -pady {0 6}

    ttk::frame .root.tabs
    pack .root.tabs -side top -fill both -expand 1

    ttk::frame .root.tabs.plan_tab
    ttk::frame .root.tabs.dispatch_tab
    ttk::frame .root.tabs.markdown_tab

    # Dispatch/task tab: vertical split with graph+context above selected diff.
    ttk::panedwindow .root.tabs.dispatch_tab.main_split -orient vertical
    pack .root.tabs.dispatch_tab.main_split -fill both -expand 1

    ttk::panedwindow .root.tabs.dispatch_tab.main_split.top_split -orient horizontal

    ttk::frame .root.tabs.dispatch_tab.main_split.top_split.graph_container
    ttk::frame .root.tabs.dispatch_tab.main_split.top_split.plans_container

    ttk::label .root.tabs.dispatch_tab.main_split.top_split.graph_container.header -text "History graph (gitk style layout)"
    ttk::frame .root.tabs.dispatch_tab.main_split.top_split.graph_container.filters
    ttk::label .root.tabs.dispatch_tab.main_split.top_split.graph_container.filters.query_label -text "Search"
    ttk::entry .root.tabs.dispatch_tab.main_split.top_split.graph_container.filters.query_entry \
        -textvariable Aitk::filter_query -width 22
    ttk::label .root.tabs.dispatch_tab.main_split.top_split.graph_container.filters.stale_label -text "Age days"
    ttk::entry .root.tabs.dispatch_tab.main_split.top_split.graph_container.filters.stale_entry \
        -textvariable Aitk::filter_stale_days -width 8
    ttk::button .root.tabs.dispatch_tab.main_split.top_split.graph_container.filters.clear_button \
        -text "Clear" \
        -command {set Aitk::filter_query ""; set Aitk::filter_stale_days ""; Aitk::refresh_views}
    ttk::frame .root.tabs.dispatch_tab.main_split.top_split.graph_container.scroller
    canvas .root.tabs.dispatch_tab.main_split.top_split.graph_container.scroller.graph_canvas \
        -width 620 -height 520 -background #ffffff \
        -xscrollcommand ".root.tabs.dispatch_tab.main_split.top_split.graph_container.scroller.graph_hscroll set" \
        -yscrollcommand ".root.tabs.dispatch_tab.main_split.top_split.graph_container.scroller.graph_vscroll set"
    ttk::scrollbar .root.tabs.dispatch_tab.main_split.top_split.graph_container.scroller.graph_vscroll \
        -orient vertical \
        -command ".root.tabs.dispatch_tab.main_split.top_split.graph_container.scroller.graph_canvas yview"
    ttk::scrollbar .root.tabs.dispatch_tab.main_split.top_split.graph_container.scroller.graph_hscroll \
        -orient horizontal \
        -command ".root.tabs.dispatch_tab.main_split.top_split.graph_container.scroller.graph_canvas xview"

    ttk::frame .root.tabs.dispatch_tab.main_split.top_split.plans_right
    ttk::label .root.tabs.dispatch_tab.main_split.top_split.plans_right.title -text "Plan context"
    ttk::frame .root.tabs.dispatch_tab.main_split.top_split.plans_right.scroller
    text .root.tabs.dispatch_tab.main_split.top_split.plans_right.scroller.plan_text \
        -wrap word -height 22 \
        -yscrollcommand ".root.tabs.dispatch_tab.main_split.top_split.plans_right.scroller.vscroll set"
    ttk::scrollbar .root.tabs.dispatch_tab.main_split.top_split.plans_right.scroller.vscroll \
        -orient vertical \
        -command ".root.tabs.dispatch_tab.main_split.top_split.plans_right.scroller.plan_text yview"
    ttk::frame .root.tabs.dispatch_tab.main_split.top_split.plans_right.actions
    ttk::button .root.tabs.dispatch_tab.main_split.top_split.plans_right.actions.open_button \
        -text "Open" \
        -command {Aitk::open_selected_plan_link}
    ttk::button .root.tabs.dispatch_tab.main_split.top_split.plans_right.actions.copy_button \
        -text "Copy" \
        -command {Aitk::copy_selected_plan_link}

    pack .root.tabs.dispatch_tab.main_split.top_split.graph_container.header -side top -anchor w
    pack .root.tabs.dispatch_tab.main_split.top_split.graph_container.filters -side top -anchor w -fill x
    pack .root.tabs.dispatch_tab.main_split.top_split.graph_container.filters.query_label -side left
    pack .root.tabs.dispatch_tab.main_split.top_split.graph_container.filters.query_entry -side left -padx 4
    pack .root.tabs.dispatch_tab.main_split.top_split.graph_container.filters.stale_label -side left
    pack .root.tabs.dispatch_tab.main_split.top_split.graph_container.filters.stale_entry -side left -padx 4
    pack .root.tabs.dispatch_tab.main_split.top_split.graph_container.filters.clear_button -side left -padx 4
    grid .root.tabs.dispatch_tab.main_split.top_split.graph_container.scroller.graph_canvas -row 0 -column 0 -sticky nsew
    grid .root.tabs.dispatch_tab.main_split.top_split.graph_container.scroller.graph_vscroll -row 0 -column 1 -sticky ns
    grid .root.tabs.dispatch_tab.main_split.top_split.graph_container.scroller.graph_hscroll -row 1 -column 0 -sticky ew
    grid rowconfigure .root.tabs.dispatch_tab.main_split.top_split.graph_container.scroller 0 -weight 1
    grid columnconfigure .root.tabs.dispatch_tab.main_split.top_split.graph_container.scroller 0 -weight 1
    pack .root.tabs.dispatch_tab.main_split.top_split.graph_container.scroller -side top -fill both -expand 1

    pack .root.tabs.dispatch_tab.main_split.top_split.plans_right.title -side top -anchor w -fill x
    pack .root.tabs.dispatch_tab.main_split.top_split.plans_right.actions -side bottom -anchor e -fill x
    pack .root.tabs.dispatch_tab.main_split.top_split.plans_right.actions.copy_button -side right -padx 4
    pack .root.tabs.dispatch_tab.main_split.top_split.plans_right.actions.open_button -side right -padx 4
    grid .root.tabs.dispatch_tab.main_split.top_split.plans_right.scroller.plan_text -row 0 -column 0 -sticky nsew
    grid .root.tabs.dispatch_tab.main_split.top_split.plans_right.scroller.vscroll -row 0 -column 1 -sticky ns
    grid rowconfigure .root.tabs.dispatch_tab.main_split.top_split.plans_right.scroller 0 -weight 1
    grid columnconfigure .root.tabs.dispatch_tab.main_split.top_split.plans_right.scroller 0 -weight 1
    pack .root.tabs.dispatch_tab.main_split.top_split.plans_right.scroller -side top -fill both -expand 1

    pack .root.tabs.dispatch_tab.main_split.top_split.plans_right -in .root.tabs.dispatch_tab.main_split.top_split.plans_container -fill both -expand 1

    bind .root.tabs.dispatch_tab.main_split.top_split.graph_container.filters.query_entry <KeyRelease> {Aitk::refresh_views}
    bind .root.tabs.dispatch_tab.main_split.top_split.graph_container.filters.stale_entry <KeyRelease> {Aitk::refresh_views}

    # bottom diff area
    ttk::frame .root.tabs.dispatch_tab.main_split.bottom_split
    ttk::label .root.tabs.dispatch_tab.main_split.bottom_split.diff_label -text "Selected snapshot diff"
    ttk::frame .root.tabs.dispatch_tab.main_split.bottom_split.diff_scroller
    if {[lsearch -exact [font names] AitkDiffFont] < 0} {
        font create AitkDiffFont -family [font actual TkFixedFont -family] -size 9
    } else {
        font configure AitkDiffFont -family [font actual TkFixedFont -family] -size 9
    }
    text .root.tabs.dispatch_tab.main_split.bottom_split.diff_scroller.diff_text \
        -wrap none -height 16 -font AitkDiffFont \
        -padx 4 -pady 1 -spacing1 0 -spacing2 0 -spacing3 0 \
        -xscrollcommand ".root.tabs.dispatch_tab.main_split.bottom_split.diff_scroller.diff_hscroll set" \
        -yscrollcommand ".root.tabs.dispatch_tab.main_split.bottom_split.diff_scroller.diff_vscroll set"
    .root.tabs.dispatch_tab.main_split.bottom_split.diff_scroller.diff_text tag configure diff_added_row \
        -background #e8f7ee -foreground #146c2e
    .root.tabs.dispatch_tab.main_split.bottom_split.diff_scroller.diff_text tag configure diff_removed_row \
        -background #fdecec -foreground #9f1d20
    .root.tabs.dispatch_tab.main_split.bottom_split.diff_scroller.diff_text tag configure diff_hunk_row \
        -background #fff6d6 -foreground #7a5a00
    .root.tabs.dispatch_tab.main_split.bottom_split.diff_scroller.diff_text tag configure diff_meta_row \
        -background #f2f4f7 -foreground #4b5563
    ttk::scrollbar .root.tabs.dispatch_tab.main_split.bottom_split.diff_scroller.diff_vscroll \
        -orient vertical \
        -command ".root.tabs.dispatch_tab.main_split.bottom_split.diff_scroller.diff_text yview"
    ttk::scrollbar .root.tabs.dispatch_tab.main_split.bottom_split.diff_scroller.diff_hscroll \
        -orient horizontal \
        -command ".root.tabs.dispatch_tab.main_split.bottom_split.diff_scroller.diff_text xview"
    pack .root.tabs.dispatch_tab.main_split.bottom_split.diff_label -side top -anchor w
    grid .root.tabs.dispatch_tab.main_split.bottom_split.diff_scroller.diff_text -row 0 -column 0 -sticky nsew
    grid .root.tabs.dispatch_tab.main_split.bottom_split.diff_scroller.diff_vscroll -row 0 -column 1 -sticky ns
    grid .root.tabs.dispatch_tab.main_split.bottom_split.diff_scroller.diff_hscroll -row 1 -column 0 -sticky ew
    grid rowconfigure .root.tabs.dispatch_tab.main_split.bottom_split.diff_scroller 0 -weight 1
    grid columnconfigure .root.tabs.dispatch_tab.main_split.bottom_split.diff_scroller 0 -weight 1
    pack .root.tabs.dispatch_tab.main_split.bottom_split.diff_scroller -side top -fill both -expand 1

    .root.tabs.dispatch_tab.main_split.top_split add .root.tabs.dispatch_tab.main_split.top_split.graph_container
    .root.tabs.dispatch_tab.main_split.top_split add .root.tabs.dispatch_tab.main_split.top_split.plans_container
    .root.tabs.dispatch_tab.main_split add .root.tabs.dispatch_tab.main_split.top_split
    .root.tabs.dispatch_tab.main_split add .root.tabs.dispatch_tab.main_split.bottom_split

    # Plan tab: same overview/detail/preview rhythm as the dispatch tab.
    ttk::panedwindow .root.tabs.plan_tab.plan_split -orient vertical
    pack .root.tabs.plan_tab.plan_split -fill both -expand 1
    ttk::panedwindow .root.tabs.plan_tab.plan_split.top_split -orient horizontal

    ttk::frame .root.tabs.plan_tab.plan_split.top_split.plan_list_container
    ttk::label .root.tabs.plan_tab.plan_split.top_split.plan_list_container.header -text "Plans"
    ttk::frame .root.tabs.plan_tab.plan_split.top_split.plan_list_container.filters
    ttk::label .root.tabs.plan_tab.plan_split.top_split.plan_list_container.filters.query_label -text "Search"
    ttk::entry .root.tabs.plan_tab.plan_split.top_split.plan_list_container.filters.query_entry \
        -textvariable Aitk::plan_filter_query -width 28
    ttk::button .root.tabs.plan_tab.plan_split.top_split.plan_list_container.filters.clear_button \
        -text "Clear" \
        -command {set Aitk::plan_filter_query ""; Aitk::refresh_plan_views}
    ttk::frame .root.tabs.plan_tab.plan_split.top_split.plan_list_container.scroller
    canvas .root.tabs.plan_tab.plan_split.top_split.plan_list_container.scroller.plan_canvas \
        -width 620 -height 520 -background #ffffff \
        -xscrollcommand ".root.tabs.plan_tab.plan_split.top_split.plan_list_container.scroller.plan_hscroll set" \
        -yscrollcommand ".root.tabs.plan_tab.plan_split.top_split.plan_list_container.scroller.plan_vscroll set"
    ttk::scrollbar .root.tabs.plan_tab.plan_split.top_split.plan_list_container.scroller.plan_vscroll \
        -orient vertical \
        -command ".root.tabs.plan_tab.plan_split.top_split.plan_list_container.scroller.plan_canvas yview"
    ttk::scrollbar .root.tabs.plan_tab.plan_split.top_split.plan_list_container.scroller.plan_hscroll \
        -orient horizontal \
        -command ".root.tabs.plan_tab.plan_split.top_split.plan_list_container.scroller.plan_canvas xview"

    ttk::frame .root.tabs.plan_tab.plan_split.top_split.plan_detail_container
    ttk::label .root.tabs.plan_tab.plan_split.top_split.plan_detail_container.title -text "Plan detail"
    ttk::frame .root.tabs.plan_tab.plan_split.top_split.plan_detail_container.scroller
    text .root.tabs.plan_tab.plan_split.top_split.plan_detail_container.scroller.plan_detail_text \
        -wrap word -height 22 \
        -yscrollcommand ".root.tabs.plan_tab.plan_split.top_split.plan_detail_container.scroller.vscroll set"
    ttk::scrollbar .root.tabs.plan_tab.plan_split.top_split.plan_detail_container.scroller.vscroll \
        -orient vertical \
        -command ".root.tabs.plan_tab.plan_split.top_split.plan_detail_container.scroller.plan_detail_text yview"
    ttk::frame .root.tabs.plan_tab.plan_split.top_split.plan_detail_container.actions
    ttk::button .root.tabs.plan_tab.plan_split.top_split.plan_detail_container.actions.open_button \
        -text "Open" \
        -command {Aitk::open_selected_plan_overview_link}
    ttk::button .root.tabs.plan_tab.plan_split.top_split.plan_detail_container.actions.copy_button \
        -text "Copy" \
        -command {Aitk::copy_selected_plan_overview_link}

    pack .root.tabs.plan_tab.plan_split.top_split.plan_list_container.header -side top -anchor w
    pack .root.tabs.plan_tab.plan_split.top_split.plan_list_container.filters -side top -anchor w -fill x
    pack .root.tabs.plan_tab.plan_split.top_split.plan_list_container.filters.query_label -side left
    pack .root.tabs.plan_tab.plan_split.top_split.plan_list_container.filters.query_entry -side left -padx 4
    pack .root.tabs.plan_tab.plan_split.top_split.plan_list_container.filters.clear_button -side left -padx 4
    grid .root.tabs.plan_tab.plan_split.top_split.plan_list_container.scroller.plan_canvas -row 0 -column 0 -sticky nsew
    grid .root.tabs.plan_tab.plan_split.top_split.plan_list_container.scroller.plan_vscroll -row 0 -column 1 -sticky ns
    grid .root.tabs.plan_tab.plan_split.top_split.plan_list_container.scroller.plan_hscroll -row 1 -column 0 -sticky ew
    grid rowconfigure .root.tabs.plan_tab.plan_split.top_split.plan_list_container.scroller 0 -weight 1
    grid columnconfigure .root.tabs.plan_tab.plan_split.top_split.plan_list_container.scroller 0 -weight 1
    pack .root.tabs.plan_tab.plan_split.top_split.plan_list_container.scroller -side top -fill both -expand 1

    pack .root.tabs.plan_tab.plan_split.top_split.plan_detail_container.title -side top -anchor w -fill x
    pack .root.tabs.plan_tab.plan_split.top_split.plan_detail_container.actions -side bottom -anchor e -fill x
    pack .root.tabs.plan_tab.plan_split.top_split.plan_detail_container.actions.copy_button -side right -padx 4
    pack .root.tabs.plan_tab.plan_split.top_split.plan_detail_container.actions.open_button -side right -padx 4
    grid .root.tabs.plan_tab.plan_split.top_split.plan_detail_container.scroller.plan_detail_text -row 0 -column 0 -sticky nsew
    grid .root.tabs.plan_tab.plan_split.top_split.plan_detail_container.scroller.vscroll -row 0 -column 1 -sticky ns
    grid rowconfigure .root.tabs.plan_tab.plan_split.top_split.plan_detail_container.scroller 0 -weight 1
    grid columnconfigure .root.tabs.plan_tab.plan_split.top_split.plan_detail_container.scroller 0 -weight 1
    pack .root.tabs.plan_tab.plan_split.top_split.plan_detail_container.scroller -side top -fill both -expand 1

    ttk::frame .root.tabs.plan_tab.plan_split.preview_container
    ttk::label .root.tabs.plan_tab.plan_split.preview_container.preview_label -text "Selected plan preview"
    ttk::frame .root.tabs.plan_tab.plan_split.preview_container.preview_scroller
    text .root.tabs.plan_tab.plan_split.preview_container.preview_scroller.plan_preview_text \
        -wrap none -height 16 -font AitkDiffFont \
        -padx 4 -pady 1 -spacing1 0 -spacing2 0 -spacing3 0 \
        -xscrollcommand ".root.tabs.plan_tab.plan_split.preview_container.preview_scroller.preview_hscroll set" \
        -yscrollcommand ".root.tabs.plan_tab.plan_split.preview_container.preview_scroller.preview_vscroll set"
    ttk::scrollbar .root.tabs.plan_tab.plan_split.preview_container.preview_scroller.preview_vscroll \
        -orient vertical \
        -command ".root.tabs.plan_tab.plan_split.preview_container.preview_scroller.plan_preview_text yview"
    ttk::scrollbar .root.tabs.plan_tab.plan_split.preview_container.preview_scroller.preview_hscroll \
        -orient horizontal \
        -command ".root.tabs.plan_tab.plan_split.preview_container.preview_scroller.plan_preview_text xview"
    pack .root.tabs.plan_tab.plan_split.preview_container.preview_label -side top -anchor w
    grid .root.tabs.plan_tab.plan_split.preview_container.preview_scroller.plan_preview_text -row 0 -column 0 -sticky nsew
    grid .root.tabs.plan_tab.plan_split.preview_container.preview_scroller.preview_vscroll -row 0 -column 1 -sticky ns
    grid .root.tabs.plan_tab.plan_split.preview_container.preview_scroller.preview_hscroll -row 1 -column 0 -sticky ew
    grid rowconfigure .root.tabs.plan_tab.plan_split.preview_container.preview_scroller 0 -weight 1
    grid columnconfigure .root.tabs.plan_tab.plan_split.preview_container.preview_scroller 0 -weight 1
    pack .root.tabs.plan_tab.plan_split.preview_container.preview_scroller -side top -fill both -expand 1

    .root.tabs.plan_tab.plan_split.top_split add .root.tabs.plan_tab.plan_split.top_split.plan_list_container
    .root.tabs.plan_tab.plan_split.top_split add .root.tabs.plan_tab.plan_split.top_split.plan_detail_container
    .root.tabs.plan_tab.plan_split add .root.tabs.plan_tab.plan_split.top_split
    .root.tabs.plan_tab.plan_split add .root.tabs.plan_tab.plan_split.preview_container

    bind .root.tabs.plan_tab.plan_split.top_split.plan_list_container.filters.query_entry <KeyRelease> {Aitk::refresh_plan_views}

    # Keep widgets dictionary in namespace scope
    dict set widgets graph_canvas .root.tabs.dispatch_tab.main_split.top_split.graph_container.scroller.graph_canvas
    dict set widgets plan_text .root.tabs.dispatch_tab.main_split.top_split.plans_right.scroller.plan_text
    dict set widgets diff_text .root.tabs.dispatch_tab.main_split.bottom_split.diff_scroller.diff_text
    dict set widgets main_split .root.tabs.dispatch_tab.main_split
    dict set widgets notebook .root.tabs
    dict set widgets plan_canvas .root.tabs.plan_tab.plan_split.top_split.plan_list_container.scroller.plan_canvas
    dict set widgets plan_detail_text .root.tabs.plan_tab.plan_split.top_split.plan_detail_container.scroller.plan_detail_text
    dict set widgets plan_preview_text .root.tabs.plan_tab.plan_split.preview_container.preview_scroller.plan_preview_text

    Aitk::select_tab plan
}

proc Aitk::plan_display_path {plan} {
    set display [Aitk::dict_get_default $plan display_path ""]
    if {$display ne ""} {
        return $display
    }
    set path [Aitk::dict_get_default $plan artifact_path ""]
    set selector [Aitk::dict_get_default $plan artifact_selector ""]
    if {$path ne "" && $selector ne ""} {
        return "$path#$selector"
    }
    return $path
}

proc Aitk::selected_snapshot_plan_contexts {} {
    variable payload
    variable state

    set snapshot_id [Aitk::dict_get_default $state selected_snapshot ""]
    if {$snapshot_id eq ""} {
        return {}
    }
    if {[dict exists $payload plan_context_by_snapshot $snapshot_id]} {
        return [dict get $payload plan_context_by_snapshot $snapshot_id]
    }
    if {[dict exists $payload snapshots_index $snapshot_id plan_contexts]} {
        return [dict get $payload snapshots_index $snapshot_id plan_contexts]
    }
    return {}
}

proc Aitk::selected_plan_context {} {
    set contexts [Aitk::selected_snapshot_plan_contexts]
    if {[llength $contexts] == 0} {
        return {}
    }
    return [lindex $contexts 0]
}

proc Aitk::plan_context_target_path {context} {
    variable payload
    set path [Aitk::dict_get_default $context artifact_path ""]
    if {$path eq ""} {
        return ""
    }
    if {[file pathtype $path] eq "absolute"} {
        return $path
    }
    set root [Aitk::dict_get_default $payload repo_root ""]
    if {$root eq ""} {
        return $path
    }
    return [file join $root $path]
}

proc Aitk::plan_target_path {plan} {
    return [Aitk::plan_context_target_path $plan]
}

proc Aitk::read_plan_file_snippet {context} {
    set target [Aitk::plan_context_target_path $context]
    if {$target eq "" || ![file exists $target]} {
        return ""
    }
    if {[catch {
        set handle [open $target r]
        set content [read $handle]
        close $handle
    } err]} {
        return ""
    }

    set ref [Aitk::dict_get_default $context plan_item_ref ""]
    set lines [split $content "\n"]
    if {$ref ne ""} {
        set idx 0
        foreach line $lines {
            if {[string first $ref $line] >= 0} {
                set start [expr {$idx - 4}]
                if {$start < 0} {
                    set start 0
                }
                set end [expr {$idx + 6}]
                if {$end >= [llength $lines]} {
                    set end [expr {[llength $lines] - 1}]
                }
                return [join [lrange $lines $start $end] "\n"]
            }
            incr idx
        }
    }

    set max_lines 120
    if {[llength $lines] < $max_lines} {
        set max_lines [llength $lines]
    }
    return [join [lrange $lines 0 [expr {$max_lines - 1}]] "\n"]
}

proc Aitk::open_selected_plan_link {} {
    set context [Aitk::selected_plan_context]
    if {$context eq ""} {
        return
    }
    set target [Aitk::plan_context_target_path $context]
    if {$target eq "" || ![file exists $target]} {
        tk_messageBox -icon warning -title "aitk" -message "Plan file not found:\n$target"
        return
    }

    if {$::tcl_platform(platform) eq "windows"} {
        exec {*}[list cmd /c start "" $target] &
    } elseif {[string match -nocase "Darwin*" $::tcl_platform(os)]} {
        exec open $target &
    } else {
        exec xdg-open $target &
    }
}

proc Aitk::copy_selected_plan_link {} {
    set context [Aitk::selected_plan_context]
    if {$context eq ""} {
        return
    }
    set link [Aitk::plan_display_path $context]
    set ref [Aitk::dict_get_default $context plan_item_ref ""]
    if {$ref ne ""} {
        append link " \[ref: $ref\]"
    }
    clipboard clear
    clipboard append $link
}

proc Aitk::render_plan_context {} {
    variable state
    variable widgets

    if {![dict exists $widgets plan_text]} {
        return
    }
    set text_widget [dict get $widgets plan_text]
    $text_widget delete 1.0 end

    set snapshot_id [Aitk::dict_get_default $state selected_snapshot ""]
    if {$snapshot_id eq ""} {
        $text_widget insert end "No snapshot selected."
        return
    }

    set contexts [Aitk::selected_snapshot_plan_contexts]
    if {[llength $contexts] == 0} {
        $text_widget insert end "No task-linked plan context for snapshot $snapshot_id."
        return
    }

    set first_context 1
    foreach context $contexts {
        if {!$first_context} {
            $text_widget insert end "\n\n---\n\n"
        }
        set first_context 0
        set title [Aitk::dict_get_default $context plan_title ""]
        set task_title [Aitk::dict_get_default $context task_title ""]
        set ref [Aitk::dict_get_default $context plan_item_ref ""]
        set path [Aitk::plan_display_path $context]
        set plan_id [Aitk::dict_get_default $context plan_id ""]
        set task_id [Aitk::dict_get_default $context task_id ""]
        set change_id [Aitk::dict_get_default $context change_id ""]
        set patchset_id [Aitk::dict_get_default $context patchset_id ""]
        set item_text [Aitk::dict_get_default $context plan_item_text ""]
        set task_intent [Aitk::dict_get_default $context task_intent ""]

        if {$title ne ""} {
            $text_widget insert end "Plan: $title\n"
        }
        if {$plan_id ne ""} {
            $text_widget insert end "Plan ID: $plan_id\n"
        }
        if {$task_title ne ""} {
            $text_widget insert end "Task: $task_id $task_title\n"
        } elseif {$task_id ne ""} {
            $text_widget insert end "Task: $task_id\n"
        }
        if {$change_id ne ""} {
            $text_widget insert end "Change: $change_id\n"
        }
        if {$patchset_id ne ""} {
            $text_widget insert end "Patchset: $patchset_id\n"
        }
        if {$ref ne ""} {
            $text_widget insert end "Ref: $ref\n"
        }
        if {$path ne ""} {
            $text_widget insert end "File: $path\n"
        }
        if {$item_text ne ""} {
            $text_widget insert end "\n$item_text\n"
        }

        set snippet [Aitk::read_plan_file_snippet $context]
        if {$snippet ne ""} {
            $text_widget insert end "\n$snippet\n"
        } elseif {$task_intent ne ""} {
            $text_widget insert end "\n$task_intent\n"
        }
    }
}

proc Aitk::render_plan_links {} {
    variable state

    Aitk::render_plan_context

    set plans [Aitk::plan_items]
    set selected [Aitk::dict_get_default $state selected_plan_link ""]
    set selected_found 0
    foreach plan $plans {
        if {[Aitk::plan_link_key $plan] eq $selected} {
            set selected_found 1
            break
        }
    }
    if {!$selected_found} {
        if {[llength $plans] > 0} {
            set selected [Aitk::plan_link_key [lindex $plans 0]]
        } else {
            set selected ""
        }
        dict set state selected_plan_link $selected
    }

    Aitk::render_plan_canvas $plans
    Aitk::render_selected_plan_detail
    Aitk::render_selected_plan_preview
}

proc Aitk::refresh_plan_views {} {
    Aitk::render_plan_links
}

proc Aitk::selected_plan_overview {} {
    variable state

    set selected [Aitk::dict_get_default $state selected_plan_link ""]
    if {$selected eq ""} {
        return {}
    }
    foreach plan [Aitk::all_plan_items] {
        if {[Aitk::plan_link_key $plan] eq $selected} {
            return $plan
        }
    }
    return {}
}

proc Aitk::select_plan_link {plan_key} {
    variable state

    dict set state selected_plan_link $plan_key
    Aitk::render_plan_links
}

proc Aitk::render_plan_canvas {plans} {
    variable widgets
    variable state
    variable plan_filter_query

    if {![dict exists $widgets plan_canvas]} {
        return
    }
    set c [dict get $widgets plan_canvas]
    $c delete all

    if {[llength $plans] == 0} {
        $c create text 20 20 -anchor nw -text "No plan payload loaded"
        Aitk::sync_graph_scrollregion $c
        return
    }

    set selected [Aitk::dict_get_default $state selected_plan_link ""]
    set y 24
    set row_h 34
    set row_width 1180
    set title_x 24
    set status_x 360
    set source_x 470
    set file_x 600
    set items_x 1060
    set index 0

    foreach plan $plans {
        set plan_key [Aitk::plan_link_key $plan]
        set y_pos [expr {$y + ($index * $row_h)}]
        set row_tag [Aitk::safe_tag planrow $plan_key]
        set fill "#ffffff"
        set outline "#ffffff"
        if {$plan_key eq $selected} {
            set fill "#e8f1ff"
            set outline "#b8d7ff"
        }
        $c create rectangle 0 [expr {$y_pos - ($row_h / 2)}] $row_width [expr {$y_pos + ($row_h / 2)}] \
            -fill $fill -outline $outline -tags [list plan_row $row_tag]
        $c bind $row_tag <Button-1> [list Aitk::select_plan_link $plan_key]

        set title [Aitk::shorten_text [Aitk::dict_get_default $plan title "<untitled plan>"] 44]
        set status [Aitk::shorten_text [Aitk::dict_get_default $plan status ""] 14]
        set source [Aitk::shorten_text [Aitk::dict_get_default $plan source [Aitk::dict_get_default $plan kind ""]] 16]
        set display_path [Aitk::shorten_text [Aitk::plan_display_path $plan] 56]
        set item_count [Aitk::plan_item_count $plan]

        $c create text $title_x $y_pos -anchor w -text $title -tags [list plan_row $row_tag]
        $c create text $status_x $y_pos -anchor w -text $status -fill #555555 -font "TkDefaultFont 9" -tags [list plan_row $row_tag]
        $c create text $source_x $y_pos -anchor w -text $source -fill #555555 -font "TkDefaultFont 9" -tags [list plan_row $row_tag]
        $c create text $file_x $y_pos -anchor w -text $display_path -fill #555555 -font "TkDefaultFont 9" -tags [list plan_row $row_tag]
        $c create text $items_x $y_pos -anchor w -text "$item_count items" -fill #555555 -font "TkDefaultFont 9" -tags [list plan_row $row_tag]
        incr index
    }

    set last_y [expr {$y + (($index - 1) * $row_h)}]
    set footer "shown $index plans"
    if {[string trim $plan_filter_query] ne ""} {
        append footer " (filtered)"
    }
    $c create text $title_x [expr {$last_y + 26}] -anchor w -fill #888888 -text $footer
    Aitk::sync_graph_scrollregion $c
}

proc Aitk::render_selected_plan_detail {} {
    variable widgets

    if {![dict exists $widgets plan_detail_text]} {
        return
    }
    set text_widget [dict get $widgets plan_detail_text]
    $text_widget delete 1.0 end

    set plan [Aitk::selected_plan_overview]
    if {$plan eq ""} {
        $text_widget insert end "No plan selected."
        return
    }

    set title [Aitk::dict_get_default $plan title ""]
    set plan_id [Aitk::dict_get_default $plan plan_id ""]
    set status [Aitk::dict_get_default $plan status ""]
    set source [Aitk::dict_get_default $plan source [Aitk::dict_get_default $plan kind ""]]
    set revision [Aitk::dict_get_default $plan head_revision_id ""]
    set revision_number [Aitk::dict_get_default $plan head_revision_number ""]
    set heading [Aitk::dict_get_default $plan artifact_heading ""]
    set path [Aitk::plan_display_path $plan]
    set count [Aitk::plan_item_count $plan]

    if {$title ne ""} {
        $text_widget insert end "Plan: $title\n"
    }
    if {$plan_id ne ""} {
        $text_widget insert end "Plan ID: $plan_id\n"
    }
    if {$status ne ""} {
        $text_widget insert end "Status: $status\n"
    }
    if {$source ne ""} {
        $text_widget insert end "Source: $source\n"
    }
    if {$revision ne ""} {
        set line "Head revision: $revision"
        if {$revision_number ne ""} {
            append line " (#$revision_number)"
        }
        $text_widget insert end "$line\n"
    }
    if {$heading ne ""} {
        $text_widget insert end "Heading: $heading\n"
    }
    if {$path ne ""} {
        $text_widget insert end "File: $path\n"
    }
    $text_widget insert end "Items: $count\n"

    if {[dict exists $plan items] && [llength [dict get $plan items]] > 0} {
        $text_widget insert end "\nPlan items:\n"
        set shown 0
        foreach item [dict get $plan items] {
            if {[catch {dict size $item}]} {
                continue
            }
            set ref [Aitk::dict_get_default $item plan_item_ref ""]
            set item_text [Aitk::shorten_text [Aitk::dict_get_default $item text ""] 120]
            set state [Aitk::dict_get_default $item checkbox_state ""]
            set label "- "
            if {$state ne ""} {
                append label "\[$state\] "
            }
            if {$ref ne ""} {
                append label "$ref: "
            }
            append label $item_text
            $text_widget insert end "$label\n"
            incr shown
            if {$shown >= 80} {
                set remaining [expr {[llength [dict get $plan items]] - $shown}]
                if {$remaining > 0} {
                    $text_widget insert end "... $remaining more items\n"
                }
                break
            }
        }
    }
}

proc Aitk::render_selected_plan_preview {} {
    variable widgets

    if {![dict exists $widgets plan_preview_text]} {
        return
    }
    set text_widget [dict get $widgets plan_preview_text]
    $text_widget delete 1.0 end

    set plan [Aitk::selected_plan_overview]
    if {$plan eq ""} {
        $text_widget insert end "No plan selected."
        return
    }

    set snippet [Aitk::read_plan_file_snippet $plan]
    if {$snippet ne ""} {
        $text_widget insert end $snippet
        return
    }

    set path [Aitk::plan_display_path $plan]
    if {$path eq ""} {
        $text_widget insert end "No plan artifact path recorded."
    } else {
        $text_widget insert end "Plan artifact preview unavailable: $path"
    }
}

proc Aitk::open_selected_plan_overview_link {} {
    set plan [Aitk::selected_plan_overview]
    if {$plan eq ""} {
        return
    }
    set target [Aitk::plan_context_target_path $plan]
    if {$target eq "" || ![file exists $target]} {
        tk_messageBox -icon warning -title "aitk" -message "Plan file not found:\n$target"
        return
    }

    if {$::tcl_platform(platform) eq "windows"} {
        exec {*}[list cmd /c start "" $target] &
    } elseif {[string match -nocase "Darwin*" $::tcl_platform(os)]} {
        exec open $target &
    } else {
        exec xdg-open $target &
    }
}

proc Aitk::copy_selected_plan_overview_link {} {
    set plan [Aitk::selected_plan_overview]
    if {$plan eq ""} {
        return
    }
    set link [Aitk::plan_display_path $plan]
    clipboard clear
    clipboard append $link
}

proc Aitk::markdown_target_path {doc_or_path} {
    variable payload
    if {[catch {dict size $doc_or_path}]} {
        set path $doc_or_path
    } else {
        set path [Aitk::dict_get_default $doc_or_path path [Aitk::dict_get_default $doc_or_path display_path ""]]
    }
    if {$path eq ""} {
        return ""
    }
    if {[file pathtype $path] eq "absolute"} {
        return $path
    }
    set root [Aitk::dict_get_default $payload repo_root ""]
    if {$root eq ""} {
        return $path
    }
    return [file join $root $path]
}

proc Aitk::markdown_doc_by_path {path} {
    if {$path eq ""} {
        return {}
    }
    foreach doc [Aitk::markdown_docs] {
        if {[Aitk::markdown_doc_key $doc] eq $path || [Aitk::dict_get_default $doc display_path ""] eq $path} {
            return $doc
        }
    }
    return {}
}

proc Aitk::read_markdown_doc {doc} {
    set target [Aitk::markdown_target_path $doc]
    if {$target eq "" || ![file exists $target]} {
        return ""
    }
    if {[catch {
        set handle [open $target r]
        set content [read $handle]
        close $handle
    } err]} {
        return ""
    }
    return $content
}

proc Aitk::create_or_configure_font {name args} {
    if {[lsearch -exact [font names] $name] < 0} {
        font create $name {*}$args
    } else {
        font configure $name {*}$args
    }
}

proc Aitk::markdown_tags {base_tags extra_tags} {
    set tags $base_tags
    foreach tag $extra_tags {
        if {[lsearch -exact $tags $tag] < 0} {
            lappend tags $tag
        }
    }
    return $tags
}

proc Aitk::clear_markdown_link_tags {text_widget} {
    foreach tag [$text_widget tag names] {
        if {[string match "md_link_target_*" $tag] || [string match "md_heading_anchor_*" $tag]} {
            $text_widget tag delete $tag
        }
    }
}

proc Aitk::next_markdown_link_tag {} {
    variable state

    set counter [Aitk::dict_get_default $state markdown_link_counter 0]
    incr counter
    dict set state markdown_link_counter $counter
    return "md_link_target_$counter"
}

proc Aitk::set_markdown_link_cursor {text_widget cursor} {
    if {[winfo exists $text_widget]} {
        $text_widget configure -cursor $cursor
    }
}

proc Aitk::register_markdown_link {text_widget target} {
    set tag [Aitk::next_markdown_link_tag]
    $text_widget tag bind $tag <Button-1> [list Aitk::open_markdown_link $target]
    $text_widget tag bind $tag <Enter> [list Aitk::set_markdown_link_cursor $text_widget hand2]
    $text_widget tag bind $tag <Leave> [list Aitk::set_markdown_link_cursor $text_widget xterm]
    return $tag
}

proc Aitk::markdown_heading_slug {text} {
    set slug [string tolower [string trim $text]]
    regsub -all {`([^`]*)`} $slug {\1} slug
    regsub -all {\[([^\]]+)\]\([^)]+\)} $slug {\1} slug
    regsub -all {[^a-z0-9 _-]+} $slug "" slug
    regsub -all {[ \t_]+} $slug "-" slug
    regsub -all -- {-+} $slug "-" slug
    return [string trim $slug "-"]
}

proc Aitk::markdown_link_parts {target} {
    set target [string trim $target]
    set target [string trim $target "<>"]
    regsub {^[ \t]*([^ \t'"]+)[ \t]+['"].*['"][ \t]*$} $target {\1} target
    regsub -all {%20} $target " " target

    set path $target
    set anchor ""
    set hash [string first "#" $target]
    if {$hash >= 0} {
        set path [string range $target 0 [expr {$hash - 1}]]
        set anchor [string range $target [expr {$hash + 1}] end]
    }
    return [list $path $anchor]
}

proc Aitk::markdown_link_is_external {target} {
    set target [string trim $target]
    if {[regexp -nocase {^[a-z][a-z0-9+.-]*:} $target] && ![regexp -nocase {^file:} $target]} {
        return 1
    }
    return 0
}

proc Aitk::normalized_existing_path {path} {
    if {$path eq ""} {
        return ""
    }
    if {[catch {file normalize $path} normalized]} {
        return ""
    }
    return $normalized
}

proc Aitk::resolve_markdown_link_doc_path {target_path} {
    variable payload
    variable state

    set target_path [string trim $target_path]
    if {$target_path eq ""} {
        return [Aitk::dict_get_default $state selected_markdown_path ""]
    }

    set candidate_paths {}
    set root [Aitk::dict_get_default $payload repo_root ""]
    if {[file pathtype $target_path] eq "absolute"} {
        lappend candidate_paths [Aitk::normalized_existing_path $target_path]
    } else {
        if {$root ne ""} {
            lappend candidate_paths [Aitk::normalized_existing_path [file join $root $target_path]]
        }
        set selected [Aitk::dict_get_default $state selected_markdown_path ""]
        set selected_doc [Aitk::markdown_doc_by_path $selected]
        if {$selected_doc ne ""} {
            set selected_target [Aitk::markdown_target_path $selected_doc]
            if {$selected_target ne ""} {
                lappend candidate_paths [Aitk::normalized_existing_path [file join [file dirname $selected_target] $target_path]]
            }
        }
    }

    set normalized_target [string map {"\\" "/"} $target_path]
    foreach doc [Aitk::markdown_docs] {
        set doc_key [Aitk::markdown_doc_key $doc]
        set display_path [Aitk::dict_get_default $doc display_path ""]
        if {$doc_key eq $target_path || $display_path eq $target_path || $doc_key eq $normalized_target || $display_path eq $normalized_target} {
            return $doc_key
        }

        set doc_target [Aitk::normalized_existing_path [Aitk::markdown_target_path $doc]]
        if {$doc_target eq ""} {
            continue
        }
        foreach candidate $candidate_paths {
            if {$candidate ne "" && $candidate eq $doc_target} {
                return $doc_key
            }
        }
    }
    return ""
}

proc Aitk::open_external_target {target} {
    if {$target eq ""} {
        return
    }

    if {$::tcl_platform(platform) eq "windows"} {
        exec {*}[list cmd /c start "" $target] &
    } elseif {[string match -nocase "Darwin*" $::tcl_platform(os)]} {
        exec open $target &
    } else {
        exec xdg-open $target &
    }
}

proc Aitk::scroll_markdown_anchor {anchor} {
    variable state
    variable widgets

    set slug [Aitk::markdown_heading_slug $anchor]
    if {$slug eq "" || ![dict exists $state markdown_anchor_indexes]} {
        return
    }
    set anchors [dict get $state markdown_anchor_indexes]
    if {![dict exists $anchors $slug] || ![dict exists $widgets markdown_detail_text]} {
        return
    }

    set text_widget [dict get $widgets markdown_detail_text]
    set index [dict get $anchors $slug]
    $text_widget see $index
    $text_widget tag remove sel 1.0 end
    $text_widget tag add sel $index "$index lineend"
}

proc Aitk::open_markdown_link {target} {
    variable markdown_filter_query

    if {[Aitk::markdown_link_is_external $target]} {
        if {[catch {Aitk::open_external_target $target} err]} {
            tk_messageBox -icon warning -title "aitk" -message "Unable to open link:\n$target"
        }
        return
    }

    lassign [Aitk::markdown_link_parts $target] target_path anchor
    set doc_path [Aitk::resolve_markdown_link_doc_path $target_path]
    if {$doc_path ne ""} {
        set markdown_filter_query ""
        Aitk::render_markdown_browser $doc_path
        if {$anchor ne ""} {
            Aitk::scroll_markdown_anchor $anchor
        }
        return
    }

    if {$target_path ne ""} {
        if {[catch {Aitk::open_external_target $target_path} err]} {
            tk_messageBox -icon warning -title "aitk" -message "Markdown link not found:\n$target"
        }
    } elseif {$anchor ne ""} {
        Aitk::scroll_markdown_anchor $anchor
    }
}

proc Aitk::find_single_markdown_marker {text marker start} {
    set length [string length $text]
    set pos $start
    while {$pos < $length} {
        set index [string first $marker $text $pos]
        if {$index < 0} {
            return -1
        }
        set previous ""
        set next ""
        if {$index > 0} {
            set previous [string index $text [expr {$index - 1}]]
        }
        if {$index + 1 < $length} {
            set next [string index $text [expr {$index + 1}]]
        }
        if {$previous ne $marker && $next ne $marker} {
            return $index
        }
        set pos [expr {$index + 1}]
    }
    return -1
}

proc Aitk::configure_markdown_detail_tags {text_widget} {
    set body_family [font actual TkDefaultFont -family]
    set body_size [font actual TkDefaultFont -size]
    if {$body_size < 0} {
        set body_size [expr {0 - $body_size}]
    }
    set fixed_family [font actual TkFixedFont -family]

    Aitk::create_or_configure_font AitkMarkdownBody \
        -family $body_family -size $body_size
    Aitk::create_or_configure_font AitkMarkdownH1 \
        -family $body_family -size [expr {$body_size + 8}] -weight bold
    Aitk::create_or_configure_font AitkMarkdownH2 \
        -family $body_family -size [expr {$body_size + 5}] -weight bold
    Aitk::create_or_configure_font AitkMarkdownH3 \
        -family $body_family -size [expr {$body_size + 3}] -weight bold
    Aitk::create_or_configure_font AitkMarkdownH4 \
        -family $body_family -size [expr {$body_size + 1}] -weight bold
    Aitk::create_or_configure_font AitkMarkdownBold \
        -family $body_family -size $body_size -weight bold
    Aitk::create_or_configure_font AitkMarkdownItalic \
        -family $body_family -size $body_size -slant italic
    Aitk::create_or_configure_font AitkMarkdownCode \
        -family $fixed_family -size [expr {$body_size - 1}]
    Aitk::create_or_configure_font AitkMarkdownTableHeader \
        -family $fixed_family -size [expr {$body_size - 1}] -weight bold

    $text_widget configure \
        -font AitkMarkdownBody \
        -background #ffffff \
        -foreground #1f2937 \
        -padx 18 -pady 14 \
        -spacing1 2 -spacing2 1 -spacing3 5
    $text_widget tag configure md_body -font AitkMarkdownBody -foreground #1f2937
    $text_widget tag configure md_h1 \
        -font AitkMarkdownH1 -foreground #111827 -spacing1 14 -spacing3 10
    $text_widget tag configure md_h2 \
        -font AitkMarkdownH2 -foreground #111827 -spacing1 12 -spacing3 8
    $text_widget tag configure md_h3 \
        -font AitkMarkdownH3 -foreground #111827 -spacing1 10 -spacing3 6
    $text_widget tag configure md_h4 \
        -font AitkMarkdownH4 -foreground #111827 -spacing1 8 -spacing3 5
    $text_widget tag configure md_bold -font AitkMarkdownBold
    $text_widget tag configure md_italic -font AitkMarkdownItalic
    $text_widget tag configure md_code \
        -font AitkMarkdownCode -background #f3f4f6 -foreground #111827
    $text_widget tag configure md_code_block \
        -font AitkMarkdownCode -background #f6f8fa -foreground #24292f \
        -lmargin1 18 -lmargin2 18 -rmargin 18 -spacing1 3 -spacing3 3
    $text_widget tag configure md_quote \
        -foreground #4b5563 -background #f9fafb \
        -lmargin1 16 -lmargin2 16 -rmargin 12 -spacing1 3 -spacing3 3
    $text_widget tag configure md_list_marker \
        -foreground #374151 -font AitkMarkdownBold
    $text_widget tag configure md_hr \
        -foreground #9ca3af -spacing1 8 -spacing3 8
    $text_widget tag configure md_link \
        -foreground #2563eb -underline 1
    $text_widget tag configure md_link_url \
        -foreground #6b7280
    $text_widget tag configure md_table \
        -font AitkMarkdownCode -background #f9fafb -foreground #374151 \
        -lmargin1 8 -lmargin2 8
    $text_widget tag configure md_table_header \
        -font AitkMarkdownTableHeader -foreground #111827
}

proc Aitk::insert_markdown_inline {text_widget text {base_tags {md_body}}} {
    set pos 0
    set length [string length $text]
    while {$pos < $length} {
        set tick [string first "`" $text $pos]
        set bold_star [string first {**} $text $pos]
        set bold_under [string first {__} $text $pos]
        set italic_star [Aitk::find_single_markdown_marker $text "*" $pos]
        set link [string first {[} $text $pos]

        set candidates {}
        foreach candidate [list \
            [list $tick code "`"] \
            [list $bold_star bold {**}] \
            [list $bold_under bold {__}] \
            [list $italic_star italic "*"] \
            [list $link link {[}]] {
            set index [lindex $candidate 0]
            if {$index >= 0} {
                lappend candidates $candidate
            }
        }
        if {[llength $candidates] == 0} {
            $text_widget insert end [string range $text $pos end] $base_tags
            return
        }

        set candidate [lindex [lsort -integer -index 0 $candidates] 0]
        set index [lindex $candidate 0]
        set kind [lindex $candidate 1]
        set marker [lindex $candidate 2]

        if {$index > $pos} {
            $text_widget insert end [string range $text $pos [expr {$index - 1}]] $base_tags
        }

        if {$kind eq "code"} {
            set end [string first "`" $text [expr {$index + 1}]]
            if {$end < 0} {
                $text_widget insert end [string range $text $index end] $base_tags
                return
            }
            set tags [Aitk::markdown_tags $base_tags {md_code}]
            $text_widget insert end [string range $text [expr {$index + 1}] [expr {$end - 1}]] $tags
            set pos [expr {$end + 1}]
        } elseif {$kind eq "bold"} {
            set end [string first $marker $text [expr {$index + 2}]]
            if {$end < 0} {
                $text_widget insert end [string range $text $index end] $base_tags
                return
            }
            set tags [Aitk::markdown_tags $base_tags {md_bold}]
            $text_widget insert end [string range $text [expr {$index + 2}] [expr {$end - 1}]] $tags
            set pos [expr {$end + 2}]
        } elseif {$kind eq "italic"} {
            set end [Aitk::find_single_markdown_marker $text "*" [expr {$index + 1}]]
            if {$end < 0} {
                $text_widget insert end [string range $text $index end] $base_tags
                return
            }
            set tags [Aitk::markdown_tags $base_tags {md_italic}]
            $text_widget insert end [string range $text [expr {$index + 1}] [expr {$end - 1}]] $tags
            set pos [expr {$end + 1}]
        } else {
            set close [string first {]} $text [expr {$index + 1}]]
            set open_paren -1
            set close_paren -1
            if {$close >= 0 && $close + 1 < $length && [string index $text [expr {$close + 1}]] eq "("} {
                set open_paren [expr {$close + 1}]
                set close_paren [string first ")" $text [expr {$open_paren + 1}]]
            }
            if {$close < 0 || $open_paren < 0 || $close_paren < 0} {
                $text_widget insert end [string index $text $index] $base_tags
                set pos [expr {$index + 1}]
                continue
            }
            set label [string range $text [expr {$index + 1}] [expr {$close - 1}]]
            set url [string range $text [expr {$open_paren + 1}] [expr {$close_paren - 1}]]
            set link_tag [Aitk::register_markdown_link $text_widget $url]
            set link_tags [Aitk::markdown_tags $base_tags [list md_link $link_tag]]
            $text_widget insert end $label $link_tags
            if {$url ne "" && $url ne $label} {
                set url_tags [Aitk::markdown_tags $base_tags [list md_link_url $link_tag]]
                $text_widget insert end " ($url)" $url_tags
            }
            set pos [expr {$close_paren + 1}]
        }
    }
}

proc Aitk::markdown_table_candidate {line} {
    return [regexp {^[ \t]*\|.*\|[ \t]*$} $line]
}

proc Aitk::markdown_table_cells {line} {
    set trimmed [string trim $line]
    if {$trimmed eq ""} {
        return {}
    }
    if {[string index $trimmed 0] eq "|"} {
        set trimmed [string range $trimmed 1 end]
    }
    if {$trimmed ne "" && [string index $trimmed end] eq "|"} {
        set trimmed [string range $trimmed 0 end-1]
    }

    set cells {}
    foreach cell [split $trimmed "|"] {
        lappend cells [string trim $cell]
    }
    return $cells
}

proc Aitk::markdown_table_separator_cells {cells} {
    if {[llength $cells] == 0} {
        return 0
    }
    foreach cell $cells {
        set normalized [string map [list " " "" "\t" ""] [string trim $cell]]
        if {![regexp {^:?-{3,}:?$} $normalized]} {
            return 0
        }
    }
    return 1
}

proc Aitk::markdown_table_alignments {separator_cells} {
    set alignments {}
    foreach cell $separator_cells {
        set normalized [string map [list " " "" "\t" ""] [string trim $cell]]
        set alignment left
        if {[string index $normalized 0] eq ":" && [string index $normalized end] eq ":"} {
            set alignment center
        } elseif {[string index $normalized end] eq ":"} {
            set alignment right
        }
        lappend alignments $alignment
    }
    return $alignments
}

proc Aitk::markdown_table_visible_text {text} {
    set visible $text
    regsub -all {`([^`]*)`} $visible {\1} visible
    regsub -all {\[([^\]]+)\]\([^)]+\)} $visible {\1} visible
    regsub -all {(\*\*|__)} $visible "" visible
    regsub -all {\*} $visible "" visible
    return $visible
}

proc Aitk::markdown_table_cell_width {text} {
    return [string length [Aitk::markdown_table_visible_text $text]]
}

proc Aitk::markdown_table_widths {rows} {
    set widths {}
    foreach row $rows {
        for {set index 0} {$index < [llength $row]} {incr index} {
            set width [Aitk::markdown_table_cell_width [lindex $row $index]]
            while {[llength $widths] <= $index} {
                lappend widths 0
            }
            if {$width > [lindex $widths $index]} {
                lset widths $index $width
            }
        }
    }
    return $widths
}

proc Aitk::insert_markdown_table_cell {text_widget text width alignment tags} {
    set text_width [Aitk::markdown_table_cell_width $text]
    set pad [expr {$width - $text_width}]
    if {$pad < 0} {
        set pad 0
    }

    set left_pad 0
    set right_pad $pad
    if {$alignment eq "right"} {
        set left_pad $pad
        set right_pad 0
    } elseif {$alignment eq "center"} {
        set left_pad [expr {$pad / 2}]
        set right_pad [expr {$pad - $left_pad}]
    }

    if {$left_pad > 0} {
        $text_widget insert end [string repeat " " $left_pad] md_table
    }
    Aitk::insert_markdown_inline $text_widget $text $tags
    if {$right_pad > 0} {
        $text_widget insert end [string repeat " " $right_pad] md_table
    }
}

proc Aitk::render_markdown_table {text_widget rows alignments} {
    if {[llength $rows] == 0} {
        return
    }

    set widths [Aitk::markdown_table_widths $rows]
    while {[llength $widths] < [llength $alignments]} {
        lappend widths 3
    }
    if {[llength $widths] == 0} {
        return
    }

    for {set row_index 0} {$row_index < [llength $rows]} {incr row_index} {
        set row [lindex $rows $row_index]
        set tags {md_table}
        if {$row_index == 0} {
            set tags {md_table md_table_header}
        }

        $text_widget insert end " " md_table
        for {set col 0} {$col < [llength $widths]} {incr col} {
            if {$col > 0} {
                $text_widget insert end " | " md_table
            }
            set cell [lindex $row $col]
            set alignment [lindex $alignments $col]
            if {$alignment eq ""} {
                set alignment left
            }
            Aitk::insert_markdown_table_cell $text_widget $cell [lindex $widths $col] $alignment $tags
        }
        $text_widget insert end " \n" md_table
    }
    $text_widget insert end "\n" md_body
}

proc Aitk::flush_markdown_table {text_widget pending_var pending_raw_var rows_var alignments_var} {
    upvar 1 $pending_var pending_header
    upvar 1 $pending_raw_var pending_raw
    upvar 1 $rows_var rows
    upvar 1 $alignments_var alignments

    if {[llength $rows] > 0} {
        Aitk::render_markdown_table $text_widget $rows $alignments
    } elseif {[llength $pending_header] > 0} {
        $text_widget insert end "$pending_raw\n" md_table
    }
    set pending_header {}
    set pending_raw ""
    set rows {}
    set alignments {}
}

proc Aitk::render_markdown_content {text_widget content} {
    variable state

    Aitk::configure_markdown_detail_tags $text_widget
    Aitk::clear_markdown_link_tags $text_widget
    dict set state markdown_link_counter 0
    set anchors {}

    $text_widget configure -state normal
    $text_widget delete 1.0 end

    set in_code_block 0
    set pending_table_header {}
    set pending_table_raw ""
    set table_rows {}
    set table_alignments {}

    foreach raw_line [split $content "\n"] {
        set line [string trimright $raw_line "\r"]
        set trimmed [string trim $line]

        if {[regexp {^```[ \t]*(.*)$} $line -> lang] || [regexp {^~~~[ \t]*(.*)$} $line -> lang]} {
            Aitk::flush_markdown_table $text_widget pending_table_header pending_table_raw table_rows table_alignments
            if {$in_code_block} {
                set in_code_block 0
                $text_widget insert end "\n" md_body
            } else {
                set in_code_block 1
                if {[string trim $lang] ne ""} {
                    $text_widget insert end [string trim $lang] {md_code_block md_code}
                    $text_widget insert end "\n" md_code_block
                }
            }
            continue
        }

        if {$in_code_block} {
            $text_widget insert end "$line\n" {md_code_block md_code}
            continue
        }

        if {$trimmed eq ""} {
            Aitk::flush_markdown_table $text_widget pending_table_header pending_table_raw table_rows table_alignments
            $text_widget insert end "\n" md_body
            continue
        }

        if {[Aitk::markdown_table_candidate $line]} {
            set cells [Aitk::markdown_table_cells $line]
            if {[llength $table_rows] > 0} {
                if {![Aitk::markdown_table_separator_cells $cells]} {
                    lappend table_rows $cells
                }
                continue
            }
            if {[llength $pending_table_header] > 0} {
                if {[Aitk::markdown_table_separator_cells $cells]} {
                    set table_rows [list $pending_table_header]
                    set table_alignments [Aitk::markdown_table_alignments $cells]
                    set pending_table_header {}
                    set pending_table_raw ""
                    continue
                }
                $text_widget insert end "$pending_table_raw\n" md_table
            }
            set pending_table_header $cells
            set pending_table_raw $line
            continue
        }

        Aitk::flush_markdown_table $text_widget pending_table_header pending_table_raw table_rows table_alignments

        if {[regexp {^(#{1,6})[ \t]+(.+)$} $line -> marks heading]} {
            set level [string length $marks]
            set tag md_h4
            if {$level == 1} {
                set tag md_h1
            } elseif {$level == 2} {
                set tag md_h2
            } elseif {$level == 3} {
                set tag md_h3
            }
            set heading [string trim $heading " \t#"]
            set heading_start [$text_widget index end]
            set slug [Aitk::markdown_heading_slug $heading]
            if {$slug ne "" && ![dict exists $anchors $slug]} {
                dict set anchors $slug $heading_start
            }
            Aitk::insert_markdown_inline $text_widget $heading [list $tag]
            if {$slug ne ""} {
                $text_widget tag add "md_heading_anchor_$slug" $heading_start "$heading_start lineend"
            }
            $text_widget insert end "\n" $tag
            continue
        }

        if {[regexp {^([-*_][ \t]*){3,}$} $trimmed]} {
            $text_widget insert end "----------------------------------------\n" md_hr
            continue
        }

        if {[regexp {^[ \t]*>[ \t]?(.*)$} $line -> quote_text]} {
            $text_widget insert end "  " md_quote
            Aitk::insert_markdown_inline $text_widget $quote_text {md_quote}
            $text_widget insert end "\n" md_quote
            continue
        }

        if {[regexp {^[ \t]*([-+*])[ \t]+(.+)$} $line -> marker item_text]} {
            $text_widget insert end "  \u2022  " {md_body md_list_marker}
            Aitk::insert_markdown_inline $text_widget $item_text {md_body}
            $text_widget insert end "\n" md_body
            continue
        }

        if {[regexp {^[ \t]*([0-9]+)[.)][ \t]+(.+)$} $line -> number item_text]} {
            $text_widget insert end "  $number.  " {md_body md_list_marker}
            Aitk::insert_markdown_inline $text_widget $item_text {md_body}
            $text_widget insert end "\n" md_body
            continue
        }

        if {[regexp {^[ \t]{4,}(.+)$} $line -> code_line]} {
            $text_widget insert end "$code_line\n" {md_code_block md_code}
            continue
        }

        Aitk::insert_markdown_inline $text_widget $line {md_body}
        $text_widget insert end "\n" md_body
    }
    Aitk::flush_markdown_table $text_widget pending_table_header pending_table_raw table_rows table_alignments

    dict set state markdown_anchor_indexes $anchors
    $text_widget configure -state disabled
}

proc Aitk::ensure_markdown_browser {} {
    variable widgets

    if {[dict exists $widgets markdown_listbox] && [winfo exists [dict get $widgets markdown_listbox]]} {
        return
    }

    ttk::panedwindow .root.tabs.markdown_tab.split -orient horizontal
    pack .root.tabs.markdown_tab.split -fill both -expand 1

    ttk::frame .root.tabs.markdown_tab.split.list
    ttk::label .root.tabs.markdown_tab.split.list.header -text "Markdown"
    ttk::frame .root.tabs.markdown_tab.split.list.filters
    ttk::entry .root.tabs.markdown_tab.split.list.filters.query \
        -textvariable Aitk::markdown_filter_query -width 28
    ttk::button .root.tabs.markdown_tab.split.list.filters.clear \
        -text "Clear" \
        -command {set Aitk::markdown_filter_query ""; Aitk::render_markdown_browser}
    ttk::frame .root.tabs.markdown_tab.split.list.scroller
    listbox .root.tabs.markdown_tab.split.list.scroller.docs \
        -activestyle dotbox -exportselection 0 \
        -yscrollcommand ".root.tabs.markdown_tab.split.list.scroller.vscroll set"
    ttk::scrollbar .root.tabs.markdown_tab.split.list.scroller.vscroll \
        -orient vertical \
        -command ".root.tabs.markdown_tab.split.list.scroller.docs yview"

    ttk::frame .root.tabs.markdown_tab.split.detail
    ttk::label .root.tabs.markdown_tab.split.detail.title -text "Document"
    ttk::frame .root.tabs.markdown_tab.split.detail.scroller
    text .root.tabs.markdown_tab.split.detail.scroller.content \
        -wrap word \
        -yscrollcommand ".root.tabs.markdown_tab.split.detail.scroller.vscroll set"
    Aitk::configure_markdown_detail_tags .root.tabs.markdown_tab.split.detail.scroller.content
    ttk::scrollbar .root.tabs.markdown_tab.split.detail.scroller.vscroll \
        -orient vertical \
        -command ".root.tabs.markdown_tab.split.detail.scroller.content yview"

    pack .root.tabs.markdown_tab.split.list.header -side top -anchor w
    pack .root.tabs.markdown_tab.split.list.filters -side top -anchor w -fill x
    pack .root.tabs.markdown_tab.split.list.filters.query -side left -padx 0 -pady 2
    pack .root.tabs.markdown_tab.split.list.filters.clear -side left -padx 4 -pady 2
    grid .root.tabs.markdown_tab.split.list.scroller.docs -row 0 -column 0 -sticky nsew
    grid .root.tabs.markdown_tab.split.list.scroller.vscroll -row 0 -column 1 -sticky ns
    grid rowconfigure .root.tabs.markdown_tab.split.list.scroller 0 -weight 1
    grid columnconfigure .root.tabs.markdown_tab.split.list.scroller 0 -weight 1
    pack .root.tabs.markdown_tab.split.list.scroller -side top -fill both -expand 1

    pack .root.tabs.markdown_tab.split.detail.title -side top -anchor w -fill x
    grid .root.tabs.markdown_tab.split.detail.scroller.content -row 0 -column 0 -sticky nsew
    grid .root.tabs.markdown_tab.split.detail.scroller.vscroll -row 0 -column 1 -sticky ns
    grid rowconfigure .root.tabs.markdown_tab.split.detail.scroller 0 -weight 1
    grid columnconfigure .root.tabs.markdown_tab.split.detail.scroller 0 -weight 1
    pack .root.tabs.markdown_tab.split.detail.scroller -side top -fill both -expand 1

    .root.tabs.markdown_tab.split add .root.tabs.markdown_tab.split.list -weight 1
    .root.tabs.markdown_tab.split add .root.tabs.markdown_tab.split.detail -weight 4

    bind .root.tabs.markdown_tab.split.list.filters.query <KeyRelease> {Aitk::render_markdown_browser}
    bind .root.tabs.markdown_tab.split.list.scroller.docs <<ListboxSelect>> {Aitk::select_markdown_from_listbox}

    dict set widgets markdown_listbox .root.tabs.markdown_tab.split.list.scroller.docs
    dict set widgets markdown_detail_text .root.tabs.markdown_tab.split.detail.scroller.content
    dict set widgets markdown_detail_title .root.tabs.markdown_tab.split.detail.title
}

proc Aitk::render_markdown_browser {{preferred_path ""}} {
    variable state
    variable widgets

    Aitk::ensure_markdown_browser
    set listbox [dict get $widgets markdown_listbox]
    $listbox delete 0 end

    set docs [Aitk::filtered_markdown_docs]
    set paths {}
    set selected [Aitk::dict_get_default $state selected_markdown_path ""]
    if {$preferred_path ne ""} {
        set selected $preferred_path
    }

    set index 0
    set selected_index -1
    foreach doc $docs {
        set path [Aitk::markdown_doc_key $doc]
        set title [Aitk::dict_get_default $doc title ""]
        set label $path
        if {$title ne "" && $title ne $path} {
            append label "  -  $title"
        }
        $listbox insert end $label
        lappend paths $path
        if {$path eq $selected} {
            set selected_index $index
        }
        incr index
    }

    if {$selected_index < 0 && [llength $paths] > 0} {
        set selected_index 0
        set selected [lindex $paths 0]
    }
    dict set state markdown_list_paths $paths
    dict set state selected_markdown_path $selected

    if {$selected_index >= 0} {
        $listbox selection set $selected_index
        $listbox activate $selected_index
        $listbox see $selected_index
    }
    Aitk::render_selected_markdown_detail
}

proc Aitk::select_markdown_from_listbox {} {
    variable state
    variable widgets

    if {![dict exists $widgets markdown_listbox]} {
        return
    }
    set listbox [dict get $widgets markdown_listbox]
    set selection [$listbox curselection]
    if {[llength $selection] == 0} {
        return
    }
    set index [lindex $selection 0]
    set paths [Aitk::dict_get_default $state markdown_list_paths {}]
    if {$index < 0 || $index >= [llength $paths]} {
        return
    }
    dict set state selected_markdown_path [lindex $paths $index]
    Aitk::render_selected_markdown_detail
}

proc Aitk::render_selected_markdown_detail {} {
    variable state
    variable widgets

    if {![dict exists $widgets markdown_detail_text]} {
        return
    }
    set text_widget [dict get $widgets markdown_detail_text]
    $text_widget configure -state normal
    $text_widget delete 1.0 end

    set path [Aitk::dict_get_default $state selected_markdown_path ""]
    set doc [Aitk::markdown_doc_by_path $path]
    if {$doc eq ""} {
        if {[dict exists $widgets markdown_detail_title]} {
            [dict get $widgets markdown_detail_title] configure -text "Document"
        }
        $text_widget insert end "No Markdown document selected."
        $text_widget configure -state disabled
        return
    }

    set title [Aitk::dict_get_default $doc title $path]
    if {[dict exists $widgets markdown_detail_title]} {
        [dict get $widgets markdown_detail_title] configure -text $title
    }
    set content [Aitk::read_markdown_doc $doc]
    if {$content eq ""} {
        $text_widget insert end "Markdown file unavailable:\n$path"
        $text_widget configure -state disabled
        return
    }
    Aitk::render_markdown_content $text_widget $content
}

proc Aitk::open_markdown_browser {{preferred_path ""}} {
    Aitk::select_tab markdown
    Aitk::render_markdown_browser $preferred_path
}

proc Aitk::render_graph {} {
    variable payload
    variable widgets
    variable state
    variable filter_query
    variable filter_stale_days
    if {![dict exists $widgets graph_canvas]} {
        return
    }
    set c [dict get $widgets graph_canvas]
    $c delete all

    set snapshots [Aitk::snapshot_items]
    if {[llength $snapshots] == 0} {
        $c create text 20 20 -anchor nw -text "No snapshot payload loaded"
        Aitk::sync_graph_scrollregion $c
        return
    }

    set x0 34
    set y 24
    set row_h 30
    set col_w 24
    set row_width 1320
    set label_x 160
    set message_x 560
    set label_max_chars 46
    set message_max_chars 86
    set radius 8
    set index 0
    set x_by_snapshot {}
    set y_by_snapshot {}

    foreach item $snapshots {
        if {![dict exists $item snapshot_id]} {
            continue
        }
        set snap_id [dict get $item snapshot_id]
        set column [Aitk::dict_get_default $item graph_column 0]
        set x_pos [expr {$x0 + ($column * $col_w)}]
        set y_pos [expr {$y + ($index * $row_h)}]
        dict set x_by_snapshot $snap_id $x_pos
        dict set y_by_snapshot $snap_id $y_pos
        incr index
    }

    set selected_snapshot [Aitk::dict_get_default $state selected_snapshot ""]
    foreach item $snapshots {
        if {![dict exists $item snapshot_id]} {
            continue
        }
        set snap_id [dict get $item snapshot_id]
        set y_pos [dict get $y_by_snapshot $snap_id]
        set row_tag [Aitk::safe_tag row $snap_id]
        set fill "#ffffff"
        set outline "#ffffff"
        if {$snap_id eq $selected_snapshot} {
            set fill "#e8f1ff"
            set outline "#b8d7ff"
        }
        $c create rectangle 0 [expr {$y_pos - ($row_h / 2)}] $row_width [expr {$y_pos + ($row_h / 2)}] \
            -fill $fill -outline $outline -tags [list row $row_tag]
        $c bind $row_tag <Button-1> [list Aitk::select_snapshot $snap_id]
    }

    foreach item $snapshots {
        if {![dict exists $item snapshot_id]} {
            continue
        }
        set snap_id [dict get $item snapshot_id]
        set from_x [dict get $x_by_snapshot $snap_id]
        set from_y [dict get $y_by_snapshot $snap_id]

        if {[dict exists $item graph_segments]} {
            foreach segment [dict get $item graph_segments] {
                set to_col [Aitk::dict_get_default $segment to_column [Aitk::dict_get_default $item graph_column 0]]
                set to_x [expr {$x0 + ($to_col * $col_w)}]
                set to_id [Aitk::dict_get_default $segment to_snapshot_id ""]
                set to_y [expr {$from_y + $row_h}]
                if {$to_id ne "" && [dict exists $y_by_snapshot $to_id]} {
                    set to_y [dict get $y_by_snapshot $to_id]
                }
                set mid_y [expr {($from_y + $to_y) / 2}]
                $c create line $from_x $from_y $from_x $mid_y $to_x $mid_y $to_x $to_y -fill #777777 -width 1
            }
        }
    }

    set index 0
    foreach item $snapshots {
        if {![dict exists $item snapshot_id]} {
            continue
        }
        set snap_id [dict get $item snapshot_id]
        set msg ""
        if {[dict exists $item message]} {
            set msg [dict get $item message]
        }
        set line_name [Aitk::line_for_snapshot $item]

        set x_pos [dict get $x_by_snapshot $snap_id]
        set y_pos [dict get $y_by_snapshot $snap_id]
        set fill "#4a90e2"
        if {[Aitk::dict_get_default $item is_main_head 0]} {
            set fill "#2e9d57"
        } elseif {[Aitk::dict_get_default $item is_head 0]} {
            set fill "#f5a623"
        }

        set tag [Aitk::safe_tag snap $snap_id]
        set row_tag [Aitk::safe_tag row $snap_id]
        $c create oval [expr {$x_pos-$radius}] [expr {$y_pos-$radius}] [expr {$x_pos+$radius}] [expr {$y_pos+$radius}] \
            -fill $fill -outline {} -tags [list node $tag $row_tag]

        set label_text "$line_name $snap_id"
        set badges [Aitk::snapshot_state_label $item]
        if {$badges ne ""} {
            append label_text " \[$badges\]"
        }
        set label_text [Aitk::shorten_text $label_text $label_max_chars]
        set msg [Aitk::shorten_text $msg $message_max_chars]
        $c create text $label_x [expr {$y_pos}] -anchor w -text $label_text -tags [list node $tag $row_tag]
        $c create text $message_x [expr {$y_pos}] -anchor w -text $msg -fill #555555 -font "TkDefaultFont 9" -tags [list node $tag $row_tag]
        incr index
    }

    if {$index > 0} {
        set last_y [expr {$y + (($index - 1) * $row_h)}]
        set footer "shown $index snapshots"
        if {[string trim $filter_query] ne "" || [string trim $filter_stale_days] ne ""} {
            append footer " (filtered)"
        }
        $c create text $x0 [expr {$last_y + 24}] -anchor w -fill #888888 -text $footer
    }
    Aitk::sync_graph_scrollregion $c
}

proc Aitk::select_snapshot {snapshot_id} {
    variable state
    variable payload

    if {$snapshot_id eq ""} {
        return
    }
    dict set state selected_snapshot $snapshot_id
    if {![dict exists $payload snapshots_index]} {
        set index {}
        foreach item [Aitk::all_snapshot_items] {
            if {[dict exists $item snapshot_id]} {
                dict set index [dict get $item snapshot_id] $item
            }
        }
        dict set payload snapshots_index $index
    }

    if {[dict exists $payload snapshots_index $snapshot_id]} {
        dict set state selected_snapshot $snapshot_id
    }
    Aitk::render_diff
    Aitk::render_plan_context
    Aitk::render_graph
}

proc Aitk::render_diff {} {
    variable state
    variable widgets
    variable payload

    if {![dict exists $widgets diff_text]} {
        return
    }
    set diff_widget [dict get $widgets diff_text]
    $diff_widget delete 1.0 end

    set snapshot_id [dict get $state selected_snapshot]
    if {$snapshot_id eq ""} {
        $diff_widget insert end "No snapshot selected."
        return
    }

    if {![dict exists $payload snapshots_index $snapshot_id]} {
        $diff_widget insert end "Selected snapshot payload not found: $snapshot_id\n"
        return
    }
    set snapshot [dict get $payload snapshots_index $snapshot_id]

    set line_name ""
    set line_name [Aitk::line_for_snapshot $snapshot]
    set message ""
    if {[dict exists $snapshot message]} {
        set message [dict get $snapshot message]
    }
    set parent ""
    if {[dict exists $snapshot parent_snapshot_id]} {
        set parent [dict get $snapshot parent_snapshot_id]
    }
    set health ""
    if {[dict exists $snapshot health]} {
        set health [dict get $snapshot health]
    }
    set badges [Aitk::snapshot_state_label $snapshot]

    if {![dict exists $snapshot parent_diff] && [Aitk::lazy_snapshot_diff_enabled]} {
        $diff_widget insert end "Loading selected snapshot diff...\n"
        update idletasks
        set snapshot [Aitk::load_parent_diff_for_snapshot $snapshot_id $snapshot]
        $diff_widget delete 1.0 end
    }

    if {[dict exists $snapshot parent_diff]} {
        set diff [dict get $snapshot parent_diff]
        if {[dict exists $diff error]} {
            $diff_widget insert end "Diff error: [dict get $diff error]\n"
            return
        }

        set files_changed 0
        set insertions 0
        set deletions 0
        if {[dict exists $diff summary files_changed]} {
            set files_changed [dict get $diff summary files_changed]
        }
        if {[dict exists $diff summary insertions]} {
            set insertions [dict get $diff summary insertions]
        }
        if {[dict exists $diff summary deletions]} {
            set deletions [dict get $diff summary deletions]
        }

        if {![dict exists $diff files] || [llength [dict get $diff files]] == 0} {
            $diff_widget insert end "No file changes in parent diff.\n"
            return
        }

        set file_details {}
        set rendered_text_diff 0
        foreach file_row [dict get $diff files] {
            set file_path [Aitk::dict_get_default $file_row path "<unknown>"]
            set file_status [Aitk::dict_get_default $file_row status "changed"]
            set detail "\[$file_status\] $file_path"

            if {[dict exists $file_row diff]} {
                set text_diff [dict get $file_row diff]
                set diff_status [Aitk::dict_get_default $text_diff status "metadata_only"]
                set file_insertions [Aitk::dict_get_default $text_diff insertions 0]
                set file_deletions [Aitk::dict_get_default $text_diff deletions 0]
                append detail " ($diff_status, +$file_insertions -$file_deletions)"
                if {$diff_status eq "text" && [dict exists $text_diff text]} {
                    set inline_text [dict get $text_diff text]
                    if {$inline_text ne ""} {
                        if {$rendered_text_diff} {
                            $diff_widget insert end "\n"
                        }
                        Aitk::insert_diff_text $diff_widget $inline_text
                        set rendered_text_diff 1
                    }
                }
            }
            lappend file_details $detail
        }

        if {!$rendered_text_diff} {
            $diff_widget insert end "No inline text diff available for this snapshot.\n"
            $diff_widget insert end "Changed files:\n"
            foreach detail $file_details {
                $diff_widget insert end "  $detail\n"
            }
        }

        $diff_widget insert end "\nSummary: parent diff, $files_changed files changed, +$insertions -$deletions\n"
        $diff_widget insert end "Snapshot: $snapshot_id\n"
        if {$line_name ne ""} {
            $diff_widget insert end "Line: $line_name\n"
        }
        if {$parent ne ""} {
            $diff_widget insert end "Parent: $parent\n"
        }
        if {$health ne ""} {
            $diff_widget insert end "Health: $health\n"
        }
        if {$badges ne ""} {
            $diff_widget insert end "Badges: $badges\n"
        }
        if {$message ne ""} {
            $diff_widget insert end "Message: $message\n"
        }
        if {$rendered_text_diff} {
            $diff_widget insert end "Files:\n"
            foreach detail $file_details {
                $diff_widget insert end "  $detail\n"
            }
        }
    } elseif {[dict exists $snapshot changed_files]} {
        $diff_widget insert end "Changed Files:\n"
        foreach file [dict get $snapshot changed_files] {
            $diff_widget insert end "  - $file\n"
        }
        $diff_widget insert end "\nSnapshot: $snapshot_id\n"
        if {$line_name ne ""} {
            $diff_widget insert end "Line: $line_name\n"
        }
    } else {
        $diff_widget insert end "No changed_files payload for this snapshot.\n"
        $diff_widget insert end "\nSnapshot: $snapshot_id\n"
        if {$line_name ne ""} {
            $diff_widget insert end "Line: $line_name\n"
        }
    }
}

# --- CLI entry / bootstrap --------------------------------------------------

proc Aitk::bootstrap {args} {
    variable payload

    if {[llength $args] < 1} {
        tk_messageBox -icon error -title "aitk" -message "Usage: aitk <payload-path>"
        exit 1
    }

    set payload_path [lindex $args 0]
    if {[catch {set payload [Aitk::load_payload $payload_path]} load_err]} {
        tk_messageBox -icon error -title "aitk" -message "Failed to load payload:\n$load_err"
        puts stderr "aitk: failed to load payload: $load_err"
        exit 1
    }

    build_ui
    render_plan_context
    render_plan_links
    render_graph
    render_diff

    # default to first snapshot if available
    set snapshots [Aitk::snapshot_items]
    if {[llength $snapshots] > 0} {
        set first [lindex $snapshots 0]
        if {[dict exists $first snapshot_id]} {
            select_snapshot [dict get $first snapshot_id]
        }
    }

    return 0
}

Aitk::bootstrap $argv
