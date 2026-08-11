use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Map as JsonMap, Value as JsonValue};

pub const PYTHON_REFERENCE_DELETION_GUARD_CONTRACT_VERSION: &str =
    "ait.server.python_reference_deletion_guard.v1";
pub const AIT_SERVER_REFERENCE_ROOT: &str = "../ait/src/ait_server";
pub const EXPECTED_REFERENCE_COUNT: usize = 0;
pub const NO_PYTHON_BOUNDARY_CHECK_ID: &str = "no_python_boundary";
pub const PYTHON_ONLY_BATCH_ACTIONS: &[&str] = &[
    "python_colocation",
    "module_merge",
    "import_rewire_only",
    "helper_rehome_only",
];
pub const RUST_MIGRATION_KINDS: &[&str] = &[
    "rust_owner_implementation",
    "caller_rewire_to_rust",
    "delete_after_rust_owner",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PythonReferenceDisposition {
    pub module: &'static str,
    pub sprint: &'static str,
    pub disposition: &'static str,
    pub authority_status: &'static str,
    pub deletion_readiness: &'static str,
    pub notes: &'static str,
    pub package_marker: bool,
}

impl PythonReferenceDisposition {
    pub fn to_json(self) -> JsonValue {
        json!({
            "module": self.module,
            "sprint": self.sprint,
            "disposition": self.disposition,
            "authority_status": self.authority_status,
            "deletion_readiness": self.deletion_readiness,
            "notes": self.notes,
            "package_marker": self.package_marker,
            "python_fallback_allowed": false,
        })
    }
}

pub const PYTHON_REFERENCE_INVENTORY: &[PythonReferenceDisposition] = &[];

pub fn python_reference_deletion_guard_contract() -> JsonValue {
    json!({
        "contract": PYTHON_REFERENCE_DELETION_GUARD_CONTRACT_VERSION,
        "reference_root": AIT_SERVER_REFERENCE_ROOT,
        "expected_reference_count": EXPECTED_REFERENCE_COUNT,
        "operations": [
            "audit",
            "classify",
            "fallback-decision",
            "migration-decision",
        ],
        "inventory": python_reference_inventory_json(),
        "package_markers": package_marker_modules(),
        "no_python_boundary": {
            "ci_check_id": NO_PYTHON_BOUNDARY_CHECK_ID,
            "ci_config": "ci/patch_ci.json",
            "command": "rg --files -uu -g '!.ait/**' -g '!.git/**' -g '!target/**' -g '*.py'",
            "allowed_result": "no output",
            "forbidden": "repo-local Python source, Python test harnesses, Python launchers, and Python fallback paths",
        },
        "cross_repo_audit": {
            "command": "rg --files ../ait/src/ait_server -g '*.py' | sort",
            "expected_count": EXPECTED_REFERENCE_COUNT,
            "disposition_source": "docs/sprints/server_python_reference_migration_task_sprints.md",
        },
        "fail_closed_policy": {
            "python_fallback_allowed": false,
            "migrated_authority": "Requests that require migrated ait-server Python authority must fail closed and route through Rust contracts/services.",
            "task_dag": "Task DAG is retired and must not be reintroduced as server authority.",
        },
        "migration_gate_policy": {
            "accepted_kinds": RUST_MIGRATION_KINDS,
            "forbidden_batch_actions": PYTHON_ONLY_BATCH_ACTIONS,
            "required_evidence": [
                "rust_owner",
                "rust_contracts",
                "rewired_callers for caller rewiring or deletion batches",
            ],
            "rule": "Python-only co-location, module merge, helper rehome, or import rewiring is not a migration; deletion requires Rust owner evidence first.",
        },
    })
}

pub fn python_reference_deletion_guard_json(
    operation: &str,
    request: &JsonValue,
) -> Result<JsonValue, String> {
    if operation == "contract" {
        return Ok(python_reference_deletion_guard_contract());
    }
    let payload = request.as_object().ok_or_else(|| {
        "python reference deletion guard payload must be a JSON object.".to_string()
    })?;
    match operation {
        "audit" => Ok(audit_python_references(payload)),
        "classify" => classify_python_reference(payload),
        "fallback-decision" => python_fallback_decision(payload),
        "migration-decision" => python_reference_migration_decision(payload),
        other => Err(format!(
            "Unsupported python reference deletion guard operation `{other}`."
        )),
    }
}

pub fn python_reference_inventory_json() -> Vec<JsonValue> {
    PYTHON_REFERENCE_INVENTORY
        .iter()
        .map(|entry| entry.to_json())
        .collect()
}

pub fn python_reference_modules() -> Vec<&'static str> {
    PYTHON_REFERENCE_INVENTORY
        .iter()
        .map(|entry| entry.module)
        .collect()
}

