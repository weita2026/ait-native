#!/usr/bin/env bash
set -euo pipefail

unset AIT_SHARED_CARGO_TARGET_DIR AIT_SHARED_CARGO_BUILD_DIR CARGO_TARGET_DIR CARGO_BUILD_BUILD_DIR
unset AIT_CARGO_BUILD_MAX_BYTES
repo_root="$(cd "$(dirname "$0")/.." && pwd -P)"
temporary_root="$(mktemp -d)"
temporary_root="$(cd "$temporary_root" && pwd -P)"
trap 'rm -rf -- "$temporary_root"' EXIT HUP INT TERM
source "$repo_root/ait.sh"
ROOT_DIR="$temporary_root/repo"
ram="$temporary_root/ram"
mkdir -p "$ROOT_DIR/.ait" "$ram" "$ROOT_DIR/.cargo"
printf '{"repo_name":"fixture","task_worktree":{"memory_root":{"root":"%s"}}}\n' "$ram" >"$ROOT_DIR/.ait/config.json"
printf '%s\n' "$CANONICAL_CARGO_SOURCE_POLICY_HEADER" '[build]' \
  'target-dir = ".ait/cargo-target"' 'build-dir = ".ait/cargo-build/canonical"' >"$ROOT_DIR/.cargo/config.toml"
primary="$ram/ait-runtime/cargo-build/fixture"
fallback="$ROOT_DIR/.ait/cargo-build-ssd"
test_capacity=8589934592
cargo_available_bytes() {
  [[ "$test_capacity" != unknown ]] || return 1
  printf '%s\n' "$test_capacity"
}

(
  AIT_CARGO_BUILD_MAX_BYTES=0
  test "$(cargo_ram_headroom)" = 4294967296
  test_capacity=1
  test "$(cargo_build_dir)" = "$fallback/canonical"
  test ! -e "$primary"
  test ! -L "$ROOT_DIR/.ait/cargo-build"
  test_capacity=unknown
  test "$(cargo_build_dir)" = "$fallback/canonical"
  test_capacity=8589934592
  AIT_CARGO_BUILD_MAX_BYTES=9663676416
  test "$(cargo_build_dir)" = "$fallback/canonical"
)
(
  test "$(cargo_build_dir)" = "$fallback/canonical"
  test ! -e "$primary"
  # Native bootstrap owns initial RAM link creation.
  mkdir -p "$primary"
  ln -s "$primary" "$ROOT_DIR/.ait/cargo-build"
  test "$(cargo_build_dir)" = "$primary/canonical"
  printf keep >"$primary/other-build"
  test "$(readlink "$ROOT_DIR/.ait/cargo-build")" = "$primary"
  test_capacity=1
  test "$(cargo_build_dir)" = "$fallback/canonical"
  test "$(readlink "$ROOT_DIR/.ait/cargo-build")" = "$primary"
  test "$(cat "$primary/other-build")" = keep
  printf '{"worktree_name":"task-one"}\n' >"$ROOT_DIR/.ait-worktree.json"
  test "$(cargo_build_dir)" = "$fallback/task-workspaces/task-one"
  rm "$ROOT_DIR/.ait-worktree.json"
)
(
  rm "$ROOT_DIR/.ait/cargo-build"
  mkdir -p "$ROOT_DIR/.ait/cargo-build"
  printf keep >"$ROOT_DIR/.ait/cargo-build/keep"
  test_capacity=unknown
  test "$(cargo_build_dir)" = "$ROOT_DIR/.ait/cargo-build/canonical"
  test "$(cat "$ROOT_DIR/.ait/cargo-build/keep")" = keep
  rm -rf "$ROOT_DIR/.ait/cargo-build"
  ln -s "$primary" "$ROOT_DIR/.ait/cargo-build"
)
(
  missing="$temporary_root/missing-ram"
  printf '{"repo_name":"fixture","task_worktree":{"memory_root":{"root":"%s"}}}\n' "$missing" >"$ROOT_DIR/.ait/config.json"
  rm "$ROOT_DIR/.ait/cargo-build"
  ln -s "$missing/ait-runtime/cargo-build/fixture" "$ROOT_DIR/.ait/cargo-build"
  test "$(cargo_build_dir)" = "$fallback/canonical"
  test ! -e "$missing"
  rm "$ROOT_DIR/.ait/cargo-build"
)
# Escaped JSON keys and paths select the same existing configuration authority.
printf '{"repo_name":"fixture","task_worktree":{"memory_root":{"root":"ignored"}},"task_worktree":{"memory_root":{"r\\u006fot":"%s"}}}\n' "$ram" >"$ROOT_DIR/.ait/config.json"
test "$(cargo_memory_root)" = "$ram"
ln -s "$primary" "$ROOT_DIR/.ait/cargo-build"
(
  cargo_memory_mount_available() { return 1; }
  test_capacity=8589934592
  test "$(cargo_build_dir)" = "$fallback/canonical"
  test "$(readlink "$ROOT_DIR/.ait/cargo-build")" = "$primary"
)
(
  mkdir -p "$temporary_root/previous-volume"
  printf keep >"$temporary_root/previous-volume/keep"
  rm "$ROOT_DIR/.ait/cargo-build"
  ln -s "$temporary_root/previous-volume" "$ROOT_DIR/.ait/cargo-build"
  test_capacity=1
  test "$(cargo_build_dir)" = "$fallback/canonical"
  test "$(cat "$temporary_root/previous-volume/keep")" = keep
  rm "$ROOT_DIR/.ait/cargo-build"
  ln -s "$primary" "$ROOT_DIR/.ait/cargo-build"
)
(
  mkdir -p "$temporary_root/foreign"
  ln -s "$temporary_root/foreign" "$fallback"
  test_capacity=1
  if cargo_build_dir; then exit 65; fi
  test "$(readlink "$fallback")" = "$temporary_root/foreign"
  rm "$fallback"
)
(
  test_capacity=1
  CARGO_BUILD_BUILD_DIR="$temporary_root/explicit-ssd"
  CARGO_TARGET_DIR="$temporary_root/explicit-target"
  test "$(cargo_build_dir)" = "$CARGO_BUILD_BUILD_DIR"
  test "$(cargo_target_dir)" = "$CARGO_TARGET_DIR"
)
(
  mkdir -p "$fallback/task-workspaces" "$temporary_root/foreign"
  ln -s "$temporary_root/foreign" "$fallback/task-workspaces/task-one"
  printf '{"worktree_name":"task-one"}\n' >"$ROOT_DIR/.ait-worktree.json"
  test_capacity=1
  if cargo_build_dir; then exit 65; fi
  test -L "$fallback/task-workspaces/task-one"
  rm "$fallback/task-workspaces/task-one" "$ROOT_DIR/.ait-worktree.json"
)
(
  rm "$ROOT_DIR/.ait/cargo-build"
  printf '{"repo_name":"fixture","task_worktree":{"memory_root":{"root":"%s"}}}\n' "$ROOT_DIR" >"$ROOT_DIR/.ait/config.json"
  test_capacity=1
  if cargo_build_dir; then exit 65; fi
  test ! -L "$ROOT_DIR/.ait/cargo-build"
)
printf '{"repo_name":"fixture","task_worktree":{"memory_root":{"root":"%s"}}}\n' "$ram" >"$ROOT_DIR/.ait/config.json"
ln -s "$primary" "$ROOT_DIR/.ait/cargo-build"

