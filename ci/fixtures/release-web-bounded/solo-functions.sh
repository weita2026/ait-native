# Frozen WT declarations for isolated CLI-boundary regression tests.
awt_in_dir() {
  awt_target_dir=$1
  shift
  (
    cd "$awt_target_dir"
    "$@"
  )
}

awt_restore_solo_local_config() {
  awt_config_restore
}

awt_start_unbound_task() {
  awt_start_key=$1
  awt_start_title=$2
  awt_start_projection=$3
  awt_start_json="$AIT_WEB_TEST_RESULTS_DIR/$awt_start_key.setup.json"
  case "$awt_start_projection" in
    compact)
      set --
      ;;
    full)
      set -- --full
      ;;
    *)
      awt_error "unsupported task-start JSON projection: $awt_start_projection"
      return 64
      ;;
  esac
  awt_config_set sprint off >"$awt_start_json.config-off" 2>&1 || return
  if awt_in_dir "$AIT_WEB_TEST_REPOSITORY_ROOT" "$AIT_WEB_TEST_AIT_BIN" task start \
    --title "$awt_start_title" --intent direct-qualification --local --json "$@" \
    >"$awt_start_json" 2>"$awt_start_json.stderr"; then
    awt_start_status=0
  else
    awt_start_status=$?
  fi
  awt_restore_solo_local_config || return 1
  [ "$awt_start_status" -eq 0 ] || return "$awt_start_status"
  awt_started_task_id=$(jq -er '.task_id | select(test("^LWTT-[0-9]+$"))' \
    "$awt_start_json") || return
  awt_started_task_ids="$awt_started_task_ids $awt_started_task_id"
  case "$awt_start_projection" in
    compact)
      jq -e '
        .contract == "ait-agent-action/v1" and .command == "task.start" and
        .ok == true and (.task_id | test("^LWTT-[0-9]+$")) and
        (has("change_ref") | not) and
        (.line_name | test("^feature/lwtt-[0-9]+$")) and
        (.worktree_name | test("^lwtt-[0-9]+$")) and
        (.edit_root | startswith("/")) and
        .next_action.code == "enter_worktree" and
        (.next_action.command | startswith("cd ")) and
        (has("worktree") | not)' "$awt_start_json" >/dev/null
      ;;
    full)
      jq -e '
        .contract == null and .task_id and .change == null and
        (.worktree.path | startswith("/")) and .worktree.name and
        .worktree.current_line' "$awt_start_json" >/dev/null
      ;;
  esac
}

awt_abandon_mutation_task() {
  if [ -n "$awt_mutation_task_id" ]; then
    awt_in_dir "$AIT_WEB_TEST_REPOSITORY_ROOT" "$AIT_WEB_TEST_AIT_BIN" task abandon \
      "$awt_mutation_task_id" --local --json >/dev/null || return
  fi
  awt_mutation_task_id=
  awt_mutation_change_ref=
  awt_mutation_worktree=
  awt_mutation_worktree_name=
  awt_mutation_line=
  awt_mutation_parent=
  awt_mutation_head=
}

awt_ensure_mutation_task() {
  [ -z "$awt_mutation_task_id" ] || return 0
  awt_start_unbound_task ait.semantic.setup-mutation-task \
    ait-web-test-local-mutation-probe full || return
  awt_setup_json="$AIT_WEB_TEST_RESULTS_DIR/ait.semantic.setup-mutation-task.setup.json"
  awt_mutation_task_id=$(jq -r '.task_id' "$awt_setup_json")
  awt_mutation_change_ref=$(jq -r '.change_ref // .change.change_ref' "$awt_setup_json")
  awt_mutation_worktree=$(jq -r '.edit_root // .worktree.path' "$awt_setup_json")
  awt_mutation_worktree_name=$(jq -r '.worktree_name // .worktree.name' "$awt_setup_json")
  awt_mutation_line=$(jq -r '.line_name // .worktree.current_line' "$awt_setup_json")
  [ -d "$awt_mutation_worktree/.ait" ]
  [ -n "$awt_mutation_line" ]
  [ "$awt_mutation_line" != null ]
}

awt_json() {
  awt_json_case=$1
  awt_json_filter=$2
  shift 2
  awt_capture_json_success "$awt_json_case" "$awt_json_filter" "$@"
}
