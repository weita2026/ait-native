fn discover_task_workflow_repo(repo_root: &str) -> PyResult<TaskWorkflowRepoRuntime> {
    TaskWorkflowRepoRuntime::discover_from_path(Path::new(repo_root)).map_err(PyValueError::new_err)
}

#[pyclass(unsendable, name = "WorkspaceCommandLock")]
struct WorkspaceCommandLockPy {
    lock: Option<RustWorkspaceCommandLock>,
    metadata: JsonValue,
}

#[pymethods]
impl WorkspaceCommandLockPy {
    #[new]
    fn new(py: Python<'_>, repo_root: &str, command_name: &str) -> PyResult<Self> {
        let repo_root = repo_root.to_string();
        let command_name = command_name.to_string();
        let (lock, metadata) = py
            .detach(move || {
                let repo = TaskWorkflowRepoRuntime::discover_from_path(Path::new(&repo_root))?;
                let lock = RustWorkspaceCommandLock::acquire(&repo, &command_name)?;
                let metadata = lock.metadata().clone();
                Ok::<_, String>((lock, metadata))
            })
            .map_err(PyValueError::new_err)?;
        Ok(Self {
            lock: Some(lock),
            metadata,
        })
    }

    fn metadata(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        render_json_dict(py, self.metadata.clone())
    }

    fn close(&mut self) {
        self.lock.take();
    }

    fn release(&mut self) {
        self.close();
    }

    fn is_closed(&self) -> bool {
        self.lock.is_none()
    }
}

#[pyfunction(name = "workspace_command_lock_path")]
fn workspace_command_lock_path_py(repo_root: &str) -> PyResult<String> {
    let repo = discover_task_workflow_repo(repo_root)?;
    Ok(rust_workspace_command_lock_path(&repo)
        .to_string_lossy()
        .to_string())
}

fn render_task_workflow_primitive_result(
    py: Python<'_>,
    result: Result<JsonValue, String>,
) -> PyResult<Py<PyDict>> {
    render_json_dict(py, result.map_err(PyValueError::new_err)?)
}

#[pyfunction(name = "task_workflow_snapshot_create")]
#[pyo3(signature = (repo_root, message=None))]
fn task_workflow_snapshot_create_py(
    py: Python<'_>,
    repo_root: &str,
    message: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| task_workflow_primitives::snapshot_create(&repo, message));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_snapshot_create_explicit")]
#[pyo3(signature = (repo_root, repo_name, line_name, message=None, parent_snapshot_id=None, update_line_ref=true, touch_line=true, record_workflow_metadata=true))]
#[expect(
    clippy::too_many_arguments,
    reason = "Python parameters are a compatibility contract"
)]
fn task_workflow_snapshot_create_explicit_py(
    py: Python<'_>,
    repo_root: &str,
    repo_name: &str,
    line_name: &str,
    message: Option<&str>,
    parent_snapshot_id: Option<&str>,
    update_line_ref: bool,
    touch_line: bool,
    record_workflow_metadata: bool,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::snapshot_create_explicit(
            &repo,
            repo_name,
            line_name,
            message,
            parent_snapshot_id,
            update_line_ref,
            touch_line,
            record_workflow_metadata,
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_snapshot_list")]
fn task_workflow_snapshot_list_py(py: Python<'_>, repo_root: &str) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| task_workflow_primitives::snapshot_list(&repo));
    render_json_value(py, payload.map_err(PyValueError::new_err)?)
}

#[pyfunction(name = "task_workflow_snapshot_show")]
fn task_workflow_snapshot_show_py(
    py: Python<'_>,
    repo_root: &str,
    snapshot_id: &str,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| task_workflow_primitives::snapshot_show(&repo, snapshot_id));
    render_json_value(py, payload.map_err(PyValueError::new_err)?)
}

#[pyfunction(name = "task_workflow_snapshot_chain")]
fn task_workflow_snapshot_chain_py(
    py: Python<'_>,
    repo_root: &str,
    snapshot_id: &str,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| task_workflow_primitives::snapshot_chain(&repo, snapshot_id));
    render_json_value(py, payload.map_err(PyValueError::new_err)?)
}

#[pyfunction(name = "task_workflow_snapshot_exists")]
fn task_workflow_snapshot_exists_py(repo_root: &str, snapshot_id: &str) -> PyResult<bool> {
    let repo = discover_task_workflow_repo(repo_root)?;
    task_workflow_primitives::snapshot_exists(&repo, snapshot_id).map_err(PyValueError::new_err)
}

#[pyfunction(name = "task_workflow_blob_read_bytes")]
fn task_workflow_blob_read_bytes_py(
    py: Python<'_>,
    repo_root: &str,
    blob_id: &str,
) -> PyResult<Py<PyBytes>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| task_workflow_primitives::blob_read_bytes(&repo, blob_id));
    Ok(PyBytes::new(py, &payload.map_err(PyValueError::new_err)?).unbind())
}

#[pyfunction(name = "task_workflow_blob_ensure_bytes")]
#[pyo3(signature = (repo_root, data, *, path_hint=None))]
fn task_workflow_blob_ensure_bytes_py(
    py: Python<'_>,
    repo_root: &str,
    data: Vec<u8>,
    path_hint: Option<&str>,
) -> PyResult<String> {
    let repo = discover_task_workflow_repo(repo_root)?;
    py.detach(|| task_workflow_primitives::blob_ensure_bytes(&repo, &data, path_hint))
        .map_err(PyValueError::new_err)
}

#[pyfunction(name = "task_workflow_workspace_delta")]
#[pyo3(signature = (repo_root, *, snapshot_id=None))]
fn task_workflow_workspace_delta_py(
    py: Python<'_>,
    repo_root: &str,
    snapshot_id: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload =
        py.detach(|| task_workflow_primitives::workspace_delta(&repo, snapshot_id));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_workspace_restore")]
#[pyo3(signature = (repo_root, *, target_snapshot_id=None, baseline_snapshot_id=None, force=false, dry_run=false))]
fn task_workflow_workspace_restore_py(
    py: Python<'_>,
    repo_root: &str,
    target_snapshot_id: Option<&str>,
    baseline_snapshot_id: Option<&str>,
    force: bool,
    dry_run: bool,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::workspace_restore(
            &repo,
            target_snapshot_id,
            baseline_snapshot_id,
            force,
            dry_run,
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_workspace_restore_paths")]
#[pyo3(signature = (repo_root, paths, *, target_snapshot_id=None, baseline_snapshot_id=None, force=false, dry_run=false))]
fn task_workflow_workspace_restore_paths_py(
    py: Python<'_>,
    repo_root: &str,
    paths: Vec<String>,
    target_snapshot_id: Option<&str>,
    baseline_snapshot_id: Option<&str>,
    force: bool,
    dry_run: bool,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::workspace_restore_paths(
            &repo,
            target_snapshot_id,
            &paths,
            baseline_snapshot_id,
            force,
            dry_run,
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_snapshot_diff")]
#[pyo3(signature = (repo_root, old_snapshot_id, new_snapshot_id, *, include_text=false, max_bytes=DEFAULT_SNAPSHOT_DIFF_MAX_BYTES))]
fn task_workflow_snapshot_diff_py(
    py: Python<'_>,
    repo_root: &str,
    old_snapshot_id: &str,
    new_snapshot_id: &str,
    include_text: bool,
    max_bytes: usize,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::snapshot_diff(
            &repo,
            old_snapshot_id,
            new_snapshot_id,
            include_text,
            max_bytes,
        )
    });
    render_json_value(py, payload.map_err(PyValueError::new_err)?)
}

#[pyfunction(name = "task_workflow_snapshot_revert")]
#[pyo3(signature = (repo_root, snapshot_id, *, force=false, dry_run=false))]
fn task_workflow_snapshot_revert_py(
    py: Python<'_>,
    repo_root: &str,
    snapshot_id: &str,
    force: bool,
    dry_run: bool,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::snapshot_revert(&repo, snapshot_id, force, dry_run)
    });
    render_json_value(py, payload.map_err(PyValueError::new_err)?)
}

#[pyfunction(name = "task_workflow_snapshot_replay")]
#[pyo3(signature = (repo_root, snapshot_id, *, onto, force=false, dry_run=false))]
fn task_workflow_snapshot_replay_py(
    py: Python<'_>,
    repo_root: &str,
    snapshot_id: &str,
    onto: &str,
    force: bool,
    dry_run: bool,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::snapshot_replay(&repo, snapshot_id, onto, force, dry_run)
    });
    render_json_value(py, payload.map_err(PyValueError::new_err)?)
}

#[pyfunction(name = "task_workflow_change_list")]
#[pyo3(signature = (repo_root, *, local=false, remote_name=None))]
fn task_workflow_change_list_py(
    py: Python<'_>,
    repo_root: &str,
    local: bool,
    remote_name: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload =
        py.detach(|| task_workflow_primitives::change_list(&repo, local, remote_name));
    render_json_value(py, payload.map_err(PyValueError::new_err)?)
}

