use crate::json_support::{encode_json_value_compact, encode_json_value_compact_or_default};

#[pyfunction(name = "artifact_blob_id")]
fn artifact_blob_id_py(markdown: &str) -> String {
    artifact_blob_id(markdown)
}

#[pyfunction(name = "artifact_candidates_open")]
fn artifact_candidates_open_py(
    py: Python<'_>,
    candidates: Bound<'_, PyAny>,
) -> PyResult<Py<PyList>> {
    let payload = parse_json_value(candidates, "candidates")?;
    render_json_list(
        py,
        match artifact_candidates_open(&payload).map_err(PyValueError::new_err)? {
            JsonValue::Array(values) => values,
            _ => {
                return Err(PyRuntimeError::new_err(
                    "Blob diff open candidates must be a list payload.",
                ))
            }
        },
    )
}

#[pyfunction(name = "plan_artifact_identity")]
fn plan_artifact_identity_py(
    py: Python<'_>,
    artifact_path: &str,
    artifact_selector: Option<&str>,
) -> PyResult<Py<PyDict>> {
    render_json_dict(py, plan_artifact_identity(artifact_path, artifact_selector))
}

#[pyfunction(name = "plan_artifact_identity_label")]
fn plan_artifact_identity_label_py(artifact_path: &str, artifact_selector: Option<&str>) -> String {
    plan_artifact_identity_label(artifact_path, artifact_selector)
}

#[pyfunction(name = "index_plans_by_artifact_path")]
fn index_plans_by_artifact_path_py(
    py: Python<'_>,
    plans: Bound<'_, PyAny>,
) -> PyResult<Py<PyDict>> {
    let payload = parse_json_value(plans, "plans")?;
    render_json_dict(
        py,
        index_plans_by_artifact_path(&payload).map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "index_plans_by_artifact_identity")]
fn index_plans_by_artifact_identity_py(
    py: Python<'_>,
    plans: Bound<'_, PyAny>,
) -> PyResult<Py<PyList>> {
    let payload = parse_json_value(plans, "plans")?;
    render_json_list(
        py,
        match index_plans_by_artifact_identity(&payload).map_err(PyValueError::new_err)? {
            JsonValue::Array(values) => values,
            _ => {
                return Err(PyRuntimeError::new_err(
                    "Blob diff identity index must be a list payload.",
                ))
            }
        },
    )
}

#[pyfunction(name = "open_generic_plans_matching_blob_id")]
fn open_generic_plans_matching_blob_id_py(
    py: Python<'_>,
    plans: Bound<'_, PyAny>,
    blob_id: &str,
) -> PyResult<Py<PyList>> {
    let payload = parse_json_value(plans, "plans")?;
    render_json_list(
        py,
        match open_generic_plans_matching_blob_id(&payload, blob_id)
            .map_err(PyValueError::new_err)?
        {
            JsonValue::Array(values) => values,
            _ => {
                return Err(PyRuntimeError::new_err(
                    "Blob diff generic plan match must be a list payload.",
                ))
            }
        },
    )
}

#[pyfunction(name = "open_plans_matching_selector")]
fn open_plans_matching_selector_py(
    py: Python<'_>,
    plans: Bound<'_, PyAny>,
    selector: &str,
) -> PyResult<Py<PyList>> {
    let payload = parse_json_value(plans, "plans")?;
    render_json_list(
        py,
        match open_plans_matching_selector(&payload, selector).map_err(PyValueError::new_err)? {
            JsonValue::Array(values) => values,
            _ => {
                return Err(PyRuntimeError::new_err(
                    "Blob diff selector plan match must be a list payload.",
                ))
            }
        },
    )
}

#[pyfunction(name = "local_plan_fully_published")]
fn local_plan_fully_published_py(plan: Bound<'_, PyAny>) -> PyResult<bool> {
    let payload = parse_json_value(plan, "plan")?;
    local_plan_fully_published(&payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "plan_heads_equivalent")]
fn plan_heads_equivalent_py(left: Bound<'_, PyAny>, right: Bound<'_, PyAny>) -> PyResult<bool> {
    let left_payload = parse_json_value(left, "left")?;
    let right_payload = parse_json_value(right, "right")?;
    plan_heads_equivalent(&left_payload, &right_payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "plan_matches_sync_artifact")]
