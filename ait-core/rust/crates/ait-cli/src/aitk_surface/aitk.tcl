#!/usr/bin/env wish

if {[catch {package require Tk} error]} {
    puts stderr "aitk: Tcl/Tk is unavailable: $error"
    exit 1
}

namespace eval Aitk {
    variable metadata {}
    variable lines {}
    variable snapshots {}
    variable paths {}
    variable loaded_diffs {}
    variable line_by_item {}
    variable snapshot_by_item {}
    variable query ""
    variable health_filter "all"
    variable selected_line ""
}

proc Aitk::decode {value} {
    if {$value eq ""} {
        return ""
    }
    return [encoding convertfrom utf-8 [binary decode base64 $value]]
}

proc Aitk::field {parts index} {
    if {$index >= [llength $parts]} {
        return ""
    }
    return [Aitk::decode [lindex $parts $index]]
}

proc Aitk::load_payload {path} {
    variable metadata
    variable lines
    variable snapshots
    variable paths

    if {![file isfile $path]} {
        error "temporary payload does not exist: $path"
    }
    set handle [open $path r]
    try {
        set content [read $handle]
    } finally {
        close $handle
    }
    set rows [split $content "\n"]
    if {[llength $rows] == 0 || [lindex $rows 0] ne "aitk-tsv-v1"} {
        error "unsupported embedded payload"
    }
    foreach raw [lrange $rows 1 end] {
        if {$raw eq ""} {
            continue
        }
        set parts [split $raw "\t"]
        set kind [lindex $parts 0]
        switch -- $kind {
            meta {
                dict set metadata [Aitk::field $parts 1] [Aitk::field $parts 2]
            }
            line {
                lappend lines [dict create \
                    line_name [Aitk::field $parts 1] \
                    head_snapshot_id [Aitk::field $parts 2] \
                    status [Aitk::field $parts 3] \
                    health [Aitk::field $parts 4] \
                    ahead_by [Aitk::field $parts 5] \
                    behind_by [Aitk::field $parts 6]]
            }
            snapshot {
                lappend snapshots [dict create \
                    snapshot_id [Aitk::field $parts 1] \
                    line_name [Aitk::field $parts 2] \
                    created_at [Aitk::field $parts 3] \
                    message [Aitk::field $parts 4] \
                    snapshot_kind [Aitk::field $parts 5] \
                    parents [Aitk::field $parts 6] \
                    head_labels [Aitk::field $parts 7] \
                    line_health [Aitk::field $parts 8] \
                    graph_column [Aitk::field $parts 9] \
                    file_count [Aitk::field $parts 10] \
                    total_bytes [Aitk::field $parts 11] \
                    changed_path_count [Aitk::field $parts 12] \
                    changed_paths_truncated [Aitk::field $parts 13] \
                    diff_error [Aitk::field $parts 14] \
                    diff_state [Aitk::field $parts 15]]
            }
            path {
                set snapshot_id [Aitk::field $parts 1]
                dict lappend paths $snapshot_id [dict create \
                    status [Aitk::field $parts 2] \
                    path [Aitk::field $parts 3]]
            }
            default {
                error "unknown embedded payload row: $kind"
            }
        }
    }
}

proc Aitk::dict_get {row key {default ""}} {
    if {[dict exists $row $key]} {
        return [dict get $row $key]
    }
    return $default
}

proc Aitk::health_color {health} {
    switch -- $health {
        current_main { return "#217a3c" }
        uncontained { return "#b45309" }
        contained { return "#64748b" }
        empty { return "#7c3aed" }
        missing_snapshot { return "#b91c1c" }
        default { return "#334155" }
    }
}

