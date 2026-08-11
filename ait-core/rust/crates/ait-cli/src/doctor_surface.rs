use ait_core::json_support::{json, JsonCodec, JsonEncodeOptions, JsonMap as Map, JsonValue};
use ait_core::plan_filesystem::operational_external_materialization_roots;
use chrono::Utc;
use postgres::{Client, NoTls};
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use zip::ZipArchive;

const RUNTIME_DATA_ENV: &str = "AIT_RUNTIME_DATA";
const LEGACY_SERVER_DATA_ENV: &str = "AIT_NATIVE_SERVER_DATA";
const DEFAULT_POSTGRES_BACKEND: &str = "postgres";
const DEFAULT_POSTGRES_CONTENT_SCHEMA: &str = "ait_native_content";
const DEFAULT_POSTGRES_CONTROL_SCHEMA: &str = "ait_native_control";
const POSTGRES_SCHEMA_VERSION_TABLE: &str = "schema_versions";
const PLAN_AUTHORITY_CONTRACT_VERSION: &str = "plan-foundation-v7";
const TASK_CONTRACT_VERSION: &str = "task-close-v1";
const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");

const PLAN_AUTHORITY_COMMAND_SURFACE: &[&str] =
    &["ait plan items", "ait plan candidates", "ait plan inspect"];

const PLAN_AUTHORITY_REQUIRED_EXPORTS: &[&str] = &[
    "plan_items_payload",
    "plan_dispatch_summary",
    "plan_task_link_indexes",
    "compute_taskable_items",
    "validate_dispatch_legality",
    "plan_candidates_payload",
    "extract_plan_items",
    "extract_plan_section",
    "find_plan_item",
    "normalize_plan_items",
];

const SUPPORTED_WHEEL_MATRIX: &[(&str, &str, &str)] = &[
    ("macos-x86_64", "macOS", "x86_64"),
    ("macos-arm64", "macOS", "arm64"),
    ("linux-x86_64", "Linux", "x86_64"),
    ("linux-aarch64", "Linux", "aarch64"),
    ("windows-x86_64", "Windows", "x86_64"),
    ("windows-arm64", "Windows", "arm64"),
];

const EXPECTED_CONTENT_SCHEMA_VERSION: i64 = 5;
const EXPECTED_CONTROL_SCHEMA_VERSION: i64 = 3;
const EXPECTED_CONTENT_SCHEMA_DESCRIPTION: &str =
    "M7 content schema for repo_id-scoped repositories, groups, refs, blobs, snapshots, and tree packs addressed by root locator.";
const EXPECTED_CONTROL_SCHEMA_DESCRIPTION: &str =
    "M6 control schema for repo_id-scoped workflow state plus authority-map persistence.";

pub fn doctor_memory_root(ensure: bool) -> Result<JsonValue, String> {
    crate::task_worktree_layout::doctor_memory_root_payload(ensure)
}

#[derive(Debug, Clone)]
struct ServerContext {
    root: PathBuf,
    db_backend: String,
    postgres_dsn: Option<String>,
    content_schema: String,
    control_schema: String,
}

pub fn doctor_runtime_root(
    repo_root: &Path,
    server_data: Option<&Path>,
) -> Result<JsonValue, String> {
    let resolved_root = resolve_path_strict_false(repo_root);
    let configured_runtime_root = resolved_runtime_data_root(server_data)?;
    let policy = workspace_ignore_policy(&resolved_root, server_data)?;
    let Some(configured_runtime_root) = configured_runtime_root else {
        return Ok(json!({
            "repo_root": path_text(&resolved_root),
            "runtime_root": JsonValue::Null,
            "runtime_root_source": "unconfigured",
            "runtime_root_relative_to_repo": JsonValue::Null,
            "inside_repo": false,
            "outside_repo": false,
            "equals_repo_root": false,
            "snapshot_ignored": false,
            "protected_from_snapshots": false,
            "state": "pass",
            "recommended_action": "configure_when_server_is_enabled",
            "reasons": ["No server runtime root is configured."],
            "next_actions": [
                format!("Configure {RUNTIME_DATA_ENV} only when this repository will run ait-server locally."),
            ],
            "ignore_policy": policy,
        }));
    };

    let resolved_runtime = resolve_path_strict_false(&configured_runtime_root);
    let runtime_rel = relative_to(&resolved_runtime, &resolved_root);
    let inside_repo = runtime_rel.is_some();
    let equals_repo_root = resolved_runtime == resolved_root;
    let ignored_runtime_roots = workspace_runtime_roots(&resolved_root, server_data)?;
    let snapshot_ignored = ignored_runtime_roots.contains(&resolved_runtime);
    let protected_from_snapshots = !inside_repo || snapshot_ignored;
    let (state, recommended_action, reasons, next_actions) = if equals_repo_root {
        (
            "fail",
            "move_runtime_root_outside_repo",
            vec!["The configured runtime root is the repository checkout itself.".to_string()],
            vec![
                format!(
                    "Set {RUNTIME_DATA_ENV} to a dedicated directory outside the repo checkout."
                ),
                "Move existing server data before creating new snapshots.".to_string(),
            ],
        )
    } else if inside_repo {
        (
            "warn",
            "prefer_external_runtime_root",
            vec![
                "The configured runtime root is inside the repository checkout but is ignored by snapshot/status scans."
                    .to_string(),
            ],
            vec![format!(
                "Prefer the default external runtime root or an absolute {RUNTIME_DATA_ENV} path outside the repo."
            )],
        )
    } else {
        (
            "pass",
            "none",
            vec!["The configured runtime root is outside the repository checkout.".to_string()],
            Vec::new(),
        )
    };

    Ok(json!({
        "repo_root": path_text(&resolved_root),
        "runtime_root": path_text(&resolved_runtime),
        "runtime_root_source": runtime_root_source(server_data),
        "runtime_root_relative_to_repo": runtime_rel.map(JsonValue::String).unwrap_or(JsonValue::Null),
        "inside_repo": inside_repo,
        "outside_repo": !inside_repo,
        "equals_repo_root": equals_repo_root,
        "snapshot_ignored": snapshot_ignored,
        "protected_from_snapshots": protected_from_snapshots,
        "state": state,
        "recommended_action": recommended_action,
        "reasons": reasons,
        "next_actions": next_actions,
        "ignore_policy": policy,
    }))
}

