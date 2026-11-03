//! Concrete bindings for existing bounded maintenance operations.
//! The underlying conversion and activation authority is unchanged.
use ait_core::binary_db_generation::{
    activate_binary_db_generation, stage_binary_db_u64_second_upgrade,
    BinaryDbGenerationActivationOptions, StageBinaryDbU64SecondUpgradeOptions,
};
use ait_core::json_support::{json, JsonValue};
use std::path::PathBuf;

pub fn convert_agent_pool_50(
    repo_root: PathBuf,
    generation_root: PathBuf,
    activate: bool,
) -> Result<JsonValue, String> {
    if ait_core::binary_db_generation::agent_pool_50_complete(&repo_root)? {
        return Ok(
            json!({"schema":"ait.agent-pool-50-conversion.v1","status":"already_converted","activated":true}),
        );
    }
    if generation_root.exists() {
        ait_core::binary_db_generation::agent_pool_50_generation_valid(&generation_root)?;
        let activation = if activate {
            Some(activate_binary_db_generation(
                BinaryDbGenerationActivationOptions {
                    repo_root,
                    generation_root: generation_root.clone(),
                    expected_current_authority_fingerprint: None,
                },
            )?)
        } else {
            None
        };
        return Ok(json!({
            "schema":"ait.agent-pool-50-conversion.v1", "status":"staged_retry",
            "generation_root":generation_root.display().to_string(), "activated":activation.is_some(),
            "retained_previous_authority":activation.as_ref().and_then(|x| x.retained_previous_authority.clone()),
        }));
    }
    let staged = ait_core::binary_db_generation::stage_agent_pool_50(
        ait_core::binary_db_generation::AgentPool50Options {
            repo_root: repo_root.clone(),
            output_root: generation_root.clone(),
        },
    )?;
    let activation = if activate {
        Some(activate_binary_db_generation(
            BinaryDbGenerationActivationOptions {
                repo_root,
                generation_root,
                expected_current_authority_fingerprint: Some(
                    staged.source_authority_fingerprint.clone(),
                ),
            },
        )?)
    } else {
        None
    };
    Ok(json!({
        "schema":"ait.agent-pool-50-conversion.v1", "source_authority_fingerprint":staged.source_authority_fingerprint,
        "content_fingerprint":staged.content_fingerprint, "generation_root":staged.generation_root.display().to_string(),
        "activated":activation.is_some(), "retained_previous_authority":activation.as_ref().and_then(|x| x.retained_previous_authority.clone()),
    }))
}

pub fn upgrade_u64_seconds(
    repo_root: PathBuf,
    generation_root: PathBuf,
    source_time_width: String,
    activate: bool,
) -> Result<JsonValue, String> {
    let staged = stage_binary_db_u64_second_upgrade(StageBinaryDbU64SecondUpgradeOptions {
        repo_root: repo_root.clone(),
        output_root: generation_root.clone(),
        source_time_width,
    })?;
    let activation = if activate {
        Some(activate_binary_db_generation(
            BinaryDbGenerationActivationOptions {
                repo_root,
                generation_root,
                expected_current_authority_fingerprint: Some(
                    staged.source_authority_fingerprint.clone(),
                ),
            },
        )?)
    } else {
        None
    };
    Ok(json!({
        "schema":"ait.binary-db-u64-second-upgrade.v1", "repo_name":staged.repo_name,
        "source_time_width":staged.source_time_width, "target_time_width":staged.target_time_width,
        "source_authority_fingerprint":staged.source_authority_fingerprint, "content_fingerprint":staged.content_fingerprint,
        "converted_file_count":staged.converted_file_count, "rebuilt_index_file_count":staged.rebuilt_index_file_count,
        "copied_file_count":staged.copied_file_count, "source_bytes":staged.source_bytes, "target_bytes":staged.target_bytes,
        "generation_root":staged.output_root.display().to_string(), "activated":activation.is_some(),
        "authority_root":activation.as_ref().map(|value| value.authority_root.clone()),
        "retained_previous_authority":activation.as_ref().and_then(|value| value.retained_previous_authority.clone()),
        "activation_strategy":activation.as_ref().map(|value| value.activation_strategy.clone()),
    }))
}

pub fn format_agent_pool_50(payload: &JsonValue) -> String {
    use crate::render::{cell, key_values_text};
    let title = "ait binary-db agent-pool50";
    match payload["status"].as_str() {
        Some("already_converted") => {
            key_values_text(title, &[("status", "already converted".into())])
        }
        Some("staged_retry") => key_values_text(
            title,
            &[
                ("status", "staged retry".into()),
                ("activated", cell(payload.get("activated"))),
            ],
        ),
        _ => key_values_text(
            title,
            &[
                ("generation root", cell(payload.get("generation_root"))),
                ("activated", cell(payload.get("activated"))),
                (
                    "retained previous authority",
                    cell(payload.get("retained_previous_authority")),
                ),
            ],
        ),
    }
}
pub fn format_upgrade_u64_seconds(payload: &JsonValue) -> String {
    use crate::render::{cell, key_values_text};
    key_values_text(
        "ait binary-db upgrade-u64-seconds",
        &[
            ("repository", cell(payload.get("repo_name"))),
            ("source time width", cell(payload.get("source_time_width"))),
            ("target time width", cell(payload.get("target_time_width"))),
            ("generation root", cell(payload.get("generation_root"))),
            ("source bytes", cell(payload.get("source_bytes"))),
            ("target bytes", cell(payload.get("target_bytes"))),
            (
                "rebuilt content indexes",
                cell(payload.get("rebuilt_index_file_count")),
            ),
            ("activated", cell(payload.get("activated"))),
            (
                "retained previous authority",
                cell(payload.get("retained_previous_authority")),
            ),
        ],
    )
}
