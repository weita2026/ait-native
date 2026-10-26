//! Caller cooperation never acquires the app's writer or starts a model turn.
use super::*;
use crate::team_remote::session_host::RegistrationState;
use sha2::{Digest, Sha256};

fn path(repo: &RepoRuntime) -> std::path::PathBuf {
    repo.authoritative_repo_root()
        .join(".ait/team-session-bootstrap/agent-0.activation")
}

fn read(repo: &RepoRuntime) -> Result<JsonValue, String> {
    let value = sessions::read_json(&path(repo))?;
    if !value.is_null()
        && (value["version"] != 1
            || value["repository"] != json!(repo.authoritative_repo_root())
            || value["agent_index"] != 0
            || !matches!(
                value["phase"].as_str(),
                Some("offered" | "acknowledged" | "complete")
            )
            || value["session_id"].as_str().is_none_or(str::is_empty)
            || value["token"].as_str().is_none_or(str::is_empty))
    {
        return Err("Invalid caller activation evidence; preserve it for recovery".into());
    }
    if value.get("replacement_reason").is_some() || value.get("previous_session_id").is_some() {
        let previous = value["previous_session_id"].as_str();
        if !matches!(
            value["replacement_reason"].as_str(),
            Some("archived" | "missing")
        ) || previous.is_none_or(str::is_empty)
            || value["previous_session_id"] != value["expected_agent"]["session"]
            || value["previous_session_id"] == value["session_id"]
        {
            return Err("Invalid former owner evidence; preserve it for recovery".into());
        }
    }
    Ok(value)
}

pub(super) fn pending(repo: &RepoRuntime) -> Result<Option<JsonValue>, String> {
    let value = read(repo)?;
    Ok((!value.is_null() && value["phase"] != "complete").then_some(value))
}

pub(super) fn caller_session() -> Result<Option<String>, String> {
    let Some(id) = std::env::var_os("CODEX_THREAD_ID") else {
        return Ok(None);
    };
    let id = id
        .into_string()
        .map_err(|_| "Invalid native caller identity")?;
    if id.is_empty() || id.trim() != id {
        return Err("Invalid native caller identity".into());
    }
    if let Some(session) = std::env::var_os("CODEX_SESSION_ID") {
        if session != std::ffi::OsStr::new(&id) {
            return Err("Native caller identities disagree; no adoption attempted".into());
        }
    }
    Ok(Some(id))
}

fn agent_state(agent: &Agent) -> JsonValue {
    json!({"name":agent.name,"session":agent.session,"role":agent.role,"archived":agent.archived})
}

fn verify_metadata(metadata: &JsonValue, saved: &JsonValue) -> Result<(), String> {
    if metadata["id"] != saved["session_id"]
        || metadata["cwd"] != saved["repository"]
        || metadata["name"] != saved["title"]
        || metadata["projectId"] != saved["project_id"]
    {
        return Err("Caller title/project readback differs; complete the listed app actions and retry acknowledgment".into());
    }
    Ok(())
}

fn verify_former_owner(
    host: &mut impl SessionHost,
    root: &Path,
    saved: &JsonValue,
) -> Result<(), String> {
    let Some(reason) = saved["replacement_reason"].as_str() else {
        return Ok(());
    };
    let id = saved["previous_session_id"]
        .as_str()
        .ok_or("Missing former owner identity")?;
    let state = host.registration_state(id, root)?;
    if matches!(
        (reason, state),
        ("archived", RegistrationState::Archived) | ("missing", RegistrationState::Missing)
    ) {
        Ok(())
    } else {
        Err(format!(
            "Former owner session {id} is no longer {reason}; repeat enable to reconcile ownership"
        ))
    }
}