proc Aitk::build_ui {} {
    variable metadata

    set repo_name [Aitk::dict_get $metadata name "repository"]
    set repo_root [Aitk::dict_get $metadata root ""]
    wm title . "aitk — $repo_name"
    wm geometry . 1280x820
    wm minsize . 900 560

    ttk::frame .root -padding 8
    pack .root -fill both -expand 1

    ttk::frame .root.header
    ttk::label .root.header.title -text "$repo_name" -font TkHeadingFont
    ttk::label .root.header.root -text $repo_root
    pack .root.header.title -side left
    pack .root.header.root -side left -padx 12
    pack .root.header -side top -fill x -pady {0 8}

    ttk::frame .root.filters
    ttk::label .root.filters.search_label -text "Search"
    ttk::entry .root.filters.search -textvariable Aitk::query -width 36
    ttk::label .root.filters.health_label -text "Health"
    ttk::combobox .root.filters.health \
        -textvariable Aitk::health_filter \
        -values {all current_main uncontained contained empty unknown missing_snapshot historical} \
        -state readonly -width 18
    ttk::button .root.filters.clear_line -text "All lines" -command {
        set Aitk::selected_line ""
        .root.main.top.lines.tree selection set {}
        Aitk::refresh_history
    }
    pack .root.filters.search_label -side left
    pack .root.filters.search -side left -padx {5 12}
    pack .root.filters.health_label -side left
    pack .root.filters.health -side left -padx 5
    pack .root.filters.clear_line -side right
    pack .root.filters -side top -fill x -pady {0 8}

    ttk::panedwindow .root.main -orient vertical
    ttk::panedwindow .root.main.top -orient horizontal
    ttk::frame .root.main.top.lines
    ttk::frame .root.main.top.history
    ttk::frame .root.main.details

    ttk::label .root.main.top.lines.label -text "Lines"
    ttk::treeview .root.main.top.lines.tree \
        -columns {health ahead behind head} -show {tree headings} \
        -selectmode browse -yscrollcommand {.root.main.top.lines.scroll set}
    ttk::scrollbar .root.main.top.lines.scroll -orient vertical \
        -command {.root.main.top.lines.tree yview}
    .root.main.top.lines.tree heading #0 -text "Line"
    .root.main.top.lines.tree heading health -text "Health"
    .root.main.top.lines.tree heading ahead -text "Ahead"
    .root.main.top.lines.tree heading behind -text "Behind"
    .root.main.top.lines.tree heading head -text "Head"
    .root.main.top.lines.tree column #0 -width 180 -stretch 1
    .root.main.top.lines.tree column health -width 105 -stretch 0
    .root.main.top.lines.tree column ahead -width 55 -stretch 0 -anchor center
    .root.main.top.lines.tree column behind -width 55 -stretch 0 -anchor center
    .root.main.top.lines.tree column head -width 135 -stretch 1
    grid .root.main.top.lines.tree -row 1 -column 0 -sticky nsew
    grid .root.main.top.lines.scroll -row 1 -column 1 -sticky ns
    grid .root.main.top.lines.label -row 0 -column 0 -sticky w
    grid rowconfigure .root.main.top.lines 1 -weight 1
    grid columnconfigure .root.main.top.lines 0 -weight 1

    ttk::label .root.main.top.history.label -text "Snapshot history"
    ttk::treeview .root.main.top.history.tree \
        -columns {graph id line heads date message} -show headings \
        -selectmode browse -yscrollcommand {.root.main.top.history.vscroll set} \
        -xscrollcommand {.root.main.top.history.hscroll set}
    ttk::scrollbar .root.main.top.history.vscroll -orient vertical \
        -command {.root.main.top.history.tree yview}
    ttk::scrollbar .root.main.top.history.hscroll -orient horizontal \
        -command {.root.main.top.history.tree xview}
    foreach {column label width stretch} {
        graph "Graph" 95 0
        id "Snapshot" 145 0
        line "Authored line" 145 0
        heads "Head labels" 150 0
        date "Created" 155 0
        message "Message" 360 1
    } {
        .root.main.top.history.tree heading $column -text $label
        .root.main.top.history.tree column $column -width $width -stretch $stretch
    }
    grid .root.main.top.history.label -row 0 -column 0 -sticky w
    grid .root.main.top.history.tree -row 1 -column 0 -sticky nsew
    grid .root.main.top.history.vscroll -row 1 -column 1 -sticky ns
    grid .root.main.top.history.hscroll -row 2 -column 0 -sticky ew
    grid rowconfigure .root.main.top.history 1 -weight 1
    grid columnconfigure .root.main.top.history 0 -weight 1

    ttk::label .root.main.details.label -text "Selected Snapshot / parent diff summary"
    text .root.main.details.text -wrap none -state disabled -font TkFixedFont \
        -yscrollcommand {.root.main.details.vscroll set} \
        -xscrollcommand {.root.main.details.hscroll set}
    ttk::scrollbar .root.main.details.vscroll -orient vertical \
        -command {.root.main.details.text yview}
    ttk::scrollbar .root.main.details.hscroll -orient horizontal \
        -command {.root.main.details.text xview}
    grid .root.main.details.label -row 0 -column 0 -sticky w
    grid .root.main.details.text -row 1 -column 0 -sticky nsew
    grid .root.main.details.vscroll -row 1 -column 1 -sticky ns
    grid .root.main.details.hscroll -row 2 -column 0 -sticky ew
    grid rowconfigure .root.main.details 1 -weight 1
    grid columnconfigure .root.main.details 0 -weight 1

    .root.main.top add .root.main.top.lines -weight 1
    .root.main.top add .root.main.top.history -weight 3
    .root.main add .root.main.top -weight 3
    .root.main add .root.main.details -weight 2
    pack .root.main -side top -fill both -expand 1

    bind .root.filters.search <KeyRelease> {Aitk::refresh_history}
    bind .root.filters.health <<ComboboxSelected>> {Aitk::refresh_history}
    bind .root.main.top.lines.tree <<TreeviewSelect>> {Aitk::select_line}
    bind .root.main.top.history.tree <<TreeviewSelect>> {Aitk::select_snapshot}

    Aitk::populate_lines
    Aitk::refresh_history
}

