#![cfg(unix)]
#![allow(dead_code)]
use ait_core::json_support::{JsonCodec, JsonValue};
use assert_cmd::prelude::*;
use std::{fs, os::unix::fs::PermissionsExt, process::Command};
use tempfile::TempDir;
pub fn command(temp: &TempDir) -> Command {
    let mut c = Command::cargo_bin("ait-cli").unwrap();
    c.current_dir(temp.path())
        .env_remove("CODEX_THREAD_ID")
        .env_remove("CODEX_SESSION_ID")
        .env("CODEX_HOME", temp.path().join("native-home"))
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", temp.path().join("bin").display()),
        );
    c
}
pub fn run(temp: &TempDir, args: &[&str], success: bool) -> JsonValue {
    let o = command(temp).args(args).output().unwrap();
    assert_eq!(
        o.status.success(),
        success,
        "{} {}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    JsonCodec::parse_slice_with_error_prefix(&o.stdout, "CLI JSON").unwrap()
}
pub fn fixture() -> TempDir {
    let temp = uninitialized_fixture();
    let init = run(&temp, &["init", "--json"], true);
    assert_eq!(init["agent_team_bootstrap"]["status"], "ready");
    temp
}
pub fn uninitialized_fixture() -> TempDir {
    let temp = TempDir::new().unwrap();
    let bin = temp.path().join("bin");
    fs::create_dir(&bin).unwrap();
    fs::create_dir(temp.path().join("native-home")).unwrap();
    let root = temp.path().canonicalize().unwrap();
    let script=r#"#!/bin/sh
while IFS= read -r req; do
  printf '%s\n' "$req" >> '@ROOT@/rpc.log'
  id=$(printf '%s' "$req" | sed -E 's/.*"id":([0-9]+).*/\1/')
  case "$req" in
    *'"method":"initialized"'*) continue;;
    *'"method":"initialize"'*) result='{}';;
    *'"method":"project/list"'*)
      result='{"data":[],"nextCursor":null}'
      if test -f '@ROOT@/project.name'; then
        read -r pname < '@ROOT@/project.name'
        result="{\"data\":[{\"id\":\"fixture-project\",\"name\":\"$pname\",\"roots\":[{\"path\":\"@ROOT@\"}]}],\"nextCursor\":null}"
      fi;;
    *'"method":"project/create"'*|*'"method":"project/update"'*)
      printf '%s' "$req" | sed -E 's/.*"name":"([^"]+)".*/\1/' > '@ROOT@/project.name'
      result='{"project":{"id":"fixture-project"}}';;
    *'"method":"project/read"'*)
      read -r pname < '@ROOT@/project.name'
      result="{\"project\":{\"id\":\"fixture-project\",\"name\":\"$pname\",\"roots\":[{\"path\":\"@ROOT@\"}]}}";;
    *'"method":"thread/metadata/update"'*)
      sid=$(printf '%s' "$req" | sed -E 's/.*"threadId":"([^"]+)".*/\1/')
      printf '%s' "$req" | sed -E 's/.*"projectId":"([^"]+)".*/\1/' > "@ROOT@/$sid.project"
      result='{}';;
    *'"method":"thread/start"'*)
      if test -f '@ROOT@/hold-create'; then
        touch '@ROOT@/entered-create'; waits=0
        while test -f '@ROOT@/hold-create' && test "$waits" -lt 250; do
          sleep 0.02; waits=$((waits+1))
        done
      fi
      n=0; test ! -f '@ROOT@/count' || read -r n < '@ROOT@/count'
      n=$((n+1)); echo "$n" > '@ROOT@/count'; sid="fake-$n"
      result="{\"thread\":{\"id\":\"$sid\",\"cwd\":\"@ROOT@\"}}";;
    *'"method":"thread/name/set"'*)
      sid=$(printf '%s' "$req" | sed -E 's/.*"threadId":"([^"]+)".*/\1/')
      printf '%s' "$req" | sed -E 's/.*"name":"([^"]+)".*/\1/' > "@ROOT@/$sid.name"
      result='{}';;
    *'"method":"thread/read"'*)
      sid=$(printf '%s' "$req" | sed -E 's/.*"threadId":"([^"]+)".*/\1/')
      if test -f "@ROOT@/$sid.missing" || test -f "@ROOT@/$sid.denied"; then
        message="thread not loaded: $sid"
        test ! -f "@ROOT@/$sid.denied" || message='permission denied'
        printf '{"id":%s,"error":{"code":-32600,"message":"%s"}}\n' "$id" "$message"
        continue
      fi
      if test -f "@ROOT@/$sid.wrong-workspace"; then
        printf '{"id":%s,"result":{"thread":{"id":"%s","cwd":"/different"}}}\n' "$id" "$sid"
        continue
      fi
      if test -f '@ROOT@/health-mode'; then
        read -r mode < '@ROOT@/health-mode'
        case "$mode" in
          missing|denied)
            message="thread not loaded: $sid"; test "$mode" != denied || message='permission denied'
            printf '{"id":%s,"error":{"code":-32600,"message":"%s"}}\n' "$id" "$message"
            continue;;
          archived)
            printf '{"id":%s,"result":{"thread":{"id":"%s","cwd":"@ROOT@","archived":true}}}\n' "$id" "$sid"
            continue;;
          mismatch)
            printf '{"id":%s,"result":{"thread":{"id":"%s","cwd":"/different"}}}\n' "$id" "$sid"
            continue;;
        esac
      fi
      if test -f '@ROOT@/bound-lost' && test "$sid" = fake-4; then
        printf '{"id":%s,"error":{"code":-32600,"message":"thread not loaded: fake-4"}}\n' "$id"
        continue
      fi
      if test "$sid" = lost; then
        printf '{"id":%s,"error":{"code":-32600,"message":"thread not loaded: lost"}}\n' "$id"
        continue
      fi
      project=''; test ! -f "@ROOT@/$sid.project" || read -r project < "@ROOT@/$sid.project"
      name=''; test ! -f "@ROOT@/$sid.name" || read -r name < "@ROOT@/$sid.name"
      status=idle; turns='[]'
      if test -f '@ROOT@/active-role'; then status=active; turns='[{"id":"active-turn","status":"inProgress"}]'; fi
      test ! -f "@ROOT@/$sid.system-error" || status=systemError
      test ! -f "@ROOT@/$sid.not-loaded" || status=notLoaded
      result="{\"thread\":{\"id\":\"$sid\",\"cwd\":\"@ROOT@\",\"name\":\"$name\",\"projectId\":\"$project\",\"status\":{\"type\":\"$status\"},\"turns\":$turns}}";;
    *'"method":"turn/interrupt"'*) rm '@ROOT@/active-role'; result='{}';;
    *'"method":"thread/resume"'*)
      if test -f '@ROOT@/writer-busy'; then
        printf '{"id":%s,"error":{"code":-32600,"message":"active writer"}}\n' "$id"
        continue
      fi

      sid=$(printf '%s' "$req" | sed -E 's/.*"threadId":"([^"]+)".*/\1/')
      test -f "@ROOT@/$sid.persisted" || exit 74
      test ! -f '@ROOT@/bad-resume' || sid=wrong-id
      result="{\"thread\":{\"id\":\"$sid\",\"cwd\":\"@ROOT@\",\"status\":{\"type\":\"idle\"}}}";;
    *'"method":"thread/list"'*)
      result='{"data":[],"nextCursor":null}'
      case "$req" in *'"archived":true'*)
        rows=''; sep=''
        for marker in '@ROOT@/'*.archived; do
          test -f "$marker" || continue
          archived_sid=${marker##*/}; archived_sid=${archived_sid%.archived}
          rows="$rows$sep{\"id\":\"$archived_sid\"}"; sep=,
        done
        result="{\"data\":[$rows],\"nextCursor\":null}";;
      esac
      if test -f '@ROOT@/health-archived'; then result='{"data":[{"id":"fake-1"}],"nextCursor":null}'; fi
      if test -f '@ROOT@/health-listed'; then
        case "$req" in *'"archived":false'*) result='{"data":[{"id":"fake-1"}],"nextCursor":null}';; esac
      fi
      if test -f '@ROOT@/listed-candidate'; then result='{"data":[{"id":"lost"}],"nextCursor":null}'; fi
      if test -f '@ROOT@/archived-candidate'; then
        case "$req" in *'"archived":true'*)
          case "$req" in
            *'"cursor":"next"'*) result='{"data":[{"id":"lost"}],"nextCursor":null}';;
            *) result='{"data":[],"nextCursor":"next"}';;
          esac;;
        esac
      fi
      if test -f '@ROOT@/bad-cursor'; then result='{"data":[],"nextCursor":"repeat"}'; fi;;
    *'"method":"thread/inject_items"'*)
      sid=$(printf '%s' "$req" | sed -E 's/.*"threadId":"([^"]+)".*/\1/')
      touch "@ROOT@/$sid.persisted"
      if test -f '@ROOT@/fail-prepare'; then
        rm '@ROOT@/fail-prepare'
        printf '{"id":%s,"error":{"code":-32600,"message":"test prepare interruption"}}\n' "$id"
        continue
      fi
      result='{}';;
    *) echo "unexpected RPC" >&2; exit 73;;
  esac
  printf '{"id":%s,"result":%s}\n' "$id" "$result"
done
"#.replace("@ROOT@",root.to_str().unwrap());
    fs::write(bin.join("codex"), script).unwrap();
    fs::set_permissions(bin.join("codex"), fs::Permissions::from_mode(0o755)).unwrap();
    temp
}
