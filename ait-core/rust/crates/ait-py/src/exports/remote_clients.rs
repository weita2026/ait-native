const DEFAULT_REQUEST_TIMEOUT_SECONDS: f64 = 30.0;
const LONG_RUNNING_REQUEST_TIMEOUT_SECONDS: f64 = 1_800.0;

#[pyclass(name = "PlanRemoteTransportConfig")]
struct PlanRemoteTransportConfigPy {
    request_timeout_seconds: f64,
    long_running_request_timeout_seconds: f64,
}

#[pymethods]
impl PlanRemoteTransportConfigPy {
    #[new]
    #[pyo3(signature = (request_timeout_seconds=None, long_running_request_timeout_seconds=None))]
    fn new(
        request_timeout_seconds: Option<f64>,
        long_running_request_timeout_seconds: Option<f64>,
    ) -> PyResult<Self> {
        Ok(Self {
            request_timeout_seconds: parse_remote_timeout("request", request_timeout_seconds)?,
            long_running_request_timeout_seconds: parse_remote_timeout(
                "long_running_request",
                long_running_request_timeout_seconds,
            )?,
        })
    }

    fn request_timeout_seconds(&self, long_running: bool) -> f64 {
        if long_running {
            self.long_running_request_timeout_seconds
        } else {
            self.request_timeout_seconds
        }
    }
}

#[pyclass(name = "PlanHttpClientManager")]
struct PlanHttpClientManagerPy {
    manager: PlanHttpClientManager,
}

#[pymethods]
impl PlanHttpClientManagerPy {
    #[new]
    #[pyo3(signature = (base_url, *, headers_json=None, timeout_ms=None, retry_attempts=0, retry_backoff_ms=0, pool_max_idle_per_host=1))]
    fn new(
        base_url: &str,
        headers_json: Option<&str>,
        timeout_ms: Option<u64>,
        retry_attempts: usize,
        retry_backoff_ms: u64,
        pool_max_idle_per_host: usize,
    ) -> PyResult<Self> {
        let manager = PlanHttpClientManager::new(build_plan_http_client_config(
            base_url,
            headers_json,
            timeout_ms,
            retry_attempts,
            retry_backoff_ms,
            pool_max_idle_per_host,
        )?)
        .map_err(plan_http_py_error)?;
        Ok(Self { manager })
    }

    fn inspect(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        render_plan_http_client_stats(py, self.manager.inspect())
    }

    fn close(&mut self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        render_plan_http_client_stats(py, self.manager.close())
    }

    #[pyo3(signature = (repo_name, *, artifact_path=None))]
    fn list_plans(
        &mut self,
        py: Python<'_>,
        repo_name: &str,
        artifact_path: Option<&str>,
    ) -> PyResult<Py<PyList>> {
        let payload = py
            .detach(|| self.manager.list_plans(repo_name, artifact_path))
            .map_err(plan_http_py_error)?;
        render_json_list(py, payload)
    }