proc Aitk::populate_lines {} {
    variable lines
    variable line_by_item
    set tree .root.main.top.lines.tree
    foreach item [$tree children {}] {
        $tree delete $item
    }
    set line_by_item {}
    foreach row $lines {
        set name [Aitk::dict_get $row line_name]
        set health [Aitk::dict_get $row health]
        set item [$tree insert {} end -text $name -values [list \
            $health \
            [Aitk::dict_get $row ahead_by] \
            [Aitk::dict_get $row behind_by] \
            [Aitk::dict_get $row head_snapshot_id]]]
        dict set line_by_item $item $row
        $tree tag configure $health -foreground [Aitk::health_color $health]
        $tree item $item -tags [list $health]
    }
}

proc Aitk::select_line {} {
    variable line_by_item
    variable selected_line
    set tree .root.main.top.lines.tree
    set selection [$tree selection]
    if {[llength $selection] == 0} {
        set selected_line ""
    } else {
        set row [dict get $line_by_item [lindex $selection 0]]
        set selected_line [dict get $row line_name]
    }
    Aitk::refresh_history
}

proc Aitk::snapshot_matches {row} {
    variable query
    variable health_filter
    variable selected_line
    if {$selected_line ne "" && [Aitk::dict_get $row line_name] ne $selected_line} {
        return 0
    }
    set health [Aitk::dict_get $row line_health historical]
    if {$health_filter ne "all" && $health ne $health_filter} {
        return 0
    }
    set needle [string tolower [string trim $query]]
    if {$needle eq ""} {
        return 1
    }
    set snapshot_id [Aitk::dict_get $row snapshot_id]
    set path_text ""
    variable paths
    if {[dict exists $paths $snapshot_id]} {
        foreach path_row [dict get $paths $snapshot_id] {
            append path_text " " [Aitk::dict_get $path_row path]
        }
    }
    set haystack [string tolower [join [list \
        $snapshot_id \
        [Aitk::dict_get $row line_name] \
        [Aitk::dict_get $row message] \
        [Aitk::dict_get $row head_labels] \
        $path_text] " "]]
    return [expr {[string first $needle $haystack] >= 0}]
}

