#[pyfunction(name = "release_adapter_check_operation")]
#[pyo3(signature = (repo_root, version, *, line_name="main", target=None))]
#[allow(clippy::too_many_arguments)]
fn release_adapter_check_operation_py(
    py: Python<'_>,
    repo_root: &str,
    version: &str,
    line_name: &str,
    target: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;

    let payload = py
        .detach(|| {
            ait_cli::binding_release::check_release_adapter(&repo, version, line_name, target)
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "release_adapter_build_operation")]
#[pyo3(signature = (repo_root, version, *, line_name="main", target=None))]
#[allow(clippy::too_many_arguments)]
fn release_adapter_build_operation_py(
    py: Python<'_>,
    repo_root: &str,
    version: &str,
    line_name: &str,
    target: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;

    let payload = py
        .detach(|| {
            ait_cli::binding_release::build_release_adapter(&repo, version, line_name, target)
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "release_candidate_create_operation")]
#[pyo3(signature = (repo_root, version, *, line_name="main", profile=None, channel=None, public_source_root=None))]
#[allow(clippy::too_many_arguments)]
fn release_candidate_create_operation_py(
    py: Python<'_>,
    repo_root: &str,
    version: &str,
    line_name: &str,
    profile: Option<&str>,
    channel: Option<&str>,
    public_source_root: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let request = ait_cli::binding_release::ReleaseCandidateRequest {
        version: version.to_string(),
        line_name: line_name.to_string(),
        profile: profile.map(str::to_string),
        channel: channel.map(str::to_string),
        public_source_root: public_source_root.map(PathBuf::from),
    };
    let payload = py
        .detach(|| ait_cli::binding_release::create_release_candidate(&repo, &request))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "release_check_operation")]
#[pyo3(signature = (repo_root, release_id, *, receipts=None, public_source_root=None, tests_command=None, skip_tests_reason=None))]
#[allow(clippy::too_many_arguments)]
fn release_check_operation_py(
    py: Python<'_>,
    repo_root: &str,
    release_id: &str,
    receipts: Option<&str>,
    public_source_root: Option<&str>,
    tests_command: Option<&str>,
    skip_tests_reason: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let request = ait_cli::binding_release::ReleaseCheckRequest {
        release_id: release_id.to_string(),
        receipts: receipts.map(PathBuf::from),
        public_source_root: public_source_root.map(PathBuf::from),
        tests_command: tests_command.map(str::to_string),
        skip_tests_reason: skip_tests_reason.map(str::to_string),
    };
    let payload = py
        .detach(|| ait_cli::binding_release::check_release(&repo, &request))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "release_build_operation")]
#[pyo3(signature = (repo_root, release_id, *, receipts=None, public_source_root=None, native_matrix_dir=None, native_command_dir=None))]
#[allow(clippy::too_many_arguments)]
fn release_build_operation_py(
    py: Python<'_>,
    repo_root: &str,
    release_id: &str,
    receipts: Option<&str>,
    public_source_root: Option<&str>,
    native_matrix_dir: Option<&str>,
    native_command_dir: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let request = ait_cli::binding_release::ReleaseBuildRequest {
        release_id: release_id.to_string(),
        receipts: receipts.map(PathBuf::from),
        public_source_root: public_source_root.map(PathBuf::from),
        native_matrix_dir: native_matrix_dir.map(PathBuf::from),
        native_command_dir: native_command_dir.map(PathBuf::from),
    };
    let payload = py
        .detach(|| ait_cli::binding_release::build_release(&repo, &request))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "release_native_source_operation")]
#[pyo3(signature = (repo_root, release_id, *, target, source_dir, runner, runner_image, rust_toolchain, rustc_path))]
#[allow(clippy::too_many_arguments)]
fn release_native_source_operation_py(
    py: Python<'_>,
    repo_root: &str,
    release_id: &str,
    target: &str,
    source_dir: &str,
    runner: &str,
    runner_image: &str,
    rust_toolchain: &str,
    rustc_path: &str,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let request = ait_cli::release_surface::NativeSourceRequest {
        release_id: release_id.to_string(),
        target: target.to_string(),
        source_dir: PathBuf::from(source_dir),
        runner: runner.to_string(),
        runner_image: runner_image.to_string(),
        rust_toolchain: rust_toolchain.to_string(),
        rustc_path: PathBuf::from(rustc_path),
    };
    let payload = py
        .detach(|| ait_cli::binding_release::capture_release_native_source(&repo, &request))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "release_native_bundle_operation")]