#[pyo3(signature = (plan, artifact, require_title_match=true))]
fn plan_matches_sync_artifact_py(
    plan: Bound<'_, PyAny>,
    artifact: Bound<'_, PyAny>,
    require_title_match: bool,
) -> PyResult<bool> {
    let plan_payload = parse_json_value(plan, "plan")?;
    let artifact_payload = parse_json_value(artifact, "artifact")?;
    plan_matches_sync_artifact(&plan_payload, &artifact_payload, require_title_match)
        .map_err(PyValueError::new_err)
}

#[pyfunction(name = "plan_filesystem_normalize_markdown_artifact_path")]
fn plan_filesystem_normalize_markdown_artifact_path_py(path_value: &str) -> String {
    normalize_markdown_artifact_path(path_value)
}

#[pyfunction(name = "plan_filesystem_is_markdown_artifact_path")]
fn plan_filesystem_is_markdown_artifact_path_py(path_value: &str) -> bool {
    is_markdown_artifact_path(path_value)
}

#[pyfunction(name = "plan_filesystem_is_lineage_only_markdown_artifact_path")]
fn plan_filesystem_is_lineage_only_markdown_artifact_path_py(path_value: &str) -> bool {
    is_lineage_only_markdown_artifact_path(path_value)
}

#[pyfunction(name = "plan_filesystem_path_is_projected_out_for_workspace")]
#[pyo3(signature = (repo_root, rel_path, *, is_worktree=false))]
fn plan_filesystem_path_is_projected_out_for_workspace_py(
    repo_root: &str,
    rel_path: &str,
    is_worktree: bool,
) -> bool {
    path_is_projected_out_for_workspace(repo_root, rel_path, is_worktree)
}

#[pyfunction(name = "plan_filesystem_workspace_path_is_ignored")]
#[pyo3(signature = (repo_root, path_value, *, ignore_rules_text=None))]
fn plan_filesystem_workspace_path_is_ignored_py(
    repo_root: &str,
    path_value: &str,
    ignore_rules_text: Option<&str>,
) -> PyResult<bool> {
    workspace_path_is_ignored(repo_root, path_value, ignore_rules_text)
        .map_err(plan_filesystem_py_error)
}

#[pyfunction(name = "plan_filesystem_list_visible_workspace_paths")]
#[pyo3(signature = (repo_root, *, ignore_rules_text=None, runtime_root=None))]
fn plan_filesystem_list_visible_workspace_paths_py(
    py: Python<'_>,
    repo_root: &str,
    ignore_rules_text: Option<&str>,
    runtime_root: Option<&str>,
) -> PyResult<Py<PyList>> {
    render_json_list(
        py,
        list_visible_workspace_paths(repo_root, ignore_rules_text, runtime_root)
            .map_err(plan_filesystem_py_error)?
            .into_iter()
            .map(JsonValue::String)
            .collect(),
    )
}

#[pyfunction(name = "plan_filesystem_list_visible_markdown_artifact_paths")]
#[pyo3(signature = (repo_root, *, ignore_rules_text=None, runtime_root=None))]
fn plan_filesystem_list_visible_markdown_artifact_paths_py(
    py: Python<'_>,
    repo_root: &str,
    ignore_rules_text: Option<&str>,
    runtime_root: Option<&str>,
) -> PyResult<Py<PyList>> {
    render_json_list(
        py,
        list_visible_markdown_artifact_paths(repo_root, ignore_rules_text, runtime_root)
            .map_err(plan_filesystem_py_error)?
            .into_iter()
            .map(JsonValue::String)
            .collect(),
    )
}

#[pyfunction(name = "plan_filesystem_read_utf8_text_file")]
fn plan_filesystem_read_utf8_text_file_py(path_value: &str) -> PyResult<String> {
    read_utf8_text_file(path_value).map_err(plan_filesystem_py_error)
}

#[pyfunction(name = "plan_filesystem_read_json_file")]
fn plan_filesystem_read_json_file_py(py: Python<'_>, path_value: &str) -> PyResult<Py<PyAny>> {
    json_value_to_py(
        py,
        &read_json_file(path_value).map_err(plan_filesystem_py_error)?,
    )
}

#[pyfunction(name = "plan_filesystem_read_binary_file")]
fn plan_filesystem_read_binary_file_py<'py>(
    py: Python<'py>,
    path_value: &str,
) -> PyResult<Bound<'py, PyBytes>> {
    let payload = read_binary_file(path_value).map_err(plan_filesystem_py_error)?;
    Ok(PyBytes::new(py, &payload))
}