proc Aitk::short_id {value} {
    if {[string length $value] <= 16} {
        return $value
    }
    return [string range $value 0 15]
}

proc Aitk::refresh_history {} {
    variable snapshots
    variable snapshot_by_item
    set tree .root.main.top.history.tree
    foreach item [$tree children {}] {
        $tree delete $item
    }
    set snapshot_by_item {}
    foreach row $snapshots {
        if {![Aitk::snapshot_matches $row]} {
            continue
        }
        set column [Aitk::dict_get $row graph_column 0]
        if {![string is integer -strict $column]} {
            set column 0
        }
        set graph "[string repeat {│ } $column]●"
        set health [Aitk::dict_get $row line_health historical]
        set item [$tree insert {} end -values [list \
            $graph \
            [Aitk::short_id [Aitk::dict_get $row snapshot_id]] \
            [Aitk::dict_get $row line_name] \
            [Aitk::dict_get $row head_labels] \
            [Aitk::dict_get $row created_at] \
            [Aitk::dict_get $row message]]]
        dict set snapshot_by_item $item $row
        $tree tag configure $health -foreground [Aitk::health_color $health]
        $tree item $item -tags [list $health]
    }
    set first [$tree children {}]
    if {[llength $first] > 0} {
        $tree selection set [lindex $first 0]
        $tree focus [lindex $first 0]
        Aitk::select_snapshot
    } else {
        Aitk::show_detail_text "No Snapshot matches the current filters.\n"
    }
}

proc Aitk::select_snapshot {} {
    variable snapshot_by_item
    variable paths
    set tree .root.main.top.history.tree
    set selection [$tree selection]
    if {[llength $selection] == 0} {
        return
    }
    set item [lindex $selection 0]
    set row [dict get $snapshot_by_item $item]
    set row [Aitk::load_lazy_diff $row]
    dict set snapshot_by_item $item $row
    set snapshot_id [Aitk::dict_get $row snapshot_id]
    set detail "Snapshot: $snapshot_id\n"
    append detail "Parents:  [Aitk::dict_get $row parents none]\n"
    append detail "Line:     [Aitk::dict_get $row line_name]\n"
    append detail "Heads:    [Aitk::dict_get $row head_labels none]\n"
    append detail "Health:   [Aitk::dict_get $row line_health]\n"
    append detail "Kind:     [Aitk::dict_get $row snapshot_kind]\n"
    append detail "Created:  [Aitk::dict_get $row created_at]\n"
    append detail "Files:    [Aitk::dict_get $row file_count] ([Aitk::dict_get $row total_bytes] bytes)\n"
    append detail "Message:  [Aitk::dict_get $row message]\n\n"
    set diff_error [Aitk::dict_get $row diff_error]
    set diff_state [Aitk::dict_get $row diff_state unavailable]
    if {$diff_state eq "not_preloaded"} {
        append detail "Parent diff was not preloaded and could not be loaded on selection.\n"
    } elseif {$diff_error ne ""} {
        append detail "Diff unavailable: $diff_error\n"
    } elseif {[dict exists $paths $snapshot_id]} {
        append detail "Parent diff — [Aitk::dict_get $row changed_path_count] changed paths:\n"
        foreach path_row [dict get $paths $snapshot_id] {
            append detail [format "  %-12s %s\n" \
                [Aitk::dict_get $path_row status] \
                [Aitk::dict_get $path_row path]]
        }
        if {[Aitk::dict_get $row changed_paths_truncated] eq "true"} {
            append detail "  … changed-path list truncated by the bounded export limit\n"
        }
    } else {
        append detail "Parent diff — no changed paths.\n"
    }
    Aitk::show_detail_text $detail
}

