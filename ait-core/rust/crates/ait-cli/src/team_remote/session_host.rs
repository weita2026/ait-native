//! Native host boundary for provisioning idle threads, never executing work.
use ait_core::json_support::{json, JsonValue};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

mod project;

pub enum RegistrationState {
    Available(JsonValue),
    Archived,
    Missing,
}

pub trait SessionHost {
    /// Observe a former owner without acquiring its writer or changing its lifecycle.
    fn registration_state(&mut self, id: &str, cwd: &Path) -> Result<RegistrationState, String> {
        self.activation_metadata(id, cwd)
            .map(RegistrationState::Available)
    }
    /// Read persisted caller metadata without acquiring or interrupting its writer.
    fn activation_metadata(&mut self, _id: &str, _cwd: &Path) -> Result<JsonValue, String> {
        Err("Host cannot verify caller metadata".into())
    }
    fn activation_project(&mut self, _cwd: &Path) -> Result<String, String> {
        Err("Host cannot verify caller project".into())
    }
    fn set_activation_project(
        &mut self,
        _id: &str,
        _cwd: &Path,
        _project: &str,
    ) -> Result<(), String> {
        Err("Host cannot update persisted caller project metadata".into())
    }
    fn set_title(&mut self, _id: &str, _cwd: &Path, _title: &str) -> Result<(), String> {
        Err("Host cannot verify title update".into())
    }
    fn reuse(&mut self, id: &str, cwd: &Path, _title: &str) -> Result<String, String> {
        self.inspect(id, cwd)
    }
    fn acquire_idle(&mut self, _id: &str, _cwd: &Path) -> Result<(), String> {
        Err("Host has no verified exclusive reload support".into())
    }
    fn acquire_for_reload(&mut self, id: &str, cwd: &Path) -> Result<(), String> {
        self.acquire_idle(id, cwd)
    }
    fn inject_role(&mut self, _id: &str, _instructions: &str) -> Result<(), String> {
        Err("Host has no role reload support".into())
    }
    fn inspect(&mut self, id: &str, cwd: &Path) -> Result<String, String>;
    fn recovery_scope(&self) -> Result<String, String> {
        Err("Host has no stable local recovery scope".into())
    }
    fn verify_missing(&mut self, _id: &str) -> Result<(), String> {
        Err("Host cannot verify missing candidates".into())
    }

    fn create(
        &mut self,
        cwd: &Path,
        title: &str,
        instructions: &str,
        record_receipt: &mut dyn FnMut(&str) -> Result<(), String>,
    ) -> Result<String, String>;
    fn prepare_existing(
        &mut self,
        id: &str,
        cwd: &Path,
        title: &str,
        instructions: &str,
    ) -> Result<(), String>;
}