    fn get_plan(&mut self, py: Python<'_>, plan_id: &str) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| self.manager.get_plan(plan_id))
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    fn list_plan_revisions(&mut self, py: Python<'_>, plan_id: &str) -> PyResult<Py<PyList>> {
        let payload = py
            .detach(|| self.manager.list_plan_revisions(plan_id))
            .map_err(plan_http_py_error)?;
        render_json_list(py, payload)
    }

    fn get_plan_revision(
        &mut self,
        py: Python<'_>,
        plan_id: &str,
        plan_revision_id: &str,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| self.manager.get_plan_revision(plan_id, plan_revision_id))
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (
        repo_name,
        title,
        artifact_path,
        artifact_selector,
        artifact_heading,
        items_json,
        *,
        summary=None,
        status="draft",
        plan_id=None,
        source_kind="manual_edit",
        artifact_body=None
    ))]
    fn create_plan(
        &mut self,
        py: Python<'_>,
        repo_name: &str,
        title: &str,
        artifact_path: &str,
        artifact_selector: Option<&str>,
        artifact_heading: &str,
        items_json: &str,
        summary: Option<&str>,
        status: &str,
        plan_id: Option<&str>,
        source_kind: &str,
        artifact_body: Option<&str>,
    ) -> PyResult<Py<PyDict>> {
        let items = parse_json_text_array(items_json, "items_json")?;
        let payload = py
            .detach(|| {
                self.manager.create_plan(
                    repo_name,
                    title,
                    artifact_path,
                    artifact_selector,
                    artifact_heading,
                    &items,
                    summary,
                    status,
                    plan_id,
                    source_kind,
                    artifact_body,
                )
            })
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (
        plan_id,
        artifact_path,
        artifact_selector,
        artifact_heading,
        items_json,
        *,
        title=None,
        summary=None,
        source_kind="manual_edit",
        artifact_body=None,
        expected_head_revision_id=None
    ))]
    fn revise_plan(
        &mut self,
        py: Python<'_>,
        plan_id: &str,
        artifact_path: &str,
        artifact_selector: Option<&str>,
        artifact_heading: &str,
        items_json: &str,
        title: Option<&str>,
        summary: Option<&str>,
        source_kind: &str,
        artifact_body: Option<&str>,
        expected_head_revision_id: Option<&str>,
    ) -> PyResult<Py<PyDict>> {
        let items = parse_json_text_array(items_json, "items_json")?;
        let payload = py
            .detach(|| {
                self.manager.revise_plan(
                    plan_id,
                    artifact_path,
                    artifact_selector,
                    artifact_heading,
                    &items,
                    title,
                    summary,
                    source_kind,
                    artifact_body,
                    expected_head_revision_id,
                )
            })
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    fn update_plan_status(
        &mut self,
        py: Python<'_>,
        plan_id: &str,
        status: &str,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| self.manager.update_plan_status(plan_id, status))
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    fn put_plan_revision_artifacts(
        &mut self,
        py: Python<'_>,
        plan_id: &str,
        plan_revision_id: &str,
        artifacts_json: &str,
    ) -> PyResult<Py<PyDict>> {
        let artifacts = parse_json_text_array(artifacts_json, "artifacts_json")?;
        let payload = py
            .detach(|| {
                self.manager
                    .put_plan_revision_artifacts(plan_id, plan_revision_id, &artifacts)
            })
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[pyo3(signature = (plan_id, *, title=None, mode="connected_local", preferred_agent=None, resume_if_active=true, planning_session_id=None))]
    #[expect(
        clippy::too_many_arguments,
        reason = "Python parameters are a compatibility contract"
    )]
    fn create_planning_session(
        &mut self,
        py: Python<'_>,
        plan_id: &str,
        title: Option<&str>,
        mode: &str,
        preferred_agent: Option<&str>,
        resume_if_active: bool,
        planning_session_id: Option<&str>,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| {
                self.manager.create_planning_session(
                    plan_id,
                    title,
                    mode,
                    preferred_agent,
                    resume_if_active,
                    planning_session_id,
                )
            })
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[pyo3(signature = (plan_id, *, status=None))]
    fn list_planning_sessions(
        &mut self,
        py: Python<'_>,
        plan_id: &str,
        status: Option<&str>,
    ) -> PyResult<Py<PyList>> {
        let payload = py
            .detach(|| self.manager.list_planning_sessions(plan_id, status))
            .map_err(plan_http_py_error)?;
        render_json_list(py, payload)
    }

    fn get_planning_session(
        &mut self,
        py: Python<'_>,
        planning_session_id: &str,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| self.manager.get_planning_session(planning_session_id))
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[pyo3(signature = (planning_session_id, event_type, payload_json=None))]
    fn append_planning_session_event(
        &mut self,
        py: Python<'_>,
        planning_session_id: &str,
        event_type: &str,
        payload_json: Option<&str>,
    ) -> PyResult<Py<PyDict>> {
        let payload_json = payload_json.unwrap_or("{}");
        let payload = parse_json_text_object(payload_json, "payload_json")?;
        let payload = JsonValue::Object(payload);
        let result = py
            .detach(|| {
                self.manager.append_planning_session_event(
                    planning_session_id,
                    event_type,
                    &payload,
                )
            })
            .map_err(plan_http_py_error)?;
        render_json_dict(py, result)
    }

    #[pyo3(signature = (planning_session_id, *, after_sequence=0, limit=200))]
    fn list_planning_session_events(
        &mut self,
        py: Python<'_>,
        planning_session_id: &str,
        after_sequence: i64,
        limit: i64,
    ) -> PyResult<Py<PyList>> {
        let payload = py
            .detach(|| {
                self.manager.list_planning_session_events(
                    planning_session_id,
                    after_sequence,
                    limit,
                )
            })
            .map_err(plan_http_py_error)?;
        render_json_list(py, payload)
    }

    #[pyo3(signature = (planning_session_id, *, surface="cli", title=None, model_name=None, resume_if_active=true))]
    fn join_planning_session(
        &mut self,
        py: Python<'_>,
        planning_session_id: &str,
        surface: &str,
        title: Option<&str>,
        model_name: Option<&str>,
        resume_if_active: bool,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| {
                self.manager.join_planning_session(
                    planning_session_id,
                    surface,
                    title,
                    model_name,
                    resume_if_active,
                )
            })
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (planning_session_id, artifact_path, artifact_selector, artifact_heading, items_json, *, title=None, summary=None, artifact_body=None))]
    fn promote_planning_session(
        &mut self,
        py: Python<'_>,
        planning_session_id: &str,
        artifact_path: &str,
        artifact_selector: &str,
        artifact_heading: &str,
        items_json: &str,
        title: Option<&str>,
        summary: Option<&str>,
        artifact_body: Option<&str>,
    ) -> PyResult<Py<PyDict>> {
        let items = parse_json_text_array(items_json, "items_json")?;
        let payload = py
            .detach(|| {
                self.manager.promote_planning_session(
                    planning_session_id,
                    artifact_path,
                    artifact_selector,
                    artifact_heading,
                    &items,
                    title,
                    summary,
                    artifact_body,
                )
            })
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[pyo3(signature = (planning_session_id, *, status="closed"))]
    fn close_planning_session(
        &mut self,
        py: Python<'_>,
        planning_session_id: &str,
        status: &str,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| {
                self.manager
                    .close_planning_session(planning_session_id, status)
            })
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }
}