pub fn doctor_postgres(
    schema_root: Option<&Path>,
    server_data: Option<&Path>,
    backend: Option<&str>,
    dsn: Option<&str>,
    content_schema: Option<&str>,
    control_schema: Option<&str>,
    connect: bool,
) -> Result<JsonValue, String> {
    let ctx =
        create_postgres_server_context(server_data, backend, dsn, content_schema, control_schema)?;
    let mut issues = Vec::<String>::new();
    let mut warnings = Vec::<String>::new();

    if ctx.db_backend != DEFAULT_POSTGRES_BACKEND {
        warnings.push(format!(
            "Server backend is configured as {:?}; set AIT_NATIVE_SERVER_DB_BACKEND=postgres or use --backend postgres to validate the PostgreSQL path.",
            ctx.db_backend
        ));
    }
    if ctx.postgres_dsn.as_deref().unwrap_or("").trim().is_empty() {
        issues.push("AIT_NATIVE_SERVER_POSTGRES_DSN is not configured.".to_string());
    }

    let mut content_schema_valid = true;
    let mut control_schema_valid = true;
    if let Err(err) = ensure_schema_name(&ctx.content_schema) {
        content_schema_valid = false;
        issues.push(err);
    }
    if let Err(err) = ensure_schema_name(&ctx.control_schema) {
        control_schema_valid = false;
        issues.push(err);
    }

    let schema_root = schema_root
        .map(PathBuf::from)
        .unwrap_or(env::current_dir().map_err(|err| err.to_string())?);
    let content_schema_file = schema_file_path(&schema_root, "content")?;
    let control_schema_file = schema_file_path(&schema_root, "control")?;
    if !content_schema_file.exists() {
        issues.push(format!(
            "Missing PostgreSQL content schema file: {}",
            content_schema_file.display()
        ));
    }
    if !control_schema_file.exists() {
        issues.push(format!(
            "Missing PostgreSQL control schema file: {}",
            control_schema_file.display()
        ));
    }

    let mut live_connection_ok = JsonValue::Null;
    let mut live_connection_error = JsonValue::Null;
    let mut schema_upgrade_checks = JsonValue::Null;
    if connect {
        if !issues.is_empty() {
            live_connection_ok = JsonValue::Bool(false);
            live_connection_error = JsonValue::String(
                "Skipped live PostgreSQL connection attempt because preflight issues were already detected."
                    .to_string(),
            );
        } else {
            match postgres_schema_checks_for_context(
                &ctx,
                Some(&content_schema_file),
                Some(&control_schema_file),
                true,
            ) {
                Ok(checks) => {
                    let ok = checks
                        .get("ok")
                        .and_then(JsonValue::as_bool)
                        .unwrap_or(false);
                    schema_upgrade_checks = checks;
                    if ok {
                        live_connection_ok = JsonValue::Bool(true);
                    } else {
                        live_connection_ok = JsonValue::Bool(false);
                        live_connection_error = JsonValue::String(
                            "PostgreSQL schema upgrade checks did not reach the expected versions."
                                .to_string(),
                        );
                        issues.push(
                            "Live PostgreSQL connection failed: PostgreSQL schema upgrade checks did not reach the expected versions."
                                .to_string(),
                        );
                    }
                }
                Err(err) => {
                    live_connection_ok = JsonValue::Bool(false);
                    live_connection_error = JsonValue::String(err.clone());
                    issues.push(format!("Live PostgreSQL connection failed: {err}"));
                }
            }
        }
    }

    let ready = issues.is_empty();
    Ok(json!({
        "backend": ctx.db_backend,
        "server_data_root": path_text(&ctx.root),
        "postgres_dsn_configured": ctx.postgres_dsn.as_deref().map(|value| !value.trim().is_empty()).unwrap_or(false),
        "content_schema": ctx.content_schema,
        "control_schema": ctx.control_schema,
        "timestamp": Utc::now().to_rfc3339(),
        "postgres_driver": "rust-postgres",
        "psycopg_installed": true,
        "postgres_driver_available": true,
        "postgres_driver_status": postgres_driver_status_payload(),
        "content_schema_valid": content_schema_valid,
        "control_schema_valid": control_schema_valid,
        "schema_files": {
            "content": {"path": path_text(&content_schema_file), "exists": content_schema_file.exists()},
            "control": {"path": path_text(&control_schema_file), "exists": control_schema_file.exists()},
        },
        "attempted_live_connect": connect,
        "live_connection_ok": live_connection_ok,
        "live_connection_error": live_connection_error,
        "schema_upgrade_checks": schema_upgrade_checks,
        "issues": issues,
        "warnings": warnings,
        "ready": ready,
    }))
}

pub fn postgres_schema_checks(
    schema_root: Option<&Path>,
    server_data: Option<&Path>,
    backend: Option<&str>,
    dsn: Option<&str>,
    content_schema: Option<&str>,
    control_schema: Option<&str>,
    apply: bool,
) -> Result<JsonValue, String> {
    let ctx =
        create_postgres_server_context(server_data, backend, dsn, content_schema, control_schema)?;
    ensure_schema_name(&ctx.content_schema)?;
    ensure_schema_name(&ctx.control_schema)?;

    let schema_root = schema_root
        .map(PathBuf::from)
        .unwrap_or(env::current_dir().map_err(|err| err.to_string())?);
    let (content_schema_file, control_schema_file) = if apply {
        let content_schema_file = schema_file_path(&schema_root, "content")?;
        let control_schema_file = schema_file_path(&schema_root, "control")?;
        if !content_schema_file.exists() {
            return Ok(postgres_schema_checks_error_payload(
                &ctx,
                apply,
                format!(
                    "Missing PostgreSQL content schema file: {}",
                    content_schema_file.display()
                ),
            ));
        }
        if !control_schema_file.exists() {
            return Ok(postgres_schema_checks_error_payload(
                &ctx,
                apply,
                format!(
                    "Missing PostgreSQL control schema file: {}",
                    control_schema_file.display()
                ),
            ));
        }
        (Some(content_schema_file), Some(control_schema_file))
    } else {
        (None, None)
    };

    postgres_schema_checks_for_context(
        &ctx,
        content_schema_file.as_deref(),
        control_schema_file.as_deref(),
        apply,
    )
}

