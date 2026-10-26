use super::*;
use crate::init_surface::{init_repo, InitRequest};
use tempfile::TempDir;
#[derive(Default)]
struct Host {
    creates: usize,
    injects: usize,
    delivered: Vec<String>,
    fail_prepare: bool,
    fail_create_before_receipt: bool,
    fail_create_after_receipt: bool,
    reappear_on_prepare: Option<String>,
    on_prepare: Option<Box<dyn FnMut()>>,
    on_reuse: Option<Box<dyn FnMut()>>,
    ids: Vec<String>,
}
impl SessionHost for Host {
    fn set_title(&mut self, _: &str, _: &Path, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn inspect(&mut self, id: &str, _: &Path) -> Result<String, String> {
        if self.ids.iter().any(|v| v == id) {
            Ok("idle".into())
        } else {
            Err("missing".into())
        }
    }
    fn reuse(&mut self, id: &str, cwd: &Path, _: &str) -> Result<String, String> {
        let state = self.inspect(id, cwd)?;
        if let Some(mut action) = self.on_reuse.take() {
            action();
        }
        Ok(state)
    }
    fn recovery_scope(&self) -> Result<String, String> {
        Ok("fake-home".into())
    }
    fn verify_missing(&mut self, id: &str) -> Result<(), String> {
        if self.ids.iter().any(|v| v == id) {
            Err("valid session".into())
        } else {
            Ok(())
        }
    }
    fn acquire_idle(&mut self, id: &str, cwd: &Path) -> Result<(), String> {
        self.inspect(id, cwd).map(|_| ())
    }
    fn inject_role(&mut self, _: &str, instructions: &str) -> Result<(), String> {
        self.injects += 1;
        self.delivered.push(instructions.into());
        Ok(())
    }
    fn create(
        &mut self,
        _: &Path,
        _: &str,
        _: &str,
        receipt: &mut dyn FnMut(&str) -> Result<(), String>,
    ) -> Result<String, String> {
        self.creates += 1;
        let id = format!("fake-{}", self.creates);
        self.ids.push(id.clone());
        if self.fail_create_before_receipt {
            return Err("unknown creation reply".into());
        }
        receipt(&id)?;
        if self.fail_create_after_receipt {
            return Err("creation stopped after receipt".into());
        }
        Ok(id)
    }
    fn prepare_existing(
        &mut self,
        _: &str,
        _: &Path,
        _: &str,
        instructions: &str,
    ) -> Result<(), String> {
        self.injects += 1;
        self.delivered.push(instructions.into());
        if let Some(id) = self.reappear_on_prepare.take() {
            self.ids.push(id);
        }
        if let Some(mut action) = self.on_prepare.take() {
            action();
        }
        if self.fail_prepare {
            Err("unknown reply".into())
        } else {
            Ok(())
        }
    }
}
fn fixture() -> (TempDir, RepoRuntime) {
    let temp = TempDir::new().unwrap();
    init_repo(&InitRequest {
        root: temp.path().to_path_buf(),
        name: Some("fixture".into()),
        default_line: "main".into(),
        policy_profile: "prototype".into(),
        default_author_mode: "ai_with_human_review".into(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
    roles::initialize(&repo).unwrap();
    single_team::ensure_pool(&repo.binary_db_stores::<1>().local_db()).unwrap();
    (temp, repo)
}

struct CallerHost {
    metadata: JsonValue,
    reads: usize,
    fail_read: Option<usize>,
    former_state: Option<Result<&'static str, &'static str>>,
    on_former_read: Option<Box<dyn FnMut()>>,
}
impl CallerHost {
    fn new(repo: &RepoRuntime) -> Self {
        Self {
            metadata: json!({"id":"caller","cwd":repo.authoritative_repo_root(),
            "name":"Existing conversation","projectId":null,"status":{"type":"active"}}),
            reads: 0,
            fail_read: None,
            former_state: None,
            on_former_read: None,
        }
    }
    fn apply_actions(&mut self, offer: &JsonValue) {
        self.metadata["name"] = offer["title"].clone();
        self.metadata["projectId"] = offer["project_id"].clone();
    }
}
impl SessionHost for CallerHost {
    fn registration_state(
        &mut self,
        id: &str,
        cwd: &Path,
    ) -> Result<crate::team_remote::session_host::RegistrationState, String> {
        use crate::team_remote::session_host::RegistrationState;
        if id != "former-owner" {
            return self
                .activation_metadata(id, cwd)
                .map(RegistrationState::Available);
        }
        if let Some(mut action) = self.on_former_read.take() {
            action();
        }
        match self.former_state.ok_or("Unverified former owner")? {
            Ok("archived") => Ok(RegistrationState::Archived),
            Ok("missing") => Ok(RegistrationState::Missing),
            Ok(state) => Ok(RegistrationState::Available(json!({"id":id,"cwd":cwd,
                "name":"［Newton］Project Manager·fixture","projectId":"caller-project","status":{"type":state}}))),
            Err(error) => Err(error.into()),
        }
    }
    fn activation_metadata(&mut self, id: &str, cwd: &Path) -> Result<JsonValue, String> {
        self.reads += 1;
        if self.fail_read == Some(self.reads) {
            return Err("interrupted readback".into());
        }
        if self.metadata["id"] != id
            || self.metadata["cwd"] != json!(cwd)
            || self.metadata["archived"] == true
            || !matches!(
                self.metadata["status"]["type"].as_str(),
                Some("active" | "idle" | "notLoaded")
            )
        {
            return Err("unverified caller".into());
        }
        Ok(self.metadata.clone())
    }
    fn activation_project(&mut self, _: &Path) -> Result<String, String> {
        Ok("caller-project".into())
    }
    fn set_activation_project(&mut self, _: &str, _: &Path, project: &str) -> Result<(), String> {
        self.metadata["projectId"] = json!(project);
        Ok(())
    }
    fn recovery_scope(&self) -> Result<String, String> {
        Ok("caller-home".into())
    }
    fn inspect(&mut self, _: &str, _: &Path) -> Result<String, String> {
        panic!("caller must not be resumed or prepared")
    }
    fn create(
        &mut self,
        _: &Path,
        _: &str,
        _: &str,
        _: &mut dyn FnMut(&str) -> Result<(), String>,
    ) -> Result<String, String> {
        panic!("no blank caller replacement")
    }
    fn prepare_existing(&mut self, _: &str, _: &Path, _: &str, _: &str) -> Result<(), String> {
        panic!("caller owns its writer")
    }
}

fn former_owner_fixture(state: &'static str) -> (TempDir, RepoRuntime, CallerHost) {
    let (temp, repo) = fixture();
    let before = sessions::current(&repo, 0).unwrap();
    single_team::assign(
        &repo.binary_db_stores::<1>().local_db(),
        0,
        &before,
        0,
        "former-owner",
    )
    .unwrap();
    let mut host = CallerHost::new(&repo);
    host.former_state = Some(Ok(state));
    let root = repo.authoritative_repo_root();
    fence(
        &root.join(".ait/team-session-bootstrap/agent-0.host"),
        &host.recovery_scope().unwrap(),
    )
    .unwrap();
    let role = roles::load(&repo, 0).unwrap();
    fence(
        &sessions::loaded_path(&root, 0),
        &sessions::loaded_record(&root, 0, "former-owner", &role).to_string(),
    )
    .unwrap();
    (temp, repo, host)
}

#[test]
fn archived_and_missing_former_owners_offer_the_current_caller_and_acknowledge_once() {
    for state in ["archived", "missing"] {
        let (_temp, repo, mut host) = former_owner_fixture(state);
        // Matching metadata and the former owner's receipt do not prove delivery to this caller.
        host.metadata["name"] = json!("［Newton］Project Manager·fixture");
        host.metadata["projectId"] = json!("caller-project");
        let offer = activation::prepare(&repo, &mut host, Some("caller")).unwrap();
        assert_eq!(offer["status"], "activation_required");
        assert_eq!(offer["previous_session_id"], "former-owner");
        assert_eq!(offer["replacement_reason"], state);
        assert!(offer["role_instructions"]
            .as_str()
            .unwrap()
            .contains("Project Manager"));
        assert_eq!(sessions::current(&repo, 0).unwrap().session, "former-owner");
        let retry = activation::prepare(&repo, &mut host, Some("caller")).unwrap();
        assert_eq!(retry["activation_token"], offer["activation_token"]);
        assert!(retry["role_instructions"].is_null());
        assert!(activation::prepare(&repo, &mut host, Some("other")).is_err());
        let result = activation::acknowledge_with_host(
            &repo,
            &mut host,
            "caller",
            offer["activation_token"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(result["status"], "ready");
        assert_eq!(result["previous_session_id"], "former-owner");
        assert_eq!(result["replacement_reason"], state);
        assert_eq!(sessions::current(&repo, 0).unwrap().session, "caller");
        assert_eq!(directory(&repo).unwrap()["bound_session_count"], 1);
        assert_eq!(directory(&repo).unwrap()["actual_member_count"], 4);
        assert_eq!(
            activation::prepare(&repo, &mut host, Some("caller")).unwrap()["action"],
            "reused_current"
        );
    }
}

#[test]
fn former_owner_reappearance_or_unarchive_blocks_stale_acknowledgment() {
    for old in ["archived", "missing"] {
        for current in ["idle", "active", "notLoaded"] {
            let (_temp, repo, mut host) = former_owner_fixture(old);
            let offer = activation::prepare(&repo, &mut host, Some("caller")).unwrap();
            host.apply_actions(&offer);
            host.former_state = Some(Ok(current));
            let error = activation::acknowledge_with_host(
                &repo,
                &mut host,
                "caller",
                offer["activation_token"].as_str().unwrap(),
            )
            .unwrap_err();
            assert!(error.contains("no longer"), "{error}");
            assert_eq!(sessions::current(&repo, 0).unwrap().session, "former-owner");
            assert_eq!(
                sessions::loaded_receipt(&repo.authoritative_repo_root(), 0).unwrap()["session_id"],
                "former-owner"
            );
        }
    }
}

#[test]
fn valid_or_unverified_former_owners_are_never_replaced() {
    for state in [
        Ok("idle"),
        Ok("active"),
        Ok("notLoaded"),
        Err("unknown state"),
        Err("permission denied"),
        Err("workspace mismatch"),
    ] {
        let (_temp, repo, mut host) = former_owner_fixture("archived");
        host.former_state = Some(state);
        let result = activation::prepare(&repo, &mut host, Some("caller"));
        if state.is_ok() {
            assert_eq!(result.unwrap()["action"], "existing_owner");
        } else {
            assert!(result.is_err());
        }
        assert_eq!(sessions::current(&repo, 0).unwrap().session, "former-owner");
        assert!(read_activation_for_test(&repo).is_null());
    }
}

fn read_activation_for_test(repo: &RepoRuntime) -> JsonValue {
    sessions::read_json(
        &repo
            .authoritative_repo_root()
            .join(".ait/team-session-bootstrap/agent-0.activation"),
    )
    .unwrap()
}

#[test]
fn interrupted_reregistration_ack_finishes_the_committed_caller_without_rechecking_retired_owner() {
    let (_temp, repo, mut host) = former_owner_fixture("archived");
    let offer = activation::prepare(&repo, &mut host, Some("caller")).unwrap();
    host.apply_actions(&offer);
    host.fail_read = Some(host.reads + 2);
    assert!(activation::acknowledge_with_host(
        &repo,
        &mut host,
        "caller",
        offer["activation_token"].as_str().unwrap()
    )
    .is_err());
    assert_eq!(sessions::current(&repo, 0).unwrap().session, "caller");
    host.fail_read = None;
    host.former_state = Some(Err("former host now unavailable"));
    let retried = activation::prepare(&repo, &mut host, Some("caller")).unwrap();
    assert_eq!(retried["status"], "ready");
    assert_eq!(retried["previous_session_id"], "former-owner");
    assert_eq!(
        sessions::loaded_receipt(&repo.authoritative_repo_root(), 0).unwrap()["session_id"],
        "caller"
    );
}

#[test]
fn reregistration_preserves_target_uniqueness_and_binding_cas() {
    for conflict in ["target", "owner"] {
        let (_temp, repo, mut host) = former_owner_fixture("missing");
        let offer = activation::prepare(&repo, &mut host, Some("caller")).unwrap();
        host.apply_actions(&offer);
        let db = repo.binary_db_stores::<1>().local_db();
        if conflict == "target" {
            let before = sessions::current(&repo, 2).unwrap();
            single_team::assign(&db, 2, &before, 1, "caller").unwrap();
        } else {
            let before = sessions::current(&repo, 0).unwrap();
            host.on_former_read = Some(Box::new(move || {
                single_team::assign(&db, 0, &before, 0, "concurrent-owner").unwrap();
            }));
        }
        assert!(activation::acknowledge_with_host(
            &repo,
            &mut host,
            "caller",
            offer["activation_token"].as_str().unwrap()
        )
        .is_err());
        assert_eq!(
            sessions::current(&repo, 0).unwrap().session,
            if conflict == "target" {
                "former-owner"
            } else {
                "concurrent-owner"
            }
        );
    }
}

#[test]
fn reregistration_rejects_foreign_store_and_corrupt_replacement_evidence() {
    let (_temp, repo, mut host) = former_owner_fixture("archived");
    let root = repo.authoritative_repo_root();
    let host_path = root.join(".ait/team-session-bootstrap/agent-0.host");
    fence(&host_path, "another-store").unwrap();
    assert!(activation::prepare(&repo, &mut host, Some("caller"))
        .unwrap_err()
        .contains("another native store"));
    fence(&host_path, &host.recovery_scope().unwrap()).unwrap();
    let offer = activation::prepare(&repo, &mut host, Some("caller")).unwrap();
    let path = root.join(".ait/team-session-bootstrap/agent-0.activation");
    let mut saved = read_activation_for_test(&repo);
    saved["previous_session_id"] = json!("unrelated-session");
    fence(&path, &saved.to_string()).unwrap();
    assert!(activation::acknowledge_with_host(
        &repo,
        &mut host,
        "caller",
        offer["activation_token"].as_str().unwrap()
    )
    .unwrap_err()
    .contains("Invalid former owner"));
    assert_eq!(sessions::current(&repo, 0).unwrap().session, "former-owner");
}

#[test]
fn caller_activation_requires_role_ack_and_metadata_then_reuses_without_full_role() {
    let (_temp, repo) = fixture();
    let mut host = CallerHost::new(&repo);
    let offer = activation::prepare(&repo, &mut host, Some("caller")).unwrap();
    assert_eq!(offer["status"], "activation_required");
    assert!(offer["role_instructions"]
        .as_str()
        .unwrap()
        .contains("Project Manager"));
    assert_eq!(offer["app_actions"].as_array().unwrap().len(), 1);
    assert!(sessions::current(&repo, 0).unwrap().session.is_empty());
    assert!(sessions::loaded_receipt(&repo.authoritative_repo_root(), 0)
        .unwrap()
        .is_null());
    let repeat = activation::prepare(&repo, &mut host, Some("caller")).unwrap();
    assert!(repeat["role_instructions"].is_null());
    assert!(offer["instructions"]
        .as_str()
        .unwrap()
        .contains("without fetching the same files again"));
    let retry_instructions = repeat["instructions"].as_str().unwrap();
    assert!(retry_instructions.contains("Reuse the complete role and AGENTS.md"));
    assert!(retry_instructions.contains("only if their complete content is missing from context"));
    assert!(retry_instructions.contains("including after compaction"));
    let token = offer["activation_token"].as_str().unwrap();
    assert!(
        activation::acknowledge_with_host(&repo, &mut host, "caller", token)
            .unwrap_err()
            .contains("readback")
    );
    assert!(activation::acknowledge_with_host(&repo, &mut host, "other", token).is_err());
    host.apply_actions(&offer);
    assert_eq!(
        activation::acknowledge_with_host(&repo, &mut host, "caller", token).unwrap()["status"],
        "ready"
    );
    assert_eq!(directory(&repo).unwrap()["bound_session_count"], 1);
    assert_eq!(
        directory(&repo).unwrap()["members"][0]["role_reload_status"],
        "complete"
    );
    assert_eq!(
        activation::prepare(&repo, &mut host, Some("caller")).unwrap()["action"],
        "reused_current"
    );
    assert_eq!(
        activation::acknowledge_with_host(&repo, &mut host, "caller", token).unwrap()["status"],
        "ready"
    );
}

#[test]
fn interrupted_caller_ack_recovers_committed_binding_without_repeating_role() {
    let (_temp, repo) = fixture();
    let mut host = CallerHost::new(&repo);
    let offer = activation::prepare(&repo, &mut host, Some("caller")).unwrap();
    host.apply_actions(&offer);
    host.fail_read = Some(host.reads + 2); // after the binding CAS, before loaded receipt
    assert!(activation::acknowledge_with_host(
        &repo,
        &mut host,
        "caller",
        offer["activation_token"].as_str().unwrap()
    )
    .is_err());
    assert_eq!(sessions::current(&repo, 0).unwrap().session, "caller");
    assert!(sessions::loaded_receipt(&repo.authoritative_repo_root(), 0)
        .unwrap()
        .is_null());
    host.fail_read = None;
    assert_eq!(
        activation::prepare(&repo, &mut host, Some("caller")).unwrap()["status"],
        "ready"
    );
}

#[test]
fn folder_title_offers_from_the_previous_version_are_rejected_and_reconciled() {
    for phase in ["offered", "acknowledged"] {
        let (_temp, repo) = fixture();
        let mut host = CallerHost::new(&repo);
        let offer = activation::prepare(&repo, &mut host, Some("caller")).unwrap();
        let root = repo.authoritative_repo_root();
        let path = root.join(".ait/team-session-bootstrap/agent-0.activation");
        let mut previous = sessions::read_json(&path).unwrap();
        previous["title"] = json!(format!(
            "［Newton］Project Manager·{}",
            root.file_name().unwrap().to_str().unwrap()
        ));
        previous["phase"] = json!(phase);
        fence(&path, &previous.to_string()).unwrap();
        if phase == "acknowledged" {
            fence(
                &root.join(".ait/team-session-bootstrap/agent-0.host"),
                &host.recovery_scope().unwrap(),
            )
            .unwrap();
            let before = sessions::current(&repo, 0).unwrap();
            single_team::assign(
                &repo.binary_db_stores::<1>().local_db(),
                0,
                &before,
                0,
                "caller",
            )
            .unwrap();
        }
        host.metadata["name"] = previous["title"].clone();
        host.metadata["projectId"] = previous["project_id"].clone();
        assert!(activation::acknowledge_with_host(
            &repo,
            &mut host,
            "caller",
            offer["activation_token"].as_str().unwrap()
        )
        .unwrap_err()
        .contains("authoritative repository name"));
        let corrected = activation::prepare(&repo, &mut host, Some("caller")).unwrap();
        assert_eq!(corrected["title"], "［Newton］Project Manager·fixture");
        assert_eq!(corrected["session_id"], "caller");
        host.apply_actions(&corrected);
        let result = activation::acknowledge_with_host(
            &repo,
            &mut host,
            "caller",
            corrected["activation_token"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(result["status"], "ready");
        assert_eq!(result["title"], corrected["title"]);
    }
}

#[test]
fn pending_caller_prevents_duplicate_fallback_and_foreign_or_owned_adoption() {
    let (_temp, repo) = fixture();
    let mut host = CallerHost::new(&repo);
    activation::prepare(&repo, &mut host, Some("caller")).unwrap();
    assert_eq!(
        activation::prepare(&repo, &mut host, None).unwrap()["session_id"],
        "caller"
    );
    assert!(activation::prepare(&repo, &mut host, Some("other")).is_err());
    assert!(sessions::ensure_with_host(&repo, &mut host, 0, None)
        .unwrap_err()
        .contains("activation"));
    let (_temp, repo) = fixture();
    let db = repo.binary_db_stores::<1>().local_db();
    let agents = single_team::read(&db).unwrap();
    single_team::assign(&db, 2, &agents[2], 1, "caller").unwrap();
    let mut host = CallerHost::new(&repo);
    assert!(activation::prepare(&repo, &mut host, Some("caller"))
        .unwrap_err()
        .contains("another Agent"));
    assert!(sessions::current(&repo, 0).unwrap().session.is_empty());
}

#[test]
fn invalid_caller_is_never_replaced_and_different_owner_is_exposed() {
    for (field, value) in [
        ("cwd", json!("/wrong")),
        ("archived", json!(true)),
        ("status", json!({"type":"unknown"})),
        ("id", json!("missing")),
    ] {
        let (_temp, repo) = fixture();
        let mut host = CallerHost::new(&repo);
        host.metadata[field] = value;
        assert!(activation::prepare(&repo, &mut host, Some("caller")).is_err());
        assert!(sessions::current(&repo, 0).unwrap().session.is_empty());
    }
    let (_temp, repo) = fixture();
    let mut host = CallerHost::new(&repo);
    let offer = activation::prepare(&repo, &mut host, Some("caller")).unwrap();
    host.apply_actions(&offer);
    activation::acknowledge_with_host(
        &repo,
        &mut host,
        "caller",
        offer["activation_token"].as_str().unwrap(),
    )
    .unwrap();
    let existing = activation::prepare(&repo, &mut host, Some("other")).unwrap();
    assert_eq!(existing["action"], "existing_owner");
    assert_eq!(existing["session_id"], "caller");
    assert!(existing["role_instructions"].is_null());
}
#[test]
fn four_seats_newton_only_dynamic_assignment_and_same_id_reuse() {
    let (_temp, repo) = fixture();
    let mut host = Host::default();
    assert_eq!(directory(&repo).unwrap()["actual_member_count"], 4);
    let lead = sessions::ensure_with_host(&repo, &mut host, 0, None).unwrap();
    assert_eq!(lead["session_id"], "fake-1");
    assert_eq!(lead["title"], "［Newton］Project Manager·fixture");
    let again = sessions::ensure_with_host(&repo, &mut host, 0, None).unwrap();
    assert_eq!(again["session_id"], lead["session_id"]);
    assert_eq!(host.creates, 1);
    assert_eq!(host.injects, 1);
    sessions::ensure_with_host(&repo, &mut host, 2, None).unwrap();
    assert_eq!(directory(&repo).unwrap()["actual_member_count"], 4);
    sessions::ensure_with_host(&repo, &mut host, 3, Some(1)).unwrap();
    assert_eq!(directory(&repo).unwrap()["actual_member_count"], 5);
    assert!(sessions::ensure_with_host(&repo, &mut host, 1, None)
        .unwrap_err()
        .contains("registration"));
    assert_eq!(
        single_team::read(&repo.binary_db_stores::<1>().local_db())
            .unwrap()
            .len(),
        50
    );
    for file in [
        "agent_roles.bin",
        "agent_teams.bin",
        "agent_teams_payload.bin",
        "agent_team_membership.bin",
    ] {
        assert!(
            !repo.ait_dir.join("binary-db").join(file).exists(),
            "new route created {file}"
        );
    }
}
#[test]
fn uncertain_prepare_and_session_uniqueness_preserve_bindings() {
    let (_temp, repo) = fixture();
    let mut host = Host {
        fail_prepare: true,
        ..Default::default()
    };
    assert!(sessions::ensure_with_host(&repo, &mut host, 3, Some(1)).is_err());
    host.fail_prepare = false;
    assert!(sessions::ensure_with_host(&repo, &mut host, 3, Some(1))
        .unwrap_err()
        .contains("unknown"));
    assert_eq!(host.creates, 1);
    assert_eq!(directory(&repo).unwrap()["actual_member_count"], 4);
    let db = repo.binary_db_stores::<1>().local_db();
    let agents = single_team::read(&db).unwrap();
    single_team::assign(&db, 2, &agents[2], 1, "owned").unwrap();
    assert!(single_team::assign(&db, 4, &agents[4], 1, "owned").is_err());
    assert_eq!(single_team::read(&db).unwrap()[4], agents[4]);
}
#[test]
fn automatic_missing_recovery_reuses_valid_binding_and_does_not_increase_members() {
    let (_temp, repo) = fixture();
    let mut host = Host::default();
    let first = sessions::ensure_with_host(&repo, &mut host, 3, Some(1)).unwrap();
    let id = first["session_id"].as_str().unwrap();
    assert_eq!(
        sessions::ensure_with_host(&repo, &mut host, 3, None).unwrap()["session_id"],
        id
    );
    host.ids.clear();
    let recovered = sessions::ensure_with_host(&repo, &mut host, 3, None).unwrap();
    assert_ne!(recovered["session_id"], first["session_id"]);
    assert_eq!(directory(&repo).unwrap()["actual_member_count"], 5);
}
#[test]
fn disabled_directory_reads_no_agent_data_and_legacy_modes_have_local_defaults() {
    let (_temp, mut repo) = fixture();
    repo.config.insert("agent_teams".into(), json!(false));
    assert_eq!(directory(&repo).unwrap()["members"], json!([]));
    for mode in ["solo_local", "solo_remote", "team_remote"] {
        repo.config.insert("workflow_mode".into(), json!(mode));
        repo.config
            .insert("task_default_scope".into(), json!("remote"));
        assert!(repo.task_uses_local_scope(false, None).unwrap());
        assert!(!repo.task_uses_local_scope(false, Some("origin")).unwrap());
    }
}

#[test]
fn collaboration_is_delivered_only_in_the_selected_complete_role() {
    let (_temp, repo) = fixture();
    let shared = fs::read_to_string(repo.authoritative_repo_root().join("AGENTS.md")).unwrap();
    assert!(!shared.contains("Newton"));
    assert!(!shared.contains("<scope-args>"));
    let mut host = Host::default();
    for (index, role) in [(0, 0), (2, 1), (7, 3)] {
        sessions::ensure_with_host(&repo, &mut host, index, Some(role)).unwrap();
        let body = roles::load(&repo, role).unwrap().body;
        let delivered = host.delivered.last().unwrap();
        assert!(delivered.contains(&body));
        assert!(delivered.contains("without fetching the same files again"));
        assert!(delivered.contains("A new Task alone requires no role reload or session ensure"));
        assert!(delivered.contains("After compaction re-read applicable AGENTS.md"));
        assert!(delivered.contains("current work order"));
    }
    assert_eq!(host.injects, 3);
    let developer = &host.delivered[1];
    assert!(developer.contains("--remote <remote-name>"));
    assert!(developer.contains("exact assigned `--remote <remote-name>`"));
    ait_core::task_store::TaskStore::create_task(
        &repo.task_store().unwrap(),
        "fixture",
        "Same-session next Task",
        "Reuse the delivered role",
        None,
        None,
        None,
        None,
    )
    .unwrap();
    sessions::ensure_with_host(&repo, &mut host, 2, None).unwrap();
    assert_eq!(host.injects, 3, "unchanged role was delivered again");
    assert_eq!(
        fs::read_to_string(repo.authoritative_repo_root().join("AGENTS.md")).unwrap(),
        shared
    );
}

#[test]
fn two_same_role_sessions_reload_and_confirmed_retry_does_not_reinject() {
    let (_temp, repo) = fixture();
    let mut host = Host::default();
    for index in [2, 3] {
        sessions::ensure_with_host(&repo, &mut host, index, Some(1)).unwrap();
    }
    let before = single_team::read(&repo.binary_db_stores::<1>().local_db()).unwrap();
    let path = "docs/roles/1_Developer.md";
    let absolute = repo.authoritative_repo_root().join(path);
    let body = fs::read_to_string(&absolute).unwrap() + "\nUpdated responsibility.\n";
    fs::write(absolute, body).unwrap();
    let result=ait_core::plan_sync_execution::execute_plan_sync_command_request_json(&json!({"root_path":repo.authoritative_repo_root(),"repo_name":repo.repo_name(),"target":path,"local":true,"plan_storage":repo.plan_binary_db_storage_request::<1>().unwrap()}).to_string()).unwrap();
    assert_eq!(result["status"], "ok");
    let role = roles::load(&repo, 1).unwrap();
    for index in [2, 3] {
        sessions::ensure_with_host(&repo, &mut host, index, None).unwrap();
    }
    assert_eq!(host.injects, 4);
    assert_eq!(host.creates, 2);
    for index in [2, 3] {
        sessions::ensure_with_host(&repo, &mut host, index, None).unwrap();
    }
    assert_eq!((host.creates, host.injects), (2, 4));
    let after = single_team::read(&repo.binary_db_stores::<1>().local_db()).unwrap();
    assert_eq!(after, before);
    for index in [2, 3] {
        assert!(sessions::receipt_matches(
            &sessions::loaded_receipt(&repo.authoritative_repo_root(), index).unwrap(),
            &after[index as usize],
            index,
            &role
        ));
    }
    let p = repo
        .authoritative_repo_root()
        .join(".ait/team-session-bootstrap/agent-3.role-loaded");
    let mut saved = sessions::loaded_receipt(&repo.authoritative_repo_root(), 3).unwrap();
    saved["phase"] = json!("loaded");
    fence(&p, &saved.to_string()).unwrap();
    sessions::reload_one(&repo, &mut host, 3, &after[3], &role).unwrap();
    assert_eq!(host.injects, 4);
    fs::remove_file(repo.authoritative_repo_root().join(path)).unwrap();
    assert!(before_plan_sync(&repo, Path::new(path), true).is_err());
}

#[test]
fn operation_lock_excludes_parallel_provisioning() {
    let (_temp, repo) = fixture();
    let (_path, _lock) = operation_lock(&repo.authoritative_repo_root()).unwrap();
    let result = ensure(&repo, 3, Some(1));
    assert_eq!(result["status"], "pending");
    assert!(result["detail"]
        .as_str()
        .unwrap()
        .contains("operation_busy"));
    assert_eq!(directory(&repo).unwrap()["bound_session_count"], 0);
}

#[test]
fn registration_enables_einstein_without_automatic_creation() {
    let (_temp, repo) = fixture();
    let path = repo.ait_dir.join("config.json");
    let mut config = repo.config.clone();
    config.insert("repository_index".into(), json!(7));
    config.insert("default_remote".into(), json!("origin"));
    config.insert(
        "remotes".into(),
        json!({"origin":{"remote_id":1,"url":"http://fixture.invalid","repo_name":"fixture"}}),
    );
    fs::write(path, json!(config).to_string()).unwrap();
    let repo = RepoRuntime::discover_from_path(&repo.root).unwrap();
    let directory = directory(&repo).unwrap();
    assert_eq!(directory["registered"], true);
    assert_eq!(directory["bound_session_count"], 0);
    let mut host = Host::default();
    let result = sessions::ensure_with_host(&repo, &mut host, 1, None).unwrap();
    assert_eq!(result["role_index"], 2);
    assert_eq!(host.creates, 1);
    assert!(repo.team_review_enabled());
}

#[test]
fn corrupt_operation_evidence_blocks_without_repeated_creation_or_injection() {
    let (_temp, repo) = fixture();
    let mut host = Host::default();
    sessions::ensure_with_host(&repo, &mut host, 3, Some(1)).unwrap();
    let before = single_team::read(&repo.binary_db_stores::<1>().local_db()).unwrap();
    let path = repo
        .authoritative_repo_root()
        .join(".ait/team-session-bootstrap/agent-3.role-loaded");
    fs::write(path, "{broken").unwrap();
    assert!(sessions::ensure_with_host(&repo, &mut host, 3, None).is_err());
    assert_eq!(host.creates, 1);
    assert_eq!(host.injects, 1);
    assert_eq!(
        single_team::read(&repo.binary_db_stores::<1>().local_db()).unwrap(),
        before
    );
}

#[test]
fn finish_handoff_reads_current_local_coordinator_binding_for_both_scopes() {
    let (_temp, repo) = fixture();
    let mut host = Host::default();
    sessions::ensure_with_host(&repo, &mut host, 0, None).unwrap();
    let db = repo.binary_db_stores::<1>().local_db();
    let before = single_team::read(&db).unwrap();
    single_team::assign(&db, 0, &before[0], 0, "latest-newton").unwrap();
    for scope in [
        crate::team_remote::FinishScope::Local,
        crate::team_remote::FinishScope::Remote,
    ] {
        let mut output =
            json!({"apply_status":"done","closeout_status":"complete_unbound","task_id":"T-1"});
        crate::team_remote::attach_finish_handoff(
            true,
            &repo.authoritative_repo_root(),
            scope,
            &mut output,
            &CoordinatorSource,
        );
        assert_eq!(
            output["team_handoff"]["coordinator"]["session_id"],
            "latest-newton"
        );
        assert_eq!(output["team_handoff"]["notification_sent"], false);
    }
}

#[test]
fn known_creation_reply_retries_same_candidate_without_new_creation() {
    let (_temp, repo) = fixture();
    let mut host = Host {
        fail_create_after_receipt: true,
        ..Default::default()
    };
    assert!(sessions::ensure_with_host(&repo, &mut host, 3, Some(1)).is_err());
    host.fail_create_after_receipt = false;
    let result = sessions::ensure_with_host(&repo, &mut host, 3, Some(1)).unwrap();
    assert_eq!(result["session_id"], "fake-1");
    assert_eq!((host.creates, host.injects), (1, 1));
}

#[test]
fn unknown_creation_reply_stays_blocked_without_duplicate() {
    let (_temp, repo) = fixture();
    let mut host = Host {
        fail_create_before_receipt: true,
        ..Default::default()
    };
    assert!(sessions::ensure_with_host(&repo, &mut host, 3, Some(1)).is_err());
    host.fail_create_before_receipt = false;
    assert!(sessions::ensure_with_host(&repo, &mut host, 3, Some(1))
        .unwrap_err()
        .contains("unknown"));
    assert_eq!((host.creates, host.injects), (1, 0));
}

#[test]
fn missing_pending_candidate_can_recover_after_unknown_injection() {
    let (_temp, repo) = fixture();
    let mut host = Host {
        fail_prepare: true,
        ..Default::default()
    };
    assert!(sessions::ensure_with_host(&repo, &mut host, 3, Some(1)).is_err());
    host.fail_prepare = false;
    assert!(sessions::ensure_with_host(&repo, &mut host, 3, Some(1))
        .unwrap_err()
        .contains("unknown"));
    host.ids.clear();
    let result = sessions::ensure_with_host(&repo, &mut host, 3, Some(1)).unwrap();
    assert_eq!(result["action"], "recovered");
    assert_eq!(result["session_id"], "fake-2");
    sessions::ensure_with_host(&repo, &mut host, 3, Some(1)).unwrap();
    assert_eq!((host.creates, host.injects), (2, 2));
}

#[test]
fn failed_bound_recovery_keeps_original_until_missing_replacement_is_recovered() {
    let (_temp, repo) = fixture();
    let mut host = Host::default();
    sessions::ensure_with_host(&repo, &mut host, 3, Some(1)).unwrap();
    host.ids.clear();
    host.fail_prepare = true;
    assert!(sessions::ensure_with_host(&repo, &mut host, 3, None).is_err());
    host.fail_prepare = false;
    assert!(sessions::ensure_with_host(&repo, &mut host, 3, None)
        .unwrap_err()
        .contains("unknown"));
    let db = repo.binary_db_stores::<1>().local_db();
    assert_eq!(single_team::read(&db).unwrap()[3].session, "fake-1");
    assert_eq!(host.creates, 2);
    host.ids.clear();
    let result = sessions::ensure_with_host(&repo, &mut host, 3, None).unwrap();
    assert_eq!(result["session_id"], "fake-3");
    sessions::ensure_with_host(&repo, &mut host, 3, None).unwrap();
    assert_eq!((host.creates, host.injects), (3, 3));
}

#[test]
fn reappearing_original_blocks_commit_and_loaded_replacement_retries_once() {
    let (_temp, repo) = fixture();
    let mut host = Host::default();
    sessions::ensure_with_host(&repo, &mut host, 3, Some(1)).unwrap();
    host.ids.clear();
    host.reappear_on_prepare = Some("fake-1".into());
    assert!(sessions::ensure_with_host(&repo, &mut host, 3, None).is_err());
    let db = repo.binary_db_stores::<1>().local_db();
    assert_eq!(single_team::read(&db).unwrap()[3].session, "fake-1");
    assert!(sessions::ensure_with_host(&repo, &mut host, 3, None)
        .unwrap_err()
        .contains("conflicts"));
    assert_eq!((host.creates, host.injects), (2, 2));
    host.ids.retain(|id| id != "fake-1");
    // An old interruption may retain assignment evidence after losing its fence.
    fs::remove_file(
        repo.root
            .join(".ait/team-session-bootstrap/agent-3.pending"),
    )
    .unwrap();
    let result = sessions::ensure_with_host(&repo, &mut host, 3, None).unwrap();
    assert_eq!(result["session_id"], "fake-2");
    assert_eq!((host.creates, host.injects), (2, 2));
}

#[test]
fn changed_binding_during_preparation_is_never_overwritten() {
    let (_temp, repo) = fixture();
    let mut host = Host::default();
    sessions::ensure_with_host(&repo, &mut host, 3, Some(1)).unwrap();
    host.ids.clear();
    host.ids.push("external-session".into());
    let root = repo.root.clone();
    host.on_prepare = Some(Box::new(move || {
        let repo = RepoRuntime::discover_from_path(&root).unwrap();
        let db = repo.binary_db_stores::<1>().local_db();
        let before = single_team::read(&db).unwrap();
        single_team::assign(&db, 3, &before[3], 1, "external-session").unwrap();
    }));
    assert!(sessions::ensure_with_host(&repo, &mut host, 3, None)
        .unwrap_err()
        .contains("binding changed"));
    assert_eq!(
        single_team::read(&repo.binary_db_stores::<1>().local_db()).unwrap()[3].session,
        "external-session"
    );
    assert!(sessions::ensure_with_host(&repo, &mut host, 3, None).is_err());
    assert_eq!(host.creates, 2);
}

#[test]
fn pending_candidate_owned_by_another_agent_is_not_injected_or_bound() {
    let (_temp, repo) = fixture();
    let mut host = Host::default();
    sessions::ensure_with_host(&repo, &mut host, 2, None).unwrap();
    let op = repo.root.join(".ait/team-session-bootstrap");
    fs::copy(op.join("agent-2.host"), op.join("agent-3.host")).unwrap();
    fs::write(op.join("agent-3.pending"), "fake-1").unwrap();
    assert!(sessions::ensure_with_host(&repo, &mut host, 3, Some(1))
        .unwrap_err()
        .contains("another Agent"));
    assert_eq!((host.creates, host.injects), (1, 1));
    assert!(
        single_team::read(&repo.binary_db_stores::<1>().local_db()).unwrap()[3]
            .session
            .is_empty()
    );
}

#[test]
fn bound_recovery_in_another_native_store_preserves_all_evidence() {
    let (_temp, repo) = fixture();
    let mut host = Host::default();
    sessions::ensure_with_host(&repo, &mut host, 3, Some(1)).unwrap();
    host.ids.clear();
    let op = repo.root.join(".ait/team-session-bootstrap");
    fs::write(op.join("agent-3.host"), "different-store").unwrap();
    assert!(sessions::ensure_with_host(&repo, &mut host, 3, None)
        .unwrap_err()
        .contains("another native store"));
    assert_eq!((host.creates, host.injects), (1, 1));
    assert_eq!(
        single_team::read(&repo.binary_db_stores::<1>().local_db()).unwrap()[3].session,
        "fake-1"
    );
    assert!(!op.join("agent-3.pending").exists());
}

#[test]
fn loaded_assignment_without_pending_fence_is_committed_without_reinjection() {
    let (_temp, repo) = fixture();
    let op = repo.root.join(".ait/team-session-bootstrap");
    fs::create_dir_all(&op).unwrap();
    fs::write(op.join("agent-3.host"), "fake-home").unwrap();
    let role = roles::load(&repo, 1).unwrap();
    fs::write(op.join("agent-3.assignment"), json!({"version":1,"expected_session":"","role_index":1,"revision":role.revision,"session_id":"resumable","phase":"loaded"}).to_string()).unwrap();
    let mut host = Host {
        ids: vec!["resumable".into()],
        ..Default::default()
    };
    let result = sessions::ensure_with_host(&repo, &mut host, 3, Some(1)).unwrap();
    assert_eq!(result["session_id"], "resumable");
    assert_eq!(result["action"], "completed_pending");
    assert_eq!((host.creates, host.injects), (0, 0));
    assert!(!op.join("agent-3.assignment").exists());
}

#[test]
fn reappearing_retired_pending_candidate_blocks_initial_commit_and_retries() {
    let (_temp, repo) = fixture();
    let mut host = Host {
        fail_prepare: true,
        ..Default::default()
    };
    assert!(sessions::ensure_with_host(&repo, &mut host, 3, Some(1)).is_err());
    host.fail_prepare = false;
    host.ids.clear();
    host.reappear_on_prepare = Some("fake-1".into());
    assert!(sessions::ensure_with_host(&repo, &mut host, 3, Some(1)).is_err());
    assert!(sessions::ensure_with_host(&repo, &mut host, 3, Some(1)).is_err());
    assert_eq!((host.creates, host.injects), (2, 2));
    assert!(
        single_team::read(&repo.binary_db_stores::<1>().local_db()).unwrap()[3]
            .session
            .is_empty()
    );
    host.ids.retain(|id| id != "fake-1");
    assert_eq!(
        sessions::ensure_with_host(&repo, &mut host, 3, Some(1)).unwrap()["session_id"],
        "fake-2"
    );
    assert_eq!((host.creates, host.injects), (2, 2));
}

#[test]
fn role_head_change_during_preparation_blocks_commit_and_does_not_duplicate() {
    let (_temp, repo) = fixture();
    let root = repo.root.clone();
    let mut host = Host {
        on_prepare: Some(Box::new(move || {
            let repo = RepoRuntime::discover_from_path(&root).unwrap();
            let path = root.join("docs/roles/1_Developer.md");
            fs::write(
                &path,
                fs::read_to_string(&path).unwrap() + "\nNew revision.\n",
            )
            .unwrap();
            let result = ait_core::plan_sync_execution::execute_plan_sync_command_request_json(&json!({"root_path":root,"repo_name":repo.repo_name(),"target":path,"local":true,"plan_storage":repo.plan_binary_db_storage_request::<1>().unwrap()}).to_string()).unwrap();
            assert_eq!(result["status"], "ok");
        })),
        ..Default::default()
    };
    assert!(sessions::ensure_with_host(&repo, &mut host, 3, Some(1))
        .unwrap_err()
        .contains("Role Plan head changed"));
    assert!(sessions::ensure_with_host(&repo, &mut host, 3, Some(1))
        .unwrap_err()
        .contains("context changed"));
    assert!(
        single_team::read(&repo.binary_db_stores::<1>().local_db()).unwrap()[3]
            .session
            .is_empty()
    );
    assert_eq!((host.creates, host.injects), (1, 1));
}

#[test]
fn changed_binding_during_reuse_is_reported_without_stale_success() {
    let (_temp, repo) = fixture();
    let mut host = Host::default();
    sessions::ensure_with_host(&repo, &mut host, 3, Some(1)).unwrap();
    let root = repo.root.clone();
    host.on_reuse = Some(Box::new(move || {
        let repo = RepoRuntime::discover_from_path(&root).unwrap();
        let db = repo.binary_db_stores::<1>().local_db();
        let before = single_team::read(&db).unwrap();
        single_team::assign(&db, 3, &before[3], 1, "external-session").unwrap();
    }));
    assert!(sessions::ensure_with_host(&repo, &mut host, 3, None)
        .unwrap_err()
        .contains("binding changed"));
    assert_eq!(
        single_team::read(&repo.binary_db_stores::<1>().local_db()).unwrap()[3].session,
        "external-session"
    );
    assert_eq!((host.creates, host.injects), (1, 1));
}

#[test]
fn interrupted_retirement_cannot_switch_native_stores() {
    let (_temp, repo) = fixture();
    let op = repo.root.join(".ait/team-session-bootstrap");
    fs::create_dir_all(&op).unwrap();
    fs::write(op.join("agent-3.host"), "different-store").unwrap();
    fs::write(op.join("agent-3.retired"), "retired-original").unwrap();
    let mut host = Host::default();
    assert!(sessions::ensure_with_host(&repo, &mut host, 3, Some(1))
        .unwrap_err()
        .contains("another native store"));
    assert_eq!((host.creates, host.injects), (0, 0));
    assert!(!op.join("agent-3.pending").exists());
}

#[test]
fn disabling_teams_preserves_solo_guidance_roles_bindings_and_unfinished_tasks() {
    use crate::config_surface::{config_set, config_show, ConfigSetRequest};
    use ait_core::task_store::TaskStore;

    for sprint in ["on", "off"] {
        let (_temp, mut repo) = fixture();
        config_set(
            &repo,
            &ConfigSetRequest {
                sprint: Some(sprint.into()),
                ..Default::default()
            },
        )
        .unwrap();
        repo = RepoRuntime::discover_from_path(&repo.root).unwrap();
        let mut host = Host::default();
        sessions::ensure_with_host(&repo, &mut host, 3, Some(1)).unwrap();
        let agents = single_team::read(&repo.binary_db_stores::<1>().local_db()).unwrap();
        let loaded = sessions::loaded_receipt(&repo.root, 3).unwrap();
        let roles = crate::role_templates::CURRENT.map(|(name, _)| {
            let path = repo.root.join("docs/roles").join(name);
            let body = fs::read_to_string(&path).unwrap();
            (path, body)
        });
        let store = repo.task_store().unwrap();
        let assigned = TaskStore::create_task(
            &store,
            "fixture",
            "Unfinished assigned work",
            "Preserve its existing ownership and gates",
            None,
            None,
            None,
            None,
        )
        .unwrap();
        for name in ["AGENTS.md", "CLAUDE.md"] {
            let path = repo.root.join(name);
            let body = fs::read_to_string(&path).unwrap();
            fs::write(
                path,
                format!("{body}\nKeep this unmanaged repository rule.\n"),
            )
            .unwrap();
        }

        for (setting, effective_author_mode) in [
            ("disabled", "ai_with_human_review"),
            ("enabled", "ai_only_experimental"),
        ] {
            config_set(
                &repo,
                &ConfigSetRequest {
                    agent_teams: Some(setting.into()),
                    ..Default::default()
                },
            )
            .unwrap();
            repo = RepoRuntime::discover_from_path(&repo.root).unwrap();
            let config = config_show(&repo).unwrap();
            assert_eq!(config["agent_teams"], setting == "enabled");
            assert_eq!(config["default_author_mode"], "ai_with_human_review");
            assert_eq!(config["effective_author_mode"], effective_author_mode);
            for name in ["AGENTS.md", "CLAUDE.md"] {
                let body = fs::read_to_string(repo.root.join(name)).unwrap();
                assert_eq!(body.matches("<!-- ait:workflow:start -->").count(), 1);
                assert_eq!(body.matches("<!-- ait:workflow:end -->").count(), 1);
                assert!(body.contains("Keep this unmanaged repository rule."));
                assert!(body.contains("Use complete applicable guidance already in context"));
                assert!(body
                    .contains("After context compaction, re-read applicable repository guidance"));
                assert!(body.contains("### Code-change path"));
                assert!(body.contains("ait task finish"));
                assert!(body.contains("Work only in the returned `edit_root`"));
                assert!(body.contains(&format!("author-mode=`{effective_author_mode}`")));
                assert!(body.contains("scope=`local`"));
                assert_eq!(body.contains("ait task start --from"), sprint == "on");
                assert_eq!(body.contains("ait task start --title"), sprint == "off");
                for team_rule in ["docs/roles/", "ait team", "ASSIGN", "READY"] {
                    assert!(!body.contains(team_rule));
                }
            }
            for (path, body) in &roles {
                assert_eq!(fs::read_to_string(path).unwrap(), *body);
            }
            assert_eq!(
                single_team::read(&repo.binary_db_stores::<1>().local_db()).unwrap(),
                agents
            );
            assert_eq!(sessions::loaded_receipt(&repo.root, 3).unwrap(), loaded);
            assert_eq!(
                TaskStore::get_task(&store, assigned["task_id"].as_str().unwrap()).unwrap(),
                assigned
            );
            if setting == "disabled" {
                assert_eq!(directory(&repo).unwrap()["members"], json!([]));
                assert!(sessions::ensure_with_host(&repo, &mut host, 3, None)
                    .unwrap_err()
                    .contains("disabled"));
                assert!(ensure(&repo, 3, None)["detail"]
                    .as_str()
                    .unwrap()
                    .contains("disabled"));
                assert!(acknowledge(&repo, "unused")["detail"]
                    .as_str()
                    .unwrap()
                    .contains("disabled"));
                let solo = TaskStore::create_task(
                    &store,
                    "fixture",
                    "New solo work",
                    "Use the repository workflow without team activation",
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
                assert!(solo["plan_id"].is_null());
            } else {
                sessions::ensure_with_host(&repo, &mut host, 3, None).unwrap();
            }
            assert_eq!((host.creates, host.injects), (1, 1));
        }
    }
}

#[test]
fn verified_default_upgrades_and_history_preserve_custom_additions() {
    use sha2::{Digest, Sha256};
    let current = crate::role_templates::CURRENT[1].1;
    // Exercise native provenance and history without archiving old policy text.
    let previous = current.replacen(
        "Handoffs contain only",
        "Earlier handoffs contained only",
        1,
    );
    assert_ne!(previous, current);
    let customize = |body: &str| {
        body.replace(
            "## Implementation and handoff",
            "Preserve this repository-specific custom rule.\n\n## Implementation and handoff",
        )
    };
    assert!(customize(current).contains("Preserve this repository-specific custom rule."));
    for saved_base in [false, true] {
        for custom in [false, true] {
            let (_temp, repo) = fixture();
            let path = "docs/roles/1_Developer.md";
            let sync = || {
                let result = ait_core::plan_sync_execution::execute_plan_sync_command_request_json(
                    &json!({"root_path":repo.root,"repo_name":repo.repo_name(),"target":path,
                        "local":true,"plan_storage":repo.plan_binary_db_storage_request::<1>().unwrap()})
                        .to_string(),
                )
                .unwrap();
                assert_eq!(result["status"], "ok");
            };
            let base = if saved_base {
                previous.as_str()
            } else {
                current
            };
            fs::write(repo.root.join(path), base).unwrap();
            sync();
            if custom {
                fs::write(repo.root.join(path), customize(base)).unwrap();
                sync();
            }
            let provenance = repo.root.join(".ait/team-session-bootstrap/role-defaults");
            if saved_base {
                let mut saved = sessions::read_json(&provenance).unwrap();
                let row = saved["roles"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|row| row["role_index"] == 1)
                    .unwrap();
                row["default_body"] = json!(base);
                row["default_sha256"] = json!(format!("{:x}", Sha256::digest(base.as_bytes())));
                fence(&provenance, &saved.to_string()).unwrap();
            } else {
                fs::remove_file(provenance).unwrap();
            }
            let upgraded = roles::initialize(&repo).unwrap();
            assert_eq!(upgraded["status"], "ready");
            assert_eq!(
                upgraded["roles"][1]["default_state"],
                if saved_base {
                    "updated_default"
                } else if custom {
                    "customized"
                } else {
                    "current"
                }
            );
            let expected = if custom {
                customize(current)
            } else {
                current.into()
            };
            assert_eq!(fs::read_to_string(repo.root.join(path)).unwrap(), expected);
            let repeated = roles::initialize(&repo).unwrap();
            assert_eq!(repeated["status"], "ready");
            assert_eq!(
                repeated["roles"][1]["default_state"],
                if custom { "customized" } else { "current" }
            );
            assert_eq!(fs::read_to_string(repo.root.join(path)).unwrap(), expected);
        }
    }
}
