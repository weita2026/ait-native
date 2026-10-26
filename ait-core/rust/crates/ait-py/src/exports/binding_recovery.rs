#[pyfunction(name = "remote_recover_head_operation")]
#[pyo3(signature = (repo_root, *, remote_name=None, jobs=8, apply=false))]
fn remote_recover_head_operation_py(
    py: Python<'_>,
    repo_root: &str,
    remote_name: Option<&str>,
    jobs: usize,
    apply: bool,
) -> PyResult<Py<PyAny>> {
    if !(1..=64).contains(&jobs) {
        return Err(PyValueError::new_err(
            "jobs must be an integer from 1 through 64",
        ));
    }
    let request = ait_cli::remote_head_recovery::RemoteHeadRecoveryRequest {
        remote_name: remote_name.map(str::to_string),
        jobs,
        apply,
    };
    let payload = py
        .detach(|| ait_cli::binding_services::recover_remote_head(Path::new(repo_root), &request))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}
#[pyfunction(name = "format_remote_recover_head")]
fn format_remote_recover_head_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "remote head recovery result")?;
    ait_cli::binding_output::format_remote_recover_head(&payload).map_err(PyValueError::new_err)
}
fn register_binding_recovery(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(remote_recover_head_operation_py, module)?)?;
    module.add_function(wrap_pyfunction!(format_remote_recover_head_py, module)?)?;
    Ok(())
}