pub fn doctor_plan_authority(backend: Option<&str>) -> Result<JsonValue, String> {
    let selected_backend = selected_plan_backend(backend)?;
    let mut payload = json!({
        "selected_backend": selected_backend,
        "selected_backend_ready": selected_backend != "rust",
        "rust_authority_ready": false,
        "compatibility": if selected_backend == "rust" { "unavailable" } else { "inactive" },
        "authority_source": "ait-core-rust",
        "extension_module": env::var("AIT_RUST_EXT_MODULE").ok().map(|value| {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() { "ait_py".to_string() } else { trimmed }
        }).unwrap_or_else(|| "ait_py".to_string()),
        "extension_loaded": false,
        "extension_path": JsonValue::Null,
        "package_version": PACKAGE_VERSION,
        "extension_package_version": JsonValue::Null,
        "extension_task_contract_version": JsonValue::Null,
        "extension_plan_contract_version": JsonValue::Null,
        "expected_plan_contract_version": PLAN_AUTHORITY_CONTRACT_VERSION,
        "surface_commands": PLAN_AUTHORITY_COMMAND_SURFACE,
        "required_exports": PLAN_AUTHORITY_REQUIRED_EXPORTS,
        "exports": {},
        "missing_exports": [],
        "issues": [],
        "env": {
            "AIT_PLAN_BACKEND": env_text("AIT_PLAN_BACKEND"),
            "AIT_PLAN_CORE_BACKEND": env_text("AIT_PLAN_CORE_BACKEND"),
            "AIT_CORE_BACKEND": env_text("AIT_CORE_BACKEND"),
            "AIT_ALLOW_RUST_BACKEND_EXPERIMENTS": env_text("AIT_ALLOW_RUST_BACKEND_EXPERIMENTS"),
            "AIT_RUST_EXT_MODULE": env_text("AIT_RUST_EXT_MODULE"),
        },
    });
    if selected_backend != "rust" {
        return Ok(payload);
    }

    let exports = PLAN_AUTHORITY_REQUIRED_EXPORTS
        .iter()
        .map(|name| ((*name).to_string(), JsonValue::Bool(true)))
        .collect::<Map<String, JsonValue>>();
    let obj = payload
        .as_object_mut()
        .ok_or_else(|| "plan-authority payload must be an object".to_string())?;
    obj.insert("selected_backend_ready".to_string(), JsonValue::Bool(true));
    obj.insert("rust_authority_ready".to_string(), JsonValue::Bool(true));
    obj.insert(
        "compatibility".to_string(),
        JsonValue::String("compatible".to_string()),
    );
    obj.insert("extension_loaded".to_string(), JsonValue::Bool(true));
    obj.insert(
        "extension_package_version".to_string(),
        JsonValue::String(PACKAGE_VERSION.to_string()),
    );
    obj.insert(
        "extension_task_contract_version".to_string(),
        JsonValue::String(TASK_CONTRACT_VERSION.to_string()),
    );
    obj.insert(
        "extension_plan_contract_version".to_string(),
        JsonValue::String(PLAN_AUTHORITY_CONTRACT_VERSION.to_string()),
    );
    obj.insert("exports".to_string(), JsonValue::Object(exports));
    Ok(payload)
}

pub fn doctor_plan_authority_wheel(
    wheel: Option<&Path>,
    repack_installed: bool,
    smoke: bool,
) -> Result<JsonValue, String> {
    if wheel.is_some() && repack_installed {
        return Err("Use either --wheel or --repack-installed, not both.".to_string());
    }
    let current_target = current_plan_authority_target();
    let supported_targets = SUPPORTED_WHEEL_MATRIX
        .iter()
        .map(|(target, os, arch)| json!({"target": target, "os": os, "arch": arch}))
        .collect::<Vec<_>>();
    let mut issues = Vec::<String>::new();
    if current_target.is_none() {
        issues.push(
            "Current platform target is outside the supported plan authority wheel matrix."
                .to_string(),
        );
    }
    let mut payload = json!({
        "supported_matrix": supported_targets,
        "current_target": current_target,
        "current_supported": current_target.is_some(),
        "installed_wheel_tag": JsonValue::Null,
        "installed_wheel_tag_source": "unavailable_without_python_runtime",
        "wheel_path": JsonValue::Null,
        "wheel_filename": JsonValue::Null,
        "wheel_tag": JsonValue::Null,
        "wheel_target": JsonValue::Null,
        "wheel_target_supported": JsonValue::Null,
        "wheel_matches_current_target": JsonValue::Null,
        "wheel_source": JsonValue::Null,
        "repack_supported": false,
        "repack_requested": repack_installed,
        "smoke_supported": false,
        "smoke_requested": smoke,
        "smoke": JsonValue::Null,
        "issues": JsonValue::Array(Vec::new()),
    });

    if repack_installed {
        issues.push(
            "Rust ait-core does not repack installed Python extension wheels; build or pass an explicit wheel artifact from the integration layer."
                .to_string(),
        );
    } else if let Some(path) = wheel {
        set_payload_string(&mut payload, "wheel_path", path_text(path))?;
        set_payload_string(&mut payload, "wheel_source", "provided")?;
    }

    if let Some(path) = wheel {
        set_payload_string(
            &mut payload,
            "wheel_filename",
            path.file_name()
                .and_then(|value| value.to_str())
                .unwrap_or(""),
        )?;
        let wheel_tag = inspect_plan_authority_wheel_tag(path)?;
        let wheel_target = wheel_target_from_tag(wheel_tag.as_deref());
        let wheel_target_supported = wheel_target.as_deref().map(|target| {
            SUPPORTED_WHEEL_MATRIX
                .iter()
                .any(|(candidate, _, _)| *candidate == target)
        });
        let wheel_matches_current_target =
            match (wheel_target.as_deref(), current_target.as_deref()) {
                (Some(wheel_target), Some(current_target)) => Some(wheel_target == current_target),
                _ => None,
            };
        set_optional_string(&mut payload, "wheel_tag", wheel_tag.clone())?;
        set_optional_string(&mut payload, "wheel_target", wheel_target.clone())?;
        set_optional_bool(
            &mut payload,
            "wheel_target_supported",
            wheel_target_supported,
        )?;
        set_optional_bool(
            &mut payload,
            "wheel_matches_current_target",
            wheel_matches_current_target,
        )?;
        if wheel_target.is_none() {
            issues.push(format!(
                "Could not derive a supported platform target from wheel tag `{}`.",
                wheel_tag.as_deref().unwrap_or("missing")
            ));
        } else if wheel_target_supported == Some(false) {
            issues.push(format!(
                "Wheel tag `{}` is outside the supported plan authority wheel matrix.",
                wheel_tag.as_deref().unwrap_or("missing")
            ));
        }
    }

    if smoke {
        let smoke_payload = json!({
            "status": "unsupported",
            "reason": "Rust ait-core does not create Python virtualenvs or import ait_py; run wheel smoke from the external integration layer.",
            "wheel_path": wheel.map(path_text).unwrap_or_default(),
            "expected_plan_contract_version": PLAN_AUTHORITY_CONTRACT_VERSION,
        });
        payload
            .as_object_mut()
            .ok_or_else(|| "wheel payload must be an object".to_string())?
            .insert("smoke".to_string(), smoke_payload);
        issues.push(
            "Plan authority wheel smoke is unavailable inside repo-pure Rust ait-core.".to_string(),
        );
    }

    payload
        .as_object_mut()
        .ok_or_else(|| "wheel payload must be an object".to_string())?
        .insert(
            "issues".to_string(),
            JsonValue::Array(issues.into_iter().map(JsonValue::String).collect()),
        );
    Ok(payload)
}