mkdir -p "$temporary_root/bin"
cat >"$temporary_root/bin/cargo" <<'CARGO'
#!/usr/bin/env bash
set -euo pipefail
printf '%s|%s|%s|%s\n' "$CARGO_BUILD_BUILD_DIR" "$CARGO_TARGET_DIR" "$*" "${AIT_CARGO_BUILD_MAX_BYTES:-unset}" >>"$TEST_CARGO_LOG"
darwin_space_diagnostic() {
  printf 'cargo:warning=write64() failed, returned -1 expected 919616, errno=28\n' >&2
  printf 'ranlib: failed to write archive libzstd.a\n' >&2
}
darwin_space_failure() {
  darwin_space_diagnostic
  exit 28
}
case "$TEST_CARGO_CASE" in
  unrelated) printf 'ordinary test mentions ENOSPC\n'; printf 'error: expected ENOSPC but got another result\n' >&2; exit 23 ;;
  darwin_unrelated_errno) printf 'error: fixture expected errno=28 from write64()\n' >&2; exit 23 ;;
  darwin_other_errno) printf 'cargo:warning=write64() failed, returned -1 expected 919616, errno=5\n' >&2; exit 23 ;;
  darwin_exit28_only) exit 28 ;;
  darwin_errno280) printf 'cargo:warning=write64() failed, returned -1 expected 919616, errno=280\n' >&2; exit 28 ;;
  other_os_error280) printf 'error: failed to write archive (os error 280)\n' >&2; exit 28 ;;
  darwin_success_warning) darwin_space_diagnostic; exit 0 ;;
  both_space) printf 'error: failed to build archive: No space left on device (os error 28)\n' >&2; exit 28 ;;
  darwin_both_space) darwin_space_failure ;;
  darwin_space_then_success)
    case "$CARGO_BUILD_BUILD_DIR" in
      *cargo-build-ssd*) exit 0 ;;
      *) darwin_space_failure ;;
    esac ;;
  space_then_success)
    case "$CARGO_BUILD_BUILD_DIR" in
      *cargo-build-ssd*) exit 0 ;;
      *) printf 'error: failed to build archive: No space left on device (os error 28)\n' >&2; exit 28 ;;
    esac ;;
  *) exit 0 ;;