#[pyclass(name = "TaskWorkflowHttpClientManager")]
struct TaskWorkflowHttpClientManagerPy {
    manager: TaskWorkflowHttpClientManager,
}

#[pymethods]
impl TaskWorkflowHttpClientManagerPy {
    #[new]
    #[pyo3(signature = (base_url, *, headers_json=None, timeout_ms=None, retry_attempts=0, retry_backoff_ms=0, pool_max_idle_per_host=1))]
    fn new(
        base_url: &str,
        headers_json: Option<&str>,
        timeout_ms: Option<u64>,
        retry_attempts: usize,
        retry_backoff_ms: u64,
        pool_max_idle_per_host: usize,
    ) -> PyResult<Self> {
        let manager = TaskWorkflowHttpClientManager::new(build_plan_http_client_config(
            base_url,
            headers_json,
            timeout_ms,
            retry_attempts,
            retry_backoff_ms,
            pool_max_idle_per_host,
        )?)
        .map_err(plan_http_py_error)?;
        Ok(Self { manager })
    }

    fn inspect(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        render_plan_http_client_stats(py, self.manager.inspect())
    }

    fn close(&mut self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        render_plan_http_client_stats(py, self.manager.close())
    }
}

#[pyclass(name = "HttpTaskRemote")]
struct HttpTaskRemotePy {
    adapter: HttpTaskRemote,
}

#[pymethods]
impl HttpTaskRemotePy {
    #[new]
    #[pyo3(signature = (base_url, *, headers_json=None, timeout_ms=None, retry_attempts=0, retry_backoff_ms=0, pool_max_idle_per_host=1))]
    fn new(
        base_url: &str,
        headers_json: Option<&str>,
        timeout_ms: Option<u64>,
        retry_attempts: usize,
        retry_backoff_ms: u64,
        pool_max_idle_per_host: usize,
    ) -> PyResult<Self> {
        let adapter = HttpTaskRemote::new(build_plan_http_client_config(
            base_url,
            headers_json,
            timeout_ms,
            retry_attempts,
            retry_backoff_ms,
            pool_max_idle_per_host,
        )?)
        .map_err(plan_http_py_error)?;
        Ok(Self { adapter })
    }