pub fn render_doctor_text(title: &str, payload: &JsonValue) -> Result<String, String> {
    let obj = payload
        .as_object()
        .ok_or_else(|| "doctor payload must be an object".to_string())?;
    let mut lines = vec![title.to_string()];
    for key in [
        "state",
        "ready",
        "recommended_action",
        "compatibility",
        "selected_backend",
        "current_target",
        "wheel_target",
        "mount_point",
        "runtime_root",
        "filesystem_total_bytes",
        "available_bytes",
        "auto_mounted",
        "platform_proof",
    ] {
        if let Some(value) = obj.get(key) {
            if !value.is_null() {
                lines.push(format!("{key}: {}", scalar_text(value)));
            }
        }
    }
    if let Some(issues) = obj.get("issues").and_then(JsonValue::as_array) {
        if !issues.is_empty() {
            lines.push("issues:".to_string());
            for issue in issues {
                if let Some(text) = issue.as_str() {
                    lines.push(format!("- {text}"));
                }
            }
        }
    }
    if let Some(warnings) = obj.get("warnings").and_then(JsonValue::as_array) {
        if !warnings.is_empty() {
            lines.push("warnings:".to_string());
            for warning in warnings {
                if let Some(text) = warning.as_str() {
                    lines.push(format!("- {text}"));
                }
            }
        }
    }
    Ok(lines.join("\n"))
}

fn create_postgres_server_context(
    server_data: Option<&Path>,
    backend: Option<&str>,
    dsn: Option<&str>,
    content_schema: Option<&str>,
    control_schema: Option<&str>,
) -> Result<ServerContext, String> {
    let root = resolve_effective_server_runtime_root(server_data)?;
    let env_backend = env::var("AIT_NATIVE_SERVER_DB_BACKEND").ok();
    let db_backend = backend
        .or(env_backend.as_deref())
        .unwrap_or(DEFAULT_POSTGRES_BACKEND)
        .trim()
        .to_ascii_lowercase();
    let db_backend = if db_backend.is_empty() {
        DEFAULT_POSTGRES_BACKEND.to_string()
    } else {
        db_backend
    };
    if db_backend != DEFAULT_POSTGRES_BACKEND {
        return Err(format!(
            "Unsupported AIT native server database backend: {db_backend:?}"
        ));
    }
    let postgres_dsn = optional_text(
        dsn.map(str::to_string)
            .or_else(|| env::var("AIT_NATIVE_SERVER_POSTGRES_DSN").ok())
            .as_deref(),
    );
    let content_schema = optional_text(
        content_schema
            .map(str::to_string)
            .or_else(|| env::var("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA").ok())
            .as_deref(),
    )
    .unwrap_or_else(|| DEFAULT_POSTGRES_CONTENT_SCHEMA.to_string());
    let control_schema = optional_text(
        control_schema
            .map(str::to_string)
            .or_else(|| env::var("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA").ok())
            .as_deref(),
    )
    .unwrap_or_else(|| DEFAULT_POSTGRES_CONTROL_SCHEMA.to_string());
    Ok(ServerContext {
        root,
        db_backend,
        postgres_dsn,
        content_schema,
        control_schema,
    })
}

fn resolve_effective_server_runtime_root(server_data: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = server_data {
        return Ok(resolve_path_strict_false(&expanduser_path(path)));
    }
    if let Some((_, value)) = configured_runtime_data_env() {
        return Ok(resolve_path_strict_false(&expanduser_str(&value)));
    }
    Err(
        "AIT_NATIVE_SERVER_DATA is required for server runtime access; platform default runtime roots are no longer supported."
            .to_string(),
    )
}