esac
CARGO
chmod +x "$temporary_root/bin/cargo"
resolve_cargo() { printf '%s\n' "$temporary_root/bin/cargo"; }
resolve_rust_tool() { :; }
export TEST_CARGO_LOG="$temporary_root/cargo.log"
export TEST_CARGO_CASE

for test_case in space_then_success darwin_space_then_success unrelated darwin_unrelated_errno darwin_other_errno darwin_exit28_only darwin_errno280 other_os_error280 darwin_success_warning both_space darwin_both_space; do
  (
    unset CARGO_TARGET_DIR CARGO_BUILD_BUILD_DIR
    export AIT_CARGO_BUILD_MAX_BYTES=0
    TEST_CARGO_CASE="$test_case"
    : >"$TEST_CARGO_LOG"
    status=0
    run_cargo build --profile release --locked || status=$?
    attempts="$(wc -l <"$TEST_CARGO_LOG" | tr -d ' ')"
    case "$test_case" in
      space_then_success|darwin_space_then_success)
        test "$status" = 0
        test "$attempts" = 2
        sed -n '1p' "$TEST_CARGO_LOG" | grep -F "$primary/canonical|" >/dev/null
        sed -n '2p' "$TEST_CARGO_LOG" | grep -F "$fallback/canonical|" >/dev/null
        ;;
      unrelated|darwin_unrelated_errno|darwin_other_errno) test "$status" = 23; test "$attempts" = 1 ;;
      darwin_exit28_only|darwin_errno280|other_os_error280) test "$status" = 28; test "$attempts" = 1 ;;
      darwin_success_warning) test "$status" = 0; test "$attempts" = 1 ;;
      both_space|darwin_both_space) test "$status" = 28; test "$attempts" = 2 ;;
    esac
    test "$(cut -d '|' -f 2- "$TEST_CARGO_LOG" | sort -u | wc -l | tr -d ' ')" = 1
    test "$(readlink "$ROOT_DIR/.ait/cargo-build")" = "$primary"
    test "$(cat "$primary/other-build")" = keep
  )
done
(
  # Both cache roots share the existing repository budget and lock/age policy.
  lsof() { return 1; }
  AIT_CARGO_BUILD_MAX_BYTES=1048576
  AIT_CARGO_BUILD_GC_INTERVAL_SECONDS=0
  mkdir -p "$primary/task-workspaces/old-ram" "$fallback/task-workspaces/old-ssd" "$fallback/unowned"
  dd if=/dev/zero of="$primary/task-workspaces/old-ram/cache" bs=1024 count=900 2>/dev/null
  dd if=/dev/zero of="$fallback/task-workspaces/old-ssd/cache" bs=1024 count=900 2>/dev/null
  printf keep >"$fallback/unowned/keep"
  auto_reclaim_cargo_build_dir "$primary/canonical"
  total_kib="$(du -sk "$primary" "$fallback" | awk '{sum+=$1} END {print sum}')"
  test "$total_kib" -le 1024
  test "$(cat "$fallback/unowned/keep")" = keep
  mkdir -p "$fallback/task-workspaces/task-retained"
  compact_output="$(compact_cargo_targets --dry-run --include-worktrees)"
  printf '%s\n' "$compact_output" | grep -F "$fallback/task-workspaces/task-retained" >/dev/null
)
(
  TEST_CARGO_CASE=darwin_both_space
  CARGO_BUILD_BUILD_DIR="$temporary_root/explicit-ssd"
  : >"$TEST_CARGO_LOG"
  status=0
  run_cargo test --profile ait-ci || status=$?
  test "$status" = 28
  test "$(wc -l <"$TEST_CARGO_LOG" | tr -d ' ')" = 1
  test "$(cut -d '|' -f 4 "$TEST_CARGO_LOG")" = unset
)
# Exercise the actual launcher in a separate process without filling a volume.
cp "$repo_root/ait.sh" "$ROOT_DIR/ait.sh"
cat >"$temporary_root/bin/df" <<'DF'
#!/usr/bin/env bash
printf 'Filesystem 1024-blocks Used Available Capacity Mounted\nfixture 16777216 0 8388608 0%% /\n'
DF
chmod +x "$temporary_root/bin/df"
mkdir -p "$ROOT_DIR/.ait/cargo-target/release"
printf original >"$ROOT_DIR/.ait/cargo-target/release/ait-cli"
: >"$TEST_CARGO_LOG"
TEST_CARGO_CASE=darwin_space_then_success CARGO_HOME="$temporary_root" AIT_CARGO_BUILD_MAX_BYTES=0 \
  bash "$ROOT_DIR/ait.sh" core build
test "$(wc -l <"$TEST_CARGO_LOG" | tr -d ' ')" = 2
test "$(cat "$ROOT_DIR/.ait/cargo-target/release/ait-cli")" = original
test "$(readlink "$ROOT_DIR/.ait/cargo-build")" = "$primary"
printf 'ait.sh RAM/SSD Cargo fallback: pass\n'