proc Aitk::load_lazy_diff {row} {
    variable metadata
    variable paths
    variable loaded_diffs

    if {[Aitk::dict_get $row diff_state unavailable] ne "not_preloaded"} {
        return $row
    }
    set snapshot_id [Aitk::dict_get $row snapshot_id]
    if {[dict exists $loaded_diffs $snapshot_id]} {
        set cached [dict get $loaded_diffs $snapshot_id]
        dict set row diff_state [dict get $cached diff_state]
        dict set row diff_error [dict get $cached diff_error]
        dict set row changed_path_count [dict get $cached changed_path_count]
        dict set row changed_paths_truncated [dict get $cached changed_paths_truncated]
        return $row
    }

    set aitk_command [Aitk::dict_get $metadata aitk_command ""]
    set repo_root [Aitk::dict_get $metadata root ""]
    if {$aitk_command eq "" || ![file isfile $aitk_command]} {
        set message "running aitk executable is unavailable for lazy diff loading"
        dict set row diff_state unavailable
        dict set row diff_error $message
        dict set loaded_diffs $snapshot_id [dict create \
            diff_state unavailable diff_error $message \
            changed_path_count "" changed_paths_truncated false]
        return $row
    }

    set command [list $aitk_command -C $repo_root --ui-diff-tsv $snapshot_id]
    if {[catch {set content [exec {*}$command]} error]} {
        dict set row diff_state unavailable
        dict set row diff_error $error
        dict set loaded_diffs $snapshot_id [dict create \
            diff_state unavailable diff_error $error \
            changed_path_count "" changed_paths_truncated false]
        return $row
    }

    set raw_rows [split $content "\n"]
    if {[llength $raw_rows] == 0 || [lindex $raw_rows 0] ne "aitk-diff-tsv-v1"} {
        set message "lazy diff returned an unsupported payload"
        dict set row diff_state unavailable
        dict set row diff_error $message
        dict set loaded_diffs $snapshot_id [dict create \
            diff_state unavailable diff_error $message \
            changed_path_count "" changed_paths_truncated false]
        return $row
    }
    set path_rows {}
    set changed_path_count 0
    set truncated false
    foreach raw [lrange $raw_rows 1 end] {
        if {$raw eq ""} {
            continue
        }
        set parts [split $raw "\t"]
        switch -- [lindex $parts 0] {
            meta {
                set key [Aitk::field $parts 1]
                set value [Aitk::field $parts 2]
                if {$key eq "changed_path_count"} {
                    set changed_path_count $value
                } elseif {$key eq "truncated"} {
                    set truncated $value
                }
            }
            path {
                lappend path_rows [dict create \
                    status [Aitk::field $parts 1] \
                    path [Aitk::field $parts 2]]
            }
        }
    }
    dict set paths $snapshot_id $path_rows
    dict set row diff_state loaded
    dict set row diff_error ""
    dict set row changed_path_count $changed_path_count
    dict set row changed_paths_truncated $truncated
    dict set loaded_diffs $snapshot_id [dict create \
        diff_state loaded diff_error "" \
        changed_path_count $changed_path_count changed_paths_truncated $truncated]
    return $row
}

proc Aitk::show_detail_text {content} {
    set widget .root.main.details.text
    $widget configure -state normal
    $widget delete 1.0 end
    $widget insert end $content
    $widget configure -state disabled
}

if {[llength $argv] != 1} {
    puts stderr "aitk: embedded UI expected one payload path"
    exit 2
}

if {[catch {
    Aitk::load_payload [lindex $argv 0]
    Aitk::build_ui
} error options]} {
    puts stderr "aitk: $error"
    tk_messageBox -icon error -title "aitk" -message $error
    exit 1
}
