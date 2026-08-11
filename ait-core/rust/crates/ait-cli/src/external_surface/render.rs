use ait_core::json_support::JsonValue;

pub fn render_external_text(command: &str, payload: &JsonValue) -> Result<String, String> {
    match command {
        "update" => render_external_update_text(payload),
        "status" => render_external_status_text(payload),
        "doctor" => render_external_doctor_text(payload),
        "link" => render_external_link_text(payload),
        "unlink" => render_external_unlink_text(payload),
        other => Err(format!("unknown external text renderer `{other}`")),
    }
}

pub fn render_external_update_text(payload: &JsonValue) -> Result<String, String> {
    let obj = payload
        .as_object()
        .ok_or_else(|| "external update payload must be an object".to_string())?;
    let mut lines = vec!["ait external update".to_string()];
    lines.push(format!("mode: {}", string_field(obj.get("mode"))));
    lines.push(format!(
        "locked: {} recursive: {} validated: {}",
        bool_field(obj.get("locked")),
        bool_field(obj.get("recursive")),
        bool_field(obj.get("validated"))
    ));
    if let Some(states) = obj.get("states").and_then(JsonValue::as_object) {
        lines.push(format!(
            "states: updated={} materialized={} unchanged={} validation_required={}",
            bool_field(states.get("updated")),
            bool_field(states.get("materialized")),
            bool_field(states.get("unchanged")),
            bool_field(states.get("validation_required"))
        ));
    }
    if let Some(changed_pins) = obj.get("changed_pins").and_then(JsonValue::as_array) {
        if !changed_pins.is_empty() {
            lines.push("changed pins:".to_string());
            for pin in changed_pins {
                let name = pin.get("name").and_then(JsonValue::as_str).unwrap_or("");
                let previous = pin
                    .get("previous_snapshot")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("");
                let new = pin
                    .get("new_snapshot")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("");
                lines.push(format!("- {name}: {previous} -> {new}"));
            }
        }
    }
    if let Some(entries) = obj
        .get("materialization")
        .and_then(|value| value.get("entries"))
        .and_then(JsonValue::as_array)
    {
        if !entries.is_empty() {
            lines.push("materialization:".to_string());
            for entry in entries {
                let state = entry.get("state").and_then(JsonValue::as_str).unwrap_or("");
                let name = entry.get("name").and_then(JsonValue::as_str).unwrap_or("");
                let snapshot = entry
                    .get("snapshot")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("");
                let path = entry
                    .get("materialize_to")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("");
                lines.push(format!("- {name} [{state}] {snapshot} -> {path}"));
            }
        }
    }
    if let Some(validation) = obj.get("validation").and_then(JsonValue::as_object) {
        if let Some(summary) = validation.get("summary").and_then(JsonValue::as_object) {
            lines.push(format!(
                "validation: passed={} errors={} warnings={}",
                bool_field(summary.get("passed")),
                integer_field(summary.get("errors")),
                integer_field(summary.get("warnings"))
            ));
        }
        if let Some(findings) = validation.get("findings").and_then(JsonValue::as_array) {
            if !findings.is_empty() {
                lines.push("validation findings:".to_string());
                for finding in findings {
                    let severity = finding
                        .get("severity")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("");
                    let code = finding
                        .get("code")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("");
                    let name = finding
                        .get("name")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("");
                    let path = finding
                        .get("path")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("");
                    lines.push(format!("- {severity} {code} {name} {path}"));
                }
            }
        }
    }
    Ok(lines.join("\n"))
}

pub fn render_external_status_text(payload: &JsonValue) -> Result<String, String> {
    let obj = payload
        .as_object()
        .ok_or_else(|| "external status payload must be an object".to_string())?;
    let mut lines = vec!["ait external status".to_string()];
    lines.push(format!("repo: {}", string_field(obj.get("repo_name"))));
    if let Some(summary) = obj.get("summary").and_then(JsonValue::as_object) {
        lines.push(format!(
            "summary: missing={} linked={} dirty={} outdated={} lock_drift={}",
            number_field(summary.get("missing")),
            number_field(summary.get("linked")),
            number_field(summary.get("dirty")),
            number_field(summary.get("outdated")),
            number_field(summary.get("lock_drift"))
        ));
    }
    if let Some(externals) = obj.get("externals").and_then(JsonValue::as_array) {
        if !externals.is_empty() {
            lines.push("externals:".to_string());
            for external in externals {
                let state = external
                    .get("state")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("");
                let name = external
                    .get("name")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("");
                let snapshot = external
                    .get("snapshot")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("");
                let path = external
                    .get("materialize_to")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("");
                let mut line = format!("- {name} [{state}] {snapshot} -> {path}");
                if let Some(link_path) = external.get("link_path").and_then(JsonValue::as_str) {
                    if !link_path.is_empty() {
                        line.push_str(&format!(" link={link_path}"));
                    }
                }
                lines.push(line);
            }
        }
    }
    if let Some(current_source) = obj
        .get("current_source_core")
        .and_then(JsonValue::as_object)
    {
        lines.push("current-source-core:".to_string());
        lines.push(format!(
            "- active_binary={} role={}",
            string_field(current_source.get("active_binary_path")),
            string_field(current_source.get("active_binary_role"))
        ));
        if let Some(summary) = current_source.get("summary").and_then(JsonValue::as_object) {
            lines.push(format!(
                "- ready={} missing={} stale={} wrong_binary={}",
                number_field(summary.get("ready")),
                number_field(summary.get("missing")),
                number_field(summary.get("stale")),
                number_field(summary.get("wrong_binary"))
            ));
        }
    }
    Ok(lines.join("\n"))
}