    fn inspect_client(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        render_plan_http_client_stats(py, self.adapter.inspect_client())
    }

    fn close_client(&mut self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        render_plan_http_client_stats(py, self.adapter.close_client())
    }

    fn change_lineage_payload(
        &self,
        py: Python<'_>,
        base_line: &str,
        line_row: Option<Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyDict>> {
        let line_row = match line_row {
            Some(value) => Some(parse_json_value(value, "line_row")?),
            None => None,
        };
        let payload = self
            .adapter
            .change_lineage_payload(base_line, line_row.as_ref())
            .map_err(PyValueError::new_err)?;
        render_json_dict(py, payload)
    }

    fn get_line(
        &mut self,
        py: Python<'_>,
        repo_name: &str,
        line_name: &str,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| self.adapter.get_line(repo_name, line_name))
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    fn get_task(
        &mut self,
        py: Python<'_>,
        task_id: &str,
        repo_name: Option<&str>,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| self.adapter.get_task(task_id, repo_name))
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    fn list_tasks(&mut self, py: Python<'_>, repo_name: &str) -> PyResult<Py<PyList>> {
        let payload = py
            .detach(|| self.adapter.list_tasks(repo_name))
            .map_err(plan_http_py_error)?;
        render_json_list(py, payload)
    }

    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (repo_name, title, intent, *, task_id=None, plan_id=None, origin_plan_revision_id=None, plan_item_ref=None))]
    fn create_task(
        &mut self,
        py: Python<'_>,
        repo_name: &str,
        title: &str,
        intent: &str,
        task_id: Option<&str>,
        plan_id: Option<&str>,
        origin_plan_revision_id: Option<&str>,
        plan_item_ref: Option<&str>,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| {
                self.adapter.create_task(
                    repo_name,
                    title,
                    intent,
                    task_id,
                    plan_id,
                    origin_plan_revision_id,
                    plan_item_ref,
                )
            })
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    fn list_changes(&mut self, py: Python<'_>, repo_name: &str) -> PyResult<Py<PyList>> {
        let payload = py
            .detach(|| self.adapter.list_changes(repo_name))
            .map_err(plan_http_py_error)?;
        render_json_list(py, payload)
    }

    fn get_change_detail(
        &mut self,
        py: Python<'_>,
        change_ref: &str,
        repo_name: Option<&str>,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| self.adapter.get_change_detail(change_ref, repo_name))
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    fn get_change(
        &mut self,
        py: Python<'_>,
        change_ref: &str,
        repo_name: Option<&str>,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| self.adapter.get_change(change_ref, repo_name))
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[pyo3(signature = (change_ref, status="archived", repo_name=None))]
    fn close_change(
        &mut self,
        py: Python<'_>,
        change_ref: &str,
        status: &str,
        repo_name: Option<&str>,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| self.adapter.close_change(change_ref, status, repo_name))
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (repo_name, task_id, title, base_line, *, change_id=None, fork_snapshot_id=None, forked_from_line=None))]
    fn create_change(
        &mut self,
        py: Python<'_>,
        repo_name: &str,
        task_id: &str,
        title: &str,
        base_line: &str,
        change_id: Option<&str>,
        fork_snapshot_id: Option<&str>,
        forked_from_line: Option<&str>,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| {
                self.adapter.create_change(
                    repo_name,
                    task_id,
                    title,
                    base_line,
                    change_id,
                    fork_snapshot_id,
                    forked_from_line,
                )
            })
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[pyo3(signature = (repo_name, line_name, *, head_snapshot_id=None, expected_head_snapshot_id=None))]
    fn update_remote_line(
        &mut self,
        py: Python<'_>,
        repo_name: &str,
        line_name: &str,
        head_snapshot_id: Option<&str>,
        expected_head_snapshot_id: Option<&str>,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| {
                self.adapter.update_remote_line(
                    repo_name,
                    line_name,
                    head_snapshot_id,
                    expected_head_snapshot_id,
                )
            })
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    fn plan_remote_zstd_bulk(
        &mut self,
        py: Python<'_>,
        repo_name: &str,
        request: Bound<'_, PyAny>,
    ) -> PyResult<Py<PyDict>> {
        let request_value = parse_json_value(request, "request")?;
        let request = ZstdBulkPlanRequestJson::stateless()
            .decode_value(request_value)
            .map_err(PyValueError::new_err)?;
        let response = py
            .detach(|| self.adapter.plan_remote_zstd_bulk(repo_name, &request))
            .map_err(plan_http_py_error)?;
        let payload = ZstdBulkPlanResponseJson::stateless()
            .encode_value(&response)
            .map_err(PyValueError::new_err)?;
        render_json_dict(py, payload)
    }

    fn put_remote_zstd_object_pack(
        &mut self,
        py: Python<'_>,
        repo_name: &str,
        pack_id: &str,
        pack_bytes: Vec<u8>,
    ) -> PyResult<Py<PyDict>> {
        let response = py
            .detach(|| {
                self.adapter
                    .put_remote_zstd_object_pack(repo_name, pack_id, &pack_bytes)
            })
            .map_err(plan_http_py_error)?;
        let payload = ZstdPackUploadResponseJson::stateless()
            .encode_value(&response)
            .map_err(PyValueError::new_err)?;
        render_json_dict(py, payload)
    }

    fn put_remote_zstd_tree_pack(
        &mut self,
        py: Python<'_>,
        repo_name: &str,
        pack_id: &str,
        pack_bytes: Vec<u8>,
    ) -> PyResult<Py<PyDict>> {
        let response = py
            .detach(|| {
                self.adapter
                    .put_remote_zstd_tree_pack(repo_name, pack_id, &pack_bytes)
            })
            .map_err(plan_http_py_error)?;
        let payload = ZstdPackUploadResponseJson::stateless()
            .encode_value(&response)
            .map_err(PyValueError::new_err)?;
        render_json_dict(py, payload)
    }

    fn commit_remote_zstd_bulk(
        &mut self,
        py: Python<'_>,
        repo_name: &str,
        request: Bound<'_, PyAny>,
    ) -> PyResult<Py<PyDict>> {
        let request_value = parse_json_value(request, "request")?;
        let request = ZstdBulkCommitRequestJson::stateless()
            .decode_value(request_value)
            .map_err(PyValueError::new_err)?;
        let response = py
            .detach(|| self.adapter.commit_remote_zstd_bulk(repo_name, &request))
            .map_err(plan_http_py_error)?;
        let payload = ZstdBulkCommitResponseJson::stateless()
            .encode_value(&response)
            .map_err(PyValueError::new_err)?;
        render_json_dict(py, payload)
    }

    fn get_remote_zstd_import_manifest(
        &mut self,
        py: Python<'_>,
        repo_name: &str,
        snapshot_id: &str,
    ) -> PyResult<Py<PyDict>> {
        let response = py
            .detach(|| {
                self.adapter
                    .get_remote_zstd_import_manifest(repo_name, snapshot_id)
            })
            .map_err(plan_http_py_error)?;
        let payload = ZstdImportManifestJson::stateless()
            .encode_value(&response)
            .map_err(PyValueError::new_err)?;
        render_json_dict(py, payload)
    }

    fn get_remote_zstd_object_pack(
        &mut self,
        py: Python<'_>,
        repo_name: &str,
        pack_id: &str,
    ) -> PyResult<Py<PyBytes>> {
        let payload = py
            .detach(|| self.adapter.get_remote_zstd_object_pack(repo_name, pack_id))
            .map_err(plan_http_py_error)?;
        Ok(PyBytes::new(py, &payload).unbind())
    }

    fn get_remote_zstd_tree_pack(
        &mut self,
        py: Python<'_>,
        repo_name: &str,
        pack_id: &str,
    ) -> PyResult<Py<PyBytes>> {
        let payload = py
            .detach(|| self.adapter.get_remote_zstd_tree_pack(repo_name, pack_id))
            .map_err(plan_http_py_error)?;
        Ok(PyBytes::new(py, &payload).unbind())
    }

    #[pyo3(signature = (repo_name, snapshot_id, *, include_content=false, path=None))]
    fn get_remote_snapshot(
        &mut self,
        py: Python<'_>,
        repo_name: &str,
        snapshot_id: &str,
        include_content: bool,
        path: Option<&str>,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| {
                self.adapter
                    .get_remote_snapshot(repo_name, snapshot_id, include_content, path)
            })
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    fn get_remote_snapshots_existence(
        &mut self,
        py: Python<'_>,
        repo_name: &str,
        snapshot_ids: Vec<String>,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| {
                self.adapter
                    .get_remote_snapshots_existence(repo_name, &snapshot_ids)
            })
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }
}