pub struct CodexSessionHost {
    child: Child,
    input: ChildStdin,
    output: Receiver<Result<JsonValue, String>>,
    next_id: u64,
    fresh_thread: Option<String>,
    reload_writers: Vec<String>,
    recovery_scope: Option<String>,
    query_deadline: Option<Instant>,
    contact_archives: Option<Result<std::collections::HashSet<String>, String>>,
}
impl CodexSessionHost {
    pub fn connect() -> Result<Self, String> {
        Self::connect_with_deadline(None)
    }
    pub(crate) fn connect_for_contacts() -> Result<Self, String> {
        Self::connect_with_deadline(Some(Instant::now() + Duration::from_secs(10)))
    }
    fn connect_with_deadline(query_deadline: Option<Instant>) -> Result<Self, String> {
        let home = std::env::var_os("CODEX_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|p| std::path::PathBuf::from(p).join(".codex"))
            })
            .ok_or("Cannot identify native Codex home")?;
        let mut child = Command::new("codex")
            .env("CODEX_HOME", &home)
            .args(["app-server", "--stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Could not start native Codex app-server: {e}"))?;
        let input = child.stdin.take().ok_or("Missing Codex stdin")?;
        let stdout = child.stdout.take().ok_or("Missing Codex stdout")?;
        let (sender, output) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let value = line.map_err(|e| e.to_string()).and_then(|line| {
                    crate::json_support::parse_value(&line, "Invalid Codex RPC response")
                });
                if sender.send(value).is_err() {
                    break;
                }
            }
        });
        let mut host = Self {
            child,
            input,
            output,
            next_id: 0,
            fresh_thread: None,
            reload_writers: Vec::new(),
            recovery_scope: None,
            query_deadline,
            contact_archives: None,
        };
        host.rpc(
            "initialize",
            json!({"clientInfo":{"name":"ait_team_bootstrap","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true,"explicitGatewayOauth":true}}),
        )?;
        writeln!(host.input, "{}", json!({"method":"initialized"})).map_err(|e| e.to_string())?;
        host.input.flush().map_err(|e| e.to_string())?;
        // A first startup may create its home. An unidentifiable store only
        // disables replacement recovery, not ordinary native provisioning.
        host.recovery_scope = home
            .canonicalize()
            .ok()
            .and_then(|p| local_recovery_scope(&p).ok());
        Ok(host)
    }
    fn rpc(&mut self, method: &str, params: JsonValue) -> Result<JsonValue, String> {
        let response = self.rpc_response(method, params)?;
        if let Some(error) = response.get("error") {
            return Err(format!("Codex {method} rejected: {error}"));
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| format!("Codex {method} returned no result"))
    }
    fn rpc_response(&mut self, method: &str, params: JsonValue) -> Result<JsonValue, String> {
        if self.query_deadline.is_some_and(|end| Instant::now() >= end) {
            return Err("Session availability query timed out; availability is unknown".into());
        }
        self.next_id += 1;
        let id = self.next_id;
        writeln!(
            self.input,
            "{}",
            json!({"id":id,"method":method,"params":params})
        )
        .and_then(|()| self.input.flush())
        .map_err(|e| format!("Codex {method} write failed: {e}"))?;
        let deadline = self
            .query_deadline
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(30));
        loop {
            let wait = deadline.saturating_duration_since(Instant::now());
            let response = self.output.recv_timeout(wait).map_err(|_| {
                format!("Codex {method} response unavailable; reconcile before retrying creation.")
            })??;
            if response.get("method").is_some() {
                if response.get("id").is_some() {
                    return Err("Codex requested host interaction; provisioning stopped without approving it.".into());
                }
                continue;
            }
            if response["id"] != json!(id) {
                continue;
            }
            return Ok(response);
        }
    }
    /// Read-only observation. Absence is meaningful only in the original store.
    pub(crate) fn contact_health(
        &mut self,
        id: &str,
        cwd: &Path,
        original_scope: Option<&str>,
    ) -> JsonValue {
        let observed = (|| -> Result<JsonValue, String> {
            let reply =
                self.rpc_response("thread/read", json!({"threadId":id,"includeTurns":false}))?;
            if self.contact_archived(id)? {
                return Ok(json!({"availability":"archived","host_status":"archived"}));
            }
            if reply.get("error").is_some() {
                validate_missing_reply(&reply, id)?;
                let scope = self.recovery_scope()?;
                if original_scope.filter(|s| !s.is_empty()) != Some(scope.as_str()) {
                    return Err("Original native store is unknown or different; absence cannot be established".into());
                }
                self.verify_missing(id)?;
                return Ok(
                    json!({"availability":"missing","host_status":"missing","detail":"Absent from the original native store at query time; binding retained."}),
                );
            }
            Ok(contact_read_health(&reply["result"], id, cwd))
        })();
        observed.unwrap_or_else(
            |error| json!({"availability":"unknown","host_status":"unknown","detail":error}),
        )
    }
    // thread/read can omit archive state. Cache inventory only for this query.
    fn contact_archived(&mut self, id: &str) -> Result<bool, String> {
        if self.contact_archives.is_none() {
            let inventory = (|| {
                let mut ids = std::collections::HashSet::new();
                let mut cursors = std::collections::HashSet::new();
                let mut cursor = JsonValue::Null;
                for _ in 0..100 {
                    let page = self.rpc("thread/list", json!({"cursor":cursor,"limit":100,
                        "archived":true,"useStateDbOnly":true,"modelProviders":[],
                        "sourceKinds":["cli","vscode","exec","appServer","subAgent","subAgentReview","subAgentCompact","subAgentThreadSpawn","subAgentOther","unknown"]}))?;
                    for row in page["data"]
                        .as_array()
                        .ok_or("Missing archived inventory")?
                    {
                        ids.insert(
                            row["id"]
                                .as_str()
                                .filter(|s| !s.is_empty())
                                .ok_or("Invalid archived identity")?
                                .to_owned(),
                        );
                    }
                    match page.get("nextCursor") {
                        Some(JsonValue::Null) => return Ok(ids),
                        Some(value) if value.as_str().is_some() => {
                            let next = value.as_str().unwrap();
                            if next.is_empty() || !cursors.insert(next.to_owned()) {
                                return Err("Invalid archived cursor".into());
                            }
                            cursor = value.clone();
                        }
                        _ => return Err("Missing archived cursor".into()),
                    }
                }
                Err("Archived inventory incomplete".into())
            })();
            self.contact_archives = Some(inventory);
        }
        self.contact_archives
            .as_ref()
            .unwrap()
            .as_ref()
            .map(|ids| ids.contains(id))
            .map_err(Clone::clone)
    }
}

