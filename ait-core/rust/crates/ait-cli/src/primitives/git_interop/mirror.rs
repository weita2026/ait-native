use super::*;

const MIRROR_CONTRACT: &str = "git-mirror-operation/v1";
const MIRROR_MAPPING_KIND: &str = "mirror_ref";

#[derive(Clone, Copy, Debug, Default)]
struct MirrorExecutionControl {
    interrupt_after_object_transfer: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MirrorDirection {
    Inbound,
    Outbound,
    Bidirectional,
}

impl MirrorDirection {
    fn parse(value: &str) -> Result<Self, String> {
        match value.trim() {
            "inbound" => Ok(Self::Inbound),
            "outbound" => Ok(Self::Outbound),
            "bidirectional" => Ok(Self::Bidirectional),
            other => Err(format!(
                "Unsupported Git mirror direction {other:?}; expected inbound, outbound, or bidirectional."
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Inbound => "inbound",
            Self::Outbound => "outbound",
            Self::Bidirectional => "bidirectional",
        }
    }

    fn permits_inbound(self) -> bool {
        matches!(self, Self::Inbound | Self::Bidirectional)
    }

    fn permits_outbound(self) -> bool {
        matches!(self, Self::Outbound | Self::Bidirectional)
    }
}

#[derive(Clone, Debug)]
struct MirrorEndpoint {
    requested: String,
    identity: String,
    fingerprint: String,
    object_format: String,
    source: Option<SourceInfo>,
    target: Option<TargetInfo>,
}

#[derive(Clone, Debug)]
struct MirrorRefState {
    git_ref_name: String,
    ait_kind: String,
    ait_name: String,
    git_object_id: Option<String>,
    ait_ref: Option<ExportRef>,
    ait_state_id: Option<String>,
    previous_git_object_id: Option<String>,
    previous_ait_state_id: Option<String>,
    classification: String,
}

impl MirrorRefState {
    fn payload(&self) -> JsonValue {
        json!({
            "git_ref_name": self.git_ref_name,
            "ait_kind": self.ait_kind,
            "ait_name": self.ait_name,
            "git_object_id": self.git_object_id,
            "snapshot_id": self.ait_ref.as_ref().map(|row| row.snapshot_id.as_str()),
            "ait_identity": self.ait_ref.as_ref().and_then(|row| row.ait_identity.as_deref()),
            "ait_state_id": self.ait_state_id,
            "previous_git_object_id": self.previous_git_object_id,
            "previous_ait_state_id": self.previous_ait_state_id,
            "classification": self.classification,
        })
    }
}

#[derive(Clone, Debug, Default)]
struct MirrorCounts {
    equal: usize,
    inbound_only: usize,
    outbound_only: usize,
    divergent: usize,
}

impl MirrorCounts {
    fn from_states(states: &[MirrorRefState]) -> Self {
        let mut counts = Self::default();
        for state in states {
            match state.classification.as_str() {
                "equal" => counts.equal += 1,
                "inbound_only" => counts.inbound_only += 1,
                "outbound_only" => counts.outbound_only += 1,
                "divergent" => counts.divergent += 1,
                _ => {}
            }
        }
        counts
    }

    fn state(&self) -> &'static str {
        if self.divergent > 0 {
            "divergent"
        } else if self.inbound_only > 0 && self.outbound_only > 0 {
            "mixed"
        } else if self.inbound_only > 0 {
            "inbound_only"
        } else if self.outbound_only > 0 {
            "outbound_only"
        } else {
            "equal"
        }
    }
}

impl InteropStore {
    fn mirror_staging_repository(&self, endpoint_fingerprint: &str) -> PathBuf {
        self.root
            .join("mirror-staging")
            .join(format!("{}.git", endpoint_fingerprint.to_ascii_lowercase()))
    }
}

pub fn git_mirror(
    repo: &RepoRuntime,
    endpoint: &str,
    direction: &str,
    dry_run: bool,
) -> Result<JsonValue, String> {
    git_mirror_with_control(
        repo,
        endpoint,
        direction,
        dry_run,
        MirrorExecutionControl::default(),
    )
}

fn git_mirror_with_control(
    repo: &RepoRuntime,
    endpoint: &str,
    direction: &str,
    dry_run: bool,
    control: MirrorExecutionControl,
) -> Result<JsonValue, String> {
    let direction = MirrorDirection::parse(direction)?;
    let endpoint = inspect_mirror_endpoint(endpoint, direction)?;
    if endpoint.object_format != OBJECT_FORMAT_SHA1 {
        return Ok(json!({
            "contract": MIRROR_CONTRACT,
            "operation": "mirror",
            "status": "blocked",
            "direction": direction.as_str(),
            "endpoint": endpoint.identity,
            "endpoint_repository_fingerprint": endpoint.fingerprint,
            "git_object_format": endpoint.object_format,
            "supported_object_formats": [OBJECT_FORMAT_SHA1],
            "state": "unsupported",
            "blockers": [{
                "kind": "unsupported_object_format",
                "count": 1,
                "disposition": "fail_closed",
            }],
            "dry_run": dry_run,
            "mutated": false,
        }));
    }

    let interop = InteropStore::new(repo);
    let mappings = interop.load_mappings()?;
    let ait_refs = collect_ait_refs(repo)?;
    let git_refs = collect_git_refs(endpoint.source.as_ref());
    let states = classify_mirror_refs(&endpoint, &mappings, &ait_refs, &git_refs)?;
    let counts = MirrorCounts::from_states(&states);
    let mut blockers = Vec::new();
    if counts.divergent > 0 {
        blockers.push(json!({
            "kind": "bidirectional_divergence",
            "count": counts.divergent,
            "disposition": "explicit_merge_or_rebase_required",
            "refs": state_names(&states, "divergent"),
        }));
    }
    if !direction.permits_inbound() && counts.inbound_only > 0 {
        blockers.push(json!({
            "kind": "inbound_changes_in_outbound_mirror",
            "count": counts.inbound_only,
            "disposition": "change_direction_or_reconcile_explicitly",
            "refs": state_names(&states, "inbound_only"),
        }));
    }
    if !direction.permits_outbound() && counts.outbound_only > 0 {
        blockers.push(json!({
            "kind": "outbound_changes_in_inbound_mirror",
            "count": counts.outbound_only,
            "disposition": "change_direction_or_reconcile_explicitly",
            "refs": state_names(&states, "outbound_only"),
        }));
    }

    let state_material = states
        .iter()
        .map(|state| {
            format!(
                "{} git={} ait={} previous_git={} previous_ait={} state={}",
                state.git_ref_name,
                state.git_object_id.as_deref().unwrap_or("none"),
                state.ait_state_id.as_deref().unwrap_or("none"),
                state.previous_git_object_id.as_deref().unwrap_or("none"),
                state.previous_ait_state_id.as_deref().unwrap_or("none"),
                state.classification,
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let plan_hash = sha256_prefixed(
        "GMP",
        format!(
            "{}\n{}\n{}\n{}\n",
            endpoint.fingerprint,
            direction.as_str(),
            endpoint.object_format,
            state_material
        )
        .as_bytes(),
        24,
    );
    let generation_id = sha256_prefixed(
        "GIT-MIR",
        format!("{}\n{plan_hash}", endpoint.fingerprint).as_bytes(),
        16,
    );
    let operation_id = sha256_prefixed(
        "GIO-MIRROR",
        format!(
            "{}\n{}\n{plan_hash}",
            endpoint.fingerprint,
            direction.as_str()
        )
        .as_bytes(),
        16,
    );

    if blockers.is_empty() && counts.inbound_only > 0 {
        if let Some(source) = endpoint.source.as_ref() {
            let validation = git_import(repo, &source.source, true, true)?;
            if json_text(&validation, "status") == Some("blocked") {
                blockers.extend(
                    validation
                        .get("blockers")
                        .and_then(JsonValue::as_array)
                        .cloned()
                        .unwrap_or_default(),
                );
            }
        }
    }
    if blockers.is_empty() && counts.outbound_only > 0 {
        let target = endpoint
            .target
            .as_ref()
            .ok_or_else(|| "Outbound Git mirror requires a local Git target path.".to_string())?;
        let validation = git_export(repo, &target.requested, true, true)?;
        if json_text(&validation, "status") == Some("blocked") {
            blockers.extend(
                validation
                    .get("blockers")
                    .and_then(JsonValue::as_array)
                    .cloned()
                    .unwrap_or_default(),
            );
        }
    }

    let base_report = mirror_report(
        &endpoint,
        direction,
        &operation_id,
        &generation_id,
        &plan_hash,
        &states,
        &counts,
    );
    if !blockers.is_empty() {
        return Ok(with_report_fields(
            base_report,
            &[
                ("status", JsonValue::String("blocked".to_string())),
                ("blockers", JsonValue::Array(blockers)),
                ("requires_decision", JsonValue::Bool(true)),
                ("dry_run", JsonValue::Bool(dry_run)),
                ("mutated", JsonValue::Bool(false)),
            ],
        ));
    }
    if dry_run {
        return Ok(with_report_fields(
            base_report,
            &[
                ("status", JsonValue::String("dry_run".to_string())),
                ("blockers", JsonValue::Array(Vec::new())),
                ("requires_decision", JsonValue::Bool(false)),
                ("dry_run", JsonValue::Bool(true)),
                ("mutated", JsonValue::Bool(false)),
            ],
        ));
    }

    let (completed, resumed) = existing_mirror_checkpoint(&interop, &operation_id, &plan_hash)?;
    if let Some(mut result) = completed {
        if let Some(object) = result.as_object_mut() {
            object.insert("status".to_string(), JsonValue::String("no_op".to_string()));
            object.insert("replayed".to_string(), JsonValue::Bool(true));
            object.insert("mutated".to_string(), JsonValue::Bool(false));
        }
        return Ok(result);
    }
    write_mirror_checkpoint(
        &interop,
        &operation_id,
        &generation_id,
        &plan_hash,
        "running",
        "planned",
        None,
    )?;

    let apply_inbound = counts.inbound_only > 0 && direction.permits_inbound();
    let apply_outbound = counts.outbound_only > 0 && direction.permits_outbound();
    let mut outbound_transfer = None;
    if apply_inbound {
        let source = endpoint
            .source
            .as_ref()
            .ok_or_else(|| "Inbound Git mirror requires a readable Git source.".to_string())?;
        prepare_retained_repository(&interop.retained_repository(&source.fingerprint), source)?;
    }
    if apply_outbound {
        outbound_transfer = Some(stage_outbound_ref_set(repo, &interop, &endpoint, &states)?);
    }
    write_mirror_checkpoint(
        &interop,
        &operation_id,
        &generation_id,
        &plan_hash,
        "running",
        "objects_transferred",
        None,
    )?;
    if control.interrupt_after_object_transfer {
        return Err(format!(
            "Injected Git mirror interruption after object transfer for {operation_id}; rerun the same mirror command to resume before public ref movement."
        ));
    }

    let mut child_results = Vec::new();
    if apply_inbound {
        let source = endpoint.source.as_ref().unwrap();
        child_results.push(run_resumable_import(repo, &source.source)?);
        apply_inbound_deletions(repo, &states)?;
    }
    if let Some(transfer) = outbound_transfer.as_ref() {
        apply_outbound_ref_transaction(repo, &endpoint, &states, transfer)?;
        child_results.push(transfer.export_result.clone());
    }
    write_mirror_checkpoint(
        &interop,
        &operation_id,
        &generation_id,
        &plan_hash,
        "running",
        "refs_moved",
        None,
    )?;

    let final_source = inspect_source(&mirror_source_argument(&endpoint)?)?;
    let final_git_refs = collect_git_refs(Some(&final_source));
    let final_ait_refs = collect_ait_refs(repo)?;
    let last_mirrored_heads = record_final_mirror_state(
        &interop,
        &endpoint,
        direction,
        &operation_id,
        &generation_id,
        &states,
        &final_git_refs,
        &final_ait_refs,
    )?;

    let status = if apply_inbound || apply_outbound {
        "completed"
    } else {
        "no_op"
    };
    let result = with_report_fields(
        base_report,
        &[
            ("status", JsonValue::String(status.to_string())),
            ("state", JsonValue::String("equal".to_string())),
            ("blockers", JsonValue::Array(Vec::new())),
            ("requires_decision", JsonValue::Bool(false)),
            ("dry_run", JsonValue::Bool(false)),
            ("resumed", JsonValue::Bool(resumed)),
            ("replayed", JsonValue::Bool(false)),
            ("mutated", JsonValue::Bool(apply_inbound || apply_outbound)),
            ("mapping_updated", JsonValue::Bool(true)),
            ("compare_and_swap", JsonValue::Bool(true)),
            ("force_updated", JsonValue::Bool(false)),
            ("child_operations", JsonValue::Array(child_results)),
            ("last_mirrored_heads", JsonValue::Array(last_mirrored_heads)),
        ],
    );
    write_mirror_checkpoint(
        &interop,
        &operation_id,
        &generation_id,
        &plan_hash,
        "completed",
        "completed",
        Some(result.clone()),
    )?;
    Ok(result)
}

fn inspect_mirror_endpoint(
    requested: &str,
    direction: MirrorDirection,
) -> Result<MirrorEndpoint, String> {
    let requested = requested.trim();
    if requested.is_empty() {
        return Err("Git mirror endpoint must not be empty.".to_string());
    }
    let (source, target, identity, object_format) = match direction {
        MirrorDirection::Inbound => {
            let source = inspect_source(requested)?;
            let identity = source.source_identity.clone();
            let object_format = source.object_format.clone();
            (Some(source), None, identity, object_format)
        }
        MirrorDirection::Outbound | MirrorDirection::Bidirectional => {
            let target = inspect_target(requested)?;
            let source = if target.existed {
                Some(inspect_source(target.path.to_string_lossy().as_ref())?)
            } else {
                None
            };
            let identity = target.path.to_string_lossy().to_string();
            let object_format = target.object_format.clone();
            (source, Some(target), identity, object_format)
        }
    };
    let fingerprint = sha256_prefixed(
        "GMR",
        format!("git-mirror-endpoint/v1\n{object_format}\n{identity}\n").as_bytes(),
        24,
    );
    Ok(MirrorEndpoint {
        requested: requested.to_string(),
        identity,
        fingerprint,
        object_format,
        source,
        target,
    })
}

fn collect_git_refs(source: Option<&SourceInfo>) -> BTreeMap<String, String> {
    source
        .into_iter()
        .flat_map(|source| source.refs.iter())
        .filter(|row| row.name.starts_with("refs/heads/") || row.name.starts_with("refs/tags/"))
        .map(|row| (row.name.clone(), row.object_id.clone()))
        .collect()
}

fn collect_ait_refs(repo: &RepoRuntime) -> Result<BTreeMap<String, ExportRef>, String> {
    Ok(selected_export_refs(repo, true)?
        .into_iter()
        .map(|row| (row.git_ref_name.clone(), row))
        .collect())
}

fn ait_ref_state_id(reference: &ExportRef) -> String {
    sha256_prefixed(
        "AIR",
        format!(
            "ait-git-ref-state/v1\n{}\n{}\n{}\n{}\n{}\n{}\n",
            reference.git_ref_name,
            reference.snapshot_id,
            reference.ait_kind,
            reference.ait_name,
            reference.message.as_deref().unwrap_or("none"),
            reference.created_at.as_deref().unwrap_or("none"),
        )
        .as_bytes(),
        24,
    )
}

fn classify_mirror_refs(
    endpoint: &MirrorEndpoint,
    mappings: &[JsonValue],
    ait_refs: &BTreeMap<String, ExportRef>,
    git_refs: &BTreeMap<String, String>,
) -> Result<Vec<MirrorRefState>, String> {
    let mut names = git_refs.keys().cloned().collect::<BTreeSet<_>>();
    names.extend(ait_refs.keys().cloned());
    names.extend(
        mappings
            .iter()
            .filter(|row| {
                json_text(row, "kind") == Some(MIRROR_MAPPING_KIND)
                    && json_text(row, "endpoint_repository_fingerprint")
                        == Some(endpoint.fingerprint.as_str())
            })
            .filter_map(|row| json_text(row, "git_ref_name").map(str::to_string)),
    );
    let mut states = Vec::new();
    for name in names {
        let ait_ref = ait_refs.get(&name).cloned();
        let ait_state_id = ait_ref.as_ref().map(ait_ref_state_id);
        let git_object_id = git_refs.get(&name).cloned();
        let previous = latest_mapping(mappings.iter().filter(|row| {
            json_text(row, "kind") == Some(MIRROR_MAPPING_KIND)
                && json_text(row, "endpoint_repository_fingerprint")
                    == Some(endpoint.fingerprint.as_str())
                && json_text(row, "git_ref_name") == Some(name.as_str())
        }));
        let previous_git_object_id = previous
            .and_then(|row| json_text(row, "git_object_id"))
            .map(str::to_string);
        let previous_ait_state_id = previous
            .and_then(|row| json_text(row, "ait_state_id"))
            .map(str::to_string);
        let aligned = current_ref_has_identity_mapping(
            endpoint,
            mappings,
            &name,
            git_object_id.as_deref(),
            ait_ref.as_ref(),
        );
        let classification = if previous.is_some() {
            let git_changed = git_object_id != previous_git_object_id;
            let ait_changed = ait_state_id != previous_ait_state_id;
            match (git_changed, ait_changed) {
                (false, false) => "equal",
                (true, false) => "inbound_only",
                (false, true) => "outbound_only",
                (true, true) if git_object_id.is_none() && ait_state_id.is_none() || aligned => {
                    "equal"
                }
                (true, true) => "divergent",
            }
        } else {
            match (git_object_id.is_some(), ait_ref.is_some()) {
                (false, false) => "equal",
                (true, false) => "inbound_only",
                (false, true) => "outbound_only",
                (true, true) if aligned => "equal",
                (true, true) => "divergent",
            }
        };
        let (ait_kind, ait_name) = ait_ref
            .as_ref()
            .map(|row| (row.ait_kind.clone(), row.ait_name.clone()))
            .or_else(|| {
                previous.map(|row| {
                    (
                        json_text(row, "ait_kind").unwrap_or("line").to_string(),
                        json_text(row, "ait_name")
                            .unwrap_or_else(|| ref_ait_name(&name).1)
                            .to_string(),
                    )
                })
            })
            .unwrap_or_else(|| {
                let (kind, name) = ref_ait_name(&name);
                (kind.to_string(), name.to_string())
            });
        states.push(MirrorRefState {
            git_ref_name: name,
            ait_kind,
            ait_name,
            git_object_id,
            ait_ref,
            ait_state_id,
            previous_git_object_id,
            previous_ait_state_id,
            classification: classification.to_string(),
        });
    }
    states.sort_by(|left, right| left.git_ref_name.cmp(&right.git_ref_name));
    Ok(states)
}

fn ref_ait_name(reference: &str) -> (&'static str, &str) {
    if let Some(name) = reference.strip_prefix("refs/tags/") {
        ("tag", name)
    } else {
        (
            "line",
            reference.strip_prefix("refs/heads/").unwrap_or(reference),
        )
    }
}

fn current_ref_has_identity_mapping(
    endpoint: &MirrorEndpoint,
    mappings: &[JsonValue],
    git_ref_name: &str,
    git_object_id: Option<&str>,
    ait_ref: Option<&ExportRef>,
) -> bool {
    let (Some(git_object_id), Some(ait_ref)) = (git_object_id, ait_ref) else {
        return false;
    };
    let imported = endpoint.source.as_ref().is_some_and(|source| {
        mappings.iter().any(|row| {
            matches!(json_text(row, "kind"), Some("ref" | "tag"))
                && json_text(row, "direction") == Some("import")
                && json_text(row, "source_repository_fingerprint")
                    == Some(source.fingerprint.as_str())
                && json_text(row, "git_ref_name") == Some(git_ref_name)
                && json_text(row, "git_object_id") == Some(git_object_id)
                && json_text(row, "snapshot_id") == Some(ait_ref.snapshot_id.as_str())
        })
    });
    if imported {
        return true;
    }
    endpoint.target.as_ref().is_some_and(|target| {
        mappings.iter().any(|row| {
            json_text(row, "kind") == Some("ref")
                && json_text(row, "direction") == Some("export")
                && json_text(row, "target_repository_fingerprint")
                    == Some(target.fingerprint.as_str())
                && json_text(row, "git_ref_name") == Some(git_ref_name)
                && json_text(row, "git_object_id") == Some(git_object_id)
                && json_text(row, "snapshot_id") == Some(ait_ref.snapshot_id.as_str())
        })
    })
}

fn state_names(states: &[MirrorRefState], classification: &str) -> JsonValue {
    JsonValue::Array(
        states
            .iter()
            .filter(|state| state.classification == classification)
            .map(|state| JsonValue::String(state.git_ref_name.clone()))
            .collect(),
    )
}

fn mirror_report(
    endpoint: &MirrorEndpoint,
    direction: MirrorDirection,
    operation_id: &str,
    generation_id: &str,
    plan_hash: &str,
    states: &[MirrorRefState],
    counts: &MirrorCounts,
) -> JsonValue {
    json!({
        "contract": MIRROR_CONTRACT,
        "operation": "mirror",
        "operation_id": operation_id,
        "generation_id": generation_id,
        "plan_hash": plan_hash,
        "direction": direction.as_str(),
        "endpoint": endpoint.requested,
        "endpoint_identity": endpoint.identity,
        "endpoint_repository_fingerprint": endpoint.fingerprint,
        "git_object_format": endpoint.object_format,
        "state": counts.state(),
        "ref_count": states.len(),
        "equal_count": counts.equal,
        "inbound_only_count": counts.inbound_only,
        "outbound_only_count": counts.outbound_only,
        "divergent_count": counts.divergent,
        "refs": JsonValue::Array(states.iter().map(MirrorRefState::payload).collect()),
        "execution_mode": "single_ref_set_transaction",
        "resume_supported": true,
    })
}

fn with_report_fields(mut payload: JsonValue, fields: &[(&str, JsonValue)]) -> JsonValue {
    if let Some(object) = payload.as_object_mut() {
        for (name, value) in fields {
            object.insert((*name).to_string(), value.clone());
        }
    }
    payload
}

fn mirror_checkpoint(
    operation_id: &str,
    generation_id: &str,
    plan_hash: &str,
    state: &str,
    phase: &str,
    result: Option<JsonValue>,
) -> JsonValue {
    json!({
        "contract": CHECKPOINT_CONTRACT,
        "operation": "mirror",
        "operation_id": operation_id,
        "generation_id": generation_id,
        "plan_hash": plan_hash,
        "state": state,
        "phase": phase,
        "next_commit_index": 0,
        "next_ref_index": 0,
        "updated_at": system_event_timestamp(),
        "result": result,
    })
}

fn write_mirror_checkpoint(
    interop: &InteropStore,
    operation_id: &str,
    generation_id: &str,
    plan_hash: &str,
    state: &str,
    phase: &str,
    result: Option<JsonValue>,
) -> Result<(), String> {
    interop.write_operation(
        operation_id,
        &mirror_checkpoint(operation_id, generation_id, plan_hash, state, phase, result),
    )
}

fn existing_mirror_checkpoint(
    interop: &InteropStore,
    operation_id: &str,
    plan_hash: &str,
) -> Result<(Option<JsonValue>, bool), String> {
    let Some(checkpoint) = interop.read_operation(operation_id)? else {
        return Ok((None, false));
    };
    if json_text(&checkpoint, "contract") != Some(CHECKPOINT_CONTRACT)
        || json_text(&checkpoint, "operation") != Some("mirror")
        || json_text(&checkpoint, "plan_hash") != Some(plan_hash)
    {
        return Err(format!(
            "Git mirror checkpoint {operation_id} does not match the current immutable ref-set plan."
        ));
    }
    if json_text(&checkpoint, "state") == Some("completed") {
        let result = checkpoint
            .get("result")
            .cloned()
            .filter(|value| !value.is_null())
            .ok_or_else(|| {
                format!("Completed Git mirror checkpoint {operation_id} has no result.")
            })?;
        return Ok((Some(result), true));
    }
    Ok((None, true))
}

fn run_resumable_import(repo: &RepoRuntime, source: &str) -> Result<JsonValue, String> {
    git_import(repo, source, true, false)
}

fn run_resumable_export(repo: &RepoRuntime, target: &Path) -> Result<JsonValue, String> {
    let target = target.to_string_lossy();
    git_export(repo, &target, true, false)
}

#[derive(Clone, Debug)]
struct OutboundTransfer {
    target_git_dir: PathBuf,
    transfer_refs: BTreeMap<String, String>,
    export_result: JsonValue,
    target_created: bool,
}

fn stage_outbound_ref_set(
    repo: &RepoRuntime,
    interop: &InteropStore,
    endpoint: &MirrorEndpoint,
    states: &[MirrorRefState],
) -> Result<OutboundTransfer, String> {
    let target = endpoint
        .target
        .as_ref()
        .ok_or_else(|| "Outbound Git mirror requires a local Git target path.".to_string())?;
    let staging_git_dir = interop.mirror_staging_repository(&endpoint.fingerprint);
    if let Some(parent) = staging_git_dir.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create Git mirror staging directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let export_result = run_resumable_export(repo, &staging_git_dir)?;
    let target_git_dir = prepare_export_target(target)?;
    let target_was_empty = git_repo_text(
        &target_git_dir,
        [
            "for-each-ref",
            "--format=%(refname)",
            "refs/heads",
            "refs/tags",
        ],
    )?
    .trim()
    .is_empty();
    let mut transfer_refs = BTreeMap::new();
    for state in states
        .iter()
        .filter(|state| state.classification == "outbound_only")
    {
        let current = git_ref_object_id(&target_git_dir, &state.git_ref_name)?;
        if current != state.git_object_id {
            return Err(format!(
                "Git mirror compare-and-swap preflight failed for {}: planned {}, current {}.",
                state.git_ref_name,
                state.git_object_id.as_deref().unwrap_or("none"),
                current.as_deref().unwrap_or("none")
            ));
        }
        if state.ait_ref.is_none() {
            continue;
        }
        let transfer_ref = format!(
            "refs/ait/mirror-transfer/{}/{}",
            endpoint.fingerprint.to_ascii_lowercase(),
            sha256_prefixed("ref", state.git_ref_name.as_bytes(), 16).to_ascii_lowercase()
        );
        fetch_cache_ref(
            &target_git_dir,
            &staging_git_dir,
            &state.git_ref_name,
            &transfer_ref,
        )?;
        transfer_refs.insert(state.git_ref_name.clone(), transfer_ref);
    }
    Ok(OutboundTransfer {
        target_git_dir,
        transfer_refs,
        export_result,
        target_created: !target.existed || target_was_empty,
    })
}

fn apply_outbound_ref_transaction(
    repo: &RepoRuntime,
    endpoint: &MirrorEndpoint,
    states: &[MirrorRefState],
    transfer: &OutboundTransfer,
) -> Result<(), String> {
    let mut script = String::from("start\n");
    let mut update_count = 0_usize;
    for state in states
        .iter()
        .filter(|state| state.classification == "outbound_only")
    {
        let current = git_ref_object_id(&transfer.target_git_dir, &state.git_ref_name)?;
        if current != state.git_object_id {
            return Err(format!(
                "Git mirror refuses ref movement for {}: expected {}, found {}.",
                state.git_ref_name,
                state.git_object_id.as_deref().unwrap_or("none"),
                current.as_deref().unwrap_or("none")
            ));
        }
        match state.ait_ref.as_ref() {
            Some(_) => {
                let transfer_ref =
                    transfer
                        .transfer_refs
                        .get(&state.git_ref_name)
                        .ok_or_else(|| {
                            format!("Missing staged object transfer for {}.", state.git_ref_name)
                        })?;
                let desired = git_ref_object_id(&transfer.target_git_dir, transfer_ref)?
                    .ok_or_else(|| format!("Missing staged Git ref {transfer_ref}."))?;
                if let Some(current) = current.as_deref() {
                    script.push_str(&format!(
                        "update {} {} {}\n",
                        state.git_ref_name, desired, current
                    ));
                } else {
                    script.push_str(&format!("create {} {}\n", state.git_ref_name, desired));
                }
            }
            None => {
                let current = current.ok_or_else(|| {
                    format!(
                        "Git ref {} disappeared before mirror delete.",
                        state.git_ref_name
                    )
                })?;
                script.push_str(&format!("delete {} {}\n", state.git_ref_name, current));
            }
        }
        update_count += 1;
    }
    if update_count > 0 {
        script.push_str("prepare\ncommit\n");
        git_repo_bytes_os(
            &transfer.target_git_dir,
            vec![OsString::from("update-ref"), OsString::from("--stdin")],
            &[],
            Some(script.as_bytes()),
        )?;
    }
    if transfer.target_created {
        let current_refs = collect_ait_refs(repo)?.into_values().collect::<Vec<_>>();
        let mappings = InteropStore::new(repo).load_mappings()?;
        if let Some(preferred) = preferred_export_head_ref(repo, &current_refs, &mappings) {
            if git_ref_object_id(&transfer.target_git_dir, &preferred)?.is_some() {
                git_repo_bytes(
                    &transfer.target_git_dir,
                    ["symbolic-ref", "HEAD", preferred.as_str()],
                )?;
            }
        }
    }
    for transfer_ref in transfer.transfer_refs.values() {
        let _ = git_repo_bytes(
            &transfer.target_git_dir,
            ["update-ref", "-d", transfer_ref.as_str()],
        );
    }
    git_repo_bytes(
        &transfer.target_git_dir,
        ["fsck", "--full", "--no-dangling"],
    )?;
    let _ = endpoint;
    Ok(())
}

fn apply_inbound_deletions(repo: &RepoRuntime, states: &[MirrorRefState]) -> Result<(), String> {
    for state in states.iter().filter(|state| {
        state.classification == "inbound_only"
            && state.git_object_id.is_none()
            && state.ait_ref.is_some()
    }) {
        if state.ait_kind == "tag" {
            let tag_store =
                FilesystemTagStore::new(repo.authoritative_repo_root().to_string_lossy().as_ref())?;
            tag_store.delete_tag(&state.ait_name)?.ok_or_else(|| {
                format!(
                    "Git mirror could not delete missing AIT tag {}.",
                    state.ait_name
                )
            })?;
        } else {
            super::super::line::line_delete_local_unlocked(repo, &state.ait_name, true)?;
        }
    }
    Ok(())
}

fn mirror_source_argument(endpoint: &MirrorEndpoint) -> Result<String, String> {
    if let Some(target) = endpoint.target.as_ref() {
        return Ok(target.path.to_string_lossy().to_string());
    }
    endpoint
        .source
        .as_ref()
        .map(|source| source.source.clone())
        .ok_or_else(|| "Git mirror endpoint is no longer readable.".to_string())
}

#[allow(clippy::too_many_arguments)]
fn record_final_mirror_state(
    interop: &InteropStore,
    endpoint: &MirrorEndpoint,
    direction: MirrorDirection,
    operation_id: &str,
    generation_id: &str,
    planned_states: &[MirrorRefState],
    git_refs: &BTreeMap<String, String>,
    ait_refs: &BTreeMap<String, ExportRef>,
) -> Result<Vec<JsonValue>, String> {
    let mut names = git_refs.keys().cloned().collect::<BTreeSet<_>>();
    names.extend(ait_refs.keys().cloned());
    names.extend(
        planned_states
            .iter()
            .map(|state| state.git_ref_name.clone()),
    );
    let mut heads = Vec::new();
    for name in names {
        let git_object_id = git_refs.get(&name).cloned();
        let ait_ref = ait_refs.get(&name);
        if git_object_id.is_some() != ait_ref.is_some() {
            return Err(format!(
                "Git mirror ref set did not converge for {name}: Git={}, AIT={}.",
                git_object_id.as_deref().unwrap_or("none"),
                ait_ref
                    .map(|row| row.snapshot_id.as_str())
                    .unwrap_or("none")
            ));
        }
        let previous = planned_states
            .iter()
            .find(|state| state.git_ref_name == name);
        let (ait_kind, ait_name) = ait_ref
            .map(|row| (row.ait_kind.as_str(), row.ait_name.as_str()))
            .or_else(|| previous.map(|state| (state.ait_kind.as_str(), state.ait_name.as_str())))
            .unwrap_or_else(|| ref_ait_name(&name));
        let ait_state_id = ait_ref.map(ait_ref_state_id);
        let mapping = json!({
            "kind": MIRROR_MAPPING_KIND,
            "direction": "mirror",
            "created_at": system_event_timestamp(),
            "generation_id": generation_id,
            "mirror_operation_id": operation_id,
            "mirror_direction": direction.as_str(),
            "endpoint_repository_fingerprint": endpoint.fingerprint,
            "source_repository_fingerprint": endpoint.source.as_ref().map(|row| row.fingerprint.as_str()),
            "target_repository_fingerprint": endpoint.target.as_ref().map(|row| row.fingerprint.as_str()),
            "git_object_format": endpoint.object_format,
            "git_ref_name": name,
            "git_object_id": git_object_id,
            "ait_kind": ait_kind,
            "ait_name": ait_name,
            "ait_identity": ait_ref.and_then(|row| row.ait_identity.as_deref()),
            "ait_state_id": ait_state_id,
            "snapshot_id": ait_ref.map(|row| row.snapshot_id.as_str()),
            "state": "equal",
            "last_mirrored": true,
        });
        interop.write_mapping(mapping)?;
        heads.push(json!({
            "git_ref_name": name,
            "git_object_id": git_object_id,
            "ait_state_id": ait_state_id,
            "snapshot_id": ait_ref.map(|row| row.snapshot_id.as_str()),
            "state": "equal",
        }));
    }
    Ok(heads)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_surface::{init_repo, InitRequest};
    use crate::primitives::snapshot_create;
    use std::process::Command;
    use tempfile::TempDir;

    fn git_output(root: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .expect("execute fixture Git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("Git output is UTF-8")
            .trim()
            .to_string()
    }

    #[test]
    fn mirror_resumes_after_typed_object_transfer_interruption() {
        let source = TempDir::new().expect("AIT mirror source");
        init_repo(&InitRequest {
            root: source.path().to_path_buf(),
            name: Some("mirror-source".to_string()),
            default_line: "main".to_string(),
            policy_profile: "prototype".to_string(),
            default_author_mode: "ai_with_human_review".to_string(),
            default_model: None,
            repair_existing: false,
        })
        .expect("initialize AIT mirror source");
        fs::write(source.path().join("native.txt"), "mirror root\n").expect("write mirror content");
        let repo = RepoRuntime::discover_from_path(source.path()).expect("discover source");
        let snapshot = snapshot_create(&repo, Some("mirror root")).expect("create source Snapshot")
            ["snapshot_id"]
            .as_str()
            .expect("snapshot id")
            .to_string();
        let target_parent = TempDir::new().expect("mirror target parent");
        let target = target_parent.path().join("mirror.git");
        let target_text = target.to_string_lossy().to_string();

        let interrupted = git_mirror_with_control(
            &repo,
            &target_text,
            "outbound",
            false,
            MirrorExecutionControl {
                interrupt_after_object_transfer: true,
            },
        )
        .expect_err("typed interruption must stop before public ref movement");
        assert!(interrupted.contains("after object transfer"));
        assert!(target.is_dir());
        assert!(git_output(
            &target,
            &[
                "for-each-ref",
                "--format=%(refname)",
                "refs/heads",
                "refs/tags",
            ],
        )
        .is_empty());

        let resumed =
            git_mirror(&repo, &target_text, "outbound", false).expect("resume mirror operation");
        assert_eq!(resumed["status"], json!("completed"));
        assert_eq!(resumed["resumed"], json!(true));
        assert_eq!(resumed["compare_and_swap"], json!(true));
        assert_eq!(
            resumed["last_mirrored_heads"][0]["snapshot_id"],
            json!(snapshot)
        );
        assert_eq!(
            git_output(&target, &["symbolic-ref", "HEAD"]),
            "refs/heads/main"
        );
        assert_eq!(git_output(&target, &["rev-list", "--all", "--count"]), "1");
        assert!(git_output(
            &target,
            &[
                "for-each-ref",
                "--format=%(refname)",
                "refs/ait/mirror-transfer",
            ],
        )
        .is_empty());
        git_output(&target, &["fsck", "--full", "--no-dangling"]);
    }
}