pub(super) fn prepare(
    repo: &RepoRuntime,
    host: &mut impl SessionHost,
    caller: Option<&str>,
) -> Result<JsonValue, String> {
    let before = sessions::current(repo, 0)?;
    let prior = read(repo)?;
    fs::create_dir_all(path(repo).parent().unwrap()).map_err(|e| e.to_string())?;
    let Some(caller) = caller else {
        if let Some(saved) = pending(repo)? {
            return Ok(
                json!({"status":"pending","action":"caller_activation_pending",
                "session_id":saved["session_id"],"detail":"Continue activation in the recorded calling conversation",
                "work_started":false}),
            );
        }
        return sessions::ensure_with_host(repo, host, 0, Some(0));
    };
    let root = repo.authoritative_repo_root();
    let scope = host.recovery_scope()?;
    let host_path = root.join(".ait/team-session-bootstrap/agent-0.host");
    if !before.session.is_empty()
        && fs::read_to_string(&host_path).map_err(|_| "Original native host evidence missing")?
            != scope
    {
        return Err("Owner belongs to another native store; binding retained".into());
    }
    if before.archived || (!before.session.is_empty() && before.role != Some(0)) {
        return Err("Owner identity is archived or has an incompatible role".into());
    }
    let role = roles::load(repo, 0)?;
    let title = sessions::session_title(repo, &before, &role);
    if single_team::read(&repo.binary_db_stores::<1>().local_db())
        .map_err(|e| e.to_string())?
        .iter()
        .skip(1)
        .any(|agent| agent.session == caller)
    {
        return Err("Calling conversation belongs to another Agent; no adoption attempted".into());
    }
    if let Some(saved) = pending(repo)? {
        if saved["session_id"] != caller || saved["host_scope"] != scope {
            return Err(
                "Another caller activation is pending; no transfer or replacement attempted".into(),
            );
        }
        if saved["phase"] == "acknowledged"
            && saved["revision"] == role.revision
            && saved["title"] == title
        {
            return acknowledge_with_host(repo, host, caller, saved["token"].as_str().unwrap());
        }
        if saved["phase"] == "offered" && saved["expected_agent"] != agent_state(&before) {
            return Err("Owner changed during caller activation; no binding changed".into());
        }
    }
    let replacement_reason = if !before.session.is_empty() && before.session != caller {
        match host.registration_state(&before.session, &root)? {
            RegistrationState::Available(metadata) => {
                let project = host.activation_project(&root)?;
                let loaded = sessions::receipt_matches(
                    &sessions::loaded_receipt(&root, 0)?,
                    &before,
                    0,
                    &role,
                );
                return Ok(
                    json!({"status":if loaded && metadata["name"]==title && metadata["projectId"]==project {"ready"} else {"pending"},
                    "action":"existing_owner","caller_adopted":false,"condition":"A different owner conversation is already bound; continue there. No transfer was attempted.",
                    "agent_index":0,"session_id":before.session,"title":title,"project_id":project,
                    "role_revision_index":role.revision,"role_reload_status":if loaded{"complete"}else{"pending"},
                    "host_status":metadata["status"]["type"],"work_started":false}),
                );
            }
            RegistrationState::Archived => Some("archived"),
            RegistrationState::Missing => Some("missing"),
        }
    } else {
        None
    };
    if replacement_reason.is_some()
        && ["agent-0.pending", "agent-0.assignment"]
            .iter()
            .any(|name| root.join(".ait/team-session-bootstrap").join(name).exists())
    {
        return Err("Existing owner preparation must be reconciled before re-registration".into());
    }
    if before.session.is_empty()
        && ["agent-0.pending", "agent-0.assignment", "agent-0.retired"]
            .iter()
            .any(|name| root.join(".ait/team-session-bootstrap").join(name).exists())
    {
        return Err("Existing owner preparation must be reconciled before caller adoption".into());
    }
    let metadata = host.activation_metadata(caller, &root)?;
    let project = host.activation_project(&root)?;
    let loaded = before.session == caller
        && sessions::receipt_matches(&sessions::loaded_receipt(&root, 0)?, &before, 0, &role);
    sessions::verify(repo, 0, &before, &role)?;
    if loaded && metadata["name"] == title && metadata["projectId"] == project {
        return Ok(
            json!({"status":"ready","action":"reused_current","agent_index":0,
            "session_id":caller,"title":title,"project_id":project,"role_revision_index":role.revision,
            "role_reload_status":"complete","work_started":false}),
        );
    }
    let mut saved = json!({"version":1,"repository":root,"agent_index":0,"session_id":caller,
        "expected_agent":agent_state(&before),"host_scope":scope,"role_index":0,
        "revision":role.revision,"role_digest":format!("{:x}",Sha256::digest(role.body.as_bytes())),
        "title":title,"project_id":project,"role_delivery_required":!loaded,"phase":"offered"});
    if let Some(reason) = replacement_reason {
        saved["previous_session_id"] = json!(before.session);
        saved["replacement_reason"] = json!(reason);
    }
    let token = format!("{:x}", Sha256::digest(saved.to_string().as_bytes()));
    saved["token"] = json!(token);
    let repeat = prior == saved;
    fence(&path(repo), &saved.to_string())?;
    if metadata["projectId"] != project {
        host.set_activation_project(caller, &root, &project)?;
        sessions::verify(repo, 0, &before, &role)?;
    }
    let mut actions = Vec::new();
    if metadata["name"] != title {
        actions.push(json!({"tool":"mcp__codex_app__set_thread_title","arguments":{"threadId":caller,"title":title}}));
    }
    let acknowledgment = format!("ait team session acknowledge --activation {token} --json");
    let instructions = format!("Continue Agent Teams activation in this same conversation. {} Execute the listed app-tool actions, then run `{acknowledgment}` from {} to acknowledge receipt and verify title/project/binding. Do not start work or another conversation. Report activation complete only after native acknowledgment returns ready.",
        if loaded {"The current role is already acknowledged; retain applicable guidance in context. Recover missing guidance after compaction."} else if repeat {"Reuse the complete role and AGENTS.md supplied by the preceding offer. Fetch role_artifact_path and repository AGENTS.md only if their complete content is missing from context, including after compaction; this exact retry does not repeat the unchanged delivery."} else {"Apply the complete role_instructions below without fetching the same files again before acknowledging."}, root.display());
    Ok(
        json!({"status":"activation_required","action":"activate_current","agent_index":0,
        "session_id":caller,"title":title,"project_id":project,"role_revision_index":role.revision,
        "previous_session_id":saved["previous_session_id"],"replacement_reason":saved["replacement_reason"],
        "role_artifact_path":role.path,"role_reload_status":if loaded{"complete"}else{"pending"},
        "activation_token":token,"instructions":instructions,"app_actions":actions,
        "role_instructions":if !loaded && !repeat {Some(sessions::context(repo,0,&role)?)}else{None},
        "acknowledgment_command":acknowledgment,"work_started":false}),
    )
}