fn contact_read_health(read: &JsonValue, id: &str, cwd: &Path) -> JsonValue {
    let thread = &read["thread"];
    if thread["id"].as_str().is_none() || thread["cwd"].as_str().is_none() {
        return json!({"availability":"unknown","host_status":"unknown","detail":"Incomplete native identity response"});
    }
    if thread["id"] != json!(id) || thread["cwd"] != json!(cwd) {
        return json!({"availability":"mismatch","host_status":"mismatch","detail":"Native identity or repository differs from the recorded binding"});
    }
    if thread["archived"] == json!(true) {
        return json!({"availability":"archived","host_status":"archived"});
    }
    match thread["status"]["type"].as_str() {
        Some(state @ ("idle" | "active" | "notLoaded")) => {
            json!({"availability":"available","host_status":state})
        }
        _ => {
            json!({"availability":"unknown","host_status":"unknown","detail":"Session exists but its runtime state is unknown"})
        }
    }
}
impl SessionHost for CodexSessionHost {
    fn registration_state(&mut self, id: &str, cwd: &Path) -> Result<RegistrationState, String> {
        let reply =
            self.rpc_response("thread/read", json!({"threadId":id,"includeTurns":false}))?;
        if reply.get("error").is_some() {
            validate_missing_reply(&reply, id)?;
            self.verify_missing(id)?;
            return Ok(RegistrationState::Missing);
        }
        let read = &reply["result"];
        if read["thread"]["id"] != json!(id) || read["thread"]["cwd"] != json!(cwd) {
            return Err(format!(
                "Session {id} identity/workspace mismatch; binding retained"
            ));
        }
        self.contact_archives = None;
        if read["thread"]["archived"] == true || self.contact_archived(id)? {
            return Ok(RegistrationState::Archived);
        }
        inspect_thread(read, id, cwd)?;
        Ok(RegistrationState::Available(read["thread"].clone()))
    }

