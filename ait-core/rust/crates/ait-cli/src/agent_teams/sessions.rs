use super::roles::Role;
use super::*;

pub(super) fn session_title(repo: &RepoRuntime, agent: &Agent, role: &Role) -> String {
    format!("［{}］{}·{}", agent.name, role.title, repo.repo_name())
}

pub(super) fn read_json(path: &Path) -> Result<JsonValue, String> {
    match fs::read_to_string(path) {
        Ok(text) => {
            let r = crate::json_support::parse_value(&text, "Invalid Agent operation receipt")?;
            if !r.is_object() {
                return Err("Invalid Agent operation receipt object".into());
            }
            Ok(r)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(JsonValue::Null),
        Err(e) => Err(format!("Cannot read Agent operation receipt: {e}")),
    }
}
pub(super) fn loaded_receipt(root: &Path, index: u32) -> Result<JsonValue, String> {
    let r = read_json(&root.join(format!(
        ".ait/team-session-bootstrap/agent-{index}.role-loaded"
    )))?;
    if !r.is_null()
        && (r["version"] != 1
            || r["repository"] != json!(root)
            || r["agent_index"] != index
            || !matches!(
                r["phase"].as_str(),
                Some("injecting" | "loaded" | "complete")
            ))
    {
        return Err("Invalid native role delivery evidence".into());
    }
    Ok(r)
}
pub(super) fn receipt_matches(r: &JsonValue, a: &Agent, index: u32, role: &Role) -> bool {
    r["version"] == 1
        && r["agent_index"] == index
        && r["session_id"] == a.session
        && r["role_index"] == role.index
        && r["revision"] == role.revision
        && r["phase"] == "complete"
}
pub(super) fn context(repo: &RepoRuntime, index: u32, role: &Role) -> Result<String, String> {
    Ok(format!("AIT Agent Teams. Repository: {}. Agent index: {index}. Role index: {}. PlanRevision: {}.\nApply the complete role and repository guidance supplied below without fetching the same files again. This role supersedes prior role instructions. Preserve this session and existing Task ownership. A new Task alone requires no role reload or session ensure. After compaction re-read applicable AGENTS.md, this complete role MD and the current work order. Loaded-role metadata records delivery, not retained context. No new work is assigned by this delivery.\nRole MD:\n{}\nRepository AGENTS.md (permissions take precedence):\n{}",repo.authoritative_repo_root().display(),role.index,role.revision,role.body,fs::read_to_string(repo.authoritative_repo_root().join("AGENTS.md")).map_err(|e|e.to_string())?))
}
pub(super) fn current(repo: &RepoRuntime, index: u32) -> Result<Agent, String> {
    single_team::read(&repo.binary_db_stores::<1>().local_db())
        .map_err(|e| e.to_string())?
        .get(index as usize)
        .cloned()
        .ok_or_else(|| "Unknown Agent".into())
}
pub(super) fn verify(
    repo: &RepoRuntime,
    index: u32,
    expected: &Agent,
    role: &Role,
) -> Result<(), String> {
    if current(repo, index)? != *expected {
        return Err("Agent binding changed during preparation".into());
    }
    if roles::load(repo, role.index)?.revision != role.revision {
        return Err("Role Plan head changed; retry with current revision".into());
    }
    Ok(())
}
pub(super) fn loaded_path(root: &Path, index: u32) -> std::path::PathBuf {
    root.join(format!(
        ".ait/team-session-bootstrap/agent-{index}.role-loaded"
    ))
}
pub(super) fn loaded_record(root: &Path, index: u32, session: &str, role: &Role) -> JsonValue {
    json!({"repository":root,"version":1,"agent_index":index,"session_id":session,"role_index":role.index,"artifact_path":role.path,"revision":role.revision,"phase":"complete"})
}

pub(super) fn ensure_with_host(
    repo: &RepoRuntime,
    host: &mut impl SessionHost,
    index: u32,
    requested: Option<u32>,
) -> Result<JsonValue, String> {
    if !enabled(repo) {
        return Err("Agent Teams is disabled".into());
    }
    if index == 0 && super::activation::pending(repo)?.is_some() {
        return Err("Caller activation is pending; complete its native acknowledgment before session ensure".into());
    }
    let root = repo.authoritative_repo_root();
    let op = root.join(".ait/team-session-bootstrap");
    fs::create_dir_all(&op).map_err(|e| e.to_string())?;
    let before = current(repo, index)?;
    if before.archived {
        return Err("Archived Agent cannot be assigned".into());
    }
    let role_index = requested
        .or(before.role.filter(|_| !before.session.is_empty()))
        .or_else(|| {
            single_team::BASE_SEATS
                .iter()
                .find(|(id, _)| *id == index)
                .map(|(_, r)| *r)
        })
        .ok_or("Specify --role for a new member")?;
    if (index == 0 && role_index != 0)
        || (index == 1 && role_index != 2)
        || (role_index == 0 && index != 0)
    {
        return Err("Newton and Einstein retain their fixed responsibilities".into());
    }
    if role_index == 2 && !registered(repo)? {
        return Err("Reviewer is pending ait-server registration".into());
    }
    let role = roles::load(repo, role_index)?;
    let title = session_title(repo, &before, &role);
    let pending = op.join(format!("agent-{index}.pending"));
    let request_path = op.join(format!("agent-{index}.assignment"));
    let host_path = op.join(format!("agent-{index}.host"));
    let retired_path = op.join(format!("agent-{index}.retired"));
    let loaded = loaded_receipt(&root, index)?;
    let mut saved = read_json(&request_path)?;
    if !saved.is_null()
        && (saved["version"] != 1
            || !matches!(
                saved["phase"].as_str(),
                Some("created" | "injecting" | "loaded")
            ))
    {
        return Err("Invalid pending assignment evidence".into());
    }
    let mut candidate = match fs::read_to_string(&pending) {
        Ok(id) => Some(id),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => saved["session_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .map(str::to_owned),
        Err(e) => return Err(format!("Cannot read pending candidate: {e}")),
    };
    if let Some(id) = candidate.as_deref() {
        if saved["session_id"]
            .as_str()
            .is_some_and(|saved| saved != id)
        {
            return Err("Pending candidate and assignment identities differ".into());
        }
    }
    if (!before.session.is_empty()
        || candidate.is_some()
        || !saved.is_null()
        || retired_path.exists())
        && fs::read_to_string(&host_path).map_err(|_| "Pending candidate host evidence missing")?
            != host.recovery_scope()?
    {
        return Err("Pending candidate belongs to another native store; no session changed".into());
    }
    let recovering = if before.session.is_empty() {
        false
    } else if let Err(error) = host.inspect(&before.session, &root) {
        host.verify_missing(&before.session).map_err(|proof| {
            format!("Session cannot be reused ({error}); absence is unproven: {proof}")
        })?;
        true
    } else {
        false
    };
    let mut retired = None;
    if before.session.is_empty() || recovering {
        if let Some(id) = candidate.as_deref() {
            if id.is_empty() {
                return Err("Native creation outcome unknown; no duplicate session".into());
            }
            if (recovering && id == before.session) || host.inspect(id, &root).is_err() {
                host.verify_missing(id)?;
                verify(repo, index, &before, &role)?;
                fence(&retired_path, id)?;
                // Keep the known ID until stale assignment removal succeeds.
                // A crash between these removals can safely repeat absence proof.
                if request_path.exists() {
                    fs::remove_file(&request_path).map_err(|e| e.to_string())?;
                }
                if pending.exists() {
                    fs::remove_file(&pending).map_err(|e| e.to_string())?;
                }
                retired = candidate.take();
                saved = JsonValue::Null;
            }
        }
    }
    if !before.session.is_empty() && !recovering {
        if fs::read_to_string(&host_path).map_err(|_| "Original native host evidence missing")?
            != host.recovery_scope()?
        {
            return Err("Native host store changed; binding retained".into());
        }
        if let Some(id) = candidate.as_deref() {
            if id != before.session {
                return Err("Pending candidate conflicts with valid binding".into());
            }
        }

        verify(repo, index, &before, &role)?;
        let completed_assignment = saved["phase"] == "loaded"
            && saved["session_id"] == before.session
            && saved["role_index"] == role_index
            && saved["revision"] == role.revision
            && before.role == Some(role_index);
        if completed_assignment {
            fence(
                &loaded_path(&root, index),
                &loaded_record(&root, index, &before.session, &role).to_string(),
            )?;
        } else if before.role != Some(role_index)
            || !receipt_matches(&loaded, &before, index, &role)
        {
            reload_one(repo, host, index, &before, &role)?;
        }
        if pending.exists() {
            fs::remove_file(&pending).map_err(|e| e.to_string())?;
        }
        if completed_assignment && request_path.exists() {
            fs::remove_file(&request_path).map_err(|e| e.to_string())?;
        }
        let state = host.reuse(&before.session, &root, &title)?;
        let mut expected = before.clone();
        expected.role = Some(role_index);
        verify(repo, index, &expected, &role)?;
        return Ok(
            json!({"status":"ready","action":"reused","agent_index":index,"name":before.name,"title":title,"session_id":before.session,"role_index":role_index,"role_revision_index":role.revision,"host_status":state,"work_started":false}),
        );
    }
    if retired.is_none() {
        retired = match fs::read_to_string(&retired_path) {
            Ok(id) if !id.is_empty() => Some(id),
            Ok(_) => return Err("Invalid retired candidate evidence".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("Cannot read retired candidate: {e}")),
        };
    }
    if !saved.is_null() {
        if saved["version"] != 1
            || saved["expected_session"] != before.session
            || saved["role_index"] != role_index
            || saved["revision"] != role.revision
        {
            return Err("Pending assignment context changed; reconcile before retrying".into());
        }
        if saved["phase"] == "injecting" {
            return Err("Native role injection outcome unknown; no repeated injection".into());
        }
    }
    if let Some(retired) = &retired {
        host.verify_missing(retired)?;
    }
    let mut request = json!({"version":1,"expected_session":before.session,"role_index":role_index,"revision":role.revision,"phase":"created"});
    let id = if let Some(id) = candidate {
        if id.is_empty() {
            return Err("Native creation outcome unknown; no duplicate session".into());
        }
        // Recover the candidate fence from a saved, confirmed assignment too.
        if !pending.exists() {
            fence(&pending, &id)?;
        }
        id
    } else {
        verify(repo, index, &before, &role)?;
        fence(&host_path, &host.recovery_scope()?)?;
        fence(&request_path, &request.to_string())?;
        fence(&pending, "")?;
        host.create(&root, &title, "", &mut |id| fence(&pending, id))?
    };
    if id == before.session || fs::read_to_string(&retired_path).ok().as_deref() == Some(&id) {
        return Err("Host returned a retired identity".into());
    }
    let agents =
        single_team::read(&repo.binary_db_stores::<1>().local_db()).map_err(|e| e.to_string())?;
    if agents
        .iter()
        .enumerate()
        .any(|(i, a)| i != index as usize && a.session == id)
    {
        return Err("Candidate belongs to another Agent".into());
    }
    verify(repo, index, &before, &role)?;
    if let Some(retired) = &retired {
        host.verify_missing(retired)?;
    }
    if recovering {
        host.verify_missing(&before.session)?;
    }
    let prepared = saved["phase"] == "loaded" && saved["session_id"] == id;
    if !prepared {
        request["session_id"] = json!(id);
        request["phase"] = json!("injecting");
        fence(&request_path, &request.to_string())?;
        host.prepare_existing(&id, &root, &title, &context(repo, index, &role)?)?;
        request["phase"] = json!("loaded");
        fence(&request_path, &request.to_string())?;
    }
    verify(repo, index, &before, &role)?;
    if let Some(retired) = &retired {
        host.verify_missing(retired)?;
    }
    if recovering {
        host.verify_missing(&before.session)?;
    }
    single_team::assign(
        &repo.binary_db_stores::<1>().local_db(),
        index,
        &before,
        role_index,
        &id,
    )
    .map_err(|e| e.to_string())?;
    if roles::load(repo, role_index)?.revision != role.revision {
        return Err("Role advanced after assignment; binding retained and reload pending".into());
    }
    fence(
        &loaded_path(&root, index),
        &loaded_record(&root, index, &id, &role).to_string(),
    )?;
    fs::remove_file(&pending).map_err(|e| e.to_string())?;
    if request_path.exists() {
        fs::remove_file(&request_path).map_err(|e| e.to_string())?;
    }
    Ok(
        json!({"status":"ready","action":if recovering || retired.is_some(){"recovered"}else if prepared{"completed_pending"}else{"created"},"agent_index":index,"name":before.name,"title":title,"session_id":id,"role_index":role_index,"role_revision_index":role.revision,"binding_updated":true,"work_started":false}),
    )
}

pub(super) fn reload_one(
    repo: &RepoRuntime,
    host: &mut impl SessionHost,
    index: u32,
    before: &Agent,
    role: &Role,
) -> Result<(), String> {
    let root = repo.authoritative_repo_root();
    let path = loaded_path(&root, index);
    let prior = loaded_receipt(&root, index)?;
    if prior["phase"] == "injecting" {
        return Err("Role reload outcome unknown; reconcile without reinjection".into());
    }
    if receipt_matches(&prior, before, index, role) {
        return Ok(());
    }
    let original =
        fs::read_to_string(root.join(format!(".ait/team-session-bootstrap/agent-{index}.host")))
            .map_err(|_| "Original native host evidence missing")?;
    if original != host.recovery_scope()? {
        return Err("Native host store changed; reload pending".into());
    }
    host.acquire_for_reload(&before.session, &root)?;
    host.set_title(&before.session, &root, &session_title(repo, before, role))?;
    if prior["phase"] == "loaded"
        && prior["session_id"] == before.session
        && prior["role_index"] == role.index
        && prior["revision"] == role.revision
    {
        verify(repo, index, before, role)?;
        single_team::assign(
            &repo.binary_db_stores::<1>().local_db(),
            index,
            before,
            role.index,
            &before.session,
        )
        .map_err(|e| e.to_string())?;
        fence(
            &path,
            &loaded_record(&root, index, &before.session, role).to_string(),
        )?;
        return Ok(());
    }
    verify(repo, index, before, role)?;
    let mut receipt = loaded_record(&root, index, &before.session, role);
    receipt["phase"] = json!("injecting");
    fence(&path, &receipt.to_string())?;
    host.inject_role(&before.session, &context(repo, index, role)?)?;
    receipt["phase"] = json!("loaded");
    fence(&path, &receipt.to_string())?;
    verify(repo, index, before, role)?;
    single_team::assign(
        &repo.binary_db_stores::<1>().local_db(),
        index,
        before,
        role.index,
        &before.session,
    )
    .map_err(|e| e.to_string())?;
    receipt["phase"] = json!("complete");
    fence(&path, &receipt.to_string())?;
    Ok(())
}