#[pyfunction(name = "plan_filesystem_zip_archive_has_member")]
fn plan_filesystem_zip_archive_has_member_py(path_value: &str, entry_name: &str) -> PyResult<bool> {
    zip_archive_has_member(path_value, entry_name).map_err(plan_filesystem_py_error)
}

#[pyfunction(name = "plan_filesystem_read_zip_archive_member")]
fn plan_filesystem_read_zip_archive_member_py<'py>(
    py: Python<'py>,
    path_value: &str,
    entry_name: &str,
) -> PyResult<Bound<'py, PyBytes>> {
    let payload =
        read_zip_archive_member(path_value, entry_name).map_err(plan_filesystem_py_error)?;
    Ok(PyBytes::new(py, &payload))
}

#[pyfunction(name = "plan_filesystem_resolve_repo_artifact_path")]
#[pyo3(signature = (repo_root, path_value, *, allow_missing=false))]
fn plan_filesystem_resolve_repo_artifact_path_py(
    py: Python<'_>,
    repo_root: &str,
    path_value: &str,
    allow_missing: bool,
) -> PyResult<Py<PyDict>> {
    render_json_dict(
        py,
        resolve_repo_artifact_path(repo_root, path_value, allow_missing)
            .map_err(plan_filesystem_py_error)?,
    )
}

#[pyfunction(name = "repo_state_read_repo_config_json")]
fn repo_state_read_repo_config_json_py(py: Python<'_>, path_value: &str) -> PyResult<Py<PyAny>> {
    render_json_value(
        py,
        rust_read_repo_config_json_file(path_value).map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "repo_state_write_repo_config_json")]
fn repo_state_write_repo_config_json_py(
    path_value: &str,
    payload: Bound<'_, PyAny>,
) -> PyResult<()> {
    let payload = parse_json_value(payload, "payload")?;
    rust_write_repo_config_json_file(path_value, &payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "repo_state_read_worktree_config_json")]
fn repo_state_read_worktree_config_json_py(
    py: Python<'_>,
    path_value: &str,
) -> PyResult<Py<PyAny>> {
    render_json_value(
        py,
        rust_read_worktree_config_json_file(path_value).map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "repo_state_write_worktree_config_json")]
fn repo_state_write_worktree_config_json_py(
    path_value: &str,
    payload: Bound<'_, PyAny>,
) -> PyResult<()> {
    let payload = parse_json_value(payload, "payload")?;
    rust_write_worktree_config_json_file(path_value, &payload).map_err(PyValueError::new_err)
}

#[pyfunction(name = "repo_state_read_worktree_metadata_json")]
fn repo_state_read_worktree_metadata_json_py(
    py: Python<'_>,
    path_value: &str,
) -> PyResult<Py<PyAny>> {
    render_json_value(
        py,
        rust_read_worktree_metadata_json_file(path_value).map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "repo_state_write_worktree_metadata_json")]
fn repo_state_write_worktree_metadata_json_py(
    path_value: &str,
    payload: Bound<'_, PyAny>,
) -> PyResult<()> {
    let payload = parse_json_value(payload, "payload")?;
    rust_write_worktree_metadata_json_file(path_value, &payload).map_err(PyValueError::new_err)
}


#[pyfunction(name = "plan_pack_substrate_build_pack_members")]
#[pyo3(signature = (blob_items, *, max_delta_chain_depth=plan_pack_substrate::DEFAULT_MAX_DELTA_CHAIN_DEPTH, initial_by_path=None))]
fn plan_pack_substrate_build_pack_members_py(
    py: Python<'_>,
    blob_items: Bound<'_, PyAny>,
    max_delta_chain_depth: usize,
    initial_by_path: Option<Bound<'_, PyAny>>,
) -> PyResult<Py<PyList>> {
    let blob_items_payload = parse_json_value(blob_items, "blob_items")?;
    let initial_payload = initial_by_path
        .map(|value| parse_json_value(value, "initial_by_path"))
        .transpose()?;
    match plan_pack_substrate::build_pack_members(
        &blob_items_payload,
        max_delta_chain_depth,
        initial_payload.as_ref(),
    )
    .map_err(PyValueError::new_err)?
    {
        JsonValue::Array(values) => render_json_list(py, values),
        _ => Err(PyRuntimeError::new_err(
            "Pack substrate build_pack_members must return a list payload.",
        )),
    }
}