pub(super) fn acknowledge_with_host(
    repo: &RepoRuntime,
    host: &mut impl SessionHost,
    caller: &str,
    token: &str,
) -> Result<JsonValue, String> {
    let mut saved = read(repo)?;
    if saved.is_null() || saved["session_id"] != caller || saved["token"] != token {
        return Err(
            "Activation acknowledgment does not match this calling conversation and offer".into(),
        );
    }
    let root = repo.authoritative_repo_root();
    let scope = host.recovery_scope()?;
    if saved["host_scope"] != scope {
        return Err("Native caller store changed; acknowledgment refused".into());
    }
    let role = roles::load(repo, 0)?;
    if saved["revision"] != role.revision
        || saved["role_digest"] != format!("{:x}", Sha256::digest(role.body.as_bytes()))
        || fs::read_to_string(root.join(&role.path)).map_err(|e| e.to_string())? != role.body
    {
        return Err(
            "Role changed after delivery; repeat enable to receive the current role".into(),
        );
    }
    let before = sessions::current(repo, 0)?;
    if saved["title"] != sessions::session_title(repo, &before, &role) {
        return Err("Activation title differs from the authoritative repository name; repeat enable to reconcile it".into());
    }
    let already_bound = !before.archived && before.session == caller && before.role == Some(0);
    if saved["expected_agent"] != agent_state(&before)
        && !(saved["phase"] != "offered"
            && already_bound
            && saved["expected_agent"]["name"] == before.name)
    {
        return Err("Owner changed after role delivery; acknowledgment refused".into());
    }
    verify_metadata(&host.activation_metadata(caller, &root)?, &saved)?;
    if host.activation_project(&root)? != saved["project_id"] {
        return Err("Native project changed; repeat enable before acknowledging".into());
    }
    if !already_bound {
        verify_former_owner(host, &root, &saved)?;
    }
    sessions::verify(repo, 0, &before, &role)?;
    // This journal records explicit caller receipt, never private-host injection.
    // A retry can finish the CAS/receipt steps without repeating the role body.
    saved["phase"] = json!("acknowledged");
    fence(&path(repo), &saved.to_string())?;
    fence(
        &root.join(".ait/team-session-bootstrap/agent-0.host"),
        &scope,
    )?;
    single_team::assign(
        &repo.binary_db_stores::<1>().local_db(),
        0,
        &before,
        0,
        caller,
    )
    .map_err(|e| e.to_string())?;
    let mut expected = before;
    expected.session = caller.into();
    expected.role = Some(0);
    sessions::verify(repo, 0, &expected, &role)?;
    verify_metadata(&host.activation_metadata(caller, &root)?, &saved)?;
    let mut loaded = sessions::loaded_record(&root, 0, caller, &role);
    loaded["delivery_method"] = json!("caller_acknowledged");
    loaded["activation_token"] = json!(token);
    fence(&sessions::loaded_path(&root, 0), &loaded.to_string())?;
    saved["phase"] = json!("complete");
    fence(&path(repo), &saved.to_string())?;
    Ok(
        json!({"status":"ready","action":"activated_current","agent_index":0,"session_id":caller,
        "previous_session_id":saved["previous_session_id"],"replacement_reason":saved["replacement_reason"],
        "title":saved["title"],"project_id":saved["project_id"],"role_revision_index":role.revision,
        "role_reload_status":"complete","delivery_method":"caller_acknowledged","work_started":false}),
    )
}
