fn binding_source_cache_request(
    namespace_root: &str,
    core_repo_root: &str,
    ext_suffix: &str,
    rustflags: &str,
    worker_id: &str,
    core_source_fingerprint: Option<&str>,
    server_source_fingerprint: Option<&str>,
) -> CurrentSourceNativeCacheRequest {
    CurrentSourceNativeCacheRequest {
        namespace_root: PathBuf::from(namespace_root),
        core_repo_root: PathBuf::from(core_repo_root),
        ext_suffix: ext_suffix.to_string(),
        rustflags: rustflags.to_string(),
        worker_id: worker_id.to_string(),
        core_source_fingerprint: core_source_fingerprint.map(str::to_string),
        server_source_fingerprint: server_source_fingerprint.map(str::to_string),
    }
}

#[pyfunction(name = "current_source_cache_mark_building_operation")]
#[pyo3(signature = (namespace_root, core_repo_root, source_mtime_ns, *, ext_suffix, rustflags="", worker_id="shared", core_source_fingerprint=None, server_source_fingerprint=None, extra_json="{}"))]
#[allow(clippy::too_many_arguments)]
fn current_source_cache_mark_building_operation_py(
    py: Python<'_>,
    namespace_root: &str,
    core_repo_root: &str,
    source_mtime_ns: u64,
    ext_suffix: &str,
    rustflags: &str,
    worker_id: &str,
    core_source_fingerprint: Option<&str>,
    server_source_fingerprint: Option<&str>,
    extra_json: &str,
) -> PyResult<Py<PyAny>> {
    let request = binding_source_cache_request(
        namespace_root,
        core_repo_root,
        ext_suffix,
        rustflags,
        worker_id,
        core_source_fingerprint,
        server_source_fingerprint,
    );
    let payload = py
        .detach(|| {
            ait_cli::binding_source_cache::mark_building(&request, source_mtime_ns, extra_json)
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "current_source_cache_activate_operation")]
#[pyo3(signature = (namespace_root, core_repo_root, source_mtime_ns, *, ext_suffix, rustflags="", worker_id="shared", core_source_fingerprint=None, server_source_fingerprint=None, extra_json="{}", register_lease=false, owner_pid=None))]
#[allow(clippy::too_many_arguments)]
fn current_source_cache_activate_operation_py(
    py: Python<'_>,
    namespace_root: &str,
    core_repo_root: &str,
    source_mtime_ns: u64,
    ext_suffix: &str,
    rustflags: &str,
    worker_id: &str,
    core_source_fingerprint: Option<&str>,
    server_source_fingerprint: Option<&str>,
    extra_json: &str,
    register_lease: bool,
    owner_pid: Option<u32>,
) -> PyResult<Py<PyAny>> {
    let request = binding_source_cache_request(
        namespace_root,
        core_repo_root,
        ext_suffix,
        rustflags,
        worker_id,
        core_source_fingerprint,
        server_source_fingerprint,
    );
    let payload = py
        .detach(|| {
            ait_cli::binding_source_cache::activate(
                &request,
                source_mtime_ns,
                extra_json,
                register_lease,
                owner_pid,
            )
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "current_source_cache_seed_canonical_operation")]
#[pyo3(signature = (namespace_root, core_repo_root, repo_root, canonical_repo_root, core_source_mtime_ns, *, ext_suffix, rustflags="", worker_id="shared", core_source_fingerprint=None, server_source_fingerprint=None))]
#[allow(clippy::too_many_arguments)]
fn current_source_cache_seed_canonical_operation_py(
    py: Python<'_>,
    namespace_root: &str,
    core_repo_root: &str,
    repo_root: &str,
    canonical_repo_root: &str,
    core_source_mtime_ns: u64,
    ext_suffix: &str,
    rustflags: &str,
    worker_id: &str,
    core_source_fingerprint: Option<&str>,
    server_source_fingerprint: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let request = binding_source_cache_request(
        namespace_root,
        core_repo_root,
        ext_suffix,
        rustflags,
        worker_id,
        core_source_fingerprint,
        server_source_fingerprint,
    );
    let payload = py
        .detach(|| {
            ait_cli::binding_source_cache::seed_canonical(
                &request,
                Path::new(repo_root),
                Path::new(canonical_repo_root),
                core_source_mtime_ns,
            )
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "current_source_cache_core_fingerprint_operation")]
fn current_source_cache_core_fingerprint_operation_py(
    py: Python<'_>,
    repo_root: &str,
) -> PyResult<Py<PyAny>> {
    let payload = py
        .detach(|| ait_cli::binding_source_cache::core_fingerprint(Path::new(repo_root)))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "current_source_cache_server_fingerprint_operation")]
fn current_source_cache_server_fingerprint_operation_py(
    py: Python<'_>,
    repo_root: &str,
) -> PyResult<Py<PyAny>> {
    let payload = py
        .detach(|| ait_cli::binding_source_cache::server_fingerprint(Path::new(repo_root)))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "current_source_cache_core_mtime_operation")]
fn current_source_cache_core_mtime_operation_py(
    py: Python<'_>,
    repo_root: &str,
) -> PyResult<Py<PyAny>> {
    let payload = py
        .detach(|| ait_cli::binding_source_cache::core_mtime(Path::new(repo_root)))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "current_source_cache_server_mtime_operation")]