#[pyfunction(name = "plan_pack_substrate_build_tree_pack_members")]
fn plan_pack_substrate_build_tree_pack_members_py(
    py: Python<'_>,
    tree_rows: Bound<'_, PyAny>,
    tree_entry_rows: Bound<'_, PyAny>,
) -> PyResult<Py<PyList>> {
    let tree_rows_payload = parse_json_value(tree_rows, "tree_rows")?;
    let tree_entry_rows_payload = parse_json_value(tree_entry_rows, "tree_entry_rows")?;
    match plan_pack_substrate::build_tree_pack_members(&tree_rows_payload, &tree_entry_rows_payload)
        .map_err(PyValueError::new_err)?
    {
        JsonValue::Array(values) => render_json_list(py, values),
        _ => Err(PyRuntimeError::new_err(
            "Pack substrate build_tree_pack_members must return a list payload.",
        )),
    }
}

#[pyfunction(name = "plan_pack_substrate_build_git_binary_delta")]
fn plan_pack_substrate_build_git_binary_delta_py(
    py: Python<'_>,
    base_data: Vec<u8>,
    target_data: Vec<u8>,
) -> PyResult<Py<PyBytes>> {
    Ok(PyBytes::new(
        py,
        &plan_pack_substrate::build_git_binary_delta(&base_data, &target_data),
    )
    .unbind())
}

#[pyfunction(name = "plan_pack_substrate_build_git_binary_delta_member")]
#[pyo3(signature = (entry_name, blob_id, base_blob_id, base_data, target_data, chain_depth))]
fn plan_pack_substrate_build_git_binary_delta_member_py(
    py: Python<'_>,
    entry_name: &str,
    blob_id: &str,
    base_blob_id: &str,
    base_data: Vec<u8>,
    target_data: Vec<u8>,
    chain_depth: usize,
) -> PyResult<Py<PyDict>> {
    render_json_dict(
        py,
        plan_pack_substrate::build_git_binary_delta_member(
            entry_name,
            blob_id,
            base_blob_id,
            &base_data,
            &target_data,
            chain_depth,
        ),
    )
}

#[pyfunction(name = "plan_pack_substrate_apply_git_binary_delta")]
fn plan_pack_substrate_apply_git_binary_delta_py(
    py: Python<'_>,
    base_data: Vec<u8>,
    delta_data: Vec<u8>,
) -> PyResult<Py<PyBytes>> {
    let output = plan_pack_substrate::apply_git_binary_delta(&base_data, &delta_data)
        .map_err(PyValueError::new_err)?;
    Ok(PyBytes::new(py, &output).unbind())
}

#[pyfunction(name = "plan_pack_substrate_apply_pack_delta")]
#[pyo3(signature = (base_data, delta_data, algorithm))]
fn plan_pack_substrate_apply_pack_delta_py(
    py: Python<'_>,
    base_data: Vec<u8>,
    delta_data: Vec<u8>,
    algorithm: &str,
) -> PyResult<Py<PyBytes>> {
    let output = plan_pack_substrate::apply_pack_delta(&base_data, &delta_data, algorithm)
        .map_err(PyValueError::new_err)?;
    Ok(PyBytes::new(py, &output).unbind())
}

#[pyfunction(name = "plan_pack_substrate_write_pack_archive")]
fn plan_pack_substrate_write_pack_archive_py(
    py: Python<'_>,
    pack_path: Bound<'_, PyAny>,
    pack_id: &str,
    created_at: &str,
    members: Bound<'_, PyAny>,
) -> PyResult<Py<PyDict>> {
    let pack_path = coerce_path_string(pack_path, "pack_path")?;
    let members_payload = parse_json_value(members, "members")?;
    render_json_dict(
        py,
        plan_pack_substrate::write_pack_archive(&pack_path, pack_id, created_at, &members_payload)
            .map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "plan_pack_substrate_read_pack_index")]
fn plan_pack_substrate_read_pack_index_py(
    py: Python<'_>,
    pack_path: Bound<'_, PyAny>,
) -> PyResult<Py<PyDict>> {
    let pack_path = coerce_path_string(pack_path, "pack_path")?;
    render_json_dict(
        py,
        plan_pack_substrate::read_pack_index(&pack_path).map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "plan_pack_substrate_pack_has_entry")]
