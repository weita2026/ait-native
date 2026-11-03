#!/bin/sh
# Native fork assertion only; no synthetic code-review or approval evidence.
awt_native_fork_in_dir() { awt_fork_cwd=$1; shift; (cd "$awt_fork_cwd" && "$@"); }
awt_native_fork_preserved() {
  awt_capture_json_success "$awt_fork_case.preserved" ".snapshot_id == \"$awt_fork_snapshot\"" \
    "$AIT_WEB_TEST_AIT_BIN" snapshot show "$awt_fork_snapshot" --json || return
  [ "$awt_fork_digest" = "$(jq -cS '{snapshot_id,parent_snapshot_id,line_name,message,files}' \
    "$(awt_case_stdout_path "$awt_fork_case.preserved")" | cksum)" ]
}
awt_native_reviewed_fork() {
  awt_fork_case=$1
  awt_fork_task=$2
  awt_fork_name=$3
  awt_fork_path=$4
  awt_fork_snapshot=$5
  awt_capture_json_success "$awt_fork_case.original" ".snapshot_id == \"$awt_fork_snapshot\"" \
    "$AIT_WEB_TEST_AIT_BIN" snapshot show "$awt_fork_snapshot" --json || return
  awt_fork_digest=$(jq -cS '{snapshot_id,parent_snapshot_id,line_name,message,files}' \
    "$(awt_case_stdout_path "$awt_fork_case.original")" | cksum) || return
  awt_capture_json_success "$awt_fork_case" ".task_id == \"$awt_fork_task\" and
    (.change_ref | startswith(\"$awt_fork_task/C-\")) and .status == \"draft\" and
    .fork_snapshot_id == \"$awt_fork_snapshot\"" \
    awt_native_fork_in_dir "$awt_fork_path" "$AIT_WEB_TEST_AIT_BIN" change create \
    "$awt_fork_task" --title ait-web-test-local-reviewed-correction \
    --fork-snapshot "$awt_fork_snapshot" --local --json || return
  awt_fork_revision=$(jq -er '.change_ref' "$(awt_case_stdout_path "$awt_fork_case")") || return
  # Used by the caller's correction capture assertion.
  # shellcheck disable=SC2034
  awt_fork_change_id=$(jq -er '.change_id' "$(awt_case_stdout_path "$awt_fork_case")") || return
  awt_capture_json_success "$awt_fork_case.owner" ".task_id == \"$awt_fork_task\" and
    .change_ref == \"$awt_fork_revision\" and .fork_snapshot_id == \"$awt_fork_snapshot\"" \
    "$AIT_WEB_TEST_AIT_BIN" change show "$awt_fork_task" --local --json || return
  # A Local revision belongs to the Task; it does not rewrite the original
  # Task-only worktree binding. Snapshot capture selects that Task's revision.
  awt_capture_json_success "$awt_fork_case.worktree" ".bound_task_id == \"$awt_fork_task\" and
    .bound_change_ref == null" \
    "$AIT_WEB_TEST_AIT_BIN" worktree show "$awt_fork_name" --json || return
  awt_native_fork_preserved
}