pub fn render_external_doctor_text(payload: &JsonValue) -> Result<String, String> {
    let obj = payload
        .as_object()
        .ok_or_else(|| "external doctor payload must be an object".to_string())?;
    let mut lines = vec!["ait external doctor".to_string()];
    lines.push(format!("repo: {}", string_field(obj.get("repo_name"))));
    lines.push(format!(
        "release_ready: {}",
        bool_field(obj.get("release_ready"))
    ));
    if let Some(checked) = obj.get("checked").and_then(JsonValue::as_object) {
        if checked
            .get("current_source_core")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
        {
            lines.push("checked: current-source-core".to_string());
        }
    }
    let findings = obj
        .get("findings")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let release_blocking = findings
        .iter()
        .filter(|finding| {
            finding
                .get("release_blocking")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    let warnings = findings
        .iter()
        .filter(|finding| finding.get("severity").and_then(JsonValue::as_str) == Some("warning"))
        .collect::<Vec<_>>();
    if !release_blocking.is_empty() {
        lines.push("release-blocking:".to_string());
        for finding in release_blocking {
            lines.push(format_finding(finding));
        }
    }
    if !warnings.is_empty() {
        lines.push("warnings:".to_string());
        for finding in warnings {
            lines.push(format_finding(finding));
        }
    }
    if findings.is_empty() {
        lines.push("findings: none".to_string());
    }
    Ok(lines.join("\n"))
}

pub fn render_external_link_text(payload: &JsonValue) -> Result<String, String> {
    let obj = payload
        .as_object()
        .ok_or_else(|| "external link payload must be an object".to_string())?;
    Ok(format!(
        "ait external link\nrepo: {}\nlinked: {} -> {}\nlinks: {}\nchanged: {}",
        string_field(obj.get("repo_name")),
        string_field(obj.get("name")),
        string_field(obj.get("path")),
        string_field(obj.get("links_path")),
        bool_field(obj.get("changed"))
    ))
}

pub fn render_external_unlink_text(payload: &JsonValue) -> Result<String, String> {
    let obj = payload
        .as_object()
        .ok_or_else(|| "external unlink payload must be an object".to_string())?;
    let mut lines = vec![format!(
        "ait external unlink\nrepo: {}\nunlinked: {}\nlinks: {}\nchanged: {}",
        string_field(obj.get("repo_name")),
        string_field(obj.get("name")),
        string_field(obj.get("links_path")),
        bool_field(obj.get("changed"))
    )];
    lines.push(format!(
        "restored: {} ({})",
        bool_field(obj.get("restored")),
        string_field(obj.get("restore_state"))
    ));
    if let Some(entries) = obj
        .get("materialization")
        .and_then(|value| value.get("entries"))
        .and_then(JsonValue::as_array)
    {
        if !entries.is_empty() {
            lines.push("materialization:".to_string());
            for entry in entries {
                let state = entry.get("state").and_then(JsonValue::as_str).unwrap_or("");
                let name = entry.get("name").and_then(JsonValue::as_str).unwrap_or("");
                let snapshot = entry
                    .get("snapshot")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("");
                let path = entry
                    .get("materialize_to")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("");
                lines.push(format!("- {name} [{state}] {snapshot} -> {path}"));
            }
        }
    }
    Ok(lines.join("\n"))
}

fn format_finding(finding: &JsonValue) -> String {
    format!(
        "- {} {} {}: {}",
        finding
            .get("code")
            .and_then(JsonValue::as_str)
            .unwrap_or("external_finding"),
        finding
            .get("name")
            .and_then(JsonValue::as_str)
            .unwrap_or(""),
        finding
            .get("path")
            .and_then(JsonValue::as_str)
            .unwrap_or(""),
        finding
            .get("message")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
    )
}

fn string_field(value: Option<&JsonValue>) -> String {
    value.and_then(JsonValue::as_str).unwrap_or("").to_string()
}

fn number_field(value: Option<&JsonValue>) -> i64 {
    value.and_then(JsonValue::as_i64).unwrap_or(0)
}

fn bool_field(value: Option<&JsonValue>) -> bool {
    value.and_then(JsonValue::as_bool).unwrap_or(false)
}

fn integer_field(value: Option<&JsonValue>) -> i64 {
    value.and_then(JsonValue::as_i64).unwrap_or(0)
}