fn postgres_schema_checks_for_context(
    ctx: &ServerContext,
    content_schema_file: Option<&Path>,
    control_schema_file: Option<&Path>,
    apply: bool,
) -> Result<JsonValue, String> {
    let Some(dsn) = ctx.postgres_dsn.as_deref() else {
        return Ok(json!({
            "backend": ctx.db_backend,
            "applied": false,
            "expected_versions": expected_postgres_schema_versions(),
            "ok": false,
            "checks": {},
            "error": "AIT_NATIVE_SERVER_POSTGRES_DSN is not configured.",
        }));
    };
    if let Some(root) = fake_postgres_root(dsn) {
        return fake_postgres_schema_checks(ctx, &root, apply);
    }

    let mut checks = Map::new();
    if apply {
        let content_schema_file = content_schema_file.ok_or_else(|| {
            "PostgreSQL content schema file is required when apply=true.".to_string()
        })?;
        let control_schema_file = control_schema_file.ok_or_else(|| {
            "PostgreSQL control schema file is required when apply=true.".to_string()
        })?;
        for (plane, schema, schema_file) in [
            ("content", ctx.content_schema.as_str(), content_schema_file),
            ("control", ctx.control_schema.as_str(), control_schema_file),
        ] {
            let mut client = Client::connect(dsn, NoTls).map_err(|err| err.to_string())?;
            apply_plane_schema(&mut client, plane, schema, schema_file)?;
        }
    }
    for (plane, schema) in [
        ("content", ctx.content_schema.as_str()),
        ("control", ctx.control_schema.as_str()),
    ] {
        let mut client = Client::connect(dsn, NoTls).map_err(|err| err.to_string())?;
        let status = if apply {
            set_search_path(&mut client, schema)?;
            schema_version_status(&mut client, plane)?
        } else {
            set_search_path_existing(&mut client, schema)?;
            schema_version_status_for_schema(&mut client, plane, schema)?
        };
        checks.insert(plane.to_string(), status);
    }
    let ok = checks
        .values()
        .all(|value| value.get("ok").and_then(JsonValue::as_bool) == Some(true));
    Ok(json!({
        "backend": ctx.db_backend,
        "applied": apply,
        "expected_versions": expected_postgres_schema_versions(),
        "checks": checks,
        "ok": ok,
    }))
}

fn fake_postgres_schema_checks(
    ctx: &ServerContext,
    root: &Path,
    apply: bool,
) -> Result<JsonValue, String> {
    let mut checks = Map::new();
    for (plane, schema) in [
        ("content", ctx.content_schema.as_str()),
        ("control", ctx.control_schema.as_str()),
    ] {
        ensure_schema_name(schema)?;
        let status_path = root.join(format!("{schema}.schema-version.json"));
        let status = if apply {
            fs::create_dir_all(root)
                .map_err(|err| format!("Failed to create {}: {err}", root.display()))?;
            let metadata = expected_schema_metadata(plane);
            let now = Utc::now().to_rfc3339();
            let value = json!({
                "plane": plane,
                "version": metadata.0,
                "description": metadata.1,
                "applied_at": now,
                "checked_at": now,
            });
            let encoded = JsonCodec::encode_value(
                &value,
                JsonEncodeOptions::pretty().with_trailing_newline(),
            )
            .map_err(|error| error.to_string())?;
            fs::write(&status_path, encoded)
                .map_err(|error| format!("Failed to write {}: {error}", status_path.display()))?;
            fake_postgres_schema_version_status(&status_path, plane)?
        } else {
            fake_postgres_schema_version_status(&status_path, plane)?
        };
        checks.insert(plane.to_string(), status);
    }
    let ok = checks
        .values()
        .all(|value| value.get("ok").and_then(JsonValue::as_bool) == Some(true));
    Ok(json!({
        "backend": ctx.db_backend,
        "applied": apply,
        "expected_versions": expected_postgres_schema_versions(),
        "checks": checks,
        "ok": ok,
    }))
}

fn apply_plane_schema(
    client: &mut Client,
    plane: &str,
    schema: &str,
    schema_file: &Path,
) -> Result<(), String> {
    set_search_path(client, schema)?;
    let script = fs::read_to_string(schema_file)
        .map_err(|err| format!("Failed to read {}: {err}", schema_file.display()))?;
    for statement in render_schema_sql(&script, plane, schema)? {
        client
            .batch_execute(&statement)
            .map_err(|err| err.to_string())?;
    }
    ensure_schema_version(client, plane)?;
    Ok(())
}

fn set_search_path(client: &mut Client, schema: &str) -> Result<(), String> {
    ensure_schema_name(schema)?;
    client
        .batch_execute(&format!(
            "create schema if not exists \"{}\"",
            escape_ident(schema)
        ))
        .map_err(|err| err.to_string())?;
    client
        .batch_execute(&format!(
            "set search_path to \"{}\", public",
            escape_ident(schema)
        ))
        .map_err(|err| err.to_string())
}

fn set_search_path_existing(client: &mut Client, schema: &str) -> Result<(), String> {
    ensure_schema_name(schema)?;
    client
        .batch_execute(&format!(
            "set search_path to \"{}\", public",
            escape_ident(schema)
        ))
        .map_err(|err| err.to_string())
}

fn ensure_schema_version(client: &mut Client, plane: &str) -> Result<(), String> {
    let metadata = expected_schema_metadata(plane);
    client
        .batch_execute(&format!(
            "create table if not exists {POSTGRES_SCHEMA_VERSION_TABLE} (
                plane text primary key,
                version integer not null,
                description text not null,
                applied_at timestamptz not null,
                checked_at timestamptz not null
            )"
        ))
        .map_err(|err| err.to_string())?;
    client
        .execute(
            &format!(
                "insert into {POSTGRES_SCHEMA_VERSION_TABLE}(plane, version, description, applied_at, checked_at)
                 values ($1, $2, $3, now(), now())
                 on conflict (plane) do update set
                    version = excluded.version,
                    description = excluded.description,
                    applied_at = excluded.applied_at,
                    checked_at = excluded.checked_at"
            ),
            &[&plane, &metadata.0, &metadata.1],
        )
        .map_err(|err| err.to_string())?;
    Ok(())
}

fn schema_version_status(client: &mut Client, plane: &str) -> Result<JsonValue, String> {
    schema_version_status_for_table(client, plane, POSTGRES_SCHEMA_VERSION_TABLE.to_string())
}

