use crate::runtime::{
    RemoteRow, RepoLocalSnapshotOperationStore, RepoRuntime, SNAPSHOT_BINARY_DB_WRITE_LAYOUT,
};
use ait_core::json_support::{json, JsonMap, JsonValue};
use ait_core::local_snapshot::{
    LocalSnapshotBlobReadStore, LocalSnapshotReadStore, LocalSnapshotTreeReadStore,
    SnapshotPathBlobRow,
};
use ait_core::plan_filesystem::is_lineage_only_markdown_artifact_path;
use ait_core::plan_http_client::{get_plan_revision, PlanHttpClientConfig};
use ait_core::plan_store::{
    get_plan_with_plan_store, list_plan_revisions_with_plan_store, list_plans_with_plan_store,
    plan_record_detail_json, plan_record_list_json, plan_revision_record_json,
};
use ait_core::snapshot_dag::{snapshot_first_parent_chain, SnapshotDagLimits};
use ait_core::snapshot_json::SnapshotJson;
use ait_core::snapshot_store::SnapshotStore;
use ait_core::task_workflow_http_adapter::{HttpTaskRemote, HttpWorkflowCloseoutRemote};
use sha2::{Digest, Sha256};
use similar::{capture_diff_slices, Algorithm, DiffOp};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

const MISSING_BLOB_ORDINAL: u32 = u32::MAX;