fn plan_pack_substrate_pack_has_entry_py(
    pack_path: Bound<'_, PyAny>,
    entry_name: &str,
) -> PyResult<bool> {
    let pack_path = coerce_path_string(pack_path, "pack_path")?;
    Ok(plan_pack_substrate::pack_has_entry(&pack_path, entry_name))
}

#[pyfunction(name = "plan_pack_substrate_write_tree_pack_archive")]
fn plan_pack_substrate_write_tree_pack_archive_py(
    py: Python<'_>,
    pack_path: Bound<'_, PyAny>,
    pack_id: &str,
    created_at: &str,
    members: Bound<'_, PyAny>,
) -> PyResult<Py<PyDict>> {
    let pack_path = coerce_path_string(pack_path, "pack_path")?;
    let members_payload = parse_json_value(members, "members")?;
    render_json_dict(
        py,
        plan_pack_substrate::write_tree_pack_archive(
            &pack_path,
            pack_id,
            created_at,
            &members_payload,
        )
        .map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "plan_pack_substrate_read_tree_pack_index")]
fn plan_pack_substrate_read_tree_pack_index_py(
    py: Python<'_>,
    pack_path: Bound<'_, PyAny>,
) -> PyResult<Py<PyDict>> {
    let pack_path = coerce_path_string(pack_path, "pack_path")?;
    render_json_dict(
        py,
        plan_pack_substrate::read_tree_pack_index(&pack_path).map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "plan_pack_substrate_read_tree_pack_tree")]
fn plan_pack_substrate_read_tree_pack_tree_py(
    py: Python<'_>,
    pack_path: Bound<'_, PyAny>,
    tree_id: &str,
) -> PyResult<Py<PyList>> {
    let pack_path = coerce_path_string(pack_path, "pack_path")?;
    match plan_pack_substrate::read_tree_pack_tree(&pack_path, tree_id)
        .map_err(PyValueError::new_err)?
    {
        JsonValue::Array(values) => render_json_list(py, values),
        _ => Err(PyRuntimeError::new_err(
            "Pack substrate read_tree_pack_tree must return a list payload.",
        )),
    }
}

#[pyfunction(name = "plan_pack_substrate_summarize_pack_archives")]
fn plan_pack_substrate_summarize_pack_archives_py(
    py: Python<'_>,
    root: Bound<'_, PyAny>,
    pack_rows: Bound<'_, PyAny>,
) -> PyResult<Py<PyDict>> {
    let root = coerce_path_string(root, "root")?;
    let pack_rows_payload = parse_json_value(pack_rows, "pack_rows")?;
    render_json_dict(
        py,
        plan_pack_substrate::summarize_pack_archives(&root, &pack_rows_payload)
            .map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "plan_pack_substrate_summarize_tree_pack_archives")]
fn plan_pack_substrate_summarize_tree_pack_archives_py(
    py: Python<'_>,
    root: Bound<'_, PyAny>,
    pack_rows: Bound<'_, PyAny>,
) -> PyResult<Py<PyDict>> {
    let root = coerce_path_string(root, "root")?;
    let pack_rows_payload = parse_json_value(pack_rows, "pack_rows")?;
    render_json_dict(
        py,
        plan_pack_substrate::summarize_tree_pack_archives(&root, &pack_rows_payload)
            .map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "plan_pack_substrate_tree_pack_manifest_path")]
fn plan_pack_substrate_tree_pack_manifest_path_py(pack_path: &str, entry_name: &str) -> String {
    plan_pack_substrate::tree_pack_manifest_path(pack_path, entry_name)
}

#[pyfunction(name = "plan_pack_substrate_read_tree_pack_tree_by_ordinal")]
fn plan_pack_substrate_read_tree_pack_tree_by_ordinal_py(
    py: Python<'_>,
    pack_path: Bound<'_, PyAny>,
    entry_ordinal: usize,
) -> PyResult<Py<PyDict>> {
    let pack_path = coerce_path_string(pack_path, "pack_path")?;
    render_json_dict(
        py,
        plan_pack_substrate::read_tree_pack_tree_by_ordinal(&pack_path, entry_ordinal)
            .map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "plan_pack_substrate_build_storage_validation_summary")]