#[pyclass(name = "HttpWorkflowCloseoutRemote")]
struct HttpWorkflowCloseoutRemotePy {
    adapter: HttpWorkflowCloseoutRemote,
}

#[pymethods]
impl HttpWorkflowCloseoutRemotePy {
    #[new]
    #[pyo3(signature = (base_url, *, headers_json=None, timeout_ms=None, retry_attempts=0, retry_backoff_ms=0, pool_max_idle_per_host=1))]
    fn new(
        base_url: &str,
        headers_json: Option<&str>,
        timeout_ms: Option<u64>,
        retry_attempts: usize,
        retry_backoff_ms: u64,
        pool_max_idle_per_host: usize,
    ) -> PyResult<Self> {
        let adapter = HttpWorkflowCloseoutRemote::new(build_plan_http_client_config(
            base_url,
            headers_json,
            timeout_ms,
            retry_attempts,
            retry_backoff_ms,
            pool_max_idle_per_host,
        )?)
        .map_err(plan_http_py_error)?;
        Ok(Self { adapter })
    }

    fn inspect_client(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        render_plan_http_client_stats(py, self.adapter.inspect_client())
    }

    fn close_client(&mut self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        render_plan_http_client_stats(py, self.adapter.close_client())
    }

    #[pyo3(signature = (*, action, source_action, delivery, response_recovery=None, result=None))]
    fn mutation_receipt(
        &self,
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
        let payload = self
            .adapter
            .mutation_receipt(
                action,
                source_action,
                delivery,
                response_recovery.as_ref(),
                result.as_ref(),
            )
            .map_err(PyValueError::new_err)?;
        render_json_dict(py, payload)
    }