#[derive(Clone, Debug, Default)]
pub struct BlameRequest {
    pub path: String,
    pub line: Option<usize>,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
    pub snapshot_id: Option<String>,
    pub via_parent_snapshot_id: Option<String>,
    pub patchset_id: Option<String>,
    pub remote_name: Option<String>,
    pub plan_id: Option<String>,
    pub plan_ref: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct BlameTarget {
    kind: String,
    line_name: Option<String>,
    patchset_id: Option<String>,
    change_id: Option<String>,
    task_id: Option<String>,
    base_snapshot_id: Option<String>,
    revision_snapshot_id: Option<String>,
    resolved_snapshot_id: Option<String>,
    remote_name: Option<String>,
    repo_name: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct BlameComputation {
    public: JsonMap<String, JsonValue>,
}

struct SnapshotBlameLineage<B: LocalSnapshotBlobReadStore> {
    blob_store: B,
    chain: Vec<String>,
    path_timeline: CompactPathBlobTimeline,
    blob_bytes_by_id: BTreeMap<String, Vec<u8>>,
    blob_lines_cache: BTreeMap<String, Vec<String>>,
    target_lines: Vec<String>,
}

trait ReverseSnapshotPathBlobStore {
    fn visit_reverse_path_blobs(
        &self,
        snapshot_ids: &[String],
        path: &str,
        visitor: &mut dyn FnMut(usize, Option<String>) -> Result<bool, String>,
    ) -> Result<(), String>;
}

impl<T> ReverseSnapshotPathBlobStore for T
where
    T: LocalSnapshotTreeReadStore,
{
    fn visit_reverse_path_blobs(
        &self,
        snapshot_ids: &[String],
        path: &str,
        visitor: &mut dyn FnMut(usize, Option<String>) -> Result<bool, String>,
    ) -> Result<(), String> {
        self.visit_snapshot_tree_path_blobs_reverse(snapshot_ids, path, visitor)
    }
}

struct CompactPathBlobTimeline {
    path_id: u32,
    blob_ids: Vec<String>,
    snapshot_blob_ordinals: Vec<u32>,
}

impl CompactPathBlobTimeline {
    fn from_rows(snapshot_count: usize, rows: &[SnapshotPathBlobRow]) -> Result<Self, String> {
        let path_id = 0_u32;
        let mut blob_ordinals = BTreeMap::new();
        let mut blob_ids = Vec::new();
        let mut encoded_entries = Vec::new();
        let mut previous_snapshot_index = 0_u32;
        let mut first_entry = true;

        for row in rows {
            let snapshot_index = u32::try_from(row.snapshot_index)
                .map_err(|_| "Snapshot path index exceeds u32 capacity.".to_string())?;
            if row.snapshot_index >= snapshot_count {
                return Err("Snapshot path row index is outside the snapshot chain.".to_string());
            }
            let blob_ordinal = if let Some(existing) = blob_ordinals.get(&row.blob_id) {
                *existing
            } else {
                if blob_ids.len() >= MISSING_BLOB_ORDINAL as usize {
                    return Err("Snapshot path blob dictionary exceeds u32 capacity.".to_string());
                }
                let next = blob_ids.len() as u32;
                blob_ordinals.insert(row.blob_id.clone(), next);
                blob_ids.push(row.blob_id.clone());
                next
            };
            let snapshot_delta = if first_entry {
                snapshot_index
            } else {
                snapshot_index
                    .checked_sub(previous_snapshot_index)
                    .ok_or_else(|| {
                        "Snapshot path rows must be ordered by snapshot index.".to_string()
                    })?
            };
            encode_u32_varint(snapshot_delta, &mut encoded_entries);
            encode_u32_varint(blob_ordinal, &mut encoded_entries);
            previous_snapshot_index = snapshot_index;
            first_entry = false;
        }

        let snapshot_blob_ordinals = decode_path_blob_ordinals(snapshot_count, &encoded_entries)?;
        Ok(Self {
            path_id,
            blob_ids,
            snapshot_blob_ordinals,
        })
    }

    fn path_id(&self) -> u32 {
        self.path_id
    }

    fn blob_id_at(&self, snapshot_index: usize) -> Option<&str> {
        let ordinal = *self.snapshot_blob_ordinals.get(snapshot_index)?;
        if ordinal == MISSING_BLOB_ORDINAL {
            return None;
        }
        self.blob_ids.get(ordinal as usize).map(String::as_str)
    }

    fn blob_ids(&self) -> Vec<String> {
        self.blob_ids.clone()
    }

    #[cfg(test)]
    fn oldest_existing_snapshot_index(&self) -> Option<usize> {
        self.snapshot_blob_ordinals
            .iter()
            .position(|ordinal| *ordinal != MISSING_BLOB_ORDINAL)
    }
}

fn encode_u32_varint(mut value: u32, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push(((value & 0x7f) as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn decode_path_blob_ordinals(
    snapshot_count: usize,
    encoded_entries: &[u8],
) -> Result<Vec<u32>, String> {
    let mut snapshot_blob_ordinals = vec![MISSING_BLOB_ORDINAL; snapshot_count];
    let mut offset = 0_usize;
    let mut previous_snapshot_index = 0_u32;
    let mut first_entry = true;
    while offset < encoded_entries.len() {
        let snapshot_delta = decode_u32_varint(encoded_entries, &mut offset)?;
        let blob_ordinal = decode_u32_varint(encoded_entries, &mut offset)?;
        let snapshot_index = if first_entry {
            snapshot_delta
        } else {
            previous_snapshot_index
                .checked_add(snapshot_delta)
                .ok_or_else(|| "Snapshot path index overflowed while decoding.".to_string())?
        };
        let index = usize::try_from(snapshot_index)
            .map_err(|_| "Snapshot path index exceeds usize capacity.".to_string())?;
        if index >= snapshot_blob_ordinals.len() {
            return Err("Snapshot path index is outside the decoded chain.".to_string());
        }
        snapshot_blob_ordinals[index] = blob_ordinal;
        previous_snapshot_index = snapshot_index;
        first_entry = false;
    }
    Ok(snapshot_blob_ordinals)
}

fn decode_u32_varint(encoded_entries: &[u8], offset: &mut usize) -> Result<u32, String> {
    let mut value = 0_u32;
    let mut shift = 0_u32;
    while *offset < encoded_entries.len() {
        let byte = encoded_entries[*offset];
        *offset += 1;
        value |= ((byte & 0x7f) as u32)
            .checked_shl(shift)
            .ok_or_else(|| "Snapshot path varint overflowed.".to_string())?;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
        if shift >= 32 {
            return Err("Snapshot path varint exceeds u32 capacity.".to_string());
        }
    }
    Err("Truncated snapshot path varint.".to_string())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SelectedLineTracker {
    output_index: usize,
    current_index: usize,
}

pub fn blame(repo: &RepoRuntime, request: &BlameRequest) -> Result<JsonValue, String> {
    let _blame_range = perfetto_range!("ait.cli.blame");
    let rel_path = {
        let _range = perfetto_range!("ait.cli.blame.validate_and_normalize");
        validate_request(request)?;
        normalize_blame_path(repo, &request.path)?
    };
    let markdown_lineage = is_lineage_only_markdown_artifact_path(&rel_path);
    let computation = if markdown_lineage {
        if request.snapshot_id.is_some() || request.patchset_id.is_some() {
            return Err(format!(
                "Path {rel_path} is lineage-only Markdown. `--snapshot` and `--patchset` are not valid for plan-lineage blame."
            ));
        }
        if request.via_parent_snapshot_id.is_some() {
            return Err(format!(
                "Path {rel_path} is lineage-only Markdown. `--via-parent` is only valid for Snapshot blame."
            ));
        }
        compute_markdown_plan_blame(
            repo,
            &rel_path,
            request.line,
            request.start_line,
            request.end_line,
            request.plan_id.as_deref(),
            request.plan_ref.as_deref(),
        )?
    } else {
        if request.plan_id.is_some() || request.plan_ref.is_some() {
            return Err(format!(
                "Path {rel_path} uses snapshot lineage. `--plan-id` and `--plan-ref` are only valid for lineage-only Markdown blame."
            ));
        }
        let target = {
            let _range = perfetto_range!("ait.cli.blame.resolve_target");
            resolve_blame_target(
                repo,
                request.snapshot_id.as_deref(),
                request.patchset_id.as_deref(),
                request.remote_name.as_deref(),
            )?
        };
        {
            let _range = perfetto_range!("ait.cli.blame.snapshot_compute");
            compute_snapshot_blame(
                repo,
                &rel_path,
                &target,
                request.line,
                request.start_line,
                request.end_line,
                request.via_parent_snapshot_id.as_deref(),
            )?
        }
    };
    Ok(JsonValue::Object(computation.public))
}

pub fn render_human_blame(payload: &JsonValue) {
    let target = payload
        .get("target")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    let hunks = payload
        .get("hunks")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let warnings = payload
        .get("warnings")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let mut header = Vec::new();
    let target_kind =
        string_field_obj(&target, "kind").unwrap_or_else(|| "current_line".to_string());
    match target_kind.as_str() {
        "patchset" => {
            header.push(format!(
                "target: patchset {}",
                string_field_obj(&target, "patchset_id").unwrap_or_default()
            ));
            if let Some(change_id) = string_field_obj(&target, "change_id") {
                header.push(format!("change: {change_id}"));
            }
            if let Some(base_snapshot_id) = string_field_obj(&target, "base_snapshot_id") {
                header.push(format!("base: {base_snapshot_id}"));
            }
            if let Some(revision_snapshot_id) = string_field_obj(&target, "revision_snapshot_id") {
                header.push(format!("revision: {revision_snapshot_id}"));
            }
        }
        "snapshot" => {
            header.push(format!(
                "target: snapshot {}",
                string_field(payload, "resolved_snapshot_id").unwrap_or_default()
            ));
        }
        "markdown_plan" => {
            header.push(format!(
                "target: markdown plan {}",
                string_field_obj(&target, "plan_id").unwrap_or_default()
            ));
            if let Some(revision_id) = string_field_obj(&target, "resolved_plan_revision_id") {
                header.push(format!("revision: {revision_id}"));
            }
        }
        _ => {
            header.push(format!(
                "target: current line {}",
                string_field(payload, "line_name")
                    .or_else(|| string_field_obj(&target, "line_name"))
                    .unwrap_or_default()
            ));
            header.push(format!(
                "resolved snapshot: {}",
                string_field(payload, "resolved_snapshot_id").unwrap_or_default()
            ));
        }
    }
    header.push(format!(
        "path: {}",
        string_field(payload, "path").unwrap_or_default()
    ));
    if let Some(selection) = payload
        .get("parent_selection")
        .and_then(JsonValue::as_object)
    {
        if let Some(selected_parent) = string_field_obj(selection, "selected_parent_snapshot_id") {
            header.push(format!("blame parent: {selected_parent}"));
        }
        let alternates = selection
            .get("alternate_parent_snapshot_ids")
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
            .filter_map(JsonValue::as_str)
            .collect::<Vec<_>>();
        if !alternates.is_empty() {
            header.push(format!("alternate parents: {}", alternates.join(", ")));
        }
    }
    let selected_range = payload
        .get("range")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    if json_u64_field_obj(&selected_range, "start").unwrap_or(0) > 0 {
        header.push(format!(
            "range: {}-{}",
            json_u64_field_obj(&selected_range, "start").unwrap_or(0),
            json_u64_field_obj(&selected_range, "end").unwrap_or(0)
        ));
    }
    println!("{}", header.join("\n"));
    println!();
    if hunks.is_empty() {
        println!("no blameable lines");
    } else {
        let rendered = hunks
            .iter()
            .map(format_hunk)
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_default();
        println!("{}", rendered.join("\n"));
    }
    if !warnings.is_empty() {
        println!();
        for warning in warnings {
            if let Some(message) = warning
                .get("message")
                .and_then(JsonValue::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                println!("warning: {message}");
            }
        }
    }
}

fn validate_request(request: &BlameRequest) -> Result<(), String> {
    if request.line.is_some() && (request.start_line.is_some() || request.end_line.is_some()) {
        return Err("Choose either --line or --start/--end.".to_string());
    }
    if request.start_line.is_some() != request.end_line.is_some() {
        return Err("Provide both --start and --end.".to_string());
    }
    if [request.line, request.start_line, request.end_line]
        .into_iter()
        .flatten()
        .any(|line| line == 0)
    {
        return Err("Line selections are 1-based and must be positive.".to_string());
    }
    if let (Some(start_line), Some(end_line)) = (request.start_line, request.end_line) {
        if end_line < start_line {
            return Err("The selected range must have end >= start.".to_string());
        }
    }
    if request.snapshot_id.is_some() && request.patchset_id.is_some() {
        return Err("Choose either --snapshot or --patchset.".to_string());
    }
    if request.via_parent_snapshot_id.is_some() && request.snapshot_id.is_none() {
        return Err("`--via-parent` requires `--snapshot`.".to_string());
    }
    if request.remote_name.is_some() && request.patchset_id.is_none() {
        return Err("`--remote` is only valid with `--patchset`.".to_string());
    }
    if request.plan_id.is_some() && request.plan_ref.is_some() {
        return Err("Choose either --plan-id or --plan-ref.".to_string());
    }
    if (request.plan_id.is_some() || request.plan_ref.is_some())
        && (request.snapshot_id.is_some() || request.patchset_id.is_some())
    {
        return Err("Plan selectors cannot be combined with --snapshot or --patchset.".to_string());
    }
    for (value, label) in [
        (request.snapshot_id.as_deref(), "--snapshot"),
        (request.via_parent_snapshot_id.as_deref(), "--via-parent"),
        (request.patchset_id.as_deref(), "--patchset"),
        (request.remote_name.as_deref(), "--remote"),
        (request.plan_id.as_deref(), "--plan-id"),
        (request.plan_ref.as_deref(), "--plan-ref"),
    ] {
        if value.is_some_and(|value| value.trim().is_empty()) {
            return Err(format!("{label} requires a non-empty value."));
        }
    }
    if request.patchset_id.as_deref().is_some_and(|patchset_id| {
        let patchset_id = patchset_id.trim();
        !patchset_id.is_empty() && patchset_id.chars().all(|ch| ch.is_ascii_digit())
    }) {
        return Err(
            "`--patchset` requires an exact published Patchset ID; numeric repo-scoped refs are ambiguous."
                .to_string(),
        );
    }
    Ok(())
}

fn resolve_blame_target(
    repo: &RepoRuntime,
    snapshot_id: Option<&str>,
    patchset_id: Option<&str>,
    remote_name: Option<&str>,
) -> Result<BlameTarget, String> {
    if let Some(patchset_id) = normalized_text(patchset_id) {
        let (remote_row, resolved_repo_name) = remote_context(repo, remote_name)?;
        let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
        let patchset = closeout_remote
            .get_patchset(&patchset_id, Some(&resolved_repo_name), None)
            .map_err(|err| err.to_string())?;
        let mut task_remote = http_task_remote(repo, &remote_row)?;
        let resolved_snapshot_id = string_field(&patchset, "revision_snapshot_id")
            .ok_or_else(|| format!("Patchset {patchset_id} is missing a revision snapshot id."))?;
        if get_local_snapshot_metadata_for_repo(repo, &resolved_snapshot_id).is_err() {
            let resolved_patchset_id =
                string_field(&patchset, "patchset_id").unwrap_or_else(|| patchset_id.clone());
            return Err(format!(
                "Patchset {resolved_patchset_id} resolved to revision snapshot {resolved_snapshot_id}, but that snapshot is not available in the local store. Materialize or import the snapshot first."
            ));
        }
        let change_id = string_field(&patchset, "change_id").unwrap_or_default();
        let change = task_remote
            .get_change_detail(&change_id, Some(&resolved_repo_name))
            .map_err(|err| err.to_string())?;
        let task = change.get("task").and_then(JsonValue::as_object).cloned();
        return Ok(BlameTarget {
            kind: "patchset".to_string(),
            patchset_id: string_field(&patchset, "patchset_id").or(Some(patchset_id)),
            change_id: string_field(&patchset, "change_id"),
            task_id: task
                .as_ref()
                .and_then(|value| string_field_obj(value, "task_id"))
                .or_else(|| string_field(&change, "task_id")),
            base_snapshot_id: string_field(&patchset, "base_snapshot_id"),
            revision_snapshot_id: string_field(&patchset, "revision_snapshot_id"),
            resolved_snapshot_id: Some(resolved_snapshot_id),
            remote_name: Some(remote_row.name),
            repo_name: Some(resolved_repo_name),
            ..BlameTarget::default()
        });
    }
    if let Some(snapshot_id) = normalized_text(snapshot_id) {
        let snapshot = get_local_snapshot_metadata_for_repo(repo, &snapshot_id)
            .map_err(|_| format!("Unknown snapshot: {snapshot_id}"))?;
        return Ok(BlameTarget {
            kind: "snapshot".to_string(),
            resolved_snapshot_id: Some(snapshot_id),
            line_name: string_field(&snapshot, "line_name"),
            ..BlameTarget::default()
        });
    }
    let line_name = repo.current_line_name()?;
    let line_row = local_snapshot_operation_store(repo)?.get_line(&line_name)?;
    let resolved_snapshot_id = string_field(&line_row, "head_snapshot_id")
        .ok_or_else(|| format!("Current line `{line_name}` has no head snapshot to blame."))?;
    Ok(BlameTarget {
        kind: "current_line".to_string(),
        line_name: Some(line_name),
        resolved_snapshot_id: Some(resolved_snapshot_id),
        ..BlameTarget::default()
    })
}

fn compute_snapshot_blame(
    repo: &RepoRuntime,
    rel_path: &str,
    target: &BlameTarget,
    line: Option<usize>,
    start_line: Option<usize>,
    end_line: Option<usize>,
    selected_parent_snapshot_id: Option<&str>,
) -> Result<BlameComputation, String> {
    let _range = perfetto_range!("ait.cli.blame.snapshot");
    let target_snapshot_id = target
        .resolved_snapshot_id
        .clone()
        .ok_or_else(|| "Resolved snapshot id is required.".to_string())?;
    if has_explicit_line_selection(line, start_line, end_line) {
        let mut lineage = {
            let _range = perfetto_range!("ait.cli.blame.lineage_load");
            load_snapshot_blame_lineage(
                repo,
                &target_snapshot_id,
                rel_path,
                false,
                selected_parent_snapshot_id,
            )?
        };
        let (selected_start, selected_end) =
            line_selection(lineage.target_lines.len(), line, start_line, end_line)?;
        let selected_owners = if selected_start == 0 {
            Vec::new()
        } else {
            {
                let _range = perfetto_range!("ait.cli.blame.selected_owners");
                compute_snapshot_selected_line_owners(
                    repo,
                    &mut lineage,
                    &target_snapshot_id,
                    rel_path,
                    selected_start,
                    selected_end,
                )?
            }
        };
        return {
            let _range = perfetto_range!("ait.cli.blame.result_build");
            build_snapshot_blame_computation(
                repo,
                rel_path,
                target,
                target_snapshot_id,
                selected_start,
                selected_end,
                selected_owners,
                selected_parent_snapshot_id,
            )
        };
    }
    let (target_lines, owners) = compute_snapshot_line_owners(
        repo,
        &target_snapshot_id,
        rel_path,
        selected_parent_snapshot_id,
    )?;
    let (selected_start, selected_end) =
        line_selection(target_lines.len(), line, start_line, end_line)?;
    let selected_owners = if selected_start == 0 {
        Vec::new()
    } else {
        owners[selected_start - 1..selected_end].to_vec()
    };
    build_snapshot_blame_computation(
        repo,
        rel_path,
        target,
        target_snapshot_id,
        selected_start,
        selected_end,
        selected_owners,
        selected_parent_snapshot_id,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "blame computation keeps bounded history inputs and caches explicit"
)]
fn build_snapshot_blame_computation(
    repo: &RepoRuntime,
    rel_path: &str,
    target: &BlameTarget,
    target_snapshot_id: String,
    selected_start: usize,
    selected_end: usize,
    selected_owners: Vec<String>,
    selected_parent_snapshot_id: Option<&str>,
) -> Result<BlameComputation, String> {
    let _range = perfetto_range!("ait.cli.blame.build_computation");
    let selected_owner_snapshot_ids = ordered_unique(&selected_owners);
    let mut snapshot_rows = BTreeMap::new();
    let mut snapshot_ids = selected_owner_snapshot_ids.clone();
    if !snapshot_ids
        .iter()
        .any(|value| value == &target_snapshot_id)
    {
        snapshot_ids.push(target_snapshot_id.clone());
    }
    {
        let _range = perfetto_range!("ait.cli.blame.owner_metadata");
        for snapshot_id in ordered_unique(&snapshot_ids) {
            let row = get_local_snapshot_metadata_for_repo(repo, &snapshot_id)?;
            snapshot_rows.insert(snapshot_id, row);
        }
    }
    let overlay = {
        let _range = perfetto_range!("ait.cli.blame.provenance_overlay");
        snapshot_overlay(repo, &selected_owner_snapshot_ids, target)?
    };
    let mut line_rows = Vec::new();
    if selected_start > 0 {
        for (offset, snapshot_id) in selected_owners.iter().enumerate() {
            let line_number = selected_start + offset;
            if line_number > selected_end {
                return Err("Blame line ownership range is longer than selected range.".to_string());
            }
            let snapshot_id = snapshot_id.clone();
            let snapshot_row = snapshot_rows
                .get(&snapshot_id)
                .ok_or_else(|| "Blame line ownership index out of bounds.".to_string())?;
            line_rows.push(line_row_payload(
                rel_path,
                line_number,
                snapshot_row,
                overlay.get(&snapshot_id),
            ));
        }
        let expected_len = selected_end - selected_start + 1;
        if selected_owners.len() != expected_len {
            return Err(
                "Blame line ownership range length does not match selected range.".to_string(),
            );
        }
    }
    let target_snapshot_row = snapshot_rows
        .get(&target_snapshot_id)
        .ok_or_else(|| format!("Unknown snapshot: {target_snapshot_id}"))?;
    let mut public = JsonMap::new();
    public.insert("target".to_string(), target.to_public_json());
    public.insert("path".to_string(), JsonValue::String(rel_path.to_string()));
    public.insert(
        "resolved_snapshot_id".to_string(),
        JsonValue::String(target_snapshot_id.clone()),
    );
    public.insert(
        "line_name".to_string(),
        target_snapshot_row
            .get("line_name")
            .cloned()
            .unwrap_or(JsonValue::Null),
    );
    let parent_snapshot_ids = target_snapshot_row
        .get("parent_snapshot_ids")
        .and_then(JsonValue::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(JsonValue::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            string_field(target_snapshot_row, "primary_parent_snapshot_id")
                .or_else(|| string_field(target_snapshot_row, "parent_snapshot_id"))
                .into_iter()
                .collect()
        });
    let primary_parent_snapshot_id = parent_snapshot_ids.first().cloned();
    let selected_parent_snapshot_id = selected_parent_snapshot_id
        .map(str::to_string)
        .or_else(|| primary_parent_snapshot_id.clone());
    let alternate_parent_snapshot_ids = parent_snapshot_ids
        .iter()
        .filter(|parent| Some(parent.as_str()) != selected_parent_snapshot_id.as_deref())
        .cloned()
        .collect::<Vec<_>>();
    public.insert(
        "parent_selection".to_string(),
        json!({
            "mode": if selected_parent_snapshot_id.is_none() {
                "root"
            } else if selected_parent_snapshot_id.as_deref() == primary_parent_snapshot_id.as_deref() {
                "primary_parent"
            } else {
                "selected_parent"
            },
            "parent_snapshot_ids": parent_snapshot_ids,
            "primary_parent_snapshot_id": primary_parent_snapshot_id,
            "selected_parent_snapshot_id": selected_parent_snapshot_id,
            "alternate_parent_snapshot_ids": alternate_parent_snapshot_ids,
        }),
    );
    public.insert(
        "range".to_string(),
        json!({"start": selected_start, "end": selected_end}),
    );
    public.insert(
        "hunks".to_string(),
        JsonValue::Array(collapse_line_rows(&line_rows)),
    );
    public.insert("lines".to_string(), JsonValue::Array(line_rows));
    Ok(BlameComputation { public })
}

fn compute_markdown_plan_blame(
    repo: &RepoRuntime,
    rel_path: &str,
    line: Option<usize>,
    start_line: Option<usize>,
    end_line: Option<usize>,
    plan_id: Option<&str>,
    plan_ref: Option<&str>,
) -> Result<BlameComputation, String> {
    let plan = current_plan_for_artifact_path(repo, rel_path, plan_id, plan_ref)?;
    let plan_store = repo
        .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .plans();
    let mut revisions =
        list_plan_revisions_with_plan_store(&plan_store, &required_string_field(&plan, "plan_id")?)
            .map_err(|err| err.to_string())?
            .iter()
            .map(plan_revision_record_json)
            .collect::<Vec<_>>();
    revisions.sort_by_key(|row| json_i64_field(row, "revision_number").unwrap_or(0));
    let head_revision = plan
        .get("head_revision")
        .cloned()
        .and_then(|value| value.as_object().cloned().map(JsonValue::Object))
        .unwrap_or(JsonValue::Null);
    let head_revision_id = string_field(&head_revision, "plan_revision_id").ok_or_else(|| {
        format!(
            "Current plan {} is missing a head revision for lineage-only Markdown path {rel_path}.",
            string_field(&plan, "plan_id").unwrap_or_default()
        )
    })?;
    let (target_lines, owners, skipped_revision_ids) =
        compute_markdown_plan_line_owners(repo, &plan, rel_path, &revisions, &head_revision_id)?;
    let (selected_start, selected_end) =
        line_selection(target_lines.len(), line, start_line, end_line)?;
    let revision_rows = markdown_plan_row_map(&plan, &revisions);
    let mut line_rows = Vec::new();
    if selected_start > 0 {
        for index in selected_start - 1..selected_end {
            let revision_id = owners
                .get(index)
                .cloned()
                .ok_or_else(|| "Blame plan revision ownership index out of bounds.".to_string())?;
            let revision_row = revision_rows
                .get(&revision_id)
                .ok_or_else(|| format!("Unknown plan revision: {revision_id}"))?;
            line_rows.push(markdown_line_row_payload(rel_path, index + 1, revision_row));
        }
    }
    let mut public = JsonMap::new();
    public.insert(
        "target".to_string(),
        json!({
            "kind": "markdown_plan",
            "plan_id": string_field(&plan, "plan_id"),
            "plan_ref": plan_head_artifact_field(&plan, "artifact_selector"),
            "artifact_path": rel_path,
            "resolved_plan_revision_id": head_revision_id.clone(),
        }),
    );
    public.insert("path".to_string(), JsonValue::String(rel_path.to_string()));
    public.insert(
        "resolved_plan_revision_id".to_string(),
        JsonValue::String(head_revision_id),
    );
    public.insert(
        "range".to_string(),
        json!({"start": selected_start, "end": selected_end}),
    );
    public.insert(
        "warnings".to_string(),
        JsonValue::Array(
            skipped_revision_ids
                .iter()
                .map(|revision_id| {
                    json!({
                        "kind": "missing_markdown_revision_body",
                        "plan_revision_id": revision_id,
                        "message": format!(
                            "Skipped unreadable historical plan revision {revision_id} while attributing lineage-only Markdown path {rel_path}."
                        ),
                    })
                })
                .collect(),
        ),
    );
    public.insert(
        "hunks".to_string(),
        JsonValue::Array(collapse_line_rows(&line_rows)),
    );
    public.insert("lines".to_string(), JsonValue::Array(line_rows));
    Ok(BlameComputation { public })
}

fn compute_snapshot_line_owners(
    repo: &RepoRuntime,
    target_snapshot_id: &str,
    rel_path: &str,
    selected_parent_snapshot_id: Option<&str>,
) -> Result<(Vec<String>, Vec<String>), String> {
    let mut lineage = load_snapshot_blame_lineage(
        repo,
        target_snapshot_id,
        rel_path,
        true,
        selected_parent_snapshot_id,
    )?;
    let target_lines = lineage.target_lines.clone();
    let mut previous_lines: Vec<String> = Vec::new();
    let mut previous_owners: Vec<String> = Vec::new();
    let mut previous_blob_id: Option<String> = None;
    let mut file_exists = false;
    for (snapshot_index, snapshot_id) in lineage.chain.clone().into_iter().enumerate() {
        let Some(blob_id) = lineage
            .path_timeline
            .blob_id_at(snapshot_index)
            .map(str::to_string)
        else {
            if file_exists {
                previous_lines.clear();
                previous_owners.clear();
                previous_blob_id = None;
                file_exists = false;
            }
            continue;
        };
        if !file_exists {
            let next_lines = blob_text_lines_for_lineage(
                &mut lineage,
                &blob_id,
                &format!("Snapshot {snapshot_id}:{rel_path}"),
            )?;
            previous_lines = next_lines;
            previous_owners = vec![snapshot_id.clone(); previous_lines.len()];
            previous_blob_id = Some(blob_id);
            file_exists = true;
            continue;
        }
        if previous_blob_id.as_deref() == Some(blob_id.as_str()) {
            continue;
        }
        let next_lines = blob_text_lines_for_lineage(
            &mut lineage,
            &blob_id,
            &format!("Snapshot {snapshot_id}:{rel_path}"),
        )?;
        previous_owners =
            apply_line_diff(&previous_lines, &next_lines, &previous_owners, &snapshot_id);
        previous_lines = next_lines;
        previous_blob_id = Some(blob_id);
    }
    Ok((target_lines, previous_owners))
}

fn load_snapshot_blame_lineage(
    repo: &RepoRuntime,
    target_snapshot_id: &str,
    rel_path: &str,
    preload_all_blobs: bool,
    selected_parent_snapshot_id: Option<&str>,
) -> Result<
    SnapshotBlameLineage<RepoLocalSnapshotOperationStore<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>>,
    String,
> {
    let _range = perfetto_range!("ait.cli.blame.load_lineage");
    let snapshot_store = {
        let _range = perfetto_range!("ait.cli.blame.store_select");
        snapshot_store(repo)?
    };
    let chain = {
        let _range = perfetto_range!("ait.cli.blame.snapshot_chain");
        snapshot_first_parent_chain(
            &snapshot_store,
            target_snapshot_id,
            selected_parent_snapshot_id,
            SnapshotDagLimits::default(),
        )?
    };
    if chain.is_empty() {
        return Err(format!("Unknown snapshot: {target_snapshot_id}"));
    }
    let target_pos = chain
        .iter()
        .position(|snapshot_id| snapshot_id == target_snapshot_id)
        .ok_or_else(|| format!("Unknown snapshot: {target_snapshot_id}"))?;
    let tree_store = local_snapshot_operation_store(repo)?;
    let mut path_rows = if preload_all_blobs {
        {
            let _range = perfetto_range!("ait.cli.blame.path_history_bulk_read");
            tree_store.snapshot_tree_path_blob_rows_for_snapshots(&chain, rel_path)?
        }
    } else {
        {
            let _range = perfetto_range!("ait.cli.blame.target_path_read");
            tree_store.snapshot_tree_path_blob_rows_for_snapshots(
                std::slice::from_ref(&chain[target_pos]),
                rel_path,
            )?
        }
    };
    if !preload_all_blobs {
        for row in &mut path_rows {
            row.snapshot_index = target_pos;
        }
    }
    let path_timeline = {
        let _range = perfetto_range!("ait.cli.blame.path_timeline_build");
        CompactPathBlobTimeline::from_rows(chain.len(), &path_rows)?
    };
    let target_blob_id = path_timeline
        .blob_id_at(target_pos)
        .ok_or_else(|| {
            format!(
                "Path id {} ({rel_path}) does not exist in snapshot {target_snapshot_id}.",
                path_timeline.path_id()
            )
        })?
        .to_string();
    let blob_ids = if preload_all_blobs {
        path_timeline.blob_ids()
    } else {
        vec![target_blob_id.clone()]
    };
    let blob_store = local_snapshot_operation_store(repo)?;
    let blob_bytes_by_id = {
        let _range = perfetto_range!("ait.cli.blame.target_blob_batch_read");
        blob_store.read_blob_bytes_batch(&blob_ids)?
    };
    let mut blob_lines_cache = BTreeMap::new();
    let target_lines = {
        let _range = perfetto_range!("ait.cli.blame.target_blob_parse");
        blob_text_lines(
            &target_blob_id,
            &blob_bytes_by_id,
            &mut blob_lines_cache,
            &format!("Snapshot {target_snapshot_id}:{rel_path}"),
        )?
    };
    Ok(SnapshotBlameLineage {
        blob_store,
        chain,
        path_timeline,
        blob_bytes_by_id,
        blob_lines_cache,
        target_lines,
    })
}

fn compute_snapshot_selected_line_owners<B>(
    repo: &RepoRuntime,
    lineage: &mut SnapshotBlameLineage<B>,
    target_snapshot_id: &str,
    rel_path: &str,
    selected_start: usize,
    selected_end: usize,
) -> Result<Vec<String>, String>
where
    B: LocalSnapshotBlobReadStore,
{
    let _range = perfetto_range!("ait.cli.blame.selected_line_walk");
    let tree_store = local_snapshot_operation_store(repo)?;
    compute_snapshot_selected_line_owners_with_reverse_store(
        lineage,
        &tree_store,
        target_snapshot_id,
        rel_path,
        selected_start,
        selected_end,
    )
}

fn compute_snapshot_selected_line_owners_with_reverse_store<B, T>(
    lineage: &mut SnapshotBlameLineage<B>,
    tree_store: &T,
    target_snapshot_id: &str,
    rel_path: &str,
    selected_start: usize,
    selected_end: usize,
) -> Result<Vec<String>, String>
where
    B: LocalSnapshotBlobReadStore,
    T: ReverseSnapshotPathBlobStore,
{
    let target_pos = lineage
        .chain
        .iter()
        .position(|snapshot_id| snapshot_id == target_snapshot_id)
        .ok_or_else(|| format!("Unknown snapshot: {target_snapshot_id}"))?;
    let target_blob_id = lineage
        .path_timeline
        .blob_id_at(target_pos)
        .ok_or_else(|| {
            format!(
                "Path id {} ({rel_path}) does not exist in snapshot {target_snapshot_id}.",
                lineage.path_timeline.path_id()
            )
        })?
        .to_string();
    let mut owners = vec![None; selected_end - selected_start + 1];
    let mut tracked = (selected_start - 1..selected_end)
        .enumerate()
        .map(|(output_index, current_index)| SelectedLineTracker {
            output_index,
            current_index,
        })
        .collect::<Vec<_>>();
    let chain = lineage.chain.clone();
    let mut child_state: Option<(usize, String)> = None;
    let mut oldest_existing_snapshot_id = None;
    {
        let _range = perfetto_range!("ait.cli.blame.selected_path_reverse_visit");
        tree_store.visit_reverse_path_blobs(
            &chain[..=target_pos],
            rel_path,
            &mut |snapshot_index, blob_id| {
                let snapshot_id = chain
                    .get(snapshot_index)
                    .cloned()
                    .ok_or_else(|| "Reverse blame snapshot index is out of bounds.".to_string())?;
                if blob_id.is_some() {
                    oldest_existing_snapshot_id = Some(snapshot_id.clone());
                }
                let Some((child_pos, child_blob_id)) = child_state.take() else {
                    let blob_id = blob_id.ok_or_else(|| {
                        format!("Path {rel_path} does not exist in snapshot {target_snapshot_id}.")
                    })?;
                    if snapshot_index != target_pos || blob_id != target_blob_id {
                        return Err(
                            "Selected blame target path changed while opening reverse lineage."
                                .to_string(),
                        );
                    }
                    child_state = Some((snapshot_index, blob_id));
                    return Ok(true);
                };
                let child_snapshot_id = chain
                    .get(child_pos)
                    .cloned()
                    .ok_or_else(|| "Reverse blame child index is out of bounds.".to_string())?;
                let Some(parent_blob_id) = blob_id else {
                    assign_tracked_owners(&mut owners, &tracked, &child_snapshot_id);
                    tracked.clear();
                    return Ok(false);
                };
                if child_blob_id != parent_blob_id {
                    let parent_snapshot_id = snapshot_id;
                    let child_lines = blob_text_lines_for_lineage(
                        lineage,
                        &child_blob_id,
                        &format!("Snapshot {child_snapshot_id}:{rel_path}"),
                    )?;
                    let parent_lines = blob_text_lines_for_lineage(
                        lineage,
                        &parent_blob_id,
                        &format!("Snapshot {parent_snapshot_id}:{rel_path}"),
                    )?;
                    tracked = map_tracked_lines_to_parent(
                        &parent_lines,
                        &child_lines,
                        &tracked,
                        &child_snapshot_id,
                        &mut owners,
                    );
                }
                child_state = Some((snapshot_index, parent_blob_id));
                Ok(!tracked.is_empty())
            },
        )?;
    }

    if !tracked.is_empty() {
        let oldest_owner =
            oldest_existing_snapshot_id.unwrap_or_else(|| target_snapshot_id.to_string());
        assign_tracked_owners(&mut owners, &tracked, &oldest_owner);
    }

    owners
        .into_iter()
        .map(|owner| {
            owner.ok_or_else(|| "Selected blame line ownership could not be resolved.".to_string())
        })
        .collect()
}

type MarkdownPlanLineOwners = (Vec<String>, Vec<String>, Vec<String>);

fn compute_markdown_plan_line_owners(
    repo: &RepoRuntime,
    plan: &JsonValue,
    rel_path: &str,
    revisions: &[JsonValue],
    current_head_revision_id: &str,
) -> Result<MarkdownPlanLineOwners, String> {
    let current_bytes = current_repo_root_bytes(repo, rel_path)?;
    let current_lines = decode_text_lines(&current_bytes, &format!("Workspace file {rel_path}"))?;
    let mut previous_lines = Vec::new();
    let mut previous_owners = Vec::new();
    let mut skipped_revision_ids = Vec::new();
    for revision in revisions {
        let revision_id = required_string_field(revision, "plan_revision_id")?;
        let revision_bytes = match plan_revision_body_bytes(
            repo,
            plan,
            rel_path,
            revision,
            current_head_revision_id,
            &current_bytes,
        ) {
            Ok(value) => value,
            Err(PlanRevisionBodyError::MissingBody(revision_id)) => {
                skipped_revision_ids.push(revision_id);
                continue;
            }
            Err(PlanRevisionBodyError::Hard(message)) => return Err(message),
        };
        let next_lines = decode_text_lines(
            &revision_bytes,
            &format!("Plan revision {revision_id}:{rel_path}"),
        )?;
        if previous_lines.is_empty() {
            previous_lines = next_lines;
            previous_owners = vec![revision_id; previous_lines.len()];
            continue;
        }
        previous_owners =
            apply_line_diff(&previous_lines, &next_lines, &previous_owners, &revision_id);
        previous_lines = next_lines;
    }
    if previous_lines.is_empty() {
        return Err(format!(
            "Lineage-only Markdown path {rel_path} does not have any readable plan revision bodies locally or from the published remote lineage."
        ));
    }
    Ok((current_lines, previous_owners, skipped_revision_ids))
}

fn snapshot_overlay(
    repo: &RepoRuntime,
    snapshot_ids: &[String],
    target: &BlameTarget,
) -> Result<BTreeMap<String, JsonMap<String, JsonValue>>, String> {
    let direct_rows = crate::primitives::resolved_snapshot_ownership_rows(repo, snapshot_ids)?;
    let direct_by_snapshot = direct_rows
        .into_iter()
        .filter_map(|row| string_field(&row, "snapshot_id").map(|snapshot_id| (snapshot_id, row)))
        .collect::<BTreeMap<_, _>>();
    let mut overlay = BTreeMap::new();
    let patchset_revision_snapshot_id = target.revision_snapshot_id.clone();
    for snapshot_id in snapshot_ids {
        let direct = direct_by_snapshot.get(snapshot_id);
        let mut entry = JsonMap::new();
        insert_optional_string(
            &mut entry,
            "task_id",
            direct.and_then(|row| string_field(row, "task_id")),
        );
        insert_optional_string(
            &mut entry,
            "change_id",
            direct.and_then(|row| string_field(row, "change_id")),
        );
        entry.insert("patchset_id".to_string(), JsonValue::Null);
        entry.insert("land_id".to_string(), JsonValue::Null);
        entry.insert("submission_id".to_string(), JsonValue::Null);
        insert_optional_string(
            &mut entry,
            "author_mode",
            direct.and_then(|row| string_field(row, "author_mode")),
        );
        insert_optional_string(
            &mut entry,
            "model_name",
            direct.and_then(|row| string_field(row, "model_name")),
        );
        insert_optional_string(
            &mut entry,
            "worktree_name",
            direct.and_then(|row| string_field(row, "worktree_name")),
        );
        entry.insert(
            "provenance_confidence".to_string(),
            JsonValue::String(
                if direct.is_some() {
                    "bound_worktree_snapshot_line"
                } else {
                    "unknown"
                }
                .to_string(),
            ),
        );
        if patchset_revision_snapshot_id.as_deref() == Some(snapshot_id.as_str()) {
            insert_optional_string(&mut entry, "patchset_id", target.patchset_id.clone());
            if string_field_obj(&entry, "change_id").is_none() {
                insert_optional_string(&mut entry, "change_id", target.change_id.clone());
            }
            if string_field_obj(&entry, "task_id").is_none() {
                insert_optional_string(&mut entry, "task_id", target.task_id.clone());
            }
            if string_field_obj(&entry, "provenance_confidence").as_deref()
                != Some("bound_worktree_snapshot_line")
            {
                entry.insert(
                    "provenance_confidence".to_string(),
                    JsonValue::String("derived_from_patchset".to_string()),
                );
            }
        }
        overlay.insert(snapshot_id.clone(), entry);
    }
    if matches!(target.kind.as_str(), "patchset" | "snapshot") {
        let mut change_ids = overlay
            .values()
            .filter_map(|row| string_field_obj(row, "change_id"))
            .collect::<Vec<_>>();
        if let Some(change_id) = target.change_id.clone() {
            change_ids.push(change_id);
        }
        let remote_overlay = remote_change_overlay(repo, target, &change_ids)?;
        for (snapshot_id, remote_entry) in remote_overlay {
            let Some(entry) = overlay.get_mut(&snapshot_id) else {
                continue;
            };
            apply_overlay_defaults(entry, &remote_entry);
            if string_field_obj(entry, "provenance_confidence").as_deref() == Some("unknown") {
                if let Some(confidence) = string_field_obj(&remote_entry, "provenance_confidence") {
                    entry.insert(
                        "provenance_confidence".to_string(),
                        JsonValue::String(confidence),
                    );
                }
            }
        }
    }
    Ok(overlay)
}

fn remote_change_overlay(
    repo: &RepoRuntime,
    target: &BlameTarget,
    change_ids: &[String],
) -> Result<BTreeMap<String, JsonMap<String, JsonValue>>, String> {
    let requested = ordered_unique(change_ids);
    if requested.is_empty() {
        return Ok(BTreeMap::new());
    }
    let remote_row = match repo.remote_row(target.remote_name.as_deref()) {
        Ok(value) => value,
        Err(_) => return Ok(BTreeMap::new()),
    };
    let repo_name = target
        .repo_name
        .clone()
        .or(remote_row.repo_name.clone())
        .unwrap_or_else(|| repo.repo_name());
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    let mut overlay = BTreeMap::new();
    for change_id in requested {
        let detail = match task_remote.get_change_detail(&change_id, Some(&repo_name)) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
        let patchsets = match closeout_remote.list_patchsets(&change_id, Some(&repo_name)) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let task = detail.get("task").and_then(JsonValue::as_object).cloned();
        let landing = detail
            .get("landing_summary")
            .and_then(JsonValue::as_object)
            .cloned()
            .unwrap_or_default();
        let landing_result = landing
            .get("result")
            .and_then(JsonValue::as_object)
            .cloned()
            .unwrap_or_default();
        for patchset in patchsets {
            let Some(revision_snapshot_id) = string_field(&patchset, "revision_snapshot_id") else {
                continue;
            };
            let row = overlay
                .entry(revision_snapshot_id)
                .or_insert_with(JsonMap::new);
            let mut defaults = JsonMap::new();
            insert_optional_string(
                &mut defaults,
                "task_id",
                task.as_ref()
                    .and_then(|value| string_field_obj(value, "task_id"))
                    .or_else(|| string_field(&detail, "task_id")),
            );
            insert_optional_string(&mut defaults, "change_id", Some(change_id.clone()));
            insert_optional_string(
                &mut defaults,
                "patchset_id",
                string_field(&patchset, "patchset_id"),
            );
            insert_optional_string(
                &mut defaults,
                "provenance_confidence",
                Some("derived_from_patchset".to_string()),
            );
            apply_overlay_defaults(row, &defaults);
        }
        let Some(landed_snapshot_id) = string_field_obj(&landing_result, "landed_snapshot_id")
        else {
            continue;
        };
        let row = overlay
            .entry(landed_snapshot_id)
            .or_insert_with(JsonMap::new);
        let mut defaults = JsonMap::new();
        insert_optional_string(
            &mut defaults,
            "task_id",
            task.as_ref()
                .and_then(|value| string_field_obj(value, "task_id"))
                .or_else(|| string_field(&detail, "task_id")),
        );
        insert_optional_string(&mut defaults, "change_id", Some(change_id));
        insert_optional_string(
            &mut defaults,
            "patchset_id",
            string_field_obj(&landing, "patchset_id"),
        );
        insert_optional_string(
            &mut defaults,
            "submission_id",
            string_field_obj(&landing, "submission_id"),
        );
        insert_optional_string(
            &mut defaults,
            "provenance_confidence",
            Some("derived_from_land".to_string()),
        );
        apply_overlay_defaults(row, &defaults);
    }
    Ok(overlay)
}

fn line_row_payload(
    rel_path: &str,
    line_number: usize,
    snapshot_row: &JsonValue,
    overlay: Option<&JsonMap<String, JsonValue>>,
) -> JsonValue {
    let empty = JsonMap::new();
    let overlay = overlay.unwrap_or(&empty);
    json!({
        "path": rel_path,
        "start_line": line_number,
        "end_line": line_number,
        "snapshot_id": snapshot_row.get("snapshot_id").cloned().unwrap_or(JsonValue::Null),
        "parent_snapshot_id": snapshot_row.get("parent_snapshot_id").cloned().unwrap_or(JsonValue::Null),
        "line_name": snapshot_row.get("line_name").cloned().unwrap_or(JsonValue::Null),
        "message": snapshot_row.get("message").cloned().unwrap_or(JsonValue::Null),
        "created_at": snapshot_row.get("created_at").cloned().unwrap_or(JsonValue::Null),
        "task_id": overlay.get("task_id").cloned().unwrap_or(JsonValue::Null),
        "change_id": overlay.get("change_id").cloned().unwrap_or(JsonValue::Null),
        "patchset_id": overlay.get("patchset_id").cloned().unwrap_or(JsonValue::Null),
        "land_id": overlay.get("land_id").cloned().unwrap_or(JsonValue::Null),
        "submission_id": overlay.get("submission_id").cloned().unwrap_or(JsonValue::Null),
        "author_mode": overlay.get("author_mode").cloned().unwrap_or(JsonValue::Null),
        "model_name": overlay.get("model_name").cloned().unwrap_or(JsonValue::Null),
        "worktree_name": overlay.get("worktree_name").cloned().unwrap_or(JsonValue::Null),
        "provenance_confidence": overlay
            .get("provenance_confidence")
            .cloned()
            .unwrap_or_else(|| JsonValue::String("unknown".to_string())),
    })
}

fn markdown_plan_row_map(plan: &JsonValue, revisions: &[JsonValue]) -> BTreeMap<String, JsonValue> {
    let plan_id = string_field(plan, "plan_id").unwrap_or_default();
    revisions
        .iter()
        .filter_map(|revision| {
            let revision_id = string_field(revision, "plan_revision_id")?;
            Some((
                revision_id,
                json!({
                    "plan_id": plan_id,
                    "plan_revision_id": revision.get("plan_revision_id").cloned().unwrap_or(JsonValue::Null),
                    "parent_plan_revision_id": revision.get("parent_plan_revision_id").cloned().unwrap_or(JsonValue::Null),
                    "revision_number": revision.get("revision_number").cloned().unwrap_or(JsonValue::Null),
                    "title_snapshot": revision.get("title_snapshot").cloned().unwrap_or(JsonValue::Null),
                    "artifact_path": revision.get("artifact_path").cloned().unwrap_or(JsonValue::Null),
                    "artifact_selector": revision.get("artifact_selector").cloned().unwrap_or(JsonValue::Null),
                    "artifact_heading": revision.get("artifact_heading").cloned().unwrap_or(JsonValue::Null),
                    "created_at": revision.get("created_at").cloned().unwrap_or(JsonValue::Null),
                    "created_by": revision.get("created_by").cloned().unwrap_or(JsonValue::Null),
                    "actor_type": revision.get("actor_type").cloned().unwrap_or(JsonValue::Null),
                    "source_kind": revision.get("source_kind").cloned().unwrap_or(JsonValue::Null),
                }),
            ))
        })
        .collect()
}

fn markdown_line_row_payload(
    rel_path: &str,
    line_number: usize,
    revision_row: &JsonValue,
) -> JsonValue {
    json!({
        "path": rel_path,
        "start_line": line_number,
        "end_line": line_number,
        "plan_id": revision_row.get("plan_id").cloned().unwrap_or(JsonValue::Null),
        "plan_revision_id": revision_row.get("plan_revision_id").cloned().unwrap_or(JsonValue::Null),
        "parent_plan_revision_id": revision_row.get("parent_plan_revision_id").cloned().unwrap_or(JsonValue::Null),
        "revision_number": revision_row.get("revision_number").cloned().unwrap_or(JsonValue::Null),
        "title_snapshot": revision_row.get("title_snapshot").cloned().unwrap_or(JsonValue::Null),
        "artifact_path": revision_row.get("artifact_path").cloned().unwrap_or(JsonValue::Null),
        "artifact_selector": revision_row.get("artifact_selector").cloned().unwrap_or(JsonValue::Null),
        "artifact_heading": revision_row.get("artifact_heading").cloned().unwrap_or(JsonValue::Null),
        "created_at": revision_row.get("created_at").cloned().unwrap_or(JsonValue::Null),
        "created_by": revision_row.get("created_by").cloned().unwrap_or(JsonValue::Null),
        "actor_type": revision_row.get("actor_type").cloned().unwrap_or(JsonValue::Null),
        "source_kind": revision_row.get("source_kind").cloned().unwrap_or(JsonValue::Null),
        "provenance_confidence": "direct_plan_revision_binding",
    })
}

fn collapse_line_rows(line_rows: &[JsonValue]) -> Vec<JsonValue> {
    let mut hunks: Vec<JsonValue> = Vec::new();
    for row in line_rows {
        if hunks.is_empty() {
            hunks.push(row.clone());
            continue;
        }
        let previous = hunks.last_mut().unwrap();
        let previous_signature = line_signature(previous);
        let current_signature = line_signature(row);
        let previous_end = json_u64_field(previous, "end_line").unwrap_or(0);
        let current_start = json_u64_field(row, "start_line").unwrap_or(0);
        if previous_signature == current_signature && previous_end + 1 == current_start {
            if let Some(obj) = previous.as_object_mut() {
                obj.insert(
                    "end_line".to_string(),
                    JsonValue::Number(ait_core::json_support::JsonNumber::from(
                        json_u64_field(row, "end_line").unwrap_or(previous_end),
                    )),
                );
            }
            continue;
        }
        hunks.push(row.clone());
    }
    hunks
}

fn line_signature(row: &JsonValue) -> Vec<String> {
    [
        "snapshot_id",
        "plan_id",
        "plan_revision_id",
        "task_id",
        "change_id",
        "patchset_id",
        "land_id",
        "submission_id",
        "author_mode",
        "model_name",
        "source_kind",
        "provenance_confidence",
    ]
    .iter()
    .map(|key| value_signature(row.get(*key)))
    .collect()
}

fn current_plan_for_artifact_path(
    repo: &RepoRuntime,
    rel_path: &str,
    plan_id: Option<&str>,
    plan_ref: Option<&str>,
) -> Result<JsonValue, String> {
    let plan_store = repo
        .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .plans();
    let plans = list_plans_with_plan_store(&plan_store)
        .map_err(|err| err.to_string())?
        .iter()
        .map(plan_record_list_json)
        .collect::<Vec<_>>();
    let candidates = plans
        .into_iter()
        .filter(|plan| plan_head_artifact_field(plan, "artifact_path").as_deref() == Some(rel_path))
        .collect::<Vec<_>>();
    let open_candidates = candidates
        .iter()
        .filter(|plan| !plan_status_is_historical(plan.get("status")))
        .cloned()
        .collect::<Vec<_>>();
    if open_candidates.is_empty() {
        if !candidates.is_empty() {
            return Err(format!(
                "Lineage-only Markdown path {rel_path} is tracked only by historical plans. Use the repo root planning surface to inspect the historical record."
            ));
        }
        return Err(format!(
            "Lineage-only Markdown path {rel_path} is not present in line snapshots and is not yet tracked in local plan lineage. Run `ait plan sync {rel_path}` first."
        ));
    }
    if let Some(plan_id) = normalized_text(plan_id) {
        let selected = open_candidates
            .iter()
            .find(|plan| string_field(plan, "plan_id").as_deref() == Some(plan_id.as_str()));
        let Some(selected) = selected else {
            return Err(format!(
                "Lineage-only Markdown path {rel_path} is not tracked by current plan {plan_id}. Use a current plan id for this artifact path."
            ));
        };
        return get_plan_with_plan_store(&plan_store, &required_string_field(selected, "plan_id")?)
            .map(|record| plan_record_detail_json(&record))
            .map_err(|err| err.to_string());
    }
    if let Some(plan_ref) = normalized_text(plan_ref) {
        let selected = open_candidates
            .iter()
            .filter(|plan| {
                plan_head_artifact_field(plan, "artifact_selector").as_deref()
                    == Some(plan_ref.as_str())
            })
            .cloned()
            .collect::<Vec<_>>();
        if selected.is_empty() {
            let known_refs = known_markdown_plan_refs(&open_candidates);
            let known_detail = if known_refs.is_empty() {
                String::new()
            } else {
                format!(" Known tracked refs: {}.", known_refs.join(", "))
            };
            return Err(format!(
                "Lineage-only Markdown path {rel_path} is not tracked by current plan ref {plan_ref}.{known_detail}"
            ));
        }
        if selected.len() > 1 {
            return Err(format!(
                "Multiple current plans track lineage-only Markdown path {rel_path} with selector {plan_ref}. Use `--plan-id` to choose one concrete plan."
            ));
        }
        return get_plan_with_plan_store(
            &plan_store,
            &required_string_field(&selected[0], "plan_id")?,
        )
        .map(|record| plan_record_detail_json(&record))
        .map_err(|err| err.to_string());
    }
    if open_candidates.len() > 1 {
        let selector_sample = known_markdown_plan_refs(&open_candidates);
        let selector_detail = if selector_sample.is_empty() {
            String::new()
        } else {
            format!(" Known tracked refs: {}.", selector_sample.join(", "))
        };
        return Err(format!(
            "Multiple current plans track lineage-only Markdown path {rel_path}.{selector_detail} Use `--plan-ref` or `--plan-id` to choose one current tracked plan."
        ));
    }
    get_plan_with_plan_store(
        &plan_store,
        &required_string_field(&open_candidates[0], "plan_id")?,
    )
    .map(|record| plan_record_detail_json(&record))
    .map_err(|err| err.to_string())
}

fn plan_status_is_historical(status: Option<&JsonValue>) -> bool {
    status
        .and_then(JsonValue::as_str)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "archived" | "superseded"
            )
        })
        .unwrap_or(false)
}

fn plan_head_artifact_field(plan: &JsonValue, field: &str) -> Option<String> {
    let nested = plan
        .get("head_revision")
        .and_then(JsonValue::as_object)
        .and_then(|head| string_field_obj(head, field));
    if nested.is_some() {
        return nested;
    }
    string_field(plan, &format!("head_{field}"))
}

fn known_markdown_plan_refs(plans: &[JsonValue]) -> Vec<String> {
    let mut refs = plans
        .iter()
        .filter_map(|plan| plan_head_artifact_field(plan, "artifact_selector"))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    refs.sort();
    refs
}

enum PlanRevisionBodyError {
    MissingBody(String),
    Hard(String),
}

fn plan_revision_body_bytes(
    repo: &RepoRuntime,
    plan: &JsonValue,
    rel_path: &str,
    revision: &JsonValue,
    current_head_revision_id: &str,
    current_bytes: &[u8],
) -> Result<Vec<u8>, PlanRevisionBodyError> {
    let blob_id = string_field(revision, "artifact_blob_id");
    let revision_id =
        required_string_field(revision, "plan_revision_id").map_err(PlanRevisionBodyError::Hard)?;
    if revision_id == current_head_revision_id {
        let expected_blob_id = format!("BLB-{}", &sha256_hex(current_bytes)[..20]);
        if let Some(ref blob_id) = blob_id {
            if blob_id != &expected_blob_id {
                return Err(PlanRevisionBodyError::Hard(format!(
                    "Lineage-only Markdown path {rel_path} has unsynced local edits relative to local plan head {revision_id}. Run `ait plan sync {rel_path}` first."
                )));
            }
        }
        return Ok(current_bytes.to_vec());
    }
    if let Some(blob_id) = blob_id {
        let blob_store =
            local_snapshot_operation_store(repo).map_err(PlanRevisionBodyError::Hard)?;
        match blob_store.read_blob_bytes(&blob_id) {
            Ok(bytes) => return Ok(bytes),
            Err(_) => {
                if let Some(repaired) =
                    repair_markdown_plan_revision_blob_bytes(repo, plan, revision, rel_path)
                {
                    return repaired.map_err(PlanRevisionBodyError::Hard);
                }
            }
        }
    }
    Err(PlanRevisionBodyError::MissingBody(revision_id))
}

fn repair_markdown_plan_revision_blob_bytes(
    repo: &RepoRuntime,
    plan: &JsonValue,
    revision: &JsonValue,
    rel_path: &str,
) -> Option<Result<Vec<u8>, String>> {
    let published_remote_name = string_field(plan, "published_remote_name")?;
    let published_plan_id =
        string_field(plan, "published_plan_id").or_else(|| string_field(plan, "plan_id"))?;
    let published_revision_id = string_field(revision, "published_plan_revision_id")?;
    let expected_blob_id = string_field(revision, "artifact_blob_id");
    let remote_row = repo.remote_row(Some(&published_remote_name)).ok()?;
    let remote_revision = get_plan_revision(
        http_config(repo, &remote_row),
        &published_plan_id,
        &published_revision_id,
    )
    .ok()?;
    let artifact_body = remote_revision.get("artifact_body")?.as_str()?.to_string();
    let mut artifact_bytes = artifact_body.into_bytes();
    if let Some(expected_blob_id) = expected_blob_id.as_deref() {
        let candidate_blob_id = format!("BLB-{}", &sha256_hex(&artifact_bytes)[..20]);
        if candidate_blob_id != expected_blob_id {
            if !artifact_bytes.ends_with(b"\n") {
                let newline_restored = {
                    let mut bytes = artifact_bytes.clone();
                    bytes.push(b'\n');
                    bytes
                };
                if format!("BLB-{}", &sha256_hex(&newline_restored)[..20]) == expected_blob_id {
                    artifact_bytes = newline_restored;
                } else {
                    return Some(Err(format!(
                        "Published remote plan revision {published_revision_id} for lineage-only Markdown path {rel_path} materialized blob {candidate_blob_id}, expected {expected_blob_id}."
                    )));
                }
            } else {
                return Some(Err(format!(
                    "Published remote plan revision {published_revision_id} for lineage-only Markdown path {rel_path} materialized blob {candidate_blob_id}, expected {expected_blob_id}."
                )));
            }
        }
    }
    let path_hint =
        string_field(&remote_revision, "artifact_path").unwrap_or_else(|| rel_path.to_string());
    let materialized_blob_id =
        match ensure_selected_blob_bytes(repo, &artifact_bytes, Some(&path_hint)) {
            Ok(value) => value,
            Err(err) => return Some(Err(err)),
        };
    if let Some(expected_blob_id) = expected_blob_id.as_deref() {
        if materialized_blob_id != expected_blob_id {
            return Some(Err(format!(
                "Published remote plan revision {published_revision_id} for lineage-only Markdown path {rel_path} materialized blob {}, expected {expected_blob_id}.",
                materialized_blob_id
            )));
        }
    }
    Some(read_selected_blob_bytes(repo, &materialized_blob_id))
}

fn local_snapshot_operation_store(
    repo: &RepoRuntime,
) -> Result<RepoLocalSnapshotOperationStore<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>, String> {
    let workspace_root = repo.workspace_root();
    repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)
}

fn snapshot_store(
    repo: &RepoRuntime,
) -> Result<RepoLocalSnapshotOperationStore<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>, String> {
    local_snapshot_operation_store(repo)
}

fn read_selected_blob_bytes(repo: &RepoRuntime, blob_id: &str) -> Result<Vec<u8>, String> {
    local_snapshot_operation_store(repo)?.read_blob_bytes(blob_id)
}

fn ensure_selected_blob_bytes(
    repo: &RepoRuntime,
    data: &[u8],
    path_hint: Option<&str>,
) -> Result<String, String> {
    let workspace_root = repo.workspace_root();
    let store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    store.ensure_blob_bytes(data, path_hint)
}

fn get_local_snapshot_metadata_for_repo(
    repo: &RepoRuntime,
    snapshot_id: &str,
) -> Result<JsonValue, String> {
    let store = snapshot_store(repo)?;
    snapshot_metadata_payload_with_store(&store, snapshot_id)
}

fn snapshot_metadata_payload_with_store<S>(
    store: &S,
    snapshot_id: &str,
) -> Result<JsonValue, String>
where
    S: SnapshotStore + ?Sized,
{
    let snapshot = store
        .snapshot_by_id(snapshot_id)?
        .ok_or_else(|| format!("Unknown snapshot: {snapshot_id}"))?;
    Ok(SnapshotJson::stateless().snapshot_record_payload(&snapshot))
}

fn blob_text_lines(
    blob_id: &str,
    blob_bytes_by_id: &BTreeMap<String, Vec<u8>>,
    cache: &mut BTreeMap<String, Vec<String>>,
    label: &str,
) -> Result<Vec<String>, String> {
    if let Some(existing) = cache.get(blob_id) {
        return Ok(existing.clone());
    }
    let bytes = blob_bytes_by_id
        .get(blob_id)
        .ok_or_else(|| format!("Blob payload missing for `{blob_id}`."))?;
    let lines = decode_text_lines(bytes, label)?;
    cache.insert(blob_id.to_string(), lines.clone());
    Ok(lines)
}

fn blob_text_lines_for_lineage<B>(
    lineage: &mut SnapshotBlameLineage<B>,
    blob_id: &str,
    label: &str,
) -> Result<Vec<String>, String>
where
    B: LocalSnapshotBlobReadStore,
{
    if let Some(existing) = lineage.blob_lines_cache.get(blob_id) {
        return Ok(existing.clone());
    }
    if !lineage.blob_bytes_by_id.contains_key(blob_id) {
        let bytes = lineage.blob_store.read_blob_bytes(blob_id)?;
        lineage.blob_bytes_by_id.insert(blob_id.to_string(), bytes);
    }
    blob_text_lines(
        blob_id,
        &lineage.blob_bytes_by_id,
        &mut lineage.blob_lines_cache,
        label,
    )
}

fn current_repo_root_bytes(repo: &RepoRuntime, rel_path: &str) -> Result<Vec<u8>, String> {
    for root in [repo.workspace_root(), repo.authoritative_repo_root()] {
        let abs_path = root.join(rel_path);
        if !abs_path.exists() {
            continue;
        }
        if abs_path.is_dir() {
            return Err(format!("Path {rel_path} is a directory, not a file."));
        }
        return fs::read(&abs_path).map_err(|err| err.to_string());
    }
    Err(format!("Workspace file {rel_path} does not exist."))
}

fn normalize_blame_path(repo: &RepoRuntime, path_value: &str) -> Result<String, String> {
    let raw = path_value.trim();
    if raw.is_empty() {
        return Err("Path is required.".to_string());
    }
    let root = repo.workspace_root();
    let root = root.canonicalize().unwrap_or(root);
    let candidate = PathBuf::from(raw);
    let normalized = if candidate.is_absolute() {
        lexical_normalize(&candidate)
    } else {
        lexical_normalize(&root.join(candidate))
    };
    let rel_path = normalized
        .strip_prefix(&root)
        .map_err(|_| format!("Path {raw:?} is outside the current workspace root."))?
        .to_string_lossy()
        .replace('\\', "/");
    if rel_path.is_empty() || rel_path == "." {
        return Err("Choose one file path to blame.".to_string());
    }
    Ok(rel_path)
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn line_selection(
    total_lines: usize,
    line: Option<usize>,
    start_line: Option<usize>,
    end_line: Option<usize>,
) -> Result<(usize, usize), String> {
    if line.is_some() && (start_line.is_some() || end_line.is_some()) {
        return Err("Choose either --line or --start/--end.".to_string());
    }
    let (start_line, end_line) = if let Some(line) = line {
        (Some(line), Some(line))
    } else {
        (start_line, end_line)
    };
    if start_line.is_none() && end_line.is_none() {
        return if total_lines == 0 {
            Ok((0, 0))
        } else {
            Ok((1, total_lines))
        };
    }
    let Some(start_line) = start_line else {
        return Err("Provide both --start and --end.".to_string());
    };
    let Some(end_line) = end_line else {
        return Err("Provide both --start and --end.".to_string());
    };
    if start_line == 0 || end_line == 0 {
        return Err("Line selections are 1-based and must be positive.".to_string());
    }
    if end_line < start_line {
        return Err("The selected range must have end >= start.".to_string());
    }
    if total_lines == 0 {
        return Err("The selected file is empty and has no blameable lines.".to_string());
    }
    if end_line > total_lines {
        return Err(format!(
            "Selected range {start_line}-{end_line} exceeds file length {total_lines}."
        ));
    }
    Ok((start_line, end_line))
}

fn has_explicit_line_selection(
    line: Option<usize>,
    start_line: Option<usize>,
    end_line: Option<usize>,
) -> bool {
    line.is_some() || start_line.is_some() || end_line.is_some()
}

fn apply_line_diff(
    old_lines: &[String],
    new_lines: &[String],
    old_owners: &[String],
    revision_id: &str,
) -> Vec<String> {
    let ops = capture_diff_slices(Algorithm::Myers, old_lines, new_lines);
    let mut owners = Vec::with_capacity(new_lines.len());
    for op in ops {
        match op {
            DiffOp::Equal { old_index, len, .. } => {
                owners.extend(old_owners[old_index..old_index + len].iter().cloned());
            }
            DiffOp::Insert { new_len, .. } | DiffOp::Replace { new_len, .. } => {
                owners.extend(std::iter::repeat_n(revision_id.to_string(), new_len));
            }
            DiffOp::Delete { .. } => {}
        }
    }
    owners
}

fn map_tracked_lines_to_parent(
    old_lines: &[String],
    new_lines: &[String],
    tracked: &[SelectedLineTracker],
    child_snapshot_id: &str,
    owners: &mut [Option<String>],
) -> Vec<SelectedLineTracker> {
    let prefix_len = common_prefix_len(old_lines, new_lines);
    let suffix_len = common_suffix_len(old_lines, new_lines, prefix_len);
    let old_middle_start = prefix_len;
    let old_middle_end = old_lines.len().saturating_sub(suffix_len);
    let new_middle_start = prefix_len;
    let new_middle_end = new_lines.len().saturating_sub(suffix_len);
    let mut middle_tracked = Vec::new();
    let mut next_tracked = Vec::with_capacity(tracked.len());

    for item in tracked {
        if item.current_index < prefix_len {
            next_tracked.push(item.clone());
        } else if item.current_index >= new_middle_end {
            let old_index = old_lines.len() - (new_lines.len() - item.current_index);
            next_tracked.push(SelectedLineTracker {
                output_index: item.output_index,
                current_index: old_index,
            });
        } else {
            middle_tracked.push(SelectedLineTracker {
                output_index: item.output_index,
                current_index: item.current_index - new_middle_start,
            });
        }
    }

    if middle_tracked.is_empty() {
        return next_tracked;
    }

    let mut mapped_middle = map_tracked_lines_to_parent_with_ops(
        &old_lines[old_middle_start..old_middle_end],
        &new_lines[new_middle_start..new_middle_end],
        &middle_tracked,
        child_snapshot_id,
        owners,
    );
    for item in &mut mapped_middle {
        item.current_index += old_middle_start;
    }
    next_tracked.extend(mapped_middle);
    next_tracked.sort_by_key(|item| item.current_index);
    next_tracked
}

fn map_tracked_lines_to_parent_with_ops(
    old_lines: &[String],
    new_lines: &[String],
    tracked: &[SelectedLineTracker],
    child_snapshot_id: &str,
    owners: &mut [Option<String>],
) -> Vec<SelectedLineTracker> {
    let ops = capture_diff_slices(Algorithm::Myers, old_lines, new_lines);
    let mut next_tracked = Vec::with_capacity(tracked.len());
    let mut cursor = 0usize;
    for op in ops {
        match op {
            DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => {
                consume_uncovered_tracked_lines(
                    tracked,
                    &mut cursor,
                    new_index,
                    owners,
                    child_snapshot_id,
                );
                let new_end = new_index + len;
                while cursor < tracked.len() && tracked[cursor].current_index < new_end {
                    let item = &tracked[cursor];
                    next_tracked.push(SelectedLineTracker {
                        output_index: item.output_index,
                        current_index: old_index + (item.current_index - new_index),
                    });
                    cursor += 1;
                }
            }
            DiffOp::Insert {
                new_index, new_len, ..
            }
            | DiffOp::Replace {
                new_index, new_len, ..
            } => {
                consume_uncovered_tracked_lines(
                    tracked,
                    &mut cursor,
                    new_index,
                    owners,
                    child_snapshot_id,
                );
                let new_end = new_index + new_len;
                while cursor < tracked.len() && tracked[cursor].current_index < new_end {
                    owners[tracked[cursor].output_index] = Some(child_snapshot_id.to_string());
                    cursor += 1;
                }
            }
            DiffOp::Delete { .. } => {}
        }
    }
    consume_uncovered_tracked_lines(tracked, &mut cursor, usize::MAX, owners, child_snapshot_id);
    next_tracked
}

fn common_prefix_len(old_lines: &[String], new_lines: &[String]) -> usize {
    old_lines
        .iter()
        .zip(new_lines.iter())
        .take_while(|(old_line, new_line)| old_line == new_line)
        .count()
}

fn common_suffix_len(old_lines: &[String], new_lines: &[String], prefix_len: usize) -> usize {
    let max_suffix = old_lines
        .len()
        .saturating_sub(prefix_len)
        .min(new_lines.len().saturating_sub(prefix_len));
    let mut suffix_len = 0usize;
    while suffix_len < max_suffix {
        let old_index = old_lines.len() - 1 - suffix_len;
        let new_index = new_lines.len() - 1 - suffix_len;
        if old_lines[old_index] != new_lines[new_index] {
            break;
        }
        suffix_len += 1;
    }
    suffix_len
}

fn consume_uncovered_tracked_lines(
    tracked: &[SelectedLineTracker],
    cursor: &mut usize,
    next_new_index: usize,
    owners: &mut [Option<String>],
    child_snapshot_id: &str,
) {
    while *cursor < tracked.len() && tracked[*cursor].current_index < next_new_index {
        owners[tracked[*cursor].output_index] = Some(child_snapshot_id.to_string());
        *cursor += 1;
    }
}

fn assign_tracked_owners(
    owners: &mut [Option<String>],
    tracked: &[SelectedLineTracker],
    owner_id: &str,
) {
    for item in tracked {
        owners[item.output_index] = Some(owner_id.to_string());
    }
}

fn decode_text_lines(data: &[u8], label: &str) -> Result<Vec<String>, String> {
    if data.contains(&0) {
        return Err(format!("{label} is binary and cannot be blamed."));
    }
    let text = std::str::from_utf8(data)
        .map_err(|_| format!("{label} is not valid UTF-8 text and cannot be blamed."))?;
    let mut lines = text
        .split_inclusive('\n')
        .map(str::to_string)
        .collect::<Vec<_>>();
    if !text.is_empty() && !text.ends_with('\n') {
        let last_len = lines.iter().map(|line| line.len()).sum::<usize>();
        if last_len < text.len() {
            lines.push(text[last_len..].to_string());
        } else if lines.is_empty() {
            lines.push(text.to_string());
        }
    }
    Ok(lines)
}

fn ordered_unique(values: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut ordered = Vec::new();
    for value in values {
        if seen.insert(value.clone()) {
            ordered.push(value.clone());
        }
    }
    ordered
}

fn format_hunk(row: &JsonValue) -> Result<String, String> {
    let start_line = json_u64_field(row, "start_line").unwrap_or(0);
    let end_line = json_u64_field(row, "end_line").unwrap_or(0);
    let line_label = if start_line == end_line {
        start_line.to_string()
    } else {
        format!("{start_line}-{end_line}")
    };
    let owner_id = string_field(row, "snapshot_id")
        .or_else(|| string_field(row, "plan_revision_id"))
        .unwrap_or_default();
    let mut details = vec![owner_id];
    if let Some(plan_id) = string_field(row, "plan_id") {
        details.push(format!("plan={plan_id}"));
    }
    if let Some(revision_number) = row.get("revision_number") {
        if !revision_number.is_null() {
            details.push(format!(
                "revision={}",
                value_signature(Some(revision_number))
            ));
        }
    }
    for (key, label) in [
        ("task_id", "task"),
        ("change_id", "change"),
        ("patchset_id", "patchset"),
        ("land_id", "land"),
        ("submission_id", "submission"),
    ] {
        if let Some(value) = string_field(row, key) {
            details.push(format!("{label}={value}"));
        }
    }
    if let Some(confidence) = string_field(row, "provenance_confidence") {
        details.push(format!("confidence={confidence}"));
    }
    Ok(format!("{line_label:<9} {}", details.join(" ")))
}

fn remote_context(
    repo: &RepoRuntime,
    remote_name: Option<&str>,
) -> Result<(RemoteRow, String), String> {
    let remote_row = repo.remote_row(remote_name)?;
    let repo_name = remote_row
        .repo_name
        .clone()
        .unwrap_or_else(|| repo.repo_name());
    Ok((remote_row, repo_name))
}

fn http_config(repo: &RepoRuntime, remote_row: &RemoteRow) -> PlanHttpClientConfig {
    PlanHttpClientConfig {
        base_url: remote_row.url.clone(),
        repository_index: repo.repository_index(),
        headers: repo.auth_headers(),
        ..PlanHttpClientConfig::default()
    }
}

fn http_task_remote(repo: &RepoRuntime, remote_row: &RemoteRow) -> Result<HttpTaskRemote, String> {
    HttpTaskRemote::new(http_config(repo, remote_row)).map_err(|err| err.to_string())
}

fn http_closeout_remote(
    repo: &RepoRuntime,
    remote_row: &RemoteRow,
) -> Result<HttpWorkflowCloseoutRemote, String> {
    HttpWorkflowCloseoutRemote::new(http_config(repo, remote_row)).map_err(|err| err.to_string())
}

fn string_field(value: &JsonValue, key: &str) -> Option<String> {
    normalized_text(value.get(key).and_then(JsonValue::as_str))
}

fn string_field_obj(value: &JsonMap<String, JsonValue>, key: &str) -> Option<String> {
    normalized_text(value.get(key).and_then(JsonValue::as_str))
}

fn required_string_field(value: &JsonValue, key: &str) -> Result<String, String> {
    string_field(value, key).ok_or_else(|| format!("Expected non-empty string field `{key}`."))
}

fn normalized_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn insert_optional_string(
    target: &mut JsonMap<String, JsonValue>,
    key: &str,
    value: Option<String>,
) {
    target.insert(
        key.to_string(),
        value.map(JsonValue::String).unwrap_or(JsonValue::Null),
    );
}

fn apply_overlay_defaults(
    target: &mut JsonMap<String, JsonValue>,
    fallback: &JsonMap<String, JsonValue>,
) {
    for (key, value) in fallback {
        if value.is_null() {
            continue;
        }
        if target.get(key).is_none_or(JsonValue::is_null) {
            target.insert(key.clone(), value.clone());
        }
    }
}

fn json_u64_field(value: &JsonValue, key: &str) -> Option<u64> {
    value.get(key).and_then(JsonValue::as_u64)
}

fn json_u64_field_obj(value: &JsonMap<String, JsonValue>, key: &str) -> Option<u64> {
    value.get(key).and_then(JsonValue::as_u64)
}

fn json_i64_field(value: &JsonValue, key: &str) -> Option<i64> {
    value.get(key).and_then(JsonValue::as_i64)
}

fn value_signature(value: Option<&JsonValue>) -> String {
    match value {
        None | Some(JsonValue::Null) => String::new(),
        Some(JsonValue::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    }
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

impl BlameTarget {
    fn to_public_json(&self) -> JsonValue {
        json!({
            "kind": self.kind,
            "line_name": self.line_name,
            "patchset_id": self.patchset_id,
            "change_id": self.change_id,
            "task_id": self.task_id,
            "base_snapshot_id": self.base_snapshot_id,
            "revision_snapshot_id": self.revision_snapshot_id,
            "resolved_snapshot_id": self.resolved_snapshot_id,
        })
    }
}

#[cfg(test)]
mod tests;