#[pyo3(signature = (*, packed_blob_count, packed_full_blob_count, packed_delta_blob_count, pack_count, pack_index_error_count, tree_pack_index_error_count=0, storage_savings_ratio, unreferenced_blob_count=0, unreferenced_tree_count=0, signals_summary=None))]
#[expect(
    clippy::too_many_arguments,
    reason = "Python parameters are a compatibility contract"
)]
fn plan_pack_substrate_build_storage_validation_summary_py(
    py: Python<'_>,
    packed_blob_count: usize,
    packed_full_blob_count: usize,
    packed_delta_blob_count: usize,
    pack_count: usize,
    pack_index_error_count: usize,
    tree_pack_index_error_count: usize,
    storage_savings_ratio: f64,
    unreferenced_blob_count: usize,
    unreferenced_tree_count: usize,
    signals_summary: Option<Bound<'_, PyAny>>,
) -> PyResult<Py<PyDict>> {
    let signals_payload = signals_summary
        .map(|value| parse_json_value(value, "signals_summary"))
        .transpose()?;
    render_json_dict(
        py,
        plan_pack_substrate::build_storage_validation_summary(
            packed_blob_count,
            packed_full_blob_count,
            packed_delta_blob_count,
            pack_count,
            pack_index_error_count,
            tree_pack_index_error_count,
            storage_savings_ratio,
            unreferenced_blob_count,
            unreferenced_tree_count,
            signals_payload.as_ref(),
        ),
    )
}

#[pyfunction(name = "plan_pack_substrate_build_tree_records")]
fn plan_pack_substrate_build_tree_records_py(
    py: Python<'_>,
    file_entries: Bound<'_, PyAny>,
) -> PyResult<Py<PyDict>> {
    let payload = parse_json_value(file_entries, "file_entries")?;
    render_json_dict(py, build_tree_records_payload(&payload)?)
}

#[pyfunction(name = "plan_pack_substrate_build_snapshot_id")]
#[pyo3(signature = (*, repo_name, line_name, parent_snapshot_id=None, message=None, root_tree_id, snapshot_kind="line"))]
fn plan_pack_substrate_build_snapshot_id_py(
    py: Python<'_>,
    repo_name: &str,
    line_name: &str,
    parent_snapshot_id: Option<&str>,
    message: Option<&str>,
    root_tree_id: &str,
    snapshot_kind: &str,
) -> PyResult<Py<PyDict>> {
    render_json_dict(
        py,
        build_snapshot_id_payload(
            repo_name,
            line_name,
            parent_snapshot_id,
            message,
            root_tree_id,
            snapshot_kind,
        ),
    )
}

#[derive(Clone, Debug)]
enum StorageTreeNode {
    Tree {
        children: BTreeMap<String, StorageTreeNode>,
    },
    Blob {
        blob_id: String,
        size_bytes: Option<i64>,
        mode: String,
    },
}

#[derive(Clone, Debug)]
struct StorageTreeEntryRow {
    tree_id: String,
    entry_name: String,
    entry_type: String,
    target_id: String,
    size_bytes: Option<i64>,
    mode: String,
}

fn build_tree_records_payload(file_entries: &JsonValue) -> PyResult<JsonValue> {
    let rows = file_entries
        .as_array()
        .ok_or_else(|| PyValueError::new_err("file_entries must be a list."))?;
    let mut sorted_rows = rows.iter().collect::<Vec<_>>();
    sorted_rows.sort_by_key(|row| {
        row.as_object()
            .and_then(|obj| optional_json_text_field(obj, "path"))
            .unwrap_or_default()
    });
    let mut root: BTreeMap<String, StorageTreeNode> = BTreeMap::new();
    for row in sorted_rows {
        let obj = row
            .as_object()
            .ok_or_else(|| PyValueError::new_err("file entry must be an object."))?;
        let path = required_json_text_field(obj, "path")?;
        let parts = path
            .trim_matches('/')
            .split('/')
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        if parts.is_empty() {
            continue;
        }
        let mut cursor = &mut root;
        for part in &parts[..parts.len() - 1] {
            cursor = match cursor
                .entry(part.clone())
                .or_insert_with(|| StorageTreeNode::Tree {
                    children: BTreeMap::new(),
                }) {
                StorageTreeNode::Tree { children } => children,
                StorageTreeNode::Blob { .. } => {
                    return Err(PyValueError::new_err(format!(
                        "Path collision while building tree metadata at {path:?}"
                    )));
                }
            };
        }
        cursor.insert(
            parts.last().unwrap().clone(),
            StorageTreeNode::Blob {
                blob_id: required_json_text_field(obj, "blob_id")?,
                size_bytes: optional_json_i64_field(obj, "size_bytes")?,
                mode: required_json_text_field(obj, "mode")?,
            },
        );
    }

    let mut tree_rows = BTreeMap::<String, JsonValue>::new();
    let mut tree_entry_rows = BTreeMap::<(String, String), StorageTreeEntryRow>::new();
    let root_tree_id = materialize_storage_tree(&root, &mut tree_rows, &mut tree_entry_rows)?;
    Ok(json!({
        "root_tree_id": root_tree_id,
        "tree_rows": tree_rows.into_values().collect::<Vec<_>>(),
        "tree_entry_rows": tree_entry_rows.into_values().map(|row| json!({
            "tree_id": row.tree_id,
            "entry_name": row.entry_name,
            "entry_type": row.entry_type,
            "target_id": row.target_id,
            "size_bytes": row.size_bytes,
            "mode": row.mode,
        })).collect::<Vec<_>>(),
    }))
}