    #[pyo3(signature = (*, code, result))]
    fn action_mutation_receipts(
        &self,
        py: Python<'_>,
        code: &str,
        result: Bound<'_, PyAny>,
    ) -> PyResult<Py<PyList>> {
        let result = parse_json_value(result, "result")?;
        let payload = self
            .adapter
            .action_mutation_receipts(code, &result)
            .map_err(PyValueError::new_err)?;
        match payload {
            JsonValue::Array(values) => render_json_list(py, values),
            _ => Err(PyRuntimeError::new_err(
                "Rust task/workflow HTTP closeout remote returned a non-list mutation receipt payload.",
            )),
        }
    }

    fn list_patchsets(
        &mut self,
        py: Python<'_>,
        change_id: &str,
        repo_name: Option<&str>,
    ) -> PyResult<Py<PyList>> {
        let payload = py
            .detach(|| self.adapter.list_patchsets(change_id, repo_name))
            .map_err(plan_http_py_error)?;
        render_json_list(py, payload)
    }

    fn get_patchset(
        &mut self,
        py: Python<'_>,
        patchset_id: &str,
        repo_name: Option<&str>,
        change_ref: Option<&str>,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| {
                self.adapter
                    .get_patchset(patchset_id, repo_name, change_ref)
            })
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (change_id, base_snapshot_id, revision_snapshot_id, summary, author_mode, repo_name=None, exact_id=false))]
    fn publish_patchset(
        &mut self,
        py: Python<'_>,
        change_id: &str,
        base_snapshot_id: &str,
        revision_snapshot_id: &str,
        summary: &str,
        author_mode: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| {
                self.adapter.publish_patchset(
                    change_id,
                    base_snapshot_id,
                    revision_snapshot_id,
                    summary,
                    author_mode,
                    repo_name,
                    exact_id,
                )
            })
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[pyo3(signature = (change_id, patchset_id, repo_name=None, exact_id=false))]
    fn select_patchset(
        &mut self,
        py: Python<'_>,
        change_id: &str,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| {
                self.adapter
                    .select_patchset(change_id, patchset_id, repo_name, exact_id)
            })
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[pyo3(signature = (patchset_id, *, trigger="manual_rerun", execution_profile=None, repo_name=None, exact_id=false))]
    fn run_patchset_ci(
        &mut self,
        py: Python<'_>,
        patchset_id: &str,
        trigger: &str,
        execution_profile: Option<&str>,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| {
                self.adapter.run_patchset_ci(
                    patchset_id,
                    trigger,
                    execution_profile,
                    repo_name,
                    exact_id,
                )
            })
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[pyo3(signature = (patchset_id, *, recent_limit=10, repo_name=None, exact_id=false))]
    fn read_patchset_ci_status(
        &mut self,
        py: Python<'_>,
        patchset_id: &str,
        recent_limit: i64,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| {
                self.adapter
                    .read_patchset_ci_status(patchset_id, recent_limit, repo_name, exact_id)
            })
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[pyo3(signature = (change_id, patchset_id, reviewer_groups, note=None, repo_name=None, exact_id=false))]
    #[expect(
        clippy::too_many_arguments,
        reason = "Python parameters are a compatibility contract"
    )]
    fn request_review(
        &mut self,
        py: Python<'_>,
        change_id: &str,
        patchset_id: &str,
        reviewer_groups: Vec<String>,
        note: Option<&str>,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| {
                self.adapter.request_review(
                    change_id,
                    patchset_id,
                    &reviewer_groups,
                    note,
                    repo_name,
                    exact_id,
                )
            })
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[pyo3(signature = (change_id, repo_name=None, exact_id=false))]
    fn list_reviews(
        &mut self,
        py: Python<'_>,
        change_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| self.adapter.list_reviews(change_id, repo_name, exact_id))
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (change_id, patchset_id, reviewer, action, comment=None, blocking=false, repo_name=None, exact_id=false))]
    fn record_review(
        &mut self,
        py: Python<'_>,
        change_id: &str,
        patchset_id: &str,
        reviewer: &str,
        action: &str,
        comment: Option<&str>,
        blocking: bool,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| {
                self.adapter.record_review(
                    change_id,
                    patchset_id,
                    reviewer,
                    action,
                    comment,
                    blocking,
                    repo_name,
                    exact_id,
                )
            })
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (patchset_id, author_mode, evaluation_summary, provenance_summary, detail, repo_name=None, exact_id=false))]
    fn put_attestation(
        &mut self,
        py: Python<'_>,
        patchset_id: &str,
        author_mode: &str,
        evaluation_summary: Bound<'_, PyAny>,
        provenance_summary: Bound<'_, PyAny>,
        detail: Bound<'_, PyAny>,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> PyResult<Py<PyDict>> {
        let evaluation_summary = parse_json_value(evaluation_summary, "evaluation_summary")?;
        let provenance_summary = parse_json_value(provenance_summary, "provenance_summary")?;
        let detail = parse_json_value(detail, "detail")?;
        let payload = py
            .detach(|| {
                self.adapter.put_attestation(
                    patchset_id,
                    author_mode,
                    &evaluation_summary,
                    &provenance_summary,
                    &detail,
                    repo_name,
                    exact_id,
                )
            })
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[pyo3(signature = (patchset_id, repo_name=None, exact_id=false))]
    fn get_attestation(
        &mut self,
        py: Python<'_>,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| {
                self.adapter
                    .get_attestation(patchset_id, repo_name, exact_id)
            })
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[pyo3(signature = (patchset_id, repo_name=None, exact_id=false))]
    fn evaluate_policy(
        &mut self,
        py: Python<'_>,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| {
                self.adapter
                    .evaluate_policy(patchset_id, repo_name, exact_id)
            })
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[pyo3(signature = (patchset_id, repo_name=None, exact_id=false))]
    fn get_policy(
        &mut self,
        py: Python<'_>,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| self.adapter.get_policy(patchset_id, repo_name, exact_id))
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[pyo3(signature = (patchset_id, rule_name, reason, *, expires_at=None, repo_name=None, exact_id=false))]
    #[expect(
        clippy::too_many_arguments,
        reason = "Python parameters are a compatibility contract"
    )]
    fn create_waiver(
        &mut self,
        py: Python<'_>,
        patchset_id: &str,
        rule_name: &str,
        reason: &str,
        expires_at: Option<&str>,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| {
                self.adapter.create_waiver(
                    patchset_id,
                    rule_name,
                    reason,
                    expires_at,
                    repo_name,
                    exact_id,
                )
            })
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[pyo3(signature = (change_id, patchset_id=None, target_line="main", mode="direct", repo_name=None))]
    fn submit_land(
        &mut self,
        py: Python<'_>,
        change_id: &str,
        patchset_id: Option<&str>,
        target_line: &str,
        mode: &str,
        repo_name: Option<&str>,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| {
                self.adapter
                    .submit_land(change_id, patchset_id, target_line, mode, repo_name)
            })
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[pyo3(signature = (submission_id, *, repo_name=None))]
    fn get_land(
        &mut self,
        py: Python<'_>,
        submission_id: &str,
        repo_name: Option<&str>,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| self.adapter.get_land(submission_id, repo_name))
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[pyo3(signature = (submission_id, *, reason=None, repo_name=None))]
    fn retry_land(
        &mut self,
        py: Python<'_>,
        submission_id: &str,
        reason: Option<&str>,
        repo_name: Option<&str>,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| self.adapter.retry_land(submission_id, reason, repo_name))
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[pyo3(signature = (task_id, *, status="completed", repo_name=None))]
    fn close_task(
        &mut self,
        py: Python<'_>,
        task_id: &str,
        status: &str,
        repo_name: Option<&str>,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| self.adapter.close_task(task_id, status, repo_name))
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }

    #[pyo3(signature = (task_id, *, repo_name=None))]
    fn restart_task(
        &mut self,
        py: Python<'_>,
        task_id: &str,
        repo_name: Option<&str>,
    ) -> PyResult<Py<PyDict>> {
        let payload = py
            .detach(|| self.adapter.restart_task(task_id, repo_name))
            .map_err(plan_http_py_error)?;
        render_json_dict(py, payload)
    }
}