#[pyfunction(name = "task_workflow_change_show")]
#[pyo3(signature = (repo_root, change_id, *, local=false, remote_name=None, repo_name=None))]
fn task_workflow_change_show_py(
    py: Python<'_>,
    repo_root: &str,
    change_id: &str,
    local: bool,
    remote_name: Option<&str>,
    repo_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::change_show(&repo, change_id, local, remote_name, repo_name)
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_change_revert")]
#[pyo3(signature = (repo_root, change_id, *, force=false, dry_run=false, local=false, remote_name=None, repo_name=None))]
#[expect(
    clippy::too_many_arguments,
    reason = "Python parameters are a compatibility contract"
)]
fn task_workflow_change_revert_py(
    py: Python<'_>,
    repo_root: &str,
    change_id: &str,
    force: bool,
    dry_run: bool,
    local: bool,
    remote_name: Option<&str>,
    repo_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::change_revert(
            &repo,
            change_id,
            force,
            dry_run,
            local,
            remote_name,
            repo_name,
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_change_replay")]
#[pyo3(signature = (repo_root, change_id, *, onto, force=false, dry_run=false, local=false, remote_name=None, repo_name=None))]
#[expect(
    clippy::too_many_arguments,
    reason = "Python parameters are a compatibility contract"
)]
fn task_workflow_change_replay_py(
    py: Python<'_>,
    repo_root: &str,
    change_id: &str,
    onto: &str,
    force: bool,
    dry_run: bool,
    local: bool,
    remote_name: Option<&str>,
    repo_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::change_replay(
            &repo,
            change_id,
            onto,
            force,
            dry_run,
            local,
            remote_name,
            repo_name,
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_change_close")]
#[pyo3(signature = (repo_root, change_id, *, local=false, remote_name=None))]
fn task_workflow_change_close_py(
    py: Python<'_>,
    repo_root: &str,
    change_id: &str,
    local: bool,
    remote_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::change_close(&repo, change_id, local, remote_name)
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_change_publish")]
#[pyo3(signature = (repo_root, change_id, *, remote_name=None))]
fn task_workflow_change_publish_py(
    py: Python<'_>,
    repo_root: &str,
    change_id: &str,
    remote_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| task_workflow_primitives::change_publish(&repo, change_id, remote_name));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_patchset_publish")]
#[pyo3(signature = (repo_root, change_id, summary, author_mode=None, remote_name=None))]
fn task_workflow_patchset_publish_py(
    py: Python<'_>,
    repo_root: &str,
    change_id: &str,
    summary: &str,
    author_mode: Option<&str>,
    remote_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::patchset_publish(
            &repo,
            change_id,
            summary,
            author_mode,
            remote_name,
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_patchset_list")]
#[pyo3(signature = (repo_root, change_id, *, remote_name=None))]
fn task_workflow_patchset_list_py(
    py: Python<'_>,
    repo_root: &str,
    change_id: &str,
    remote_name: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload =
        py.detach(|| task_workflow_primitives::patchset_list(&repo, change_id, remote_name));
    render_json_value(py, payload.map_err(PyValueError::new_err)?)
}

#[pyfunction(name = "task_workflow_patchset_show")]
#[pyo3(signature = (repo_root, patchset_id, *, remote_name=None))]
fn task_workflow_patchset_show_py(
    py: Python<'_>,
    repo_root: &str,
    patchset_id: &str,
    remote_name: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload =
        py.detach(|| task_workflow_primitives::patchset_show(&repo, patchset_id, remote_name));
    render_json_value(py, payload.map_err(PyValueError::new_err)?)
}

#[pyfunction(name = "task_workflow_patchset_select")]
#[pyo3(signature = (repo_root, patchset_id, *, remote_name=None))]
fn task_workflow_patchset_select_py(
    py: Python<'_>,
    repo_root: &str,
    patchset_id: &str,
    remote_name: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload =
        py.detach(|| task_workflow_primitives::patchset_select(&repo, patchset_id, remote_name));
    render_json_value(py, payload.map_err(PyValueError::new_err)?)
}

#[pyfunction(name = "task_workflow_patchset_publish_explicit")]
#[pyo3(signature = (repo_root, change_id, base_snapshot_id, revision_snapshot_id, summary, *, author_mode=None, remote_name=None, repo_name=None))]
#[expect(
    clippy::too_many_arguments,
    reason = "Python parameters are a compatibility contract"
)]
fn task_workflow_patchset_publish_explicit_py(
    py: Python<'_>,
    repo_root: &str,
    change_id: &str,
    base_snapshot_id: &str,
    revision_snapshot_id: &str,
    summary: &str,
    author_mode: Option<&str>,
    remote_name: Option<&str>,
    repo_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::patchset_publish_explicit(
            &repo,
            change_id,
            base_snapshot_id,
            revision_snapshot_id,
            summary,
            author_mode,
            remote_name,
            repo_name,
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_patchset_ci_status")]
#[pyo3(signature = (repo_root, patchset_id, *, remote_name=None))]
fn task_workflow_patchset_ci_status_py(
    py: Python<'_>,
    repo_root: &str,
    patchset_id: &str,
    remote_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload =
        py.detach(|| task_workflow_primitives::patchset_ci_status(&repo, patchset_id, remote_name));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_patchset_rerun_ci")]
#[pyo3(signature = (repo_root, patchset_id, *, remote_name=None))]
fn task_workflow_patchset_rerun_ci_py(
    py: Python<'_>,
    repo_root: &str,
    patchset_id: &str,
    remote_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload =
        py.detach(|| task_workflow_primitives::patchset_rerun_ci(&repo, patchset_id, remote_name));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_review_team_approve")]
#[pyo3(signature = (repo_root, change_id, *, patchset_id=None, reviewer=None, message=None, remote_name=None))]
fn task_workflow_review_team_approve_py(
    py: Python<'_>,
    repo_root: &str,
    change_id: &str,
    patchset_id: Option<&str>,
    reviewer: Option<&str>,
    message: Option<&str>,
    remote_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::review_team_approve(
            &repo,
            change_id,
            patchset_id,
            reviewer,
            message,
            remote_name,
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_review_show")]
#[pyo3(signature = (repo_root, change_id, *, remote_name=None))]
fn task_workflow_review_show_py(
    py: Python<'_>,
    repo_root: &str,
    change_id: &str,
    remote_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::review_show(&repo, change_id, remote_name, None)
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_review_request")]
#[pyo3(signature = (repo_root, change_id, *, reviewer_groups, patchset_id=None, note=None, remote_name=None))]
fn task_workflow_review_request_py(
    py: Python<'_>,
    repo_root: &str,
    change_id: &str,
    reviewer_groups: Vec<String>,
    patchset_id: Option<&str>,
    note: Option<&str>,
    remote_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::review_request(
            &repo,
            change_id,
            patchset_id,
            &reviewer_groups,
            note,
            remote_name,
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_review_task_approve")]
#[pyo3(signature = (repo_root, change_id, patchset_id, *, message, remote_name=None))]
fn task_workflow_review_task_approve_py(
    py: Python<'_>,
    repo_root: &str,
    change_id: &str,
    patchset_id: &str,
    message: &str,
    remote_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::review_task_approve(
            &repo,
            change_id,
            patchset_id,
            message,
            remote_name,
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_review_record")]
#[pyo3(signature = (repo_root, change_id, action, *, blocking=false, patchset_id=None, reviewer=None, message=None, remote_name=None))]
#[expect(
    clippy::too_many_arguments,
    reason = "Python parameters are a compatibility contract"
)]
fn task_workflow_review_record_py(
    py: Python<'_>,
    repo_root: &str,
    change_id: &str,
    action: &str,
    blocking: bool,
    patchset_id: Option<&str>,
    reviewer: Option<&str>,
    message: Option<&str>,
    remote_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::review_record(
            &repo,
            change_id,
            action,
            blocking,
            patchset_id,
            reviewer,
            message,
            remote_name,
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_review_code_submit")]
#[pyo3(signature = (repo_root, change_id, patchset_id, *, message, remote_name=None))]
fn task_workflow_review_code_submit_py(
    py: Python<'_>,
    repo_root: &str,
    change_id: &str,
    patchset_id: &str,
    message: &str,
    remote_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::review_code_submit(
            &repo,
            change_id,
            patchset_id,
            message,
            remote_name,
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_review_code_template")]
#[pyo3(signature = (*, style=None))]
fn task_workflow_review_code_template_py(
    py: Python<'_>,
    style: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let payload = py.detach(|| task_workflow_primitives::review_code_template(style));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_attest_put")]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (
    repo_root,
    *,
    patchset_id=None,
    change_id=None,
    tests=None,
    lint=None,
    security=None,
    license=None,
    author_mode=None,
    model=None,
    remote_name=None,
    repo_name=None
))]
fn task_workflow_attest_put_py(
    py: Python<'_>,
    repo_root: &str,
    patchset_id: Option<&str>,
    change_id: Option<&str>,
    tests: Option<&str>,
    lint: Option<&str>,
    security: Option<&str>,
    license: Option<&str>,
    author_mode: Option<&str>,
    model: Option<&str>,
    remote_name: Option<&str>,
    repo_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::attest_put(
            &repo,
            patchset_id,
            change_id,
            tests,
            lint,
            security,
            license,
            author_mode,
            model,
            remote_name,
            repo_name,
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_attest_show")]
#[pyo3(signature = (repo_root, patchset_id, *, remote_name=None))]
fn task_workflow_attest_show_py(
    py: Python<'_>,
    repo_root: &str,
    patchset_id: &str,
    remote_name: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::attest_show(&repo, patchset_id, remote_name, None)
    });
    render_json_value(py, payload.map_err(PyValueError::new_err)?)
}

#[pyfunction(name = "task_workflow_policy_eval")]
#[pyo3(signature = (repo_root, patchset_id, *, remote_name=None, repo_name=None))]
fn task_workflow_policy_eval_py(
    py: Python<'_>,
    repo_root: &str,
    patchset_id: &str,
    remote_name: Option<&str>,
    repo_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::policy_eval(&repo, patchset_id, remote_name, repo_name)
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_policy_show")]
#[pyo3(signature = (repo_root, patchset_id, *, remote_name=None, repo_name=None))]
fn task_workflow_policy_show_py(
    py: Python<'_>,
    repo_root: &str,
    patchset_id: &str,
    remote_name: Option<&str>,
    repo_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::policy_show(&repo, patchset_id, remote_name, repo_name)
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_policy_waive")]
#[pyo3(signature = (repo_root, patchset_id, *, rule_name, reason, expires_at=None, remote_name=None, repo_name=None))]
#[expect(
    clippy::too_many_arguments,
    reason = "Python parameters are a compatibility contract"
)]
fn task_workflow_policy_waive_py(
    py: Python<'_>,
    repo_root: &str,
    patchset_id: &str,
    rule_name: &str,
    reason: &str,
    expires_at: Option<&str>,
    remote_name: Option<&str>,
    repo_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::policy_waive(
            &repo,
            patchset_id,
            rule_name,
            reason,
            expires_at,
            remote_name,
            repo_name,
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_land_submit")]
#[pyo3(signature = (repo_root, change_id, *, patchset_id=None, target_line, mode, remote_name=None))]
fn task_workflow_land_submit_py(
    py: Python<'_>,
    repo_root: &str,
    change_id: &str,
    patchset_id: Option<&str>,
    target_line: &str,
    mode: &str,
    remote_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::land_submit(
            &repo,
            change_id,
            patchset_id,
            target_line,
            mode,
            remote_name,
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_land_show")]
#[pyo3(signature = (repo_root, submission_id, *, remote_name=None, repo_name=None))]
fn task_workflow_land_show_py(
    py: Python<'_>,
    repo_root: &str,
    submission_id: &str,
    remote_name: Option<&str>,
    repo_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::land_show(&repo, submission_id, remote_name, repo_name)
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_land_retry")]
#[pyo3(signature = (repo_root, submission_id, *, reason=None, remote_name=None, repo_name=None))]
fn task_workflow_land_retry_py(
    py: Python<'_>,
    repo_root: &str,
    submission_id: &str,
    reason: Option<&str>,
    remote_name: Option<&str>,
    repo_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::land_retry(&repo, submission_id, reason, remote_name, repo_name)
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "current_core_source_fingerprint")]
fn current_core_source_fingerprint_py(core_repo_root: &str) -> PyResult<String> {
    rust_current_core_source_fingerprint(Path::new(core_repo_root)).map_err(PyValueError::new_err)
}

#[pyfunction(name = "current_server_source_fingerprint")]
fn current_server_source_fingerprint_py(server_core_repo_root: &str) -> PyResult<String> {
    rust_current_server_source_fingerprint(Path::new(server_core_repo_root))
        .map_err(PyValueError::new_err)
}

#[pyfunction(name = "current_source_native_cache_contract")]
#[pyo3(signature = (
    namespace_root,
    core_repo_root,
    *,
    ext_suffix,
    rustflags="",
    worker_id="shared",
    core_source_fingerprint=None,
    server_source_fingerprint=None
))]
#[expect(
    clippy::too_many_arguments,
    reason = "Python parameters are a compatibility contract"
)]
fn current_source_native_cache_contract_py(
    py: Python<'_>,
    namespace_root: &str,
    core_repo_root: &str,
    ext_suffix: &str,
    rustflags: &str,
    worker_id: &str,
    core_source_fingerprint: Option<&str>,
    server_source_fingerprint: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let request = CurrentSourceNativeCacheRequest {
        namespace_root: PathBuf::from(namespace_root),
        core_repo_root: PathBuf::from(core_repo_root),
        core_source_fingerprint: core_source_fingerprint.map(ToString::to_string),
        server_source_fingerprint: server_source_fingerprint.map(ToString::to_string),
        ext_suffix: ext_suffix.to_string(),
        rustflags: rustflags.to_string(),
        worker_id: worker_id.to_string(),
    };
    let payload = py
        .detach(|| current_source_native_cache_contract_json(&request))
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "task_workflow_resolve_task_worktree_location")]
#[pyo3(signature = (repo_root, *, worktree_name, debug_probe_override=None))]
fn task_workflow_resolve_task_worktree_location_py(
    py: Python<'_>,
    repo_root: &str,
    worktree_name: &str,
    debug_probe_override: Option<Bound<'_, PyAny>>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let debug_override_value = match debug_probe_override {
        Some(value) => Some(parse_json_value(value, "debug_probe_override")?),
        None => None,
    };
    let payload = py.detach(|| {
        task_workflow_primitives::task_resolve_worktree_location(
            &repo,
            worktree_name,
            debug_override_value.as_ref(),
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_resolve_main_seed_mirror_location")]
#[pyo3(signature = (repo_root, *, seed_name))]
fn task_workflow_resolve_main_seed_mirror_location_py(
    py: Python<'_>,
    repo_root: &str,
    seed_name: &str,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::task_resolve_main_seed_mirror_location(&repo, seed_name)
    });
    match payload {
        Ok(JsonValue::Null) => Ok(py.None()),
        Ok(value) => json_value_to_py(py, &value),
        Err(err) => Err(PyRuntimeError::new_err(err)),
    }
}

#[pyfunction(name = "task_workflow_ensure_main_seed_mirror")]
#[pyo3(signature = (repo_root, *, force_refresh=false, line_name=None))]
fn task_workflow_ensure_main_seed_mirror_py(
    py: Python<'_>,
    repo_root: &str,
    force_refresh: bool,
    line_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::task_ensure_main_seed_mirror(&repo, force_refresh, line_name)
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_queue_summary")]
#[pyo3(signature = (repo_root, *, remote_name=None))]
fn task_workflow_queue_summary_py(
    py: Python<'_>,
    repo_root: &str,
    remote_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| task_workflow_primitives::queue_summary(&repo, remote_name));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_auth_whoami")]
#[pyo3(signature = (repo_root, *, remote_name=None, repo_name=None))]
fn task_workflow_auth_whoami_py(
    py: Python<'_>,
    repo_root: &str,
    remote_name: Option<String>,
    repo_name: Option<String>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let request = AuthRemoteRequest {
        remote_name,
        repo_name,
    };
    let payload = py.detach(|| rust_task_workflow_auth_whoami(&repo, &request));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_auth_grant")]
#[pyo3(signature = (repo_root, *, actor_identity, roles, remote_name=None, repo_name=None))]
fn task_workflow_auth_grant_py(
    py: Python<'_>,
    repo_root: &str,
    actor_identity: String,
    roles: Vec<String>,
    remote_name: Option<String>,
    repo_name: Option<String>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let request = AuthGrantRequest {
        remote_name,
        repo_name,
        actor_identity,
        roles,
    };
    let payload = py.detach(|| rust_task_workflow_auth_grant(&repo, &request));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_auth_bindings")]
#[pyo3(signature = (repo_root, *, remote_name=None, repo_name=None))]
fn task_workflow_auth_bindings_py(
    py: Python<'_>,
    repo_root: &str,
    remote_name: Option<String>,
    repo_name: Option<String>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let request = AuthRemoteRequest {
        remote_name,
        repo_name,
    };
    let payload = py.detach(|| rust_task_workflow_auth_bindings(&repo, &request));
    match payload {
        Ok(value) => json_value_to_py(py, &value),
        Err(err) => Err(PyRuntimeError::new_err(err)),
    }
}

#[pyfunction(name = "task_workflow_remote_add")]
#[pyo3(signature = (repo_root, *, payload))]
fn task_workflow_remote_add_py(
    py: Python<'_>,
    repo_root: &str,
    payload: Bound<'_, PyAny>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload_value = parse_json_value(payload, "payload")?;
    let result = py.detach(|| rust_task_workflow_remote_add(&repo, &payload_value));
    render_task_workflow_primitive_result(py, result)
}

#[pyfunction(name = "task_workflow_remote_list")]
fn task_workflow_remote_list_py(py: Python<'_>, repo_root: &str) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| rust_task_workflow_remote_list(&repo));
    match payload {
        Ok(value) => json_value_to_py(py, &value),
        Err(err) => Err(PyRuntimeError::new_err(err)),
    }
}

#[pyfunction(name = "task_workflow_repo_command")]
#[pyo3(signature = (repo_root, *, payload))]
fn task_workflow_repo_command_py(
    py: Python<'_>,
    repo_root: &str,
    payload: Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload_value = parse_json_value(payload, "payload")?;
    let result = py.detach(|| rust_task_workflow_repo_command(&repo, &payload_value));
    render_json_value(py, result.map_err(PyValueError::new_err)?)
}

#[pyfunction(name = "task_workflow_doctor_runtime_root")]
fn task_workflow_doctor_runtime_root_py(
    py: Python<'_>,
    repo_root: &str,
) -> PyResult<Py<PyDict>> {
    let payload = py.detach(|| rust_task_workflow_doctor_runtime_root(Path::new(repo_root)));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_doctor_plan_authority")]
fn task_workflow_doctor_plan_authority_py(py: Python<'_>) -> PyResult<Py<PyDict>> {
    let payload = py.detach(rust_task_workflow_doctor_plan_authority);
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_init")]
#[pyo3(signature = (
    repo_root,
    *,
    name=None,
    default_line="main",
    policy_profile="prototype",
    default_author_mode="ai_with_human_review",
    default_model=None,
    repair_existing=false
))]
#[expect(
    clippy::too_many_arguments,
    reason = "Python parameters are a compatibility contract"
)]
fn task_workflow_init_py(
    py: Python<'_>,
    repo_root: &str,
    name: Option<&str>,
    default_line: &str,
    policy_profile: &str,
    default_author_mode: &str,
    default_model: Option<&str>,
    repair_existing: bool,
) -> PyResult<Py<PyDict>> {
    let request = InitRequest {
        root: PathBuf::from(repo_root),
        name: name.map(str::to_string),
        default_line: default_line.to_string(),
        policy_profile: policy_profile.to_string(),
        default_author_mode: default_author_mode.to_string(),
        default_model: default_model.map(str::to_string),
        repair_existing,
    };
    let payload = py.detach(|| rust_task_workflow_init(&request));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_status")]
fn task_workflow_status_py(py: Python<'_>, repo_root: &str) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| task_workflow_primitives::repo_status(&repo));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_pull")]
#[pyo3(signature = (repo_root, *, remote_name=None, line_name=None, merge=false, restore=false, force=false))]
fn task_workflow_pull_py(
    py: Python<'_>,
    repo_root: &str,
    remote_name: Option<&str>,
    line_name: Option<&str>,
    merge: bool,
    restore: bool,
    force: bool,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::pull(&repo, remote_name, line_name, merge, restore, force)
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_push")]
#[pyo3(signature = (repo_root, *, remote_name=None, line_name=None))]
fn task_workflow_push_py(
    py: Python<'_>,
    repo_root: &str,
    remote_name: Option<&str>,
    line_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| task_workflow_primitives::push(&repo, remote_name, line_name));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_upload_snapshot_chain")]
#[pyo3(signature = (repo_root, snapshot_id, *, remote_name=None, line_name=None, reason=None))]
fn task_workflow_upload_snapshot_chain_py(
    py: Python<'_>,
    repo_root: &str,
    snapshot_id: &str,
    remote_name: Option<&str>,
    line_name: Option<&str>,
    reason: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::upload_snapshot_chain(
            &repo,
            remote_name,
            snapshot_id,
            line_name,
            reason,
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_release_candidate_create")]
fn task_workflow_release_candidate_create_py(
    py: Python<'_>,
    repo_root: &str,
    version: &str,
    line: &str,
    profile: &str,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_release_surface::release_candidate_create(&repo, version, line, profile)
    });
    render_json_value(py, payload.map_err(PyValueError::new_err)?)
}

#[pyfunction(name = "task_workflow_release_check")]
#[pyo3(signature = (repo_root, release_id, *, tests_command=None, skip_tests_reason=None))]
fn task_workflow_release_check_py(
    py: Python<'_>,
    repo_root: &str,
    release_id: &str,
    tests_command: Option<&str>,
    skip_tests_reason: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_release_surface::release_check(
            &repo,
            release_id,
            tests_command,
            skip_tests_reason,
        )
    });
    render_json_value(py, payload.map_err(PyValueError::new_err)?)
}

#[pyfunction(name = "task_workflow_release_build")]
fn task_workflow_release_build_py(
    py: Python<'_>,
    repo_root: &str,
    release_id: &str,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload =
        py.detach(|| task_workflow_release_surface::release_build(&repo, release_id));
    render_json_value(py, payload.map_err(PyValueError::new_err)?)
}

#[pyfunction(name = "task_workflow_release_formula")]
fn task_workflow_release_formula_py(
    py: Python<'_>,
    repo_root: &str,
    release_id: &str,
    name: &str,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py
        .detach(|| task_workflow_release_surface::release_formula(&repo, release_id, name));
    render_json_value(py, payload.map_err(PyValueError::new_err)?)
}

#[pyfunction(name = "task_workflow_release_show")]
#[pyo3(signature = (repo_root, release_id, *, remote_name=None))]
fn task_workflow_release_show_py(
    py: Python<'_>,
    repo_root: &str,
    release_id: &str,
    remote_name: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_release_surface::release_show(&repo, release_id, remote_name)
    });
    render_json_value(py, payload.map_err(PyValueError::new_err)?)
}

#[pyfunction(name = "task_workflow_release_publish")]
#[pyo3(signature = (repo_root, release_id, *, remote_name=None))]
fn task_workflow_release_publish_py(
    py: Python<'_>,
    repo_root: &str,
    release_id: &str,
    remote_name: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_release_surface::release_publish(&repo, release_id, remote_name)
    });
    render_json_value(py, payload.map_err(PyValueError::new_err)?)
}

#[pyfunction(name = "create_workflow_release")]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (
    repo_root,
    release_id,
    repo_name,
    version,
    line_name,
    snapshot_id,
    manifest_hash,
    profile,
    *,
    package_name=None,
    package_version=None,
    package_requires_python=None,
    status=None,
    checks_json="[]",
    artifacts_json="[]",
    formula_json="{}",
    metadata_json="{}"
))]
fn create_workflow_release_py(
    py: Python<'_>,
    repo_root: &str,
    release_id: &str,
    repo_name: &str,
    version: &str,
    line_name: &str,
    snapshot_id: &str,
    manifest_hash: &str,
    profile: &str,
    package_name: Option<&str>,
    package_version: Option<&str>,
    package_requires_python: Option<&str>,
    status: Option<&str>,
    checks_json: &str,
    artifacts_json: &str,
    formula_json: &str,
    metadata_json: &str,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_release_surface::create_workflow_release_explicit(
            &repo,
            release_id,
            repo_name,
            version,
            line_name,
            snapshot_id,
            manifest_hash,
            profile,
            package_name,
            package_version,
            package_requires_python,
            status,
            checks_json,
            artifacts_json,
            formula_json,
            metadata_json,
        )
    });
    render_json_dict(py, payload.map_err(release_store_py_error)?)
}

#[pyfunction(name = "list_workflow_releases")]
fn list_workflow_releases_py(py: Python<'_>, repo_root: &str) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| task_workflow_release_surface::list_workflow_releases(&repo));
    render_json_value(py, payload.map_err(release_store_py_error)?)
}

#[pyfunction(name = "get_workflow_release")]
fn get_workflow_release_py(
    py: Python<'_>,
    repo_root: &str,
    release_id: &str,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload =
        py.detach(|| task_workflow_release_surface::get_workflow_release(&repo, release_id));
    render_json_dict(py, payload.map_err(release_store_py_error)?)
}

#[pyfunction(name = "update_workflow_release")]
#[pyo3(signature = (
    repo_root,
    release_id,
    *,
    status=None,
    checks_json=None,
    checks_set=false,
    artifacts_json=None,
    artifacts_set=false,
    formula_json=None,
    formula_set=false,
    metadata_json=None,
    metadata_set=false
))]
#[allow(clippy::too_many_arguments)]
fn update_workflow_release_py(
    py: Python<'_>,
    repo_root: &str,
    release_id: &str,
    status: Option<&str>,
    checks_json: Option<&str>,
    checks_set: bool,
    artifacts_json: Option<&str>,
    artifacts_set: bool,
    formula_json: Option<&str>,
    formula_set: bool,
    metadata_json: Option<&str>,
    metadata_set: bool,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_release_surface::update_workflow_release_explicit(
            &repo,
            release_id,
            status,
            checks_set.then_some(checks_json.unwrap_or("null")),
            artifacts_set.then_some(artifacts_json.unwrap_or("null")),
            formula_set.then_some(formula_json.unwrap_or("null")),
            metadata_set.then_some(metadata_json.unwrap_or("null")),
        )
    });
    render_json_dict(py, payload.map_err(release_store_py_error)?)
}