fn materialize_storage_tree(
    children: &BTreeMap<String, StorageTreeNode>,
    tree_rows: &mut BTreeMap<String, JsonValue>,
    tree_entry_rows: &mut BTreeMap<(String, String), StorageTreeEntryRow>,
) -> PyResult<String> {
    let mut serialized_entries = Vec::<JsonValue>::new();
    let mut pending_rows = Vec::<StorageTreeEntryRow>::new();
    for (name, node) in children {
        match node {
            StorageTreeNode::Tree { children } => {
                let child_tree_id = materialize_storage_tree(children, tree_rows, tree_entry_rows)?;
                serialized_entries.push(json!({
                    "name": name,
                    "type": "tree",
                    "target_id": child_tree_id,
                }));
                pending_rows.push(StorageTreeEntryRow {
                    tree_id: String::new(),
                    entry_name: name.clone(),
                    entry_type: "tree".to_string(),
                    target_id: child_tree_id,
                    size_bytes: None,
                    mode: "tree".to_string(),
                });
            }
            StorageTreeNode::Blob {
                blob_id,
                size_bytes,
                mode,
            } => {
                serialized_entries.push(json!({
                    "name": name,
                    "type": "blob",
                    "target_id": blob_id,
                    "size_bytes": size_bytes,
                    "mode": mode,
                }));
                pending_rows.push(StorageTreeEntryRow {
                    tree_id: String::new(),
                    entry_name: name.clone(),
                    entry_type: "blob".to_string(),
                    target_id: blob_id.clone(),
                    size_bytes: *size_bytes,
                    mode: mode.clone(),
                });
            }
        }
    }
    let digest_payload = JsonValue::Array(serialized_entries.clone());
    let digest_text = encode_json_value_compact(&digest_payload).map_err(PyValueError::new_err)?;
    let digest = sha256_hex_py(digest_text.as_bytes());
    let tree_id = format!("TRE-{}", digest[..20].to_ascii_uppercase());
    let row_tree_id = tree_id.clone();
    tree_rows.entry(tree_id.clone()).or_insert_with(|| {
        json!({
            "tree_id": row_tree_id,
            "entry_count": serialized_entries.len(),
        })
    });
    for mut row in pending_rows {
        row.tree_id = tree_id.clone();
        tree_entry_rows.insert((row.tree_id.clone(), row.entry_name.clone()), row);
    }
    Ok(tree_id)
}

fn build_snapshot_id_payload(
    repo_name: &str,
    line_name: &str,
    parent_snapshot_id: Option<&str>,
    message: Option<&str>,
    root_tree_id: &str,
    snapshot_kind: &str,
) -> JsonValue {
    let payload = json!({
        "repo_name": repo_name,
        "line_name": line_name,
        "parent_snapshot_id": parent_snapshot_id,
        "message": message,
        "root_tree_id": root_tree_id,
        "snapshot_kind": snapshot_kind,
    });
    let revision_text = encode_json_value_compact_or_default(&payload);
    let revision_hash = sha256_hex_py(revision_text.as_bytes());
    json!({
        "snapshot_id": format!("SNP-{}", revision_hash[..12].to_ascii_uppercase()),
        "revision_hash": revision_hash,
    })
}

