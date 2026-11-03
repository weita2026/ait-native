# Frozen WT declarations for isolated CLI-boundary regression tests.
awt_sanitize_field() {
  printf '%s' "$1" | tr '\t\r\n' '   '
}

awt_case_file_key() {
  printf '%s' "$1" | tr -c 'A-Za-z0-9._-' '_'
}

awt_case_stdout_path() {
  printf '%s/%s.stdout\n' "$AIT_WEB_TEST_RESULTS_DIR" "$(awt_case_file_key "$1")"
}

awt_case_stderr_path() {
  printf '%s/%s.stderr\n' "$AIT_WEB_TEST_RESULTS_DIR" "$(awt_case_file_key "$1")"
}

awt_record_argv() {
  awt_argv_case=$1
  shift
  awt_argv_path="$AIT_WEB_TEST_RESULTS_DIR/$(awt_case_file_key "$awt_argv_case").argv"
  : >"$awt_argv_path"
  for awt_argv_value in "$@"; do
    printf '%s\n' "$(awt_sanitize_field "$awt_argv_value")" >>"$awt_argv_path"
  done
}

awt_capture_success() {
  awt_capture_case=$1
  shift
  awt_capture_stdout=$(awt_case_stdout_path "$awt_capture_case")
  awt_capture_stderr=$(awt_case_stderr_path "$awt_capture_case")
  awt_record_argv "$awt_capture_case" "$@"
  if "$@" >"$awt_capture_stdout" 2>"$awt_capture_stderr"; then
    return 0
  else
    return $?
  fi
}

awt_capture_json_success() {
  awt_capture_case=$1
  awt_capture_filter=$2
  shift 2
  awt_capture_success "$awt_capture_case" "$@" || return
  jq -e "$awt_capture_filter" "$(awt_case_stdout_path "$awt_capture_case")" >/dev/null
}