pub fn package_marker_modules() -> Vec<&'static str> {
    PYTHON_REFERENCE_INVENTORY
        .iter()
        .filter(|entry| entry.package_marker)
        .map(|entry| entry.module)
        .collect()
}

pub fn python_reference_disposition(module: &str) -> Option<PythonReferenceDisposition> {
    let module = normalize_module(module);
    PYTHON_REFERENCE_INVENTORY
        .iter()
        .copied()
        .find(|entry| entry.module == module)
}

fn audit_python_references(payload: &JsonMap<String, JsonValue>) -> JsonValue {
    let observed = array_texts(payload.get("references")).unwrap_or_default();
    let expected = python_reference_modules();
    let expected_set = expected.iter().copied().collect::<BTreeSet<_>>();
    let mut counts = BTreeMap::<String, usize>::new();
    for reference in &observed {
        *counts
            .entry(normalize_module(reference).to_string())
            .or_default() += 1;
    }
    let observed_set = counts.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let missing_expected = expected_set
        .difference(&observed_set)
        .copied()
        .collect::<Vec<_>>();
    let unknown_observed = observed_set
        .difference(&expected_set)
        .copied()
        .collect::<Vec<_>>();
    let duplicate_observed = counts
        .iter()
        .filter_map(|(module, count)| {
            if *count > 1 {
                Some(json!({"module": module, "count": count}))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let package_markers_observed = package_marker_modules()
        .into_iter()
        .filter(|module| observed_set.contains(module))
        .collect::<Vec<_>>();
    let matches_expected =
        missing_expected.is_empty() && unknown_observed.is_empty() && duplicate_observed.is_empty();

    json!({
        "contract": PYTHON_REFERENCE_DELETION_GUARD_CONTRACT_VERSION,
        "expected_count": expected.len(),
        "observed_count": observed.len(),
        "unique_observed_count": observed_set.len(),
        "matches_expected": matches_expected,
        "missing_expected": missing_expected,
        "unknown_observed": unknown_observed,
        "duplicate_observed": duplicate_observed,
        "package_markers_observed": package_markers_observed,
        "remaining_reference_count": observed.len(),
    })
}

fn classify_python_reference(payload: &JsonMap<String, JsonValue>) -> Result<JsonValue, String> {
    let module = required_text(payload.get("module"), "module")?;
    let disposition = python_reference_disposition(&module)
        .ok_or_else(|| format!("Unknown ait-server Python reference module: `{module}`"))?;
    Ok(json!({
        "contract": PYTHON_REFERENCE_DELETION_GUARD_CONTRACT_VERSION,
        "classification": disposition.to_json(),
    }))
}

fn python_fallback_decision(payload: &JsonMap<String, JsonValue>) -> Result<JsonValue, String> {
    let module = required_text(payload.get("module"), "module")?;
    let disposition = python_reference_disposition(&module)
        .ok_or_else(|| format!("Unknown ait-server Python reference module: `{module}`"))?;
    let requires_python_fallback =
        value_bool(payload.get("requires_python_fallback")).unwrap_or(true);
    if requires_python_fallback {
        return Err(format!(
            "Python fallback is forbidden for ait-server reference `{}` (disposition `{}`); route through Rust authority or keep it as documented ../ait compatibility glue.",
            disposition.module, disposition.disposition
        ));
    }
    Ok(json!({
        "contract": PYTHON_REFERENCE_DELETION_GUARD_CONTRACT_VERSION,
        "module": disposition.module,
        "python_fallback_allowed": false,
        "requires_python_fallback": false,
        "decision": "no_python_fallback_required",
    }))
}

fn python_reference_migration_decision(
    payload: &JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let module = required_text(payload.get("module"), "module")?;
    let disposition = python_reference_disposition(&module)
        .ok_or_else(|| format!("Unknown ait-server Python reference module: `{module}`"))?;
    if disposition.authority_status == "out_of_scope" {
        return Err(format!(
            "Reference `{}` is not an ait-server migration target; keep it out of Python reference deletion work.",
            disposition.module
        ));
    }

    let migration_kind = required_text(payload.get("migration_kind"), "migration_kind")?;
    if !RUST_MIGRATION_KINDS.contains(&migration_kind.as_str()) {
        return Err(format!(
            "Unsupported migration_kind `{migration_kind}`; expected one of: {}.",
            RUST_MIGRATION_KINDS.join(", ")
        ));
    }

    if value_bool(payload.get("python_only_change")).unwrap_or(false) {
        return Err(format!(
            "Python-only migration batches are forbidden for `{}`; implement or consume Rust authority before deleting Python.",
            disposition.module
        ));
    }

    let batch_actions = array_texts(payload.get("batch_actions")).unwrap_or_default();
    if let Some(action) = batch_actions
        .iter()
        .find(|action| PYTHON_ONLY_BATCH_ACTIONS.contains(&action.as_str()))
    {
        return Err(format!(
            "Python-only batch action `{action}` is forbidden for `{}`; co-location or import rewiring is not Rust migration evidence.",
            disposition.module
        ));
    }

    let rust_owner = required_text(payload.get("rust_owner"), "rust_owner")?;
    let rust_contracts = required_text_array(payload.get("rust_contracts"), "rust_contracts")?;
    let rewired_callers = if migration_kind == "caller_rewire_to_rust"
        || migration_kind == "delete_after_rust_owner"
    {
        required_text_array(payload.get("rewired_callers"), "rewired_callers")?
    } else {
        array_texts(payload.get("rewired_callers")).unwrap_or_default()
    };

    Ok(json!({
        "contract": PYTHON_REFERENCE_DELETION_GUARD_CONTRACT_VERSION,
        "module": disposition.module,
        "migration_kind": migration_kind,
        "authority_status": disposition.authority_status,
        "deletion_readiness": disposition.deletion_readiness,
        "rust_owner": rust_owner,
        "rust_contracts": rust_contracts,
        "rewired_callers": rewired_callers,
        "batch_actions": batch_actions,
        "python_only_change": false,
        "decision": "rust_migration_evidence_required_and_present",
    }))
}

fn normalize_module(module: &str) -> &str {
    module.trim()
}

fn array_texts(value: Option<&JsonValue>) -> Option<Vec<String>> {
    value.and_then(JsonValue::as_array).map(|values| {
        values
            .iter()
            .filter_map(|value| value.as_str().map(str::to_string))
            .collect()
    })
}

fn required_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{field} is required."))
}

fn required_text_array(value: Option<&JsonValue>, field: &str) -> Result<Vec<String>, String> {
    let values = array_texts(value).unwrap_or_default();
    let values = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if values.is_empty() {
        return Err(format!(
            "{field} must include at least one non-empty string."
        ));
    }
    Ok(values)
}

fn value_bool(value: Option<&JsonValue>) -> Option<bool> {
    value.and_then(JsonValue::as_bool)
}