fn required_json_text_field(obj: &Map<String, JsonValue>, field: &str) -> PyResult<String> {
    optional_json_text_field(obj, field)
        .ok_or_else(|| PyValueError::new_err(format!("{field} must be a non-empty string.")))
}

fn optional_json_text_field(obj: &Map<String, JsonValue>, field: &str) -> Option<String> {
    match obj.get(field) {
        None | Some(JsonValue::Null) => None,
        Some(JsonValue::String(value)) => {
            let text = value.trim();
            (!text.is_empty()).then(|| text.to_string())
        }
        Some(value) => Some(value.to_string()),
    }
}

fn optional_json_i64_field(obj: &Map<String, JsonValue>, field: &str) -> PyResult<Option<i64>> {
    match obj.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Number(value)) => value
            .as_i64()
            .ok_or_else(|| PyValueError::new_err(format!("{field} must fit in i64.")))
            .map(Some),
        Some(value) => value
            .to_string()
            .parse::<i64>()
            .map(Some)
            .map_err(|_| PyValueError::new_err(format!("{field} must be an integer."))),
    }
}

fn sha256_hex_py(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

fn coerce_path_string(value: Bound<'_, PyAny>, field_name: &str) -> PyResult<String> {
    if let Ok(text) = value.extract::<String>() {
        if !text.trim().is_empty() {
            return Ok(text);
        }
    }
    let rendered = value
        .str()
        .map_err(|_| PyValueError::new_err(format!("{field_name} must be path-like.")))?
        .extract::<String>()?;
    if rendered.trim().is_empty() {
        Err(PyValueError::new_err(format!(
            "{field_name} must be a non-empty path-like value."
        )))
    } else {
        Ok(rendered)
    }
}

fn register_plan_filesystem_storage(_py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(artifact_blob_id_py, module)?)?;
    module.add_function(wrap_pyfunction!(artifact_candidates_open_py, module)?)?;
    module.add_function(wrap_pyfunction!(plan_artifact_identity_py, module)?)?;
    module.add_function(wrap_pyfunction!(plan_artifact_identity_label_py, module)?)?;
    module.add_function(wrap_pyfunction!(index_plans_by_artifact_path_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        index_plans_by_artifact_identity_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        open_generic_plans_matching_blob_id_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(open_plans_matching_selector_py, module)?)?;
    module.add_function(wrap_pyfunction!(local_plan_fully_published_py, module)?)?;
    module.add_function(wrap_pyfunction!(plan_heads_equivalent_py, module)?)?;
    module.add_function(wrap_pyfunction!(plan_matches_sync_artifact_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        plan_filesystem_normalize_markdown_artifact_path_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_filesystem_is_markdown_artifact_path_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_filesystem_is_lineage_only_markdown_artifact_path_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_filesystem_path_is_projected_out_for_workspace_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_filesystem_workspace_path_is_ignored_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_filesystem_list_visible_workspace_paths_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_filesystem_list_visible_markdown_artifact_paths_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_filesystem_read_utf8_text_file_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(plan_filesystem_read_json_file_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        plan_filesystem_read_binary_file_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_filesystem_zip_archive_has_member_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_filesystem_read_zip_archive_member_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_filesystem_resolve_repo_artifact_path_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        repo_state_read_repo_config_json_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        repo_state_write_repo_config_json_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        repo_state_read_worktree_config_json_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        repo_state_write_worktree_config_json_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        repo_state_read_worktree_metadata_json_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        repo_state_write_worktree_metadata_json_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_pack_substrate_build_pack_members_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_pack_substrate_build_tree_pack_members_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_pack_substrate_build_git_binary_delta_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_pack_substrate_build_git_binary_delta_member_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_pack_substrate_apply_git_binary_delta_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_pack_substrate_apply_pack_delta_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_pack_substrate_write_pack_archive_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_pack_substrate_read_pack_index_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_pack_substrate_pack_has_entry_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_pack_substrate_write_tree_pack_archive_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_pack_substrate_read_tree_pack_index_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_pack_substrate_read_tree_pack_tree_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_pack_substrate_summarize_pack_archives_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_pack_substrate_summarize_tree_pack_archives_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_pack_substrate_tree_pack_manifest_path_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_pack_substrate_read_tree_pack_tree_by_ordinal_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_pack_substrate_build_storage_validation_summary_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_pack_substrate_build_tree_records_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        plan_pack_substrate_build_snapshot_id_py,
        module
    )?)?;
    Ok(())
}