fn schema_version_status_for_schema(
    client: &mut Client,
    plane: &str,
    schema: &str,
) -> Result<JsonValue, String> {
    ensure_schema_name(schema)?;
    schema_version_status_for_table(
        client,
        plane,
        format!(
            "\"{}\".\"{}\"",
            escape_ident(schema),
            POSTGRES_SCHEMA_VERSION_TABLE
        ),
    )
}

fn schema_version_status_for_table(
    client: &mut Client,
    plane: &str,
    table_expr: String,
) -> Result<JsonValue, String> {
    let metadata = expected_schema_metadata(plane);
    let query = format!(
        "select plane, version, description, applied_at::text, checked_at::text from {table_expr} where plane = $1"
    );
    let rows = match client.query(&query, &[&plane]) {
        Ok(rows) => rows,
        Err(err) => {
            return Ok(json!({
                "plane": plane,
                "backend": DEFAULT_POSTGRES_BACKEND,
                "table_present": false,
                "expected_version": metadata.0,
                "version": JsonValue::Null,
                "ok": false,
                "error": err.to_string(),
            }));
        }
    };
    let Some(row) = rows.first() else {
        return Ok(json!({
            "plane": plane,
            "backend": DEFAULT_POSTGRES_BACKEND,
            "table_present": true,
            "expected_version": metadata.0,
            "version": JsonValue::Null,
            "ok": false,
            "error": "schema version row is missing",
        }));
    };
    let version: i64 = row.get::<_, i32>(1) as i64;
    Ok(json!({
        "plane": plane,
        "backend": DEFAULT_POSTGRES_BACKEND,
        "table_present": true,
        "expected_version": metadata.0,
        "version": version,
        "description": row.get::<_, String>(2),
        "applied_at": row.get::<_, String>(3),
        "checked_at": row.get::<_, String>(4),
        "ok": version == metadata.0,
        "error": if version == metadata.0 { JsonValue::Null } else { JsonValue::String(format!("expected schema version {}, found {version}", metadata.0)) },
    }))
}

fn fake_postgres_schema_version_status(
    status_path: &Path,
    plane: &str,
) -> Result<JsonValue, String> {
    if !status_path.exists() {
        return Ok(schema_version_missing_status(
            plane,
            format!(
                "fake PostgreSQL schema status is missing: {}",
                status_path.display()
            ),
        ));
    }
    let text = fs::read_to_string(status_path)
        .map_err(|error| format!("Failed to read {}: {error}", status_path.display()))?;
    let value = JsonCodec::parse_value(&text, "fake PostgreSQL schema status")
        .map_err(|error| error.to_string())?;
    let metadata = expected_schema_metadata(plane);
    let version = value.get("version").and_then(JsonValue::as_i64);
    let ok = value.get("plane").and_then(JsonValue::as_str) == Some(plane)
        && version == Some(metadata.0);
    Ok(json!({
        "plane": plane,
        "backend": DEFAULT_POSTGRES_BACKEND,
        "table_present": true,
        "expected_version": metadata.0,
        "version": version.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "description": value.get("description").cloned().unwrap_or(JsonValue::Null),
        "applied_at": value.get("applied_at").cloned().unwrap_or(JsonValue::Null),
        "checked_at": value.get("checked_at").cloned().unwrap_or(JsonValue::Null),
        "ok": ok,
        "error": if ok { JsonValue::Null } else { JsonValue::String(format!("expected schema version {}, found {:?}", metadata.0, version)) },
    }))
}

fn schema_version_missing_status(plane: &str, error: String) -> JsonValue {
    let metadata = expected_schema_metadata(plane);
    json!({
        "plane": plane,
        "backend": DEFAULT_POSTGRES_BACKEND,
        "table_present": false,
        "expected_version": metadata.0,
        "version": JsonValue::Null,
        "ok": false,
        "error": error,
    })
}

fn render_schema_sql(script: &str, plane: &str, schema: &str) -> Result<Vec<String>, String> {
    let default_schema = if plane == "content" {
        DEFAULT_POSTGRES_CONTENT_SCHEMA
    } else {
        DEFAULT_POSTGRES_CONTROL_SCHEMA
    };
    let rendered = script.replace(
        &format!("\"{default_schema}\""),
        &format!("\"{}\"", escape_ident(schema)),
    );
    let mut statements = Vec::new();
    for statement in split_sql_script(&rendered) {
        let lowered = statement.trim().to_ascii_lowercase();
        if lowered == "begin" || lowered == "commit" {
            continue;
        }
        if lowered.starts_with("create or replace view ") {
            let view_name = statement
                .split_whitespace()
                .nth(4)
                .ok_or_else(|| "Could not parse CREATE OR REPLACE VIEW statement".to_string())?;
            statements.push(format!("drop view if exists {view_name}"));
            statements.push(statement.replacen("create or replace view", "create view", 1));
            continue;
        }
        statements.push(statement);
    }
    Ok(statements)
}

fn split_sql_script(script: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut previous = '\0';
    for ch in script.chars() {
        if ch == '\'' && !in_double && previous != '\\' {
            in_single = !in_single;
        } else if ch == '"' && !in_single && previous != '\\' {
            in_double = !in_double;
        }
        if ch == ';' && !in_single && !in_double {
            let statement = current.trim();
            if !statement.is_empty() {
                statements.push(statement.to_string());
            }
            current.clear();
        } else {
            current.push(ch);
        }
        previous = ch;
    }
    let tail = current.trim();
    if !tail.is_empty() {
        statements.push(tail.to_string());
    }
    statements
}

fn expected_postgres_schema_versions() -> JsonValue {
    json!({
        "content": {
            "version": EXPECTED_CONTENT_SCHEMA_VERSION,
            "description": EXPECTED_CONTENT_SCHEMA_DESCRIPTION,
        },
        "control": {
            "version": EXPECTED_CONTROL_SCHEMA_VERSION,
            "description": EXPECTED_CONTROL_SCHEMA_DESCRIPTION,
        },
    })
}