fn current_source_cache_server_mtime_operation_py(
    py: Python<'_>,
    repo_root: &str,
) -> PyResult<Py<PyAny>> {
    let payload = py
        .detach(|| ait_cli::binding_source_cache::server_mtime(Path::new(repo_root)))
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "current_source_cache_release_lease_operation")]
#[pyo3(signature = (lease_path, namespace_root, *, remove_unleased_ready=false))]
fn current_source_cache_release_lease_operation_py(
    py: Python<'_>,
    lease_path: &str,
    namespace_root: &str,
    remove_unleased_ready: bool,
) -> PyResult<Py<PyAny>> {
    let payload = py
        .detach(|| {
            ait_core::current_source_cache::release_current_source_native_cache_lease_json(
                Path::new(lease_path),
                Path::new(namespace_root),
                remove_unleased_ready,
            )
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "current_source_cache_prune_operation")]
#[pyo3(signature = (namespace_root, *, now=None, idle_ttl_seconds=None, build_stale_seconds=None, max_bytes=None, remove_unleased_ready=false))]
fn current_source_cache_prune_operation_py(
    py: Python<'_>,
    namespace_root: &str,
    now: Option<f64>,
    idle_ttl_seconds: Option<u64>,
    build_stale_seconds: Option<u64>,
    max_bytes: Option<u64>,
    remove_unleased_ready: bool,
) -> PyResult<Py<PyAny>> {
    let request = ait_core::current_source_cache::CurrentSourceNativeCachePruneRequest {
        namespace_root: PathBuf::from(namespace_root),
        now,
        idle_ttl_seconds: idle_ttl_seconds
            .unwrap_or(ait_core::current_source_cache::CURRENT_SOURCE_CACHE_IDLE_TTL_SECONDS),
        build_stale_seconds: build_stale_seconds
            .unwrap_or(ait_core::current_source_cache::CURRENT_SOURCE_CACHE_BUILD_STALE_SECONDS),
        max_bytes: max_bytes
            .unwrap_or(ait_core::current_source_cache::CURRENT_SOURCE_CACHE_MAX_BYTES),
        remove_unleased_ready,
    };
    let payload = py
        .detach(|| {
            ait_core::current_source_cache::prune_current_source_native_caches_json(&request)
        })
        .map_err(PyValueError::new_err)?;
    render_json_value(py, payload)
}

#[pyfunction(name = "current_source_cache_extension_fresh_operation")]
fn current_source_cache_extension_fresh_operation_py(
    py: Python<'_>,
    metadata_path: &str,
    extension_path: &str,
    source_mtime_ns: u64,
    source_fingerprint: &str,
) -> PyResult<Py<PyAny>> {
    let request = ait_core::current_source_cache::CurrentSourceExtensionFreshnessRequest {
        metadata_path: PathBuf::from(metadata_path),
        extension_path: PathBuf::from(extension_path),
        source_mtime_ns,
        source_fingerprint: source_fingerprint.to_string(),
    };
    let payload = py.detach(|| {
        ait_core::current_source_cache::current_source_extension_is_fresh_json(&request)
    });
    render_json_value(py, payload)
}

#[pyfunction(name = "current_source_cache_binary_fresh_operation")]
#[allow(clippy::too_many_arguments)]
fn current_source_cache_binary_fresh_operation_py(
    py: Python<'_>,
    metadata_path: &str,
    binary_path: &str,
    metadata_fingerprint_key: &str,
    metadata_source_mtime_key: &str,
    metadata_mtime_key: &str,
    metadata_sha_key: &str,
    source_mtime_ns: u64,
    source_fingerprint: &str,
) -> PyResult<Py<PyAny>> {
    let request = ait_core::current_source_cache::CurrentSourceBinaryFreshnessRequest {
        metadata_path: PathBuf::from(metadata_path),
        binary_path: PathBuf::from(binary_path),
        metadata_fingerprint_key: metadata_fingerprint_key.to_string(),
        metadata_source_mtime_key: metadata_source_mtime_key.to_string(),
        metadata_mtime_key: metadata_mtime_key.to_string(),
        metadata_sha_key: metadata_sha_key.to_string(),
        source_mtime_ns,
        source_fingerprint: source_fingerprint.to_string(),
    };
    let payload =
        py.detach(|| ait_core::current_source_cache::current_source_binary_is_fresh_json(&request));
    render_json_value(py, payload)
}

#[pyfunction(name = "format_current_source_cache_contract")]
fn format_current_source_cache_contract_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Current-source cache result")?;
    ait_cli::binding_output::format_current_source_cache_contract(&payload)
        .map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_current_source_cache_mark_building")]
fn format_current_source_cache_mark_building_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Current-source cache result")?;
    ait_cli::binding_output::format_current_source_cache_mark_building(&payload)
        .map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_current_source_cache_activate")]
fn format_current_source_cache_activate_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Current-source cache result")?;
    ait_cli::binding_output::format_current_source_cache_activate(&payload)
        .map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_current_source_cache_release_lease")]
fn format_current_source_cache_release_lease_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Current-source cache result")?;
    ait_cli::binding_output::format_current_source_cache_release_lease(&payload)
        .map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_current_source_cache_prune")]
fn format_current_source_cache_prune_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Current-source cache result")?;
    ait_cli::binding_output::format_current_source_cache_prune(&payload)
        .map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_current_source_cache_seed_canonical")]
fn format_current_source_cache_seed_canonical_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Current-source cache result")?;
    ait_cli::binding_output::format_current_source_cache_seed_canonical(&payload)
        .map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_current_source_cache_extension_fresh")]
fn format_current_source_cache_extension_fresh_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Current-source cache result")?;
    ait_cli::binding_output::format_current_source_cache_extension_fresh(&payload)
        .map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_current_source_cache_binary_fresh")]
fn format_current_source_cache_binary_fresh_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Current-source cache result")?;
    ait_cli::binding_output::format_current_source_cache_binary_fresh(&payload)
        .map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_current_source_cache_core_fingerprint")]
fn format_current_source_cache_core_fingerprint_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Current-source cache result")?;
    ait_cli::binding_output::format_current_source_cache_core_fingerprint(&payload)
        .map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_current_source_cache_server_fingerprint")]
fn format_current_source_cache_server_fingerprint_py(
    payload: Bound<'_, PyAny>,
) -> PyResult<String> {
    let payload = parse_json_value(payload, "Current-source cache result")?;
    ait_cli::binding_output::format_current_source_cache_server_fingerprint(&payload)
        .map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_current_source_cache_core_mtime")]
fn format_current_source_cache_core_mtime_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Current-source cache result")?;
    ait_cli::binding_output::format_current_source_cache_core_mtime(&payload)
        .map_err(PyValueError::new_err)
}

#[pyfunction(name = "format_current_source_cache_server_mtime")]
fn format_current_source_cache_server_mtime_py(payload: Bound<'_, PyAny>) -> PyResult<String> {
    let payload = parse_json_value(payload, "Current-source cache result")?;
    ait_cli::binding_output::format_current_source_cache_server_mtime(&payload)
        .map_err(PyValueError::new_err)
}

fn register_binding_source_cache(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(
        current_source_cache_mark_building_operation_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        current_source_cache_activate_operation_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        current_source_cache_seed_canonical_operation_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        current_source_cache_core_fingerprint_operation_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        current_source_cache_server_fingerprint_operation_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        current_source_cache_core_mtime_operation_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        current_source_cache_server_mtime_operation_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        current_source_cache_release_lease_operation_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        current_source_cache_prune_operation_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        current_source_cache_extension_fresh_operation_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        current_source_cache_binary_fresh_operation_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        format_current_source_cache_contract_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        format_current_source_cache_mark_building_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        format_current_source_cache_activate_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        format_current_source_cache_release_lease_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        format_current_source_cache_prune_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        format_current_source_cache_seed_canonical_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        format_current_source_cache_extension_fresh_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        format_current_source_cache_binary_fresh_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        format_current_source_cache_core_fingerprint_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        format_current_source_cache_server_fingerprint_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        format_current_source_cache_core_mtime_py,
        module
    )?)?;
    module.add_function(wrap_pyfunction!(
        format_current_source_cache_server_mtime_py,
        module
    )?)?;
    Ok(())
}
