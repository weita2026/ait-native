#![cfg(unix)]
#[path = "support/agent_teams_cli.rs"]
mod fixture;
#[path = "support/legacy_pm.rs"]
mod legacy_pm;
use ait_cli::init_surface::{init_repo, InitRequest};
use ait_core::json_support::{JsonCodec, JsonValue};
use std::fs;
use tempfile::TempDir;

fn fixture() -> TempDir {
    named_fixture(None)
}
fn named_fixture(name: Option<&str>) -> TempDir {
    let temp = fixture::uninitialized_fixture();
    init_repo(&InitRequest {
        root: temp.path().to_path_buf(),
        name: name.map(str::to_owned),
        default_line: "main".into(),
        policy_profile: "prototype".into(),
        default_author_mode: "ai_with_human_review".into(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    fs::write(temp.path().join("active-role"), "").unwrap();
    fs::write(temp.path().join("writer-busy"), "").unwrap();
    temp
}
fn caller(temp: &TempDir, id: &str, args: &[&str], success: bool) -> JsonValue {
    let output = fixture::command(temp)
        .env("CODEX_THREAD_ID", id)
        .env("CODEX_SESSION_ID", id)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(
        output.status.success(),
        success,
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    JsonCodec::parse_slice_with_error_prefix(&output.stdout, "caller CLI").unwrap()
}
fn enable(temp: &TempDir, id: &str, success: bool) -> JsonValue {
    caller(
        temp,
        id,
        &["config", "set", "--agent-teams", "enabled", "--json"],
        success,
    )["team_bootstrap"]
        .clone()
}
fn ensure_owner(temp: &TempDir, id: &str, success: bool) -> JsonValue {
    caller(
        temp,
        id,
        &["team", "session", "ensure", "--agent", "0", "--json"],
        success,
    )
}

fn acknowledge(temp: &TempDir, offer: &JsonValue, success: bool) -> JsonValue {
    acknowledge_as(temp, "caller", offer, success)
}
fn acknowledge_as(temp: &TempDir, id: &str, offer: &JsonValue, success: bool) -> JsonValue {
    caller(
        temp,
        id,
        &[
            "team",
            "session",
            "acknowledge",
            "--activation",
            offer["activation_token"].as_str().unwrap(),
            "--json",
        ],
        success,
    )
}
fn title_action(temp: &TempDir, offer: &JsonValue) {
    assert_eq!(
        offer["app_actions"][0]["tool"],
        "mcp__codex_app__set_thread_title"
    );
    fs::write(
        temp.path()
            .join(format!("{}.name", offer["session_id"].as_str().unwrap())),
        offer["title"].as_str().unwrap(),
    )
    .unwrap();
}

#[test]
fn init_and_enable_preserve_unresolved_activation_without_host_access_or_implicit_ack() {
    let temp = fixture();
    let offer = enable(&temp, "caller", true);
    let journal = temp
        .path()
        .join(".ait/team-session-bootstrap/agent-0.activation");
    let original = fs::read(&journal).unwrap();
    let rpc = fs::read(temp.path().join("rpc.log")).unwrap();
    for phase in ["offered", "acknowledged"] {
        let mut saved = JsonCodec::parse_slice_with_error_prefix(&original, "activation").unwrap();
        saved["phase"] = phase.into();
        fs::write(
            &journal,
            JsonCodec::encode_value_to_vec(
                &saved,
                ait_core::json_support::JsonEncodeOptions::compact(),
            )
            .unwrap(),
        )
        .unwrap();
        let before = fs::read(&journal).unwrap();
        for id in ["caller", "foreign"] {
            for args in [
                vec!["init", "--json"],
                vec!["config", "set", "--agent-teams", "enabled", "--json"],
            ] {
                let result = caller(&temp, id, &args, true);
                let team = if args[0] == "init" {
                    &result["agent_team_bootstrap"]
                } else {
                    &result["team_bootstrap"]
                };
                assert_eq!(team["status"], "preserved");
                assert_eq!(team["session_readiness"], "pending");
                assert!(team["role_instructions"].is_null());
                assert_eq!(fs::read(&journal).unwrap(), before);
                assert_eq!(fs::read(temp.path().join("rpc.log")).unwrap(), rpc);
            }
        }
        assert!(!temp
            .path()
            .join(".ait/team-session-bootstrap/agent-0.role-loaded")
            .exists());
    }
    fs::write(&journal, original).unwrap();
    title_action(&temp, &offer);
    assert_eq!(acknowledge(&temp, &offer, true)["status"], "ready");
}

#[test]
fn existing_init_and_enable_preserve_archived_or_missing_owners() {
    for state in ["archived", "missing"] {
        let temp = fixture();
        let offer = enable(&temp, "caller", true);
        title_action(&temp, &offer);
        acknowledge(&temp, &offer, true);
        fs::write(temp.path().join(format!("caller.{state}")), "").unwrap();
        let journal = temp
            .path()
            .join(".ait/team-session-bootstrap/agent-0.activation");
        let before = fs::read(&journal).unwrap();
        let rpc = fs::read(temp.path().join("rpc.log")).unwrap();
        for args in [
            vec!["init", "--json"],
            vec!["config", "set", "--agent-teams", "enabled", "--json"],
        ] {
            let output = caller(&temp, "replacement", &args, true);
            let team = if args[0] == "init" {
                &output["agent_team_bootstrap"]
            } else {
                &output["team_bootstrap"]
            };
            assert_eq!(team["status"], "preserved");
            assert_eq!(team["session_id"], "caller");
            assert!(team["activation_token"].is_null());
            assert_eq!(fs::read(&journal).unwrap(), before);
            assert_eq!(fs::read(temp.path().join("rpc.log")).unwrap(), rpc);
        }
        assert!(!temp.path().join("count").exists());
    }
}

#[test]
fn existing_enable_preserves_uncertain_or_mismatched_owners_without_host_inspection() {
    for failure in ["denied", "system-error", "wrong-workspace", "bad-cursor"] {
        let temp = fixture();
        let initial = enable(&temp, "caller", true);
        title_action(&temp, &initial);
        acknowledge(&temp, &initial, true);
        let journal = temp
            .path()
            .join(".ait/team-session-bootstrap/agent-0.activation");
        let previous = fs::read(&journal).unwrap();
        let marker = if failure == "bad-cursor" {
            "bad-cursor".into()
        } else {
            format!("caller.{failure}")
        };
        fs::write(temp.path().join(marker), "").unwrap();
        if failure == "bad-cursor" {
            fs::write(temp.path().join("caller.missing"), "").unwrap();
        }
        if failure == "wrong-workspace" {
            fs::write(temp.path().join("caller.archived"), "").unwrap();
        }
        let result = enable(&temp, "replacement", true);
        assert_eq!(result["status"], "preserved");
        assert_eq!(result["session_id"], "caller");
        assert_eq!(fs::read(&journal).unwrap(), previous);
        assert!(!temp.path().join("replacement.project").exists());
        assert!(!temp.path().join("count").exists());
    }
}

#[test]
fn pending_initial_offer_refresh_requires_explicit_ensure_from_its_original_caller() {
    let temp = fixture();
    let offer = enable(&temp, "caller", true);
    let path = temp.path().join("docs/roles/0_Project Manager.md");
    fs::write(
        &path,
        fs::read_to_string(&path).unwrap() + "\nUpdated initial responsibility.\n",
    )
    .unwrap();
    let journal = temp
        .path()
        .join(".ait/team-session-bootstrap/agent-0.activation");
    let before = fs::read(&journal).unwrap();
    assert_eq!(enable(&temp, "caller", true)["status"], "preserved");
    assert_eq!(fs::read(&journal).unwrap(), before);
    assert_eq!(ensure_owner(&temp, "foreign", false)["status"], "pending");
    assert_eq!(fs::read(&journal).unwrap(), before);
    assert!(acknowledge(&temp, &offer, false)["detail"]
        .as_str()
        .unwrap()
        .contains("Role changed"));
    let refreshed = ensure_owner(&temp, "caller", true);
    assert_eq!(refreshed["status"], "activation_required");
    assert_ne!(refreshed["activation_token"], offer["activation_token"]);
    assert!(refreshed["role_instructions"]
        .as_str()
        .unwrap()
        .contains("Updated initial responsibility."));
    title_action(&temp, &refreshed);
    assert_eq!(acknowledge(&temp, &refreshed, true)["status"], "ready");
    let rpc = fs::read_to_string(temp.path().join("rpc.log")).unwrap();
    for method in [
        "thread/start",
        "thread/resume",
        "thread/inject_items",
        "turn/interrupt",
    ] {
        assert!(!rpc.contains(&format!("\"method\":\"{method}\"")));
    }
}

#[test]
fn enable_acknowledgment_and_changed_revision_use_the_busy_active_caller() {
    let temp = fixture();
    let history = b"preexisting caller history";
    fs::write(temp.path().join("caller.persisted"), history).unwrap();
    let offer = enable(&temp, "caller", true);
    assert_eq!(offer["status"], "activation_required");
    assert_eq!(offer["session_id"], "caller");
    assert_eq!(offer["work_started"], false);
    assert!(offer["role_instructions"]
        .as_str()
        .unwrap()
        .contains("Role MD:"));
    let revision = offer["role_revision_index"].clone();
    assert_eq!(acknowledge(&temp, &offer, false)["status"], "pending");
    title_action(&temp, &offer);
    assert_eq!(
        acknowledge(&temp, &offer, true)["delivery_method"],
        "caller_acknowledged"
    );
    assert_eq!(
        fs::read(temp.path().join("caller.persisted")).unwrap(),
        history
    );
    let repeated = enable(&temp, "caller", true);
    assert_eq!(repeated["status"], "preserved");
    assert!(repeated["role_instructions"].is_null());
    let existing = enable(&temp, "different-caller", true);
    assert_eq!(existing["action"], "bindings_preserved");
    assert_eq!(existing["session_id"], "caller");
    let role = temp.path().join("docs/roles/0_Project Manager.md");
    fs::write(
        &role,
        fs::read_to_string(&role).unwrap() + "\nUpdated caller responsibility.\n",
    )
    .unwrap();
    fixture::run(
        &temp,
        &[
            "plan",
            "sync",
            "docs/roles/0_Project Manager.md",
            "--local",
            "--json",
        ],
        true,
    );
    assert_eq!(acknowledge(&temp, &offer, false)["status"], "pending");
    let rpc_before = fs::read(temp.path().join("rpc.log")).unwrap();
    assert_eq!(enable(&temp, "caller", true)["status"], "preserved");
    assert_eq!(fs::read(temp.path().join("rpc.log")).unwrap(), rpc_before);
    assert_eq!(ensure_owner(&temp, "caller", false)["status"], "pending");
    fs::remove_file(temp.path().join("writer-busy")).unwrap();
    let changed = ensure_owner(&temp, "caller", true);
    assert_ne!(changed["role_revision_index"], revision);
    assert_eq!(changed["status"], "ready");
    let contacts = fixture::run(&temp, &["team", "contacts", "--json"], true);
    assert_eq!(contacts["bound_session_count"], 1);
    assert_eq!(
        contacts["members"][0]["loaded_role_revision_index"],
        changed["role_revision_index"]
    );
    assert!(
        !temp.path().join("count").exists(),
        "created a blank session"
    );
    let rpc = fs::read_to_string(temp.path().join("rpc.log")).unwrap();
    assert!(rpc.contains("\"method\":\"thread/resume\""));
    assert!(rpc.contains("\"method\":\"thread/inject_items\""));
}

#[test]
fn incomplete_offer_requires_same_caller_and_native_metadata_before_closeout() {
    let temp = fixture();
    let offer = enable(&temp, "caller", true);
    let retry = enable(&temp, "caller", true);
    assert_eq!(retry["status"], "preserved");
    assert!(retry["role_instructions"].is_null());
    assert_eq!(enable(&temp, "foreign", true)["status"], "preserved");
    let missing = fixture::run(
        &temp,
        &["config", "set", "--agent-teams", "enabled", "--json"],
        true,
    );
    assert_eq!(missing["team_bootstrap"]["session_id"], "caller");
    fixture::run(
        &temp,
        &["team", "session", "ensure", "--agent", "0", "--json"],
        false,
    );
    title_action(&temp, &offer);
    fs::write(temp.path().join("caller.project"), "wrong-project").unwrap();
    assert_eq!(acknowledge(&temp, &offer, false)["status"], "pending");
    ensure_owner(&temp, "caller", true); // explicit recovery reconciles the project
    assert_eq!(acknowledge(&temp, &offer, true)["status"], "ready");
}

#[test]
fn wrong_workspace_archived_unknown_and_conflicting_native_ids_never_adopt() {
    for mode in ["mismatch", "archived", "missing", "denied"] {
        let temp = fixture();
        fs::write(temp.path().join("health-mode"), mode).unwrap();
        assert_eq!(enable(&temp, "caller", false)["status"], "pending");
        assert!(!temp.path().join("count").exists());
    }
    let temp = fixture();
    let output = fixture::command(&temp)
        .env("CODEX_THREAD_ID", "caller")
        .env("CODEX_SESSION_ID", "foreign")
        .args(["config", "set", "--agent-teams", "enabled", "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("identities disagree"));
    assert!(!temp.path().join("count").exists());
}

fn sync_role(temp: &TempDir) -> JsonValue {
    fixture::run(
        temp,
        &[
            "plan",
            "sync",
            "docs/roles/0_Project Manager.md",
            "--local",
            "--json",
        ],
        true,
    )
}

#[test]
fn two_repository_entrypoints_refresh_defaults_and_local_drift_without_crossing_bindings() {
    let first = fixture();
    let path = first.path().join("docs/roles/0_Project Manager.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, legacy_pm::BODY).unwrap();
    let old = sync_role(&first);
    let offer = enable(&first, "caller", true);
    assert_eq!(offer["synchronization"]["contacts"]["added_count"], 50);
    assert_eq!(
        offer["synchronization"]["roles"]["roles"][0]["default_state"],
        "updated_default"
    );
    assert!(!fs::read_to_string(&path)
        .unwrap()
        .contains("You are Newton, the team's sole coordination entrance"));
    let plans = fixture::run(&first, &["plan", "list", "--local", "--json"], true);
    let role = plans
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["head_artifact_path"] == "docs/roles/0_Project Manager.md")
        .unwrap();
    assert_eq!(role["plan_id"], old["results"][0]["plan_id"]);
    title_action(&first, &offer);
    acknowledge(&first, &offer, true);
    fixture::run(
        &first,
        &["config", "set", "--agent-teams", "disabled", "--json"],
        true,
    );
    fs::write(
        &path,
        fs::read_to_string(&path).unwrap() + "\nRepository-specific responsibility.\n",
    )
    .unwrap();
    assert_eq!(enable(&first, "caller", true)["status"], "preserved");
    fs::write(first.path().join("caller.persisted"), "existing caller").unwrap();
    fs::remove_file(first.path().join("active-role")).unwrap();
    fs::remove_file(first.path().join("writer-busy")).unwrap();
    let refreshed = ensure_owner(&first, "caller", true);
    assert_ne!(
        refreshed["role_revision_index"],
        offer["role_revision_index"]
    );
    let repeated =
        caller(&first, "caller", &["init", "--json"], true)["agent_team_bootstrap"].clone();
    assert_eq!(repeated["action"], "bindings_preserved");
    assert_eq!(repeated["synchronization"]["contacts"]["added_count"], 0);
    assert!(repeated["role_instructions"].is_null());
    let second = fixture::uninitialized_fixture();
    fs::write(second.path().join("active-role"), "").unwrap();
    fs::write(second.path().join("writer-busy"), "").unwrap();
    let fresh =
        caller(&second, "second-caller", &["init", "--json"], true)["agent_team_bootstrap"].clone();
    assert_eq!(fresh["session_id"], "second-caller");
    assert_ne!(fresh["title"], offer["title"]);
    assert!(fresh["title"]
        .as_str()
        .unwrap()
        .ends_with(second.path().file_name().unwrap().to_str().unwrap()));
    fs::write(
        second.path().join("second-caller.name"),
        fresh["title"].as_str().unwrap(),
    )
    .unwrap();
    caller(
        &second,
        "second-caller",
        &[
            "team",
            "session",
            "acknowledge",
            "--activation",
            fresh["activation_token"].as_str().unwrap(),
            "--json",
        ],
        true,
    );
    let contacts = fixture::run(&second, &["team", "contacts", "--json"], true);
    assert_eq!(contacts["members"][0]["session_id"], "second-caller");
    assert_eq!(contacts["bound_session_count"], 1);
    let first_contacts = fixture::run(&first, &["team", "contacts", "--json"], true);
    assert_eq!(first_contacts["members"][0]["session_id"], "caller");
    assert_eq!(
        first_contacts["members"][0]["loaded_role_revision_index"],
        refreshed["role_revision_index"]
    );
    assert!(!first.path().join("count").exists());
    assert!(!second.path().join("count").exists());
}

#[test]
fn reviewer_default_refresh_preserves_custom_text_without_false_delivery() {
    let relative = "docs/roles/2_Reviewer.md";
    // Exact preceding Reviewer default from SNP-EB3214AAA5F3.
    let previous = include_str!("support/pre_result_delivery_reviewer.txt");
    let fresh = fixture::uninitialized_fixture();
    fs::write(fresh.path().join("active-role"), "").unwrap();
    fs::write(fresh.path().join("writer-busy"), "").unwrap();
    caller(&fresh, "fresh-caller", &["init", "--json"], true);
    let current = fs::read_to_string(fresh.path().join(relative)).unwrap();

    for custom in ["", "\nRepository-specific Reviewer responsibility.\n"] {
        let temp = fixture();
        assert!(!temp
            .path()
            .join(".ait/team-session-bootstrap/role-defaults")
            .exists());
        let path = temp.path().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, previous).unwrap();
        let old = fixture::run(
            &temp,
            &["plan", "sync", relative, "--local", "--json"],
            true,
        );
        fs::write(&path, format!("{previous}{custom}")).unwrap();
        let result = enable(&temp, "caller", true);
        assert_eq!(
            result["synchronization"]["roles"]["roles"][2]["default_state"],
            "updated_default"
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("{current}{custom}")
        );
        let plans = fixture::run(&temp, &["plan", "list", "--local", "--json"], true);
        let role = plans
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["head_artifact_path"] == relative)
            .unwrap();
        assert_eq!(role["plan_id"], old["results"][0]["plan_id"]);
        assert_eq!(role["head_match"], "yes");
        let contacts = fixture::run(&temp, &["team", "contacts", "--json"], true);
        assert_eq!(contacts["bound_session_count"], 0);
        assert!(contacts["members"][0]["loaded_role_revision_index"].is_null());
        let reviewers = contacts["members"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|member| member["role_index"] == 2)
            .collect::<Vec<_>>();
        assert!(!reviewers.is_empty());
        assert!(reviewers
            .iter()
            .all(|member| member["loaded_role_revision_index"].is_null()));
        assert!(!temp.path().join("count").exists());
    }
}

#[test]
fn customized_upgrade_conflict_preserves_and_syncs_local_role_without_false_delivery() {
    let temp = fixture();
    let path = temp.path().join("docs/roles/0_Project Manager.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, legacy_pm::BODY).unwrap();
    sync_role(&temp);
    let custom = legacy_pm::BODY.replace(
        "You are Newton, the team's sole coordination entrance.",
        "This repository assigns its coordinator differently.",
    );
    fs::write(&path, &custom).unwrap();
    let result = enable(&temp, "caller", false);
    assert_eq!(result["synchronization"]["status"], "pending");
    assert_eq!(
        result["synchronization"]["roles"]["conflicts"][0]["reason"],
        "customized_default_upgrade_conflict"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), custom);
    assert!(result["role_instructions"].is_null());
    assert!(!temp.path().join("count").exists());
    let plans = fixture::run(&temp, &["plan", "list", "--local", "--json"], true);
    let role = plans
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["head_artifact_path"] == "docs/roles/0_Project Manager.md")
        .unwrap();
    assert_eq!(
        role["head_match"], "yes",
        "custom local Markdown was not synced"
    );
    assert_eq!(
        enable(&temp, "caller", false)["synchronization"]["status"],
        "pending"
    );
}

#[test]
fn acknowledgment_rejects_unsynced_drift_and_host_failure_preserves_sync_outcome() {
    let temp = fixture();
    let offer = enable(&temp, "caller", true);
    title_action(&temp, &offer);
    let path = temp.path().join("docs/roles/0_Project Manager.md");
    fs::write(
        &path,
        fs::read_to_string(&path).unwrap() + "\nNew unsynced caller responsibility.\n",
    )
    .unwrap();
    let stale = acknowledge(&temp, &offer, false);
    assert!(stale["detail"]
        .as_str()
        .unwrap()
        .contains("Role changed after delivery"));
    assert_eq!(enable(&temp, "caller", true)["status"], "preserved");
    let current = ensure_owner(&temp, "caller", true);
    assert!(current["role_instructions"]
        .as_str()
        .unwrap()
        .contains("New unsynced caller responsibility."));
    acknowledge(&temp, &current, true);
    let offline = fixture();
    fs::remove_file(offline.path().join("bin/codex")).unwrap();
    let result = enable(&offline, "caller", false);
    assert_eq!(result["synchronization"]["status"], "ready");
    assert_eq!(result["session_readiness"], "pending");
    assert_eq!(result["session_id"], "caller");
}

#[test]
fn repository_name_controls_init_config_acknowledgment_and_current_binding_reconciliation() {
    for entrypoint in ["config", "init"] {
        let temp = named_fixture(Some("logical-repository"));
        let expected = "［Newton］Project Manager·logical-repository";
        let folder = temp.path().file_name().unwrap().to_str().unwrap();
        assert_ne!(folder, "logical-repository");
        let offer = if entrypoint == "init" {
            caller(&temp, "caller", &["init", "--json"], true)["agent_team_bootstrap"].clone()
        } else {
            enable(&temp, "caller", true)
        };
        assert_eq!(offer["title"], expected);
        let project = offer["project_id"].clone();
        assert_eq!(
            fs::read_to_string(temp.path().join("project.name"))
                .unwrap()
                .trim(),
            folder
        );
        let incorrect = format!("［Newton］Project Manager·{folder}");
        fs::write(temp.path().join("caller.name"), &incorrect).unwrap();
        assert_eq!(acknowledge(&temp, &offer, false)["status"], "pending");
        title_action(&temp, &offer);
        let accepted = acknowledge(&temp, &offer, true);
        assert_eq!(accepted["title"], expected);
        assert_eq!(accepted["project_id"], project);
        assert_eq!(
            enable(&temp, "caller", true)["action"],
            "bindings_preserved"
        );
        fs::write(temp.path().join("caller.name"), incorrect).unwrap();
        assert_eq!(enable(&temp, "caller", true)["status"], "preserved");
        fs::write(temp.path().join("caller.persisted"), "existing caller").unwrap();
        fs::remove_file(temp.path().join("active-role")).unwrap();
        fs::remove_file(temp.path().join("writer-busy")).unwrap();
        let reconciliation = ensure_owner(&temp, "caller", true);
        assert_eq!(reconciliation["title"], expected);
        assert!(reconciliation["role_instructions"].is_null());
        assert_eq!(
            reconciliation["role_revision_index"],
            offer["role_revision_index"]
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("caller.project")).unwrap(),
            project.as_str().unwrap()
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("caller.name")).unwrap(),
            expected
        );
        assert_eq!(
            fixture::run(&temp, &["team", "contacts", "--json"], true)["bound_session_count"],
            1
        );
        assert!(!temp.path().join("count").exists());
    }
}
