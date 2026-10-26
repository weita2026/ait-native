#![cfg(unix)]
#[path = "support/agent_teams_cli.rs"]
mod fixture;
use fixture::*;
use std::fs;

#[test]
fn not_loaded_bound_session_is_reused_without_reload_or_replacement() {
    let t = fixture();
    fs::write(t.path().join("fake-1.not-loaded"), "").unwrap();
    let before = fs::read_to_string(t.path().join("rpc.log")).unwrap();
    let ready = run(
        &t,
        &["team", "session", "ensure", "--agent", "0", "--json"],
        true,
    );
    assert_eq!(ready["action"], "reused");
    assert_eq!(ready["session_id"], "fake-1");
    assert_eq!(ready["host_status"], "notLoaded");
    let after = fs::read_to_string(t.path().join("rpc.log")).unwrap();
    let added = after.strip_prefix(&before).unwrap();
    for method in ["thread/start", "thread/resume", "thread/inject_items"] {
        assert!(
            !added.contains(&format!("\"method\":\"{method}\"")),
            "unexpected {method}"
        );
    }
}

#[test]
fn on_demand_role_assignment_and_reuse() {
    let t = fixture();
    run(
        &t,
        &["team", "session", "ensure", "--agent", "2", "--json"],
        true,
    );
    let first = run(
        &t,
        &[
            "team", "session", "ensure", "--agent", "3", "--role", "1", "--json",
        ],
        true,
    );
    let again = run(
        &t,
        &["team", "session", "ensure", "--agent", "3", "--json"],
        true,
    );
    assert_eq!(again["session_id"], first["session_id"]);
    assert_eq!(
        run(&t, &["team", "contacts", "--json"], true)["actual_member_count"],
        5
    );
    assert_eq!(
        fs::read_to_string(t.path().join("count")).unwrap().trim(),
        "3"
    );
}
#[test]
fn uncertain_prepare_preserves_candidate_without_duplicate_creation() {
    let t = fixture();
    fs::write(t.path().join("fail-prepare"), "").unwrap();
    run(
        &t,
        &[
            "team", "session", "ensure", "--agent", "3", "--role", "1", "--json",
        ],
        false,
    );
    let retry = run(
        &t,
        &[
            "team", "session", "ensure", "--agent", "3", "--role", "1", "--json",
        ],
        false,
    );
    assert!(retry["detail"].as_str().unwrap().contains("unknown"));
    assert_eq!(
        fs::read_to_string(t.path().join("count")).unwrap().trim(),
        "2"
    );
    assert_eq!(
        run(&t, &["team", "contacts", "--json"], true)["actual_member_count"],
        4
    );
}
#[test]
fn bound_recovery_preserves_valid_id_and_replaces_only_proven_missing() {
    let t = fixture();
    for index in ["2", "3", "7"] {
        run(
            &t,
            &[
                "team",
                "session",
                "ensure",
                "--agent",
                index,
                "--role",
                if index == "7" { "3" } else { "1" },
                "--json",
            ],
            true,
        );
    }
    let reused = run(
        &t,
        &["team", "session", "ensure", "--agent", "7", "--json"],
        true,
    );
    assert_eq!(reused["session_id"], "fake-4");
    fs::write(t.path().join("bound-lost"), "").unwrap();
    let recovered = run(
        &t,
        &["team", "session", "ensure", "--agent", "7", "--json"],
        true,
    );
    assert_eq!(recovered["session_id"], "fake-5");
    let again = run(
        &t,
        &["team", "session", "ensure", "--agent", "7", "--json"],
        true,
    );
    assert_eq!(again["session_id"], "fake-5");
    assert_eq!(
        fs::read_to_string(t.path().join("count")).unwrap().trim(),
        "5"
    );
}
#[test]
fn role_plan_sync_saves_revision_without_host_io_or_loaded_evidence_changes() {
    let t = fixture();
    for id in ["2", "3"] {
        run(
            &t,
            &[
                "team", "session", "ensure", "--agent", id, "--role", "1", "--json",
            ],
            true,
        );
    }
    let receipt = |id| {
        t.path().join(format!(
            ".ait/team-session-bootstrap/agent-{id}.role-loaded"
        ))
    };
    let before: Vec<_> = [2, 3].map(|id| fs::read(receipt(id)).unwrap()).into();
    let agents = fs::read(t.path().join(".ait/binary-db/agents.bin")).unwrap();
    let path = t.path().join("docs/roles/1_Developer.md");
    fs::write(
        &path,
        fs::read_to_string(&path).unwrap() + "\nUpdated rule.\n",
    )
    .unwrap();
    fs::write(t.path().join("writer-busy"), "").unwrap();
    fs::write(t.path().join("rpc.log"), "").unwrap();

    let sync = run(
        &t,
        &[
            "plan",
            "sync",
            "docs/roles/1_Developer.md",
            "--local",
            "--json",
        ],
        true,
    );
    assert_eq!(sync["status"], "ok");
    assert!(sync.get("role_reload").is_none());
    assert_eq!(fs::read_to_string(t.path().join("rpc.log")).unwrap(), "");
    for (i, id) in [2, 3].into_iter().enumerate() {
        assert_eq!(fs::read(receipt(id)).unwrap(), before[i]);
    }
    assert_eq!(
        fs::read(t.path().join(".ait/binary-db/agents.bin")).unwrap(),
        agents
    );
    let repeat = run(
        &t,
        &["plan", "sync", "docs/roles", "--local", "--json"],
        true,
    );
    assert_eq!(repeat["status"], "ok");
    assert!(repeat.get("role_reload").is_none());
    assert_eq!(fs::read_to_string(t.path().join("rpc.log")).unwrap(), "");

    // Only Newton's explicit ensure may attempt delivery; a busy host retains evidence.
    let busy = run(
        &t,
        &["team", "session", "ensure", "--agent", "2", "--json"],
        false,
    );
    assert_eq!(busy["status"], "pending");
    assert_eq!(fs::read(receipt(2)).unwrap(), before[0]);
    fs::remove_file(t.path().join("writer-busy")).unwrap();
    let refresh = run(
        &t,
        &["team", "session", "ensure", "--agent", "2", "--json"],
        true,
    );
    assert_eq!(refresh["action"], "reused");
    assert_eq!(refresh["session_id"], "fake-2");
    assert_ne!(fs::read(receipt(2)).unwrap(), before[0]);
    assert_eq!(fs::read(receipt(3)).unwrap(), before[1]);

    let directory = run(&t, &["team", "contacts", "--json"], true);
    let member = |id| {
        directory["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["agent_index"] == id)
            .unwrap()
    };
    assert_eq!(member(2)["role_reload_status"], "complete");
    assert_eq!(member(3)["role_reload_status"], "pending");
    assert_eq!(
        member(2)["loaded_role_revision_index"],
        member(2)["role_revision_index"]
    );
    assert_ne!(
        member(3)["loaded_role_revision_index"],
        member(3)["role_revision_index"]
    );
    assert_eq!(member(2)["session_id"], "fake-2");
    assert_eq!(member(3)["session_id"], "fake-3");
    fs::write(t.path().join("rpc.log"), "").unwrap();
    run(
        &t,
        &["team", "session", "ensure", "--agent", "2", "--json"],
        true,
    );
    assert!(!fs::read_to_string(t.path().join("rpc.log"))
        .unwrap()
        .contains("thread/inject_items"));
    assert_eq!(
        fs::read_to_string(t.path().join("count")).unwrap().trim(),
        "3"
    );
}

#[test]
fn active_session_plan_sync_does_not_interrupt_until_explicit_ensure() {
    let t = fixture();
    run(
        &t,
        &["team", "session", "ensure", "--agent", "2", "--json"],
        true,
    );
    let path = t.path().join("docs/roles/1_Developer.md");
    fs::write(
        &path,
        fs::read_to_string(&path).unwrap() + "\nUpdated active role.\n",
    )
    .unwrap();
    fs::write(t.path().join("rpc.log"), "").unwrap();
    fs::write(t.path().join("active-role"), "").unwrap();
    let sync = run(
        &t,
        &[
            "plan",
            "sync",
            "docs/roles/1_Developer.md",
            "--local",
            "--json",
        ],
        true,
    );
    assert_eq!(sync["status"], "ok");
    assert!(sync.get("role_reload").is_none());
    assert!(t.path().join("active-role").exists());
    assert_eq!(fs::read_to_string(t.path().join("rpc.log")).unwrap(), "");
    let ensured = run(
        &t,
        &["team", "session", "ensure", "--agent", "2", "--json"],
        true,
    );
    assert_eq!(ensured["action"], "reused");
    assert_eq!(ensured["session_id"], "fake-2");
    let log = fs::read_to_string(t.path().join("rpc.log")).unwrap();
    assert!(log.find("turn/interrupt").unwrap() < log.find("thread/inject_items").unwrap());
    assert_eq!(
        fs::read_to_string(t.path().join("count")).unwrap().trim(),
        "2"
    );
}

#[test]
fn disabled_role_plan_sync_keeps_solo_route_and_never_calls_host() {
    let t = fixture();
    run(
        &t,
        &["config", "set", "--agent-teams", "disabled", "--json"],
        true,
    );
    let agents = fs::read(t.path().join(".ait/binary-db/agents.bin")).unwrap();
    let file = t.path().join("docs/roles/1_Developer.md");
    fs::write(
        &file,
        fs::read_to_string(&file).unwrap() + "\nSolo document update.\n",
    )
    .unwrap();
    fs::write(t.path().join("rpc.log"), "").unwrap();
    let sync = run(
        &t,
        &["plan", "sync", "docs/roles", "--local", "--json"],
        true,
    );
    assert_eq!(sync["status"], "ok");
    assert!(sync.get("role_reload").is_none());
    assert_eq!(fs::read_to_string(t.path().join("rpc.log")).unwrap(), "");
    assert_eq!(
        fs::read(t.path().join(".ait/binary-db/agents.bin")).unwrap(),
        agents
    );
    assert_eq!(
        run(&t, &["team", "contacts", "--json"], true)["team_enabled"],
        false
    );
    assert!(fs::read_to_string(t.path().join("AGENTS.md"))
        .unwrap()
        .contains("scope=`local`"));
}

#[test]
fn contacts_rejects_loaded_evidence_that_changes_during_host_inspection() {
    let t = fixture();
    run(
        &t,
        &["team", "session", "ensure", "--agent", "2", "--json"],
        true,
    );
    let codex = t.path().join("bin/codex");
    let receipt = t
        .path()
        .join(".ait/team-session-bootstrap/agent-2.role-loaded");
    let marker = t.path().join("change-proof");
    let branch = "*'\"method\":\"thread/read\"'*)";
    let mutation = format!(
        "{branch}\n      if test -f '{}'; then\n        sed 's/\"phase\":\"complete\"/\"phase\":\"loaded\"/' '{}' > '{}.tmp'\n        mv '{}.tmp' '{}'\n        rm '{}'\n      fi",
        marker.display(), receipt.display(), receipt.display(),
        receipt.display(), receipt.display(), marker.display()
    );
    let source = fs::read_to_string(&codex).unwrap();
    assert!(source.contains(branch));
    fs::write(&codex, source.replacen(branch, &mutation, 1)).unwrap();
    fs::write(marker, "").unwrap();
    let changed = run(&t, &["team", "contacts", "--json"], false);
    assert_eq!(changed["status"], "blocked");
    assert!(changed["detail"]
        .as_str()
        .unwrap()
        .contains("Directory changed"));
    let fresh = run(&t, &["team", "contacts", "--json"], true);
    let member = fresh["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["agent_index"] == 2)
        .unwrap();
    assert_eq!(member["role_reload_status"], "pending");
    assert!(member["loaded_role_revision_index"].is_null());
    assert_eq!(member["session_id"], "fake-2");
}

#[test]
fn known_missing_pending_candidate_recovers_automatically_but_inventory_ambiguity_blocks() {
    let t = fixture();
    let op = t.path().join(".ait/team-session-bootstrap");
    fs::copy(op.join("agent-0.host"), op.join("agent-3.host")).unwrap();
    fs::write(op.join("agent-3.pending"), "lost").unwrap();
    let before = fs::read(t.path().join(".ait/binary-db/agents.bin")).unwrap();
    let args = [
        "team", "session", "ensure", "--agent", "3", "--role", "1", "--json",
    ];
    for marker in ["listed-candidate", "archived-candidate", "bad-cursor"] {
        fs::write(t.path().join(marker), "").unwrap();
        run(&t, &args, false);
        assert_eq!(
            fs::read_to_string(op.join("agent-3.pending")).unwrap(),
            "lost"
        );
        assert_eq!(
            fs::read(t.path().join(".ait/binary-db/agents.bin")).unwrap(),
            before
        );
        assert_eq!(
            fs::read_to_string(t.path().join("count")).unwrap().trim(),
            "1"
        );
        fs::remove_file(t.path().join(marker)).unwrap();
    }
    let recovered = run(&t, &args, true);
    assert_eq!(recovered["action"], "recovered");
    assert_eq!(recovered["session_id"], "fake-2");
    assert_eq!(run(&t, &args, true)["session_id"], "fake-2");
    assert_eq!(
        fs::read_to_string(t.path().join("count")).unwrap().trim(),
        "2"
    );
    assert_eq!(
        fs::read_to_string(t.path().join("rpc.log"))
            .unwrap()
            .matches("\"method\":\"thread/inject_items\"")
            .count(),
        2
    );
}

#[test]
fn unavailable_archived_mismatched_or_still_listed_bound_session_is_not_replaced() {
    let t = fixture();
    let before = fs::read(t.path().join(".ait/binary-db/agents.bin")).unwrap();
    let args = ["team", "session", "ensure", "--agent", "0", "--json"];
    for mode in ["denied", "archived", "mismatch"] {
        fs::write(t.path().join("health-mode"), mode).unwrap();
        run(&t, &args, false);
        assert_eq!(
            fs::read(t.path().join(".ait/binary-db/agents.bin")).unwrap(),
            before
        );
    }
    fs::write(t.path().join("health-mode"), "missing").unwrap();
    for marker in ["health-listed", "health-archived", "bad-cursor"] {
        fs::write(t.path().join(marker), "").unwrap();
        run(&t, &args, false);
        fs::remove_file(t.path().join(marker)).unwrap();
    }
    assert_eq!(
        fs::read(t.path().join(".ait/binary-db/agents.bin")).unwrap(),
        before
    );
    assert_eq!(
        fs::read_to_string(t.path().join("count")).unwrap().trim(),
        "1"
    );
}

fn concurrent_ensure_retries_same_identity(t: &tempfile::TempDir, args: &[&str], expected: &str) {
    use std::{
        process::Stdio,
        time::{Duration, Instant},
    };
    fs::write(t.path().join("hold-create"), "").unwrap();
    let child = command(t)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !t.path().join("entered-create").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let entered = t.path().join("entered-create").exists();
    let second = entered.then(|| command(t).args(args).output().unwrap());
    fs::remove_file(t.path().join("hold-create")).unwrap();
    let first = child.wait_with_output().unwrap();
    assert!(entered, "first ensure never reached creation");
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first = ait_core::json_support::JsonCodec::parse_slice_with_error_prefix(
        &first.stdout,
        "first ensure",
    )
    .unwrap();
    assert_eq!(first["session_id"], expected);
    let second = second.unwrap();
    assert!(
        !second.status.success(),
        "concurrent ensure unexpectedly acquired preparation"
    );
    let second = ait_core::json_support::JsonCodec::parse_slice_with_error_prefix(
        &second.stdout,
        "second ensure",
    )
    .unwrap();
    assert_eq!(second["status"], "pending");
    assert_eq!(run(t, args, true)["session_id"], expected);
}

#[test]
fn concurrent_creation_then_retry_does_not_create_duplicate() {
    let t = fixture();
    concurrent_ensure_retries_same_identity(
        &t,
        &[
            "team", "session", "ensure", "--agent", "3", "--role", "1", "--json",
        ],
        "fake-2",
    );
    assert_eq!(
        fs::read_to_string(t.path().join("count")).unwrap().trim(),
        "2"
    );
}

#[test]
fn concurrent_missing_recovery_then_retry_does_not_create_duplicate() {
    let t = fixture();
    for index in ["2", "3", "7"] {
        run(
            &t,
            &[
                "team",
                "session",
                "ensure",
                "--agent",
                index,
                "--role",
                if index == "7" { "3" } else { "1" },
                "--json",
            ],
            true,
        );
    }
    fs::write(t.path().join("bound-lost"), "").unwrap();
    concurrent_ensure_retries_same_identity(
        &t,
        &["team", "session", "ensure", "--agent", "7", "--json"],
        "fake-5",
    );
    assert_eq!(
        fs::read_to_string(t.path().join("count")).unwrap().trim(),
        "5"
    );
}

#[test]
fn removed_recovery_options_are_rejected_without_mutation() {
    let t = fixture();
    let before = fs::read(t.path().join(".ait/binary-db/agents.bin")).unwrap();
    for option in ["--recover-session", "--recover-pending"] {
        let output = command(&t)
            .args([
                "team", "session", "ensure", "--agent", "0", option, "fake-1", "--json",
            ])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("unexpected argument"));
    }
    assert_eq!(
        fs::read(t.path().join(".ait/binary-db/agents.bin")).unwrap(),
        before
    );
    assert_eq!(
        fs::read_to_string(t.path().join("count")).unwrap().trim(),
        "1"
    );
}

#[test]
fn disabled_ensure_preserves_solo_and_does_not_contact_host() {
    let t = fixture();
    run(
        &t,
        &["config", "set", "--agent-teams", "disabled", "--json"],
        true,
    );
    let before = fs::read(t.path().join(".ait/binary-db/agents.bin")).unwrap();
    fs::write(t.path().join("rpc.log"), "").unwrap();
    assert_eq!(
        run(
            &t,
            &["team", "session", "ensure", "--agent", "0", "--json"],
            false
        )["status"],
        "pending"
    );
    assert_eq!(fs::read_to_string(t.path().join("rpc.log")).unwrap(), "");
    assert_eq!(
        fs::read(t.path().join(".ait/binary-db/agents.bin")).unwrap(),
        before
    );
    assert!(fs::read_to_string(t.path().join("AGENTS.md"))
        .unwrap()
        .contains("scope=`local`"));
}

#[test]
fn unavailable_host_preserves_binding_without_new_pending_candidate() {
    let t = fixture();
    let before = fs::read(t.path().join(".ait/binary-db/agents.bin")).unwrap();
    fs::rename(
        t.path().join("bin/codex"),
        t.path().join("bin/codex-unavailable"),
    )
    .unwrap();
    let result = run(
        &t,
        &["team", "session", "ensure", "--agent", "0", "--json"],
        false,
    );
    assert_eq!(result["status"], "pending");
    assert_eq!(
        fs::read(t.path().join(".ait/binary-db/agents.bin")).unwrap(),
        before
    );
    assert_eq!(
        fs::read_to_string(t.path().join("count")).unwrap().trim(),
        "1"
    );
    assert!(!t
        .path()
        .join(".ait/team-session-bootstrap/agent-0.pending")
        .exists());
}