#[pyo3(signature = (repo_root, release_id, native_matrix_dir))]
#[allow(clippy::too_many_arguments)]
fn release_native_bundle_operation_py(
    py: Python<'_>,
    repo_root: &str,
    release_id: &str,
    native_matrix_dir: &str,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;

    let payload = py
        .detach(|| {
            ait_cli::binding_release::bundle_release_native(
                &repo,
                release_id,
                Path::new(native_matrix_dir),
            )
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "release_package_operation")]
#[pyo3(signature = (repo_root, release_id, channel, *, public_source_root=None))]
#[allow(clippy::too_many_arguments)]
fn release_package_operation_py(
    py: Python<'_>,
    repo_root: &str,
    release_id: &str,
    channel: &str,
    public_source_root: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;

    let payload = py
        .detach(|| {
            ait_cli::binding_release::package_release(
                &repo,
                release_id,
                channel,
                public_source_root.map(Path::new),
            )
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "release_promote_operation")]
#[pyo3(signature = (repo_root, release_id, channel, *, public_source_root=None))]
#[allow(clippy::too_many_arguments)]
fn release_promote_operation_py(
    py: Python<'_>,
    repo_root: &str,
    release_id: &str,
    channel: &str,
    public_source_root: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;

    let payload = py
        .detach(|| {
            ait_cli::binding_release::promote_release(
                &repo,
                release_id,
                channel,
                public_source_root.map(Path::new),
            )
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "release_formula_operation")]
#[pyo3(signature = (repo_root, release_id, name, *, python_formula=None))]
#[allow(clippy::too_many_arguments)]
fn release_formula_operation_py(
    py: Python<'_>,
    repo_root: &str,
    release_id: &str,
    name: &str,
    python_formula: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;

    let payload = py
        .detach(|| {
            ait_cli::binding_release::generate_release_formula(
                &repo,
                release_id,
                name,
                python_formula.unwrap_or(ait_cli::release_surface::DEFAULT_HOMEBREW_PYTHON_FORMULA),
            )
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "release_show_operation")]
#[pyo3(signature = (repo_root, release_id, *, remote_name=None, public_source_root=None))]
#[allow(clippy::too_many_arguments)]
fn release_show_operation_py(
    py: Python<'_>,
    repo_root: &str,
    release_id: &str,
    remote_name: Option<&str>,
    public_source_root: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;
    let request = ait_cli::binding_release::ReleaseShowRequest {
        release_id: release_id.to_string(),
        remote: remote_name.map(str::to_string),
        public_source_root: public_source_root.map(PathBuf::from),
    };
    let payload = py
        .detach(|| ait_cli::binding_release::show_release(&repo, &request))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "release_publish_operation")]
#[pyo3(signature = (repo_root, release_id, *, remote_name=None))]
#[allow(clippy::too_many_arguments)]
fn release_publish_operation_py(
    py: Python<'_>,
    repo_root: &str,
    release_id: &str,
    remote_name: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let repo = discover_task_workflow_repo(repo_root)?;

    let payload = py
        .detach(|| ait_cli::binding_release::publish_release(&repo, release_id, remote_name))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "format_release")]
fn format_release_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "release payload")?;
    Ok(ait_cli::release_surface::format_release_text(&payload))
}

fn register_binding_release(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(
        release_adapter_check_operation_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        release_adapter_build_operation_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        release_candidate_create_operation_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(release_check_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(release_build_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(
        release_native_source_operation_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        release_native_bundle_operation_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(release_package_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(release_promote_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(release_formula_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(release_show_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(release_publish_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_release_py, module)?)?;
    Ok(())
}