#[pyfunction(name = "task_workflow_ensure_status_manifest")]
fn task_workflow_ensure_status_manifest_py(
    py: Python<'_>,
    repo_root: &str,
    snapshot_id: &str,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload =
        py.detach(|| task_workflow_primitives::ensure_status_manifest(&repo, snapshot_id));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_blame")]
#[pyo3(signature = (
    repo_root,
    path,
    *,
    line=None,
    start_line=None,
    end_line=None,
    snapshot_id=None,
    via_parent_snapshot_id=None,
    patchset_id=None,
    remote_name=None,
    plan_id=None,
    plan_ref=None
))]
#[expect(
    clippy::too_many_arguments,
    reason = "Python parameters are a compatibility contract"
)]
fn task_workflow_blame_py(
    py: Python<'_>,
    repo_root: &str,
    path: &str,
    line: Option<usize>,
    start_line: Option<usize>,
    end_line: Option<usize>,
    snapshot_id: Option<&str>,
    via_parent_snapshot_id: Option<&str>,
    patchset_id: Option<&str>,
    remote_name: Option<&str>,
    plan_id: Option<&str>,
    plan_ref: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let request = BlameRequest {
        path: path.to_string(),
        line,
        start_line,
        end_line,
        snapshot_id: snapshot_id.map(str::to_string),
        via_parent_snapshot_id: via_parent_snapshot_id.map(str::to_string),
        patchset_id: patchset_id.map(str::to_string),
        remote_name: remote_name.map(str::to_string),
        plan_id: plan_id.map(str::to_string),
        plan_ref: plan_ref.map(str::to_string),
    };
    let payload = py.detach(|| rust_task_workflow_blame(&repo, &request));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_snapshot_restore_lines")]
#[expect(
    clippy::too_many_arguments,
    reason = "the function mirrors the stable Python positional and keyword ABI"
)]
#[pyo3(signature = (
    repo_root,
    snapshot_id,
    path,
    *,
    line=None,
    start_line=None,
    end_line=None,
    yes=false
))]
fn task_workflow_snapshot_restore_lines_py(
    py: Python<'_>,
    repo_root: &str,
    snapshot_id: &str,
    path: &str,
    line: Option<usize>,
    start_line: Option<usize>,
    end_line: Option<usize>,
    yes: bool,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let request = SnapshotRestoreLinesRequest {
        snapshot_id: snapshot_id.to_string(),
        path: path.to_string(),
        line,
        start_line,
        end_line,
        apply: yes,
    };
    let payload = py.detach(|| rust_task_workflow_snapshot_restore_lines(&repo, &request));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_stash_list")]
fn task_workflow_stash_list_py(py: Python<'_>, repo_root: &str) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| task_workflow_primitives::stash_list(&repo));
    render_json_value(py, payload.map_err(PyValueError::new_err)?)
}

#[pyfunction(name = "task_workflow_stash_show")]
fn task_workflow_stash_show_py(
    py: Python<'_>,
    repo_root: &str,
    stash_id: &str,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| task_workflow_primitives::stash_show(&repo, stash_id));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_stash_drop")]
fn task_workflow_stash_drop_py(
    py: Python<'_>,
    repo_root: &str,
    stash_id: &str,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| task_workflow_primitives::stash_drop(&repo, stash_id));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_config_show")]
fn task_workflow_config_show_py(py: Python<'_>, repo_root: &str) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| rust_task_workflow_config_show(&repo));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_config_set")]
#[pyo3(signature = (repo_root, *, payload))]
fn task_workflow_config_set_py(
    py: Python<'_>,
    repo_root: &str,
    payload: Bound<'_, PyAny>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload_value = parse_json_value(payload, "payload")?;
    let result = py.detach(|| rust_task_workflow_config_set(&repo, &payload_value));
    render_task_workflow_primitive_result(py, result)
}

#[pyfunction(name = "task_workflow_line_list")]
#[pyo3(signature = (repo_root, *, include_all=false, archived=false, remote_name=None))]
fn task_workflow_line_list_py(
    py: Python<'_>,
    repo_root: &str,
    include_all: bool,
    archived: bool,
    remote_name: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::line_list(&repo, include_all, archived, remote_name)
    });
    match payload {
        Ok(value) => json_value_to_py(py, &value),
        Err(err) => Err(PyRuntimeError::new_err(err)),
    }
}