fn expected_schema_metadata(plane: &str) -> (i64, &'static str) {
    if plane == "content" {
        (
            EXPECTED_CONTENT_SCHEMA_VERSION,
            EXPECTED_CONTENT_SCHEMA_DESCRIPTION,
        )
    } else {
        (
            EXPECTED_CONTROL_SCHEMA_VERSION,
            EXPECTED_CONTROL_SCHEMA_DESCRIPTION,
        )
    }
}

fn postgres_schema_checks_error_payload(
    ctx: &ServerContext,
    apply: bool,
    error: String,
) -> JsonValue {
    json!({
        "backend": ctx.db_backend,
        "applied": false,
        "requested_apply": apply,
        "expected_versions": expected_postgres_schema_versions(),
        "checks": {},
        "ok": false,
        "error": error,
    })
}

fn postgres_driver_status_payload() -> JsonValue {
    json!({
        "available": true,
        "binary_path": JsonValue::Null,
        "capability": "ait-core-native-postgres",
        "error": JsonValue::Null,
    })
}

fn ensure_schema_name(schema: &str) -> Result<&str, String> {
    let value = schema.trim();
    if value.is_empty() {
        return Err("Schema name must not be empty.".to_string());
    }
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err("Schema name must not be empty.".to_string());
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return Err(format!(
            "Invalid schema name {schema:?}. Only letters, digits, and underscores are allowed, and the first character must be a letter or underscore."
        ));
    }
    if chars.any(|ch| !(ch == '_' || ch.is_ascii_alphanumeric())) {
        return Err(format!(
            "Invalid schema name {schema:?}. Only letters, digits, and underscores are allowed, and the first character must be a letter or underscore."
        ));
    }
    Ok(value)
}

fn schema_file_path(root: &Path, plane: &str) -> Result<PathBuf, String> {
    let file_name = match plane {
        "content" => "ait_native_postgres_content_schema.sql",
        "control" => "ait_native_postgres_control_schema.sql",
        other => return Err(format!("Unsupported PostgreSQL plane: {other:?}")),
    };
    Ok(root.join("sql").join(file_name))
}

fn fake_postgres_root(dsn: &str) -> Option<PathBuf> {
    let rest = dsn.strip_prefix("fake-postgres:///")?;
    Some(PathBuf::from(format!("/{rest}")))
}

fn selected_plan_backend(override_backend: Option<&str>) -> Result<String, String> {
    let value = override_backend
        .map(str::to_string)
        .or_else(|| env::var("AIT_PLAN_CORE_BACKEND").ok())
        .or_else(|| env::var("AIT_PLAN_BACKEND").ok())
        .or_else(|| env::var("AIT_CORE_BACKEND").ok())
        .unwrap_or_else(|| "rust".to_string());
    normalize_backend_name(&value)
}

fn normalize_backend_name(value: &str) -> Result<String, String> {
    let normalized = value.trim().to_ascii_lowercase();
    let normalized = if normalized.is_empty() {
        "python".to_string()
    } else {
        normalized
    };
    if normalized == "python" || normalized == "rust" {
        Ok(normalized)
    } else {
        Err(format!("Unsupported core backend: {value}"))
    }
}

fn current_plan_authority_target() -> Option<String> {
    let os = env::consts::OS;
    let arch = env::consts::ARCH;
    match (os, arch) {
        ("macos", "x86_64") => Some("macos-x86_64".to_string()),
        ("macos", "aarch64") => Some("macos-arm64".to_string()),
        ("linux", "x86_64") => Some("linux-x86_64".to_string()),
        ("linux", "aarch64") => Some("linux-aarch64".to_string()),
        ("windows", "x86_64") => Some("windows-x86_64".to_string()),
        ("windows", "aarch64") => Some("windows-arm64".to_string()),
        _ => None,
    }
}

fn inspect_plan_authority_wheel_tag(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Err(format!("Wheel path does not exist: {}", path.display()));
    }
    let file = fs::File::open(path).map_err(|err| err.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|err| err.to_string())?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|err| err.to_string())?;
        if !entry.name().ends_with(".dist-info/WHEEL") {
            continue;
        }
        let mut text = String::new();
        entry
            .read_to_string(&mut text)
            .map_err(|err| err.to_string())?;
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("Tag:") {
                return Ok(optional_text(Some(rest)).filter(|value| !value.is_empty()));
            }
        }
        return Ok(None);
    }
    Ok(None)
}

fn wheel_target_from_tag(tag: Option<&str>) -> Option<String> {
    let normalized = tag?.trim().to_ascii_lowercase();
    if normalized.contains("macosx") {
        if normalized.ends_with("_x86_64") {
            return Some("macos-x86_64".to_string());
        }
        if normalized.ends_with("_arm64") {
            return Some("macos-arm64".to_string());
        }
        return None;
    }
    if normalized.contains("manylinux") || normalized.contains("linux") {
        if normalized.ends_with("_x86_64") {
            return Some("linux-x86_64".to_string());
        }
        if normalized.ends_with("_aarch64") {
            return Some("linux-aarch64".to_string());
        }
        return None;
    }
    if normalized.contains("win_amd64") {
        return Some("windows-x86_64".to_string());
    }
    if normalized.contains("win_arm64") {
        return Some("windows-arm64".to_string());
    }
    None
}