    fn activation_metadata(&mut self, id: &str, cwd: &Path) -> Result<JsonValue, String> {
        let read = self.rpc("thread/read", json!({"threadId":id,"includeTurns":false}))?;
        inspect_thread(&read, id, cwd)?;
        self.contact_archives = None;
        if self.contact_archived(id)? {
            return Err(format!("Session {id} is archived; activation stopped"));
        }
        match read["thread"]["status"]["type"].as_str() {
            // A private reader sees another host's loaded caller as notLoaded.
            // It must never resume that caller to discover or change its state.
            Some("active" | "idle" | "notLoaded") => Ok(read["thread"].clone()),
            _ => Err(format!(
                "Session {id} runtime state is unknown; activation stopped"
            )),
        }
    }
    fn activation_project(&mut self, cwd: &Path) -> Result<String, String> {
        project::ensure_project(cwd, |method, params| self.rpc(method, params))
    }
    fn set_activation_project(
        &mut self,
        id: &str,
        cwd: &Path,
        project: &str,
    ) -> Result<(), String> {
        self.activation_metadata(id, cwd)?;
        // This endpoint patches stored metadata. Native app-server supports it
        // across writers; it neither resumes the caller nor changes its context.
        self.rpc(
            "thread/metadata/update",
            json!({"threadId":id,"projectId":project}),
        )?;
        if self.activation_metadata(id, cwd)?["projectId"] != project {
            return Err("Caller project readback differs".into());
        }
        Ok(())
    }
    fn set_title(&mut self, id: &str, cwd: &Path, title: &str) -> Result<(), String> {
        if !self.reload_writers.iter().any(|writer| writer == id) {
            return Err("Title update requires native writer ownership".into());
        }
        self.rpc("thread/name/set", json!({"threadId":id,"name":title}))?;
        let read = self.rpc("thread/read", json!({"threadId":id,"includeTurns":false}))?;
        inspect_thread(&read, id, cwd)?;
        if read["thread"]["name"] != title {
            return Err("Native title readback differs".into());
        }
        Ok(())
    }
    fn acquire_for_reload(&mut self, id: &str, cwd: &Path) -> Result<(), String> {
        let read = self.rpc("thread/read", json!({"threadId":id,"includeTurns":false}))?;
        if inspect_thread(&read, id, cwd)? == "active" {
            let full = self.rpc("thread/read", json!({"threadId":id,"includeTurns":true}))?;
            let turn = full["thread"]["turns"]
                .as_array()
                .and_then(|turns| turns.iter().rev().find(|t| t["status"] == "inProgress"))
                .and_then(|t| t["id"].as_str())
                .ok_or("Active turn identity unavailable; reload pending")?;
            self.rpc("turn/interrupt", json!({"threadId":id,"turnId":turn}))?;
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let stopped =
                    self.rpc("thread/read", json!({"threadId":id,"includeTurns":false}))?;
                if inspect_thread(&stopped, id, cwd)? == "idle" {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err("Turn interruption not confirmed; reload pending".into());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        self.acquire_idle(id, cwd)
    }
    fn reuse(&mut self, id: &str, cwd: &Path, title: &str) -> Result<String, String> {
        let read = self.rpc("thread/read", json!({"threadId":id,"includeTurns":false}))?;
        let state = inspect_thread(&read, id, cwd)?;
        let project_id = project::ensure_project(cwd, |method, params| self.rpc(method, params))?;
        if read["thread"]["name"] == json!(title)
            && read["thread"]["projectId"] == json!(project_id)
        {
            return Ok(state);
        }
        // Reconcile metadata only under native writer ownership, never inject
        // role context or start/fork a replacement for an already bound member.
        self.acquire_idle(id, cwd)?;
        self.rpc(
            "thread/metadata/update",
            json!({"threadId":id,"projectId":project_id}),
        )?;
        self.rpc("thread/name/set", json!({"threadId":id,"name":title}))?;
        let read = self.rpc("thread/read", json!({"threadId":id,"includeTurns":false}))?;
        validate_prepared_candidate(&read, id, cwd, title, &project_id)?;
        Ok("idle".into())
    }

    fn acquire_idle(&mut self, id: &str, cwd: &Path) -> Result<(), String> {
        // A private app-server owns no prior threads. Resume acquires the native
        // persistent writer; another host's active writer is an error. No override,
        // interrupt, archive, takeover, title change or model turn is requested.
        let read = self.rpc("thread/read", json!({"threadId":id,"includeTurns":false}))?;
        if read["thread"]["id"] != json!(id) || read["thread"]["cwd"] != json!(cwd) {
            return Err("Role reload identity/workspace mismatch".into());
        }
        validate_candidate_state(&read)?;
        let resumed = self.rpc("thread/resume", json!({"threadId":id,"excludeTurns":true}))?;
        validate_resumed_candidate(&resumed, id, cwd)?;
        let read = self.rpc("thread/read", json!({"threadId":id,"includeTurns":false}))?;
        validate_resumed_candidate(&read, id, cwd)?;
        self.reload_writers.push(id.to_owned());
        Ok(())
    }
    fn inject_role(&mut self, id: &str, instructions: &str) -> Result<(), String> {
        if !self.reload_writers.iter().any(|writer| writer == id) {
            return Err("Role reload requires this host's exclusive native writer".into());
        }
        self.rpc("thread/inject_items", json!({"threadId":id,"items":[{"type":"message","role":"developer","content":[{"type":"input_text","text":instructions}]}]}))?;
        Ok(())
    }
    fn recovery_scope(&self) -> Result<String, String> {
        self.recovery_scope
            .clone()
            .ok_or_else(|| "Native host recovery scope is unavailable".into())
    }
    fn verify_missing(&mut self, id: &str) -> Result<(), String> {
        let read = self.rpc_response("thread/read", json!({"threadId":id,"includeTurns":false}))?;
        validate_missing_reply(&read, id)?;
        // A read failure alone is insufficient. Include all sources, providers
        // and both archive states, without a cwd filter (the thread may move).
        for archived in [false, true] {
            let mut cursor = JsonValue::Null;
            let mut seen = std::collections::HashSet::new();
            let mut complete = false;
            for _ in 0..100 {
                let page = self.rpc("thread/list", json!({"cursor":cursor,"limit":100,
                    "archived":archived,"useStateDbOnly":true,"modelProviders":[],
                    "sourceKinds":["cli","vscode","exec","appServer","subAgent","subAgentReview","subAgentCompact","subAgentThreadSpawn","subAgentOther","unknown"]}))?;
                let rows = page["data"].as_array().ok_or("Missing thread list data")?;
                for row in rows {
                    let listed = row["id"].as_str().ok_or("Invalid thread list identity")?;
                    if listed == id {
                        return Err("Candidate still exists in native history; resume or reconcile it, never replace it".into());
                    }
                }
                match page.get("nextCursor") {
                    Some(JsonValue::Null) => {
                        complete = true;
                        break;
                    }
                    Some(value) if value.as_str().is_some() => {
                        let next = value.as_str().unwrap();
                        if next.is_empty() || !seen.insert(next.to_owned()) {
                            return Err("Invalid thread list cursor".into());
                        }
                        cursor = value.clone();
                    }
                    _ => return Err("Missing thread list cursor".into()),
                }
            }
            if !complete {
                return Err("Thread inventory incomplete; recovery stopped".into());
            }
        }
        // Repeat after inventory; a newly visible original invalidates recovery.
        let read = self.rpc_response("thread/read", json!({"threadId":id,"includeTurns":false}))?;
        validate_missing_reply(&read, id)
    }
    fn inspect(&mut self, id: &str, cwd: &Path) -> Result<String, String> {
        let read = self.rpc("thread/read", json!({"threadId":id,"includeTurns":false}))?;
        inspect_thread(&read, id, cwd)
    }

    fn create(
        &mut self,
        cwd: &Path,
        _title: &str,
        _instructions: &str,
        record_receipt: &mut dyn FnMut(&str) -> Result<(), String>,
    ) -> Result<String, String> {
        let project_id = project::ensure_project(cwd, |method, params| self.rpc(method, params))?;
        let response = self.rpc(
            "thread/start",
            // The role is injected durably only after the binding guard is
            // rechecked in prepare_existing. Seeding it here duplicates the
            // model-visible body and can expose an invalidated role too early.
            json!({"cwd":cwd,"projectId":project_id,"ephemeral":false}),
        )?;
        let id = response["thread"]["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("Native thread/start returned no identity")?
            .to_owned();
        record_receipt(&id)?;
        self.fresh_thread = Some(id.clone());
        if response["thread"]["cwd"] != json!(cwd) {
            return Err(format!(
                "Created thread {id}, but its workspace differs; reconcile before retrying"
            ));
        }
        Ok(id)
    }
    fn prepare_existing(
        &mut self,
        id: &str,
        cwd: &Path,
        title: &str,
        instructions: &str,
    ) -> Result<(), String> {
        let read = self.rpc("thread/read", json!({"threadId":id,"includeTurns":false}))?;
        if read["thread"]["id"] != json!(id) || read["thread"]["cwd"] != json!(cwd) {
            return Err(
                "Existing thread does not match the requested identity and workspace.".into(),
            );
        }
        validate_candidate_state(&read)?;
        // A thread started by this host is already loaded and may not have a
        // rollout yet. Resume requires a persisted rollout. Only skip it for
        // this exact host-local receipt; pending candidates still use resume.
        if self.fresh_thread.as_deref() != Some(id) {
            let resumed = self.rpc(
                "thread/resume",
                json!({"threadId":id,"cwd":cwd,"excludeTurns":true}),
            )?;
            validate_resumed_candidate(&resumed, id, cwd)?;
            let current = self.rpc("thread/read", json!({"threadId":id,"includeTurns":false}))?;
            validate_resumed_candidate(&current, id, cwd)?;
        }
        self.rpc("thread/inject_items", json!({"threadId":id,"items":[{"type":"message","role":"developer","content":[{"type":"input_text","text":instructions}]}]}))?;
        let project_id = project::ensure_project(cwd, |method, params| self.rpc(method, params))?;
        self.rpc(
            "thread/metadata/update",
            json!({"threadId":id,"projectId":project_id}),
        )?;
        self.rpc("thread/name/set", json!({"threadId":id,"name":title}))?;
        let read = self.rpc("thread/read", json!({"threadId":id,"includeTurns":false}))?;
        validate_prepared_candidate(&read, id, cwd, title, &project_id)?;
        Ok(())
    }
}
fn inspect_thread(read: &JsonValue, id: &str, cwd: &Path) -> Result<String, String> {
    if read["thread"]["id"] != json!(id) || read["thread"]["cwd"] != json!(cwd) {
        return Err("Existing thread identity/workspace mismatch".into());
    }
    if read["thread"]["archived"] == json!(true) {
        return Err("Existing thread is archived; replacement is not supported".into());
    }
    match read["thread"]["status"]["type"].as_str() {
        Some(state @ ("active" | "idle" | "notLoaded")) => Ok(state.into()),
        _ => Err(format!(
            "Session {id} runtime state is unknown; operation stopped"
        )),
    }
}

/// One readback validates all mutated metadata and the still-idle identity.
fn validate_prepared_candidate(
    read: &JsonValue,
    id: &str,
    cwd: &Path,
    title: &str,
    project_id: &str,
) -> Result<(), String> {
    validate_resumed_candidate(read, id, cwd)?;
    if read["thread"]["name"] != json!(title) || read["thread"]["projectId"] != json!(project_id) {
        return Err("Native thread title/project readback did not match".into());
    }
    Ok(())
}
fn local_recovery_scope(home: &Path) -> Result<String, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(home).map_err(|e| e.to_string())?;
        if !meta.is_dir() {
            return Err("Codex home is not a directory".into());
        }
        Ok(format!("{}\n{}:{}", home.display(), meta.dev(), meta.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = home;
        Err("Native recovery requires a stable home identity on this platform".into())
    }
}
fn validate_resumed_candidate(read: &JsonValue, id: &str, cwd: &Path) -> Result<(), String> {
    if read["thread"]["id"] != json!(id) || read["thread"]["cwd"] != json!(cwd) {
        return Err(
            "Resumed candidate identity/workspace mismatch; no items or title changed".into(),
        );
    }
    validate_candidate_state(read)?;
    if read["thread"]["status"]["type"] != json!("idle") {
        return Err("Resumed candidate is not idle; no items or title changed".into());
    }
    Ok(())
}
fn validate_missing_reply(reply: &JsonValue, id: &str) -> Result<(), String> {
    let error = &reply["error"];
    if reply.get("result").is_none()
        && error["code"] == json!(-32600)
        && error["message"] == json!(format!("thread not loaded: {id}"))
    {
        Ok(())
    } else {
        Err(format!(
            "Native absence is not established; recovery stopped: {reply}"
        ))
    }
}
fn validate_candidate_state(read: &JsonValue) -> Result<(), String> {
    if read["thread"]["archived"] == json!(true) {
        return Err("Candidate is archived; no context or binding changed".into());
    }
    match read["thread"]["status"]["type"].as_str() {
        Some("idle" | "notLoaded") => Ok(()),
        _ => {
            Err("Candidate is active or its state is unknown; no context or binding changed".into())
        }
    }
}

impl Drop for CodexSessionHost {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contacts_distinguish_available_archived_mismatch_and_unknown() {
        let cwd = Path::new("/repo");
        for state in ["idle", "active", "notLoaded"] {
            let read = json!({"thread":{"id":"s","cwd":"/repo","status":{"type":state}}});
            assert_eq!(
                contact_read_health(&read, "s", cwd)["availability"],
                json!("available")
            );
            assert_eq!(
                contact_read_health(&read, "s", cwd)["host_status"],
                json!(state)
            );
        }
        let archived = json!({"thread":{"id":"s","cwd":"/repo","archived":true}});
        assert_eq!(
            contact_read_health(&archived, "s", cwd)["availability"],
            json!("archived")
        );
        assert_eq!(
            contact_read_health(&archived, "other", cwd)["availability"],
            json!("mismatch")
        );
        assert_eq!(
            contact_read_health(&archived, "s", Path::new("/different"))["availability"],
            json!("mismatch")
        );
        for read in [
            json!({}),
            json!({"thread":{"id":"s","cwd":"/repo","status":{"type":"systemError"}}}),
        ] {
            assert_eq!(
                contact_read_health(&read, "s", cwd)["availability"],
                json!("unknown")
            );
        }
    }
    #[test]
    fn resumed_candidate_requires_exact_identity_workspace_and_idle_state() {
        let root = Path::new("/test");
        let good = json!({"thread":{"id":"candidate","cwd":"/test","status":{"type":"idle"}}});
        assert!(validate_resumed_candidate(&good, "candidate", root).is_ok());
        assert!(validate_resumed_candidate(&good, "other", root).is_err());
        assert!(validate_resumed_candidate(&good, "candidate", Path::new("/other")).is_err());
        for state in ["active", "notLoaded", "unknown"] {
            let mut bad = good.clone();
            bad["thread"]["status"]["type"] = json!(state);
            assert!(validate_resumed_candidate(&bad, "candidate", root).is_err());
        }
        let mut archived = good;
        archived["thread"]["archived"] = json!(true);
        assert!(validate_resumed_candidate(&archived, "candidate", root).is_err());
    }
    #[test]
    fn combined_readback_rejects_metadata_identity_and_state_drift() {
        let root = Path::new("/repo");
        let good = json!({"thread":{"id":"s","cwd":"/repo","name":"title",
            "projectId":"p","status":{"type":"idle"}}});
        assert!(validate_prepared_candidate(&good, "s", root, "title", "p").is_ok());
        for (field, value) in [
            ("id", json!("other")),
            ("cwd", json!("/elsewhere")),
            ("name", json!("old")),
            ("projectId", json!("other")),
            ("archived", json!(true)),
            ("status", json!({"type":"active"})),
            ("status", json!({"type":"notLoaded"})),
            ("status", JsonValue::Null),
        ] {
            let mut bad = good.clone();
            bad["thread"][field] = value;
            assert!(validate_prepared_candidate(&bad, "s", root, "title", "p").is_err());
        }
    }
    #[cfg(unix)]
    #[test]
    fn local_scope_distinguishes_different_native_stores() {
        let a = tempfile::TempDir::new().unwrap();
        let b = tempfile::TempDir::new().unwrap();
        assert_ne!(
            local_recovery_scope(a.path()).unwrap(),
            local_recovery_scope(b.path()).unwrap()
        );
        assert_eq!(
            local_recovery_scope(a.path()).unwrap(),
            local_recovery_scope(a.path()).unwrap()
        );
    }
    #[test]
    fn absence_requires_exact_native_error_not_permission_or_transport_guess() {
        assert!(validate_missing_reply(
            &json!({"error":{"code":-32600,"message":"thread not loaded: lost"}}),
            "lost"
        )
        .is_ok());
        for reply in [
            json!({"result":{"thread":{"id":"lost"}}}),
            json!({"error":{"code":-32600,"message":"permission denied"}}),
            json!({"error":{"code":-32600,"message":"thread not loaded: other"}}),
            json!({"error":{"code":-32603,"message":"thread not loaded: lost"}}),
            json!({}),
        ] {
            assert!(validate_missing_reply(&reply, "lost").is_err());
        }
    }
    #[test]
    fn candidate_context_requires_known_safe_state() {
        for state in ["active", "systemError", "unknown"] {
            assert!(
                validate_candidate_state(&json!({"thread":{"status":{"type":state}}})).is_err()
            );
        }
        assert!(validate_candidate_state(&json!({"thread":{}})).is_err());
        assert!(validate_candidate_state(
            &json!({"thread":{"archived":true,"status":{"type":"idle"}}})
        )
        .is_err());
        for state in ["idle", "notLoaded"] {
            assert!(validate_candidate_state(&json!({"thread":{"status":{"type":state}}})).is_ok());
        }
    }
}