#[pyfunction(name = "normalize_remote_text")]
fn normalize_remote_text_py(value: Option<&str>) -> Option<String> {
    normalize_remote_text(value)
}

fn register_remote_clients(_py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(normalize_remote_text_py, module)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    const REMOTE_CLIENTS_RS: &str = include_str!("remote_clients.rs");

    #[test]
    fn ait_py_zstd_bulk_bridge_matches_rust_http_remote_contract() {
        let implementation = REMOTE_CLIENTS_RS
            .split("#[cfg(test)]")
            .next()
            .expect("remote_clients.rs must contain implementation before test module");

        for method in [
            "fn plan_remote_zstd_bulk(",
            "fn put_remote_zstd_object_pack(",
            "fn put_remote_zstd_tree_pack(",
            "fn commit_remote_zstd_bulk(",
            "fn get_remote_zstd_import_manifest(",
            "fn get_remote_zstd_object_pack(",
            "fn get_remote_zstd_tree_pack(",
        ] {
            assert!(
                implementation.contains(method),
                "HttpTaskRemotePy is missing zstd bridge method {method}"
            );
        }

        for wrapper in [
            "ZstdBulkPlanRequestJson::stateless()",
            "ZstdBulkPlanResponseJson::stateless()",
            "ZstdBulkCommitRequestJson::stateless()",
            "ZstdBulkCommitResponseJson::stateless()",
            "ZstdImportManifestJson::stateless()",
            "ZstdPackUploadResponseJson::stateless()",
        ] {
            assert!(
                implementation.contains(wrapper),
                "ait-py zstd bridge must use typed JSON wrapper {wrapper}"
            );
        }
    }
}