#[pyfunction(name = "task_workflow_line_create")]
#[pyo3(signature = (repo_root, name, *, from_snapshot=None, switch=false))]
fn task_workflow_line_create_py(
    py: Python<'_>,
    repo_root: &str,
    name: &str,
    from_snapshot: Option<&str>,
    switch: bool,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload =
        py.detach(|| task_workflow_primitives::line_create(&repo, name, from_snapshot, switch));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_line_switch")]
#[pyo3(signature = (repo_root, name, *, restore=false, force=false))]
fn task_workflow_line_switch_py(
    py: Python<'_>,
    repo_root: &str,
    name: &str,
    restore: bool,
    force: bool,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload =
        py.detach(|| task_workflow_primitives::line_switch(&repo, name, restore, force));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_line_show")]
#[pyo3(signature = (repo_root, *, name=None))]
fn task_workflow_line_show_py(
    py: Python<'_>,
    repo_root: &str,
    name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| task_workflow_primitives::line_show(&repo, name));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_line_archive")]
#[pyo3(signature = (repo_root, name, *, remote_name=None))]
fn task_workflow_line_archive_py(
    py: Python<'_>,
    repo_root: &str,
    name: &str,
    remote_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload =
        py.detach(|| task_workflow_primitives::line_archive(&repo, name, remote_name));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_line_rename")]
#[pyo3(signature = (repo_root, old_name, new_name, *, remote_name=None))]
fn task_workflow_line_rename_py(
    py: Python<'_>,
    repo_root: &str,
    old_name: &str,
    new_name: &str,
    remote_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::line_rename(&repo, old_name, new_name, remote_name)
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_line_delete")]
#[pyo3(signature = (repo_root, name, *, remote_name=None, confirm=false))]
fn task_workflow_line_delete_py(
    py: Python<'_>,
    repo_root: &str,
    name: &str,
    remote_name: Option<&str>,
    confirm: bool,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::line_delete(&repo, name, remote_name, confirm)
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_line_set_head")]
#[pyo3(signature = (repo_root, name, snapshot_id=None))]
fn task_workflow_line_set_head_py(
    py: Python<'_>,
    repo_root: &str,
    name: &str,
    snapshot_id: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload =
        py.detach(|| task_workflow_primitives::line_set_head(&repo, name, snapshot_id));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_line_cleanup")]
#[pyo3(signature = (repo_root, *, idle_for=None, cleanup_kind=None, limit=None, include_protected=false, apply=false))]
fn task_workflow_line_cleanup_py(
    py: Python<'_>,
    repo_root: &str,
    idle_for: Option<&str>,
    cleanup_kind: Option<&str>,
    limit: Option<usize>,
    include_protected: bool,
    apply: bool,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::line_cleanup(
            &repo,
            idle_for,
            cleanup_kind,
            limit,
            include_protected,
            apply,
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_worktree_get")]
#[pyo3(signature = (repo_root, *, name=None, refresh_status=true))]
fn task_workflow_worktree_get_py(
    py: Python<'_>,
    repo_root: &str,
    name: Option<&str>,
    refresh_status: bool,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload =
        py.detach(|| task_workflow_primitives::worktree_get(&repo, name, refresh_status));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_worktree_status")]
#[pyo3(signature = (repo_root, *, name=None, snapshot_id=None, line_name=None))]
fn task_workflow_worktree_status_py(
    py: Python<'_>,
    repo_root: &str,
    name: Option<&str>,
    snapshot_id: Option<&str>,
    line_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::worktree_status(&repo, name, snapshot_id, line_name)
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_worktree_restore")]
#[pyo3(signature = (repo_root, *, name=None, snapshot_id=None, line_name=None, paths=Vec::new(), force=false, dry_run=false))]
#[expect(
    clippy::too_many_arguments,
    reason = "Python parameters are a compatibility contract"
)]
fn task_workflow_worktree_restore_py(
    py: Python<'_>,
    repo_root: &str,
    name: Option<&str>,
    snapshot_id: Option<&str>,
    line_name: Option<&str>,
    paths: Vec<String>,
    force: bool,
    dry_run: bool,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::worktree_restore(
            &repo,
            name,
            snapshot_id,
            line_name,
            &paths,
            force,
            dry_run,
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_worktree_list")]
#[pyo3(signature = (repo_root, *, refresh_status=true))]
fn task_workflow_worktree_list_py(
    py: Python<'_>,
    repo_root: &str,
    refresh_status: bool,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload =
        py.detach(|| task_workflow_primitives::worktree_list(&repo, refresh_status));
    match payload {
        Ok(value) => json_value_to_py(py, &value),
        Err(err) => Err(PyRuntimeError::new_err(err)),
    }
}

#[pyfunction(name = "task_workflow_worktree_touch_usage")]
#[pyo3(signature = (repo_root, *, name=None))]
fn task_workflow_worktree_touch_usage_py(
    py: Python<'_>,
    repo_root: &str,
    name: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| task_workflow_primitives::worktree_touch_usage(&repo, name));
    match payload {
        Ok(JsonValue::Null) => Ok(py.None()),
        Ok(value) => json_value_to_py(py, &value),
        Err(err) => Err(PyRuntimeError::new_err(err)),
    }
}

#[pyfunction(name = "task_workflow_worktree_doctor")]
#[pyo3(signature = (repo_root, *, refresh_status=true))]
fn task_workflow_worktree_doctor_py(
    py: Python<'_>,
    repo_root: &str,
    refresh_status: bool,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload =
        py.detach(|| task_workflow_primitives::worktree_doctor(&repo, refresh_status));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_worktree_cleanup_candidates")]
#[pyo3(signature = (repo_root, *, older_than=None, cleanup_policy=None, include_protected=false, allow_manual_only=false))]
fn task_workflow_worktree_cleanup_candidates_py(
    py: Python<'_>,
    repo_root: &str,
    older_than: Option<&str>,
    cleanup_policy: Option<&str>,
    include_protected: bool,
    allow_manual_only: bool,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::worktree_cleanup_candidates(
            &repo,
            older_than,
            cleanup_policy,
            include_protected,
            allow_manual_only,
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_worktree_cleanup")]
#[pyo3(signature = (repo_root, *, older_than=None, cleanup_policy=None, allow_manual_only=false, limit=None, dry_run=false))]
fn task_workflow_worktree_cleanup_py(
    py: Python<'_>,
    repo_root: &str,
    older_than: Option<&str>,
    cleanup_policy: Option<&str>,
    allow_manual_only: bool,
    limit: Option<usize>,
    dry_run: bool,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::worktree_cleanup(
            &repo,
            older_than,
            cleanup_policy,
            allow_manual_only,
            limit,
            dry_run,
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_worktree_prune_stale")]
#[pyo3(signature = (repo_root, *, dry_run=false))]
fn task_workflow_worktree_prune_stale_py(
    py: Python<'_>,
    repo_root: &str,
    dry_run: bool,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload =
        py.detach(|| task_workflow_primitives::worktree_prune_stale(&repo, dry_run));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_worktree_remove")]
#[pyo3(signature = (repo_root, names, *, all_stale=false, delete_path=false, force=false, dry_run=false))]
fn task_workflow_worktree_remove_py(
    py: Python<'_>,
    repo_root: &str,
    names: Vec<String>,
    all_stale: bool,
    delete_path: bool,
    force: bool,
    dry_run: bool,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::worktree_remove(
            &repo,
            &names,
            all_stale,
            delete_path,
            force,
            dry_run,
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_worktree_bind_existing")]
#[pyo3(signature = (repo_root, *, worktree_name, task_id=None, change_id=None, auto_created_for_task=false, target_base_line=None, fork_snapshot_id=None, forked_from_line=None))]
#[expect(
    clippy::too_many_arguments,
    reason = "Python parameters are a compatibility contract"
)]
fn task_workflow_worktree_bind_existing_py(
    py: Python<'_>,
    repo_root: &str,
    worktree_name: &str,
    task_id: Option<&str>,
    change_id: Option<&str>,
    auto_created_for_task: bool,
    target_base_line: Option<&str>,
    fork_snapshot_id: Option<&str>,
    forked_from_line: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::worktree_bind_existing(
            &repo,
            worktree_name,
            task_id,
            change_id,
            auto_created_for_task,
            target_base_line,
            fork_snapshot_id,
            forked_from_line,
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_worktree_sync")]
#[pyo3(signature = (repo_root, *, name=None, line_name=None, force=false, dry_run=false))]
fn task_workflow_worktree_sync_py(
    py: Python<'_>,
    repo_root: &str,
    name: Option<&str>,
    line_name: Option<&str>,
    force: bool,
    dry_run: bool,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::worktree_sync(&repo, name, line_name, force, dry_run)
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_worktree_sync_all")]
#[pyo3(signature = (repo_root, *, force=false, dry_run=false))]
fn task_workflow_worktree_sync_all_py(
    py: Python<'_>,
    repo_root: &str,
    force: bool,
    dry_run: bool,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload =
        py.detach(|| task_workflow_primitives::worktree_sync_all(&repo, force, dry_run));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_worktree_recreate")]
#[pyo3(signature = (repo_root, *, name=None, dry_run=false))]
fn task_workflow_worktree_recreate_py(
    py: Python<'_>,
    repo_root: &str,
    name: Option<&str>,
    dry_run: bool,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload =
        py.detach(|| task_workflow_primitives::worktree_recreate(&repo, name, dry_run));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_worktree_restore_owned_head")]
#[pyo3(signature = (repo_root, *, name=None, dry_run=false))]
fn task_workflow_worktree_restore_owned_head_py(
    py: Python<'_>,
    repo_root: &str,
    name: Option<&str>,
    dry_run: bool,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::worktree_restore_owned_head(&repo, name, dry_run)
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_worktree_preview_rebase")]
#[pyo3(signature = (repo_root, *, name=None, onto_line_name=None))]
fn task_workflow_worktree_preview_rebase_py(
    py: Python<'_>,
    repo_root: &str,
    name: Option<&str>,
    onto_line_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::worktree_preview_rebase(&repo, name, onto_line_name)
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_worktree_rebase")]
#[pyo3(signature = (repo_root, *, name=None, onto_line_name=None))]
fn task_workflow_worktree_rebase_py(
    py: Python<'_>,
    repo_root: &str,
    name: Option<&str>,
    onto_line_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload =
        py.detach(|| task_workflow_primitives::worktree_rebase(&repo, name, onto_line_name));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_worktree_continue_rebase")]
#[pyo3(signature = (repo_root, *, name=None))]
fn task_workflow_worktree_continue_rebase_py(
    py: Python<'_>,
    repo_root: &str,
    name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload =
        py.detach(|| task_workflow_primitives::worktree_continue_rebase(&repo, name));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_worktree_abort_rebase")]
#[pyo3(signature = (repo_root, *, name=None))]
fn task_workflow_worktree_abort_rebase_py(
    py: Python<'_>,
    repo_root: &str,
    name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| task_workflow_primitives::worktree_abort_rebase(&repo, name));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_task_start")]
#[pyo3(signature = (
    repo_root,
    title,
    intent,
    *,
    local=false,
    remote_name=None
))]
fn task_workflow_task_start_py(
    py: Python<'_>,
    repo_root: &str,
    title: &str,
    intent: &str,
    local: bool,
    remote_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::task_start(&repo, title, intent, local, remote_name)
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_task_list")]
#[pyo3(signature = (repo_root, *, local=false, remote_name=None, all=false))]
fn task_workflow_task_list_py(
    py: Python<'_>,
    repo_root: &str,
    local: bool,
    remote_name: Option<&str>,
    all: bool,
) -> PyResult<Py<PyList>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload =
        py.detach(|| task_workflow_primitives::task_list(&repo, local, remote_name));
    match payload.map_err(PyValueError::new_err)? {
        JsonValue::Array(mut values) => {
            if !all {
                values.retain(|row| {
                    !matches!(
                        row.get("status").and_then(JsonValue::as_str),
                        Some(
                            "completed"
                                | "abandoned"
                                | "canceled"
                                | "later_promotion_excluded"
                        )
                    )
                });
            }
            render_json_list(py, values)
        }
        _ => Err(PyValueError::new_err(
            "task list payload must decode to a JSON array.",
        )),
    }
}

#[pyfunction(name = "task_workflow_task_show")]
#[pyo3(signature = (repo_root, task_id, *, local=false, remote_name=None))]
fn task_workflow_task_show_py(
    py: Python<'_>,
    repo_root: &str,
    task_id: &str,
    local: bool,
    remote_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload =
        py.detach(|| task_workflow_primitives::task_show(&repo, task_id, local, remote_name));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_task_audit")]
#[pyo3(signature = (repo_root, task_id, *, local=false, remote_name=None))]
fn task_workflow_task_audit_py(
    py: Python<'_>,
    repo_root: &str,
    task_id: &str,
    local: bool,
    remote_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload =
        py.detach(|| task_workflow_primitives::task_audit(&repo, task_id, local, remote_name));
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_task_abandon")]
#[pyo3(signature = (repo_root, task_id, *, local=false, remote_name=None))]
fn task_workflow_task_abandon_py(
    py: Python<'_>,
    repo_root: &str,
    task_id: &str,
    local: bool,
    remote_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::task_abandon(&repo, task_id, local, remote_name)
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_workflow_task_land")]
#[pyo3(signature = (repo_root, task_or_change_id, *, local=false, remote_name=None))]
fn task_workflow_task_land_py(
    py: Python<'_>,
    repo_root: &str,
    task_or_change_id: &str,
    local: bool,
    remote_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let use_local_scope = repo
        .task_uses_local_scope(local, remote_name)
        .map_err(PyValueError::new_err)?;
    let payload = py.detach(|| {
        task_workflow_primitives::task_land_apply_scoped(
            &repo,
            task_or_change_id,
            use_local_scope,
            remote_name,
            None::<fn(&JsonValue) -> Result<(), String>>,
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "task_remote_change_lineage_payload")]
fn task_remote_change_lineage_payload_py(
    py: Python<'_>,
    base_line: &str,
    line_row: Option<Bound<'_, PyAny>>,
) -> PyResult<Py<PyDict>> {
    let line_row = match line_row {
        Some(value) => Some(parse_json_value(value, "line_row")?),
        None => None,
    };
    let payload = rust_task_remote_change_lineage_payload(base_line, line_row.as_ref())
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "task_workflow_timestamp_facts")]
#[pyo3(signature = (now=None))]
fn task_workflow_timestamp_facts_py(py: Python<'_>, now: Option<&str>) -> PyResult<Py<PyDict>> {
    render_json_dict(
        py,
        build_rust_task_workflow_timestamp_facts(now).map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "task_workflow_sequence_identity_facts")]
#[pyo3(signature = (family, number, namespace_prefix=None, width=4))]
fn task_workflow_sequence_identity_facts_py(
    py: Python<'_>,
    family: &str,
    number: i64,
    namespace_prefix: Option<&str>,
    width: usize,
) -> PyResult<Py<PyDict>> {
    render_json_dict(
        py,
        build_rust_task_workflow_sequence_identity_facts(family, number, namespace_prefix, width)
            .map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "task_workflow_workflow_id_facts")]
#[pyo3(signature = (family, namespace_prefix=None, timestamp_ms=None, randomness_hex=None))]
fn task_workflow_workflow_id_facts_py(
    py: Python<'_>,
    family: &str,
    namespace_prefix: Option<&str>,
    timestamp_ms: Option<i64>,
    randomness_hex: Option<&str>,
) -> PyResult<Py<PyDict>> {
    render_json_dict(
        py,
        build_rust_task_workflow_workflow_id_facts(
            family,
            namespace_prefix,
            timestamp_ms,
            randomness_hex,
        )
        .map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "task_workflow_runtime_selection_facts")]
#[pyo3(signature = (overrides=None))]
fn task_workflow_runtime_selection_facts_py(
    py: Python<'_>,
    overrides: Option<Bound<'_, PyAny>>,
) -> PyResult<Py<PyDict>> {
    let overrides_value = match overrides {
        Some(value) => Some(parse_json_value(value, "overrides")?),
        None => None,
    };
    render_json_dict(
        py,
        build_rust_task_workflow_runtime_selection_facts(overrides_value.as_ref())
            .map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "task_workflow_read_utf8_text_file")]
fn task_workflow_read_utf8_text_file_py(path_value: &str) -> PyResult<String> {
    rust_task_workflow_read_utf8_text_file(path_value).map_err(PyValueError::new_err)
}

#[pyfunction(name = "task_workflow_read_json_file")]
fn task_workflow_read_json_file_py(py: Python<'_>, path_value: &str) -> PyResult<Py<PyAny>> {
    let payload = rust_task_workflow_read_json_file(path_value).map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "task_workflow_read_binary_file")]
fn task_workflow_read_binary_file_py(py: Python<'_>, path_value: &str) -> PyResult<Py<PyBytes>> {
    let payload = rust_task_workflow_read_binary_file(path_value).map_err(PyValueError::new_err)?;
    Ok(PyBytes::new(py, &payload).unbind())
}

#[pyfunction(name = "task_workflow_resolve_repo_artifact_path")]
#[pyo3(signature = (repo_root, path_value, allow_missing=false))]
fn task_workflow_resolve_repo_artifact_path_py(
    py: Python<'_>,
    repo_root: &str,
    path_value: &str,
    allow_missing: bool,
) -> PyResult<Py<PyDict>> {
    render_json_dict(
        py,
        rust_task_workflow_resolve_repo_artifact_path(repo_root, path_value, allow_missing)
            .map_err(PyValueError::new_err)?,
    )
}


#[pyfunction(name = "build_linked_task_lookup_payload")]
fn build_linked_task_lookup_payload_py(
    py: Python<'_>,
    task_links_by_item_rows: Option<Bound<'_, PyAny>>,
    tasks_by_plan_rows: Option<Bound<'_, PyAny>>,
) -> PyResult<Py<PyDict>> {
    let task_links_value = match task_links_by_item_rows {
        Some(value) => Some(parse_json_value(value, "task_links_by_item_rows")?),
        None => None,
    };
    let tasks_by_plan_value = match tasks_by_plan_rows {
        Some(value) => Some(parse_json_value(value, "tasks_by_plan_rows")?),
        None => None,
    };
    render_json_dict(
        py,
        build_rust_linked_task_lookup_payload(
            task_links_value.as_ref(),
            tasks_by_plan_value.as_ref(),
        )
        .map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "build_linked_change_lookup_payload")]
fn build_linked_change_lookup_payload_py(
    py: Python<'_>,
    change_links_by_task_rows: Option<Bound<'_, PyAny>>,
) -> PyResult<Py<PyDict>> {
    let payload_value = match change_links_by_task_rows {
        Some(value) => Some(parse_json_value(value, "change_links_by_task_rows")?),
        None => None,
    };
    render_json_dict(
        py,
        build_rust_linked_change_lookup_payload(payload_value.as_ref())
            .map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "build_task_tracking_title_payload")]
fn build_task_tracking_title_payload_py(
    py: Python<'_>,
    task: Bound<'_, PyAny>,
) -> PyResult<Py<PyDict>> {
    let task_value = parse_json_value(task, "task")?;
    render_json_dict(
        py,
        build_rust_task_tracking_title_payload(&task_value).map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "build_task_tracking_metadata_payload")]
fn build_task_tracking_metadata_payload_py(
    py: Python<'_>,
    task: Bound<'_, PyAny>,
    author_mode: &str,
    tracking_policy: &str,
) -> PyResult<Py<PyDict>> {
    let task_value = parse_json_value(task, "task")?;
    render_json_dict(
        py,
        build_rust_task_tracking_metadata_payload(&task_value, author_mode, tracking_policy)
            .map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "build_task_audit_verdict_payload")]
#[pyo3(signature = (task, change_rows, target_line))]
fn build_task_audit_verdict_payload_py(
    py: Python<'_>,
    task: Bound<'_, PyAny>,
    change_rows: Bound<'_, PyAny>,
    target_line: &str,
) -> PyResult<Py<PyDict>> {
    let task_value = parse_json_value(task, "task")?;
    let change_rows_value = parse_json_value(change_rows, "change_rows")?;
    render_json_dict(
        py,
        build_rust_task_audit_verdict_payload(&task_value, &change_rows_value, target_line)
            .map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "workflow_landed_facts")]
fn workflow_landed_facts_py(py: Python<'_>, state: Bound<'_, PyAny>) -> PyResult<Py<PyDict>> {
    let state_value = parse_json_value(state, "state")?;
    render_json_dict(
        py,
        build_rust_workflow_landed_facts(&state_value).map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "workflow_land_full_facts")]
fn workflow_land_full_facts_py(py: Python<'_>, state: Bound<'_, PyAny>) -> PyResult<Py<PyDict>> {
    let state_value = parse_json_value(state, "state")?;
    render_json_dict(
        py,
        build_rust_workflow_land_full_facts(&state_value).map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "workflow_ready_facts")]
fn workflow_ready_facts_py(py: Python<'_>, state: Bound<'_, PyAny>) -> PyResult<Py<PyDict>> {
    let state_value = parse_json_value(state, "state")?;
    render_json_dict(
        py,
        build_rust_workflow_ready_facts(&state_value).map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "workflow_land_phase_facts")]
fn workflow_land_phase_facts_py(
    py: Python<'_>,
    state: Bound<'_, PyAny>,
    ready_state: Bound<'_, PyAny>,
) -> PyResult<Py<PyDict>> {
    let state_value = parse_json_value(state, "state")?;
    let ready_state_value = parse_json_value(ready_state, "ready_state")?;
    render_json_dict(
        py,
        build_rust_workflow_land_phase_facts(&state_value, &ready_state_value)
            .map_err(PyValueError::new_err)?,
    )
}

#[pyfunction(name = "project_workflow_landed_read_model")]
fn project_workflow_landed_read_model_py(
    py: Python<'_>,
    ctx: Bound<'_, PyAny>,
    state: Bound<'_, PyAny>,
) -> PyResult<Py<PyDict>> {
    let state_value = parse_json_value(state.clone(), "state")?;
    let facts = build_rust_workflow_landed_facts(&state_value).map_err(PyValueError::new_err)?;
    let facts_state = facts.as_object().cloned().unwrap_or_default();
    let command_hints = rust_workflow_land_command_hints(
        &ctx,
        string_or_empty(&facts_state, "change", "change_id").as_str(),
        string_or_empty(&facts_state, "task", "task_id").as_str(),
        facts_state.get("patchset"),
        string_or_empty_root(&facts_state, "target_line").as_str(),
        string_or_empty_root(&facts_state, "target_line").as_str(),
        None,
        0,
        false,
        true,
    )?;
    let payload = project_rust_workflow_landed_read_model(&facts, &command_hints)
        .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "project_workflow_land_full_read_model")]
fn project_workflow_land_full_read_model_py(
    py: Python<'_>,
    ctx: Bound<'_, PyAny>,
    state: Bound<'_, PyAny>,
    apply_owned_continuation: bool,
) -> PyResult<Py<PyDict>> {
    let state_value = parse_json_value(state.clone(), "state")?;
    let landed_facts =
        build_rust_workflow_landed_facts(&state_value).map_err(PyValueError::new_err)?;
    if landed_facts
        .as_object()
        .and_then(|value| value.get("landed"))
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        return project_workflow_landed_read_model_py(py, ctx, state);
    }
    let facts = build_rust_workflow_land_full_facts(&state_value).map_err(PyValueError::new_err)?;
    let facts_state = facts.as_object().cloned().unwrap_or_default();
    let task_review_enabled = rust_task_review_enabled(&ctx)?;
    let command_hints = rust_workflow_land_command_hints(
        &ctx,
        string_or_empty_root(&facts_state, "resolved_change_id").as_str(),
        string_or_empty(&facts_state, "task", "task_id").as_str(),
        facts_state.get("patchset"),
        string_or_empty_root(&facts_state, "base_line_name").as_str(),
        string_or_empty_root(&facts_state, "target_line").as_str(),
        facts_state.get("worktree_retarget"),
        int_or_zero_root(&facts_state, "review_blocking"),
        bool_or_false_root(&facts_state, "requires_code_review_summary"),
        task_review_enabled,
    )?;
    let task_review_config = rust_effective_task_review(&ctx)?;
    let mut payload = project_rust_workflow_land_full_read_model(
        &facts,
        &command_hints,
        &task_review_config,
        apply_owned_continuation,
    )
    .map_err(PyValueError::new_err)?;
    let next_action = payload
        .as_object()
        .and_then(|value| value.get("next_action"))
        .cloned()
        .unwrap_or(JsonValue::Null);
    if let Some(wait_hint) = rust_workflow_closeout_wait_hint(&ctx, &state_value, &next_action)? {
        if let JsonValue::Object(ref mut object) = payload {
            object.insert("wait_hint".to_string(), wait_hint);
        }
    }
    render_json_dict(py, payload)
}

#[pyfunction(name = "project_workflow_ready_read_model")]
fn project_workflow_ready_read_model_py(
    py: Python<'_>,
    ctx: Bound<'_, PyAny>,
    state: Bound<'_, PyAny>,
    ignore_workspace_authoring: bool,
    patchset_is_authoritative: bool,
    apply_owned_continuation: bool,
) -> PyResult<Py<PyDict>> {
    let state_value = parse_json_value(state.clone(), "state")?;
    let facts = build_rust_workflow_ready_facts(&state_value).map_err(PyValueError::new_err)?;
    let facts_state = facts.as_object().cloned().unwrap_or_default();
    let command_hints = rust_workflow_ready_command_hints(
        &ctx,
        string_or_empty(&facts_state, "change", "change_id").as_str(),
        facts_state.get("patchset"),
        string_or_empty_root(&facts_state, "base_line_name").as_str(),
        facts_state.get("worktree_retarget"),
    )?;
    let mut payload = project_rust_workflow_ready_read_model(
        &facts,
        &command_hints,
        ignore_workspace_authoring,
        patchset_is_authoritative,
        apply_owned_continuation,
    )
    .map_err(PyValueError::new_err)?;
    let next_action = payload
        .as_object()
        .and_then(|value| value.get("next_action"))
        .cloned()
        .unwrap_or(JsonValue::Null);
    if let Some(wait_hint) = rust_workflow_closeout_wait_hint(&ctx, &state_value, &next_action)? {
        if let JsonValue::Object(ref mut object) = payload {
            object.insert("wait_hint".to_string(), wait_hint);
        }
    }
    render_json_dict(py, payload)
}

#[pyfunction(name = "project_workflow_land_phase_read_model")]
fn project_workflow_land_phase_read_model_py(
    py: Python<'_>,
    state: Bound<'_, PyAny>,
    ready_state: Bound<'_, PyAny>,
) -> PyResult<Py<PyDict>> {
    let state_value = parse_json_value(state, "state")?;
    let ready_state_value = parse_json_value(ready_state, "ready_state")?;
    let facts = build_rust_workflow_land_phase_facts(&state_value, &ready_state_value)
        .map_err(PyValueError::new_err)?;
    let payload =
        project_rust_workflow_land_phase_read_model(&facts).map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "workflow_remote_mutation_receipt")]
fn workflow_remote_mutation_receipt_py(
    py: Python<'_>,
    action: &str,
    source_action: &str,
    delivery: &str,
    response_recovery: Option<Bound<'_, PyAny>>,
    result: Option<Bound<'_, PyAny>>,
) -> PyResult<Py<PyDict>> {
    let response_recovery = match response_recovery {
        Some(value) => Some(parse_json_value(value, "response_recovery")?),
        None => None,
    };
    let result = match result {
        Some(value) => Some(parse_json_value(value, "result")?),
        None => None,
    };
    let payload = rust_workflow_remote_mutation_receipt(
        action,
        source_action,
        delivery,
        response_recovery.as_ref(),
        result.as_ref(),
    )
    .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "workflow_remote_action_mutation_receipts")]
fn workflow_remote_action_mutation_receipts_py(
    py: Python<'_>,
    code: &str,
    result: Bound<'_, PyAny>,
) -> PyResult<Py<PyList>> {
    let payload =
        rust_workflow_remote_action_mutation_receipts(code, &parse_json_value(result, "result")?)
            .map_err(PyValueError::new_err)?;
    match payload {
        JsonValue::Array(values) => render_json_list(py, values),
        _ => Err(PyRuntimeError::new_err(
            "Rust workflow closeout remote backend returned a non-list receipt payload.",
        )),
    }
}

#[pyfunction(name = "workflow_apply_phase_payload")]
#[pyo3(signature = (phase, code, detail=None, resumed_from_authoritative_state=false))]
fn workflow_apply_phase_payload_py(
    py: Python<'_>,
    phase: &str,
    code: &str,
    detail: Option<&str>,
    resumed_from_authoritative_state: bool,
) -> PyResult<Py<PyDict>> {
    let payload =
        rust_workflow_apply_phase_payload(phase, code, detail, resumed_from_authoritative_state)
            .map_err(PyValueError::new_err)?;
    render_json_dict(py, payload)
}

#[pyfunction(name = "workflow_apply_phase_summary")]
fn workflow_apply_phase_summary_py(apply_phase: Bound<'_, PyAny>) -> PyResult<Option<String>> {
    let apply_phase_value = parse_json_value(apply_phase, "apply_phase")?;
    rust_workflow_apply_phase_summary(&apply_phase_value).map_err(PyValueError::new_err)
}

#[pyfunction(name = "workflow_mutation_receipt_summary")]
fn workflow_mutation_receipt_summary_py(receipt: Bound<'_, PyAny>) -> PyResult<String> {
    let receipt_value = parse_json_value(receipt, "receipt")?;
    rust_workflow_mutation_receipt_summary(&receipt_value).map_err(PyValueError::new_err)
}

#[pyfunction(name = "workflow_applied_action_summary")]
fn workflow_applied_action_summary_py(action: Bound<'_, PyAny>) -> PyResult<String> {
    let action_value = parse_json_value(action, "action")?;
    rust_workflow_applied_action_summary(&action_value).map_err(PyValueError::new_err)
}


#[pyfunction(name = "workflow_ready_payload_direct")]
#[pyo3(signature = (repo_root, change_id, remote_name=None))]
fn workflow_ready_payload_direct_py(
    py: Python<'_>,
    repo_root: &str,
    change_id: &str,
    remote_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::workflow_ready_payload(&repo, change_id, remote_name)
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "workflow_land_payload_direct")]
#[pyo3(signature = (repo_root, change_id, remote_name=None))]
fn workflow_land_payload_direct_py(
    py: Python<'_>,
    repo_root: &str,
    change_id: &str,
    remote_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::workflow_land_payload(&repo, change_id, remote_name)
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "workflow_ready_apply_direct")]
#[pyo3(signature = (
    repo_root,
    change_id,
    snapshot_message=None,
    summary=None,
    tests=None,
    lint=None,
    security=None,
    license=None,
    author_mode=None,
    model=None,
    remote_name=None
))]
#[expect(
    clippy::too_many_arguments,
    reason = "Python parameters are a compatibility contract"
)]
fn workflow_ready_apply_direct_py(
    py: Python<'_>,
    repo_root: &str,
    change_id: &str,
    snapshot_message: Option<&str>,
    summary: Option<&str>,
    tests: Option<&str>,
    lint: Option<&str>,
    security: Option<&str>,
    license: Option<&str>,
    author_mode: Option<&str>,
    model: Option<&str>,
    remote_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::workflow_ready_apply(
            &repo,
            change_id,
            snapshot_message,
            summary,
            tests,
            lint,
            security,
            license,
            author_mode,
            model,
            remote_name,
            None::<fn(&JsonValue) -> Result<(), String>>,
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

#[pyfunction(name = "workflow_land_apply_direct")]
#[pyo3(signature = (repo_root, change_id, review_message=None, remote_name=None))]
fn workflow_land_apply_direct_py(
    py: Python<'_>,
    repo_root: &str,
    change_id: &str,
    review_message: Option<&str>,
    remote_name: Option<&str>,
) -> PyResult<Py<PyDict>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let payload = py.detach(|| {
        task_workflow_primitives::workflow_land_apply(
            &repo,
            change_id,
            review_message,
            remote_name,
            None::<fn(&JsonValue) -> Result<(), String>>,
        )
    });
    render_task_workflow_primitive_result(py, payload)
}

fn register_task_workflow(_py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(task_workflow_snapshot_create_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_snapshot_create_explicit_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(task_workflow_snapshot_list_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_snapshot_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_snapshot_chain_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_snapshot_exists_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_snapshot_restore_lines_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(task_workflow_blob_read_bytes_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_blob_ensure_bytes_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(task_workflow_workspace_delta_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_workspace_restore_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_workspace_restore_paths_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(task_workflow_snapshot_diff_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_snapshot_revert_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_snapshot_replay_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_change_list_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_change_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_change_revert_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_change_replay_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_change_close_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_change_publish_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_release_candidate_create_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(task_workflow_release_check_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_release_build_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_release_formula_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_release_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_release_publish_py, module)?)?;
    module.add_function(wrap_pyfunction!(create_workflow_release_py, module)?)?;
    module.add_function(wrap_pyfunction!(list_workflow_releases_py, module)?)?;
    module.add_function(wrap_pyfunction!(get_workflow_release_py, module)?)?;
    module.add_function(wrap_pyfunction!(update_workflow_release_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_patchset_publish_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_patchset_list_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_patchset_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_patchset_select_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_patchset_publish_explicit_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_patchset_ci_status_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_patchset_rerun_ci_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(task_workflow_review_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_review_request_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_review_team_approve_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_review_task_approve_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(task_workflow_review_record_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_review_code_submit_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_review_code_template_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(task_workflow_attest_put_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_attest_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_policy_eval_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_policy_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_policy_waive_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_land_submit_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_land_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_land_retry_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        current_core_source_fingerprint_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        current_server_source_fingerprint_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        current_source_native_cache_contract_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_resolve_task_worktree_location_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_resolve_main_seed_mirror_location_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_ensure_main_seed_mirror_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(task_workflow_queue_summary_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_auth_whoami_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_auth_grant_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_auth_bindings_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_remote_add_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_remote_list_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_repo_command_py, module)?)?;
    module.add_function(wrap_pyfunction!(workspace_command_lock_path_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_init_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_status_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_pull_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_push_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_upload_snapshot_chain_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_ensure_status_manifest_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_doctor_runtime_root_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_doctor_plan_authority_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(task_workflow_blame_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_stash_list_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_stash_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_stash_drop_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_config_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_config_set_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_line_list_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_line_create_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_line_switch_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_line_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_line_archive_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_line_rename_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_line_delete_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_line_set_head_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_line_cleanup_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_worktree_get_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_worktree_status_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_worktree_restore_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_worktree_list_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_worktree_touch_usage_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(task_workflow_worktree_doctor_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_worktree_cleanup_candidates_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(task_workflow_worktree_cleanup_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_worktree_prune_stale_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(task_workflow_worktree_remove_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_worktree_bind_existing_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(task_workflow_worktree_sync_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_worktree_sync_all_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_worktree_recreate_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_worktree_restore_owned_head_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_worktree_preview_rebase_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(task_workflow_worktree_rebase_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_worktree_continue_rebase_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_worktree_abort_rebase_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(task_workflow_task_start_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_task_list_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_task_show_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_task_audit_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_task_abandon_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_task_land_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        task_remote_change_lineage_payload_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(task_workflow_timestamp_facts_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_sequence_identity_facts_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_workflow_id_facts_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_runtime_selection_facts_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_read_utf8_text_file_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(task_workflow_read_json_file_py, module)?)?;
    module.add_function(wrap_pyfunction!(task_workflow_read_binary_file_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        task_workflow_resolve_repo_artifact_path_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        build_linked_task_lookup_payload_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        build_linked_change_lookup_payload_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        build_task_tracking_title_payload_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        build_task_tracking_metadata_payload_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        build_task_audit_verdict_payload_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(workflow_landed_facts_py, module)?)?;
    module.add_function(wrap_pyfunction!(workflow_land_full_facts_py, module)?)?;
    module.add_function(wrap_pyfunction!(workflow_ready_facts_py, module)?)?;
    module.add_function(wrap_pyfunction!(workflow_land_phase_facts_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        project_workflow_landed_read_model_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        project_workflow_land_full_read_model_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        project_workflow_ready_read_model_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        project_workflow_land_phase_read_model_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        workflow_remote_mutation_receipt_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        workflow_remote_action_mutation_receipts_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(workflow_apply_phase_payload_py, module)?)?;
    module.add_function(wrap_pyfunction!(workflow_apply_phase_summary_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        workflow_mutation_receipt_summary_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        workflow_applied_action_summary_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(workflow_ready_payload_direct_py, module)?)?;
    module.add_function(wrap_pyfunction!(workflow_land_payload_direct_py, module)?)?;
    module.add_function(wrap_pyfunction!(workflow_ready_apply_direct_py, module)?)?;
    module.add_function(wrap_pyfunction!(workflow_land_apply_direct_py, module)?)?;
    Ok(())
}