fn workspace_ignore_policy(
    repo_root: &Path,
    runtime_root: Option<&Path>,
) -> Result<JsonValue, String> {
    let mut operational_roots = vec![".ait".to_string(), ".ait-runtime".to_string()];
    let mut external_roots =
        operational_external_materialization_roots(repo_root.to_string_lossy().as_ref())
            .unwrap_or_default();
    let mut runtime_roots = Vec::new();
    for runtime_root in workspace_runtime_roots(repo_root, runtime_root)? {
        let runtime_rel =
            relative_to(&runtime_root, repo_root).unwrap_or_else(|| path_text(&runtime_root));
        runtime_roots.push(runtime_rel.clone());
        operational_roots.push(runtime_rel);
    }
    operational_roots.extend(external_roots.iter().cloned());
    operational_roots.sort();
    operational_roots.dedup();
    external_roots.sort();
    external_roots.dedup();
    runtime_roots.sort();
    runtime_roots.dedup();
    let custom_patterns = load_workspace_ignore_rule_sources(repo_root);
    let mut payload = json!({
        "dir_names": [".ait", ".ait-runtime", ".ait-server", ".ait-worktree", ".ait-worktree-links", ".git", "__pycache__", ".pytest_cache", ".venv", "venv", ".mypy_cache"],
        "file_names": [".DS_Store", ".ait-worktree.json"],
        "operational_roots": operational_roots,
        "external_materialization_roots": external_roots,
        "runtime_roots": runtime_roots,
    });
    if !custom_patterns.is_empty() {
        let obj = payload
            .as_object_mut()
            .ok_or_else(|| "ignore policy payload must be an object".to_string())?;
        obj.insert(
            "rule_files".to_string(),
            JsonValue::Array(vec![JsonValue::String(".aitignore".to_string())]),
        );
        obj.insert(
            "custom_patterns".to_string(),
            JsonValue::Array(custom_patterns.into_iter().map(JsonValue::String).collect()),
        );
    }
    Ok(payload)
}

fn workspace_runtime_roots(
    repo_root: &Path,
    runtime_root: Option<&Path>,
) -> Result<Vec<PathBuf>, String> {
    let configured_runtime_root = resolved_runtime_data_root(runtime_root)?;
    let Some(configured_runtime_root) = configured_runtime_root else {
        return Ok(Vec::new());
    };
    let resolved_root = resolve_path_strict_false(repo_root);
    let resolved_runtime = resolve_path_strict_false(&configured_runtime_root);
    if resolved_runtime == resolved_root || !resolved_runtime.starts_with(&resolved_root) {
        return Ok(Vec::new());
    }
    Ok(vec![resolved_runtime])
}

fn resolved_runtime_data_root(runtime_root: Option<&Path>) -> Result<Option<PathBuf>, String> {
    if let Some(path) = runtime_root {
        return Ok(Some(resolve_path_strict_false(&expanduser_path(path))));
    }
    let Some((_, value)) = configured_runtime_data_env() else {
        return Ok(None);
    };
    Ok(Some(resolve_path_strict_false(&expanduser_str(&value))))
}

fn runtime_root_source(runtime_root: Option<&Path>) -> String {
    if runtime_root.is_some() {
        return "explicit".to_string();
    }
    configured_runtime_data_env()
        .map(|(name, _)| name)
        .unwrap_or_else(|| "unconfigured".to_string())
}

fn configured_runtime_data_env() -> Option<(String, String)> {
    for name in [RUNTIME_DATA_ENV, LEGACY_SERVER_DATA_ENV] {
        if let Ok(value) = env::var(name) {
            if !value.trim().is_empty() {
                return Some((name.to_string(), value.trim().to_string()));
            }
        }
    }
    None
}

fn load_workspace_ignore_rule_sources(repo_root: &Path) -> Vec<String> {
    let path = repo_root.join(".aitignore");
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect()
}

fn relative_to(path: &Path, root: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    if rel.as_os_str().is_empty() {
        Some(".".to_string())
    } else {
        Some(rel.to_string_lossy().replace('\\', "/"))
    }
}

fn resolve_path_strict_false(path: &Path) -> PathBuf {
    if let Ok(resolved) = path.canonicalize() {
        return resolved;
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    normalize_existing_ancestor(&absolute)
}

fn normalize_existing_ancestor(path: &Path) -> PathBuf {
    let mut cursor = path.to_path_buf();
    let mut suffix = Vec::new();
    while !cursor.exists() {
        let Some(name) = cursor.file_name().map(|value| value.to_os_string()) else {
            return normalize_path(path);
        };
        suffix.push(name);
        let Some(parent) = cursor.parent() else {
            return normalize_path(path);
        };
        cursor = parent.to_path_buf();
    }
    let mut resolved = cursor
        .canonicalize()
        .unwrap_or_else(|_| normalize_path(&cursor));
    for component in suffix.iter().rev() {
        resolved.push(component);
    }
    normalize_path(&resolved)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                output.pop();
            }
            other => output.push(other.as_os_str()),
        }
    }
    output
}

fn expanduser_str(value: &str) -> PathBuf {
    if value == "~" {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home);
        }
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(value)
}

fn expanduser_path(path: &Path) -> PathBuf {
    expanduser_str(path.to_string_lossy().as_ref())
}

fn escape_ident(value: &str) -> String {
    value.replace('"', "\"\"")
}

fn env_text(name: &str) -> JsonValue {
    env::var(name)
        .ok()
        .and_then(|value| optional_text(Some(&value)))
        .map(JsonValue::String)
        .unwrap_or(JsonValue::Null)
}

fn optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn scalar_text(value: &JsonValue) -> String {
    match value {
        JsonValue::String(value) => value.clone(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Number(value) => value.to_string(),
        JsonValue::Null => "null".to_string(),
        other => other.to_string(),
    }
}

fn set_payload_string(
    payload: &mut JsonValue,
    key: &str,
    value: impl Into<String>,
) -> Result<(), String> {
    payload
        .as_object_mut()
        .ok_or_else(|| "payload must be an object".to_string())?
        .insert(key.to_string(), JsonValue::String(value.into()));
    Ok(())
}

fn set_optional_string(
    payload: &mut JsonValue,
    key: &str,
    value: Option<String>,
) -> Result<(), String> {
    payload
        .as_object_mut()
        .ok_or_else(|| "payload must be an object".to_string())?
        .insert(
            key.to_string(),
            value.map(JsonValue::String).unwrap_or(JsonValue::Null),
        );
    Ok(())
}

fn set_optional_bool(
    payload: &mut JsonValue,
    key: &str,
    value: Option<bool>,
) -> Result<(), String> {
    payload
        .as_object_mut()
        .ok_or_else(|| "payload must be an object".to_string())?
        .insert(
            key.to_string(),
            value.map(JsonValue::Bool).unwrap_or(JsonValue::Null),
        );
    Ok(())
}
