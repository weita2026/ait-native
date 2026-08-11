use super::*;

pub fn land_freshness_result(
    target_line: &str,
    patchset: &JsonMap<String, JsonValue>,
    target_line_head: Option<&str>,
    alignment: Option<&JsonMap<String, JsonValue>>,
    checked_at: &str,
) -> JsonMap<String, JsonValue> {
    let normalized_target = target_line_head
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let expected_base_snapshot_id = patchset
        .get("base_snapshot_id")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let revision_snapshot_id = patchset
        .get("revision_snapshot_id")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    let fallback_target_matches_revision_snapshot = normalized_target.as_deref().is_some()
        && revision_snapshot_id.as_deref().is_some()
        && normalized_target == revision_snapshot_id;
    let target_matches_revision_snapshot = alignment_bool_or_fallback(
        alignment,
        "target_matches_revision_snapshot",
        fallback_target_matches_revision_snapshot,
    );
    let target_matches_revision_tree = alignment_bool_or_fallback(
        alignment,
        "target_matches_revision_tree",
        target_matches_revision_snapshot,
    );

    let mut result = JsonMap::new();
    result.insert(
        "checked_at".to_string(),
        JsonValue::String(checked_at.to_string()),
    );
    result.insert(
        "target_line".to_string(),
        JsonValue::String(target_line.to_string()),
    );
    result.insert(
        "target_line_head".to_string(),
        normalized_target
            .clone()
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    result.insert(
        "expected_base_snapshot_id".to_string(),
        expected_base_snapshot_id
            .clone()
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    result.insert(
        "revision_snapshot_id".to_string(),
        revision_snapshot_id
            .clone()
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    result.insert(
        "base_is_fresh".to_string(),
        JsonValue::Bool(
            normalized_target.as_deref().is_some()
                && expected_base_snapshot_id.as_deref().is_some()
                && normalized_target == expected_base_snapshot_id,
        ),
    );
    result.insert(
        "target_matches_revision_snapshot".to_string(),
        JsonValue::Bool(target_matches_revision_snapshot),
    );
    result.insert(
        "target_matches_revision_tree".to_string(),
        JsonValue::Bool(target_matches_revision_tree),
    );
    result.insert(
        "already_aligned_equivalent".to_string(),
        JsonValue::Bool(target_matches_revision_tree),
    );
    result
}

pub fn land_snapshot_alignment(
    target_line_head: Option<&str>,
    revision_snapshot_id: Option<&str>,
    target_manifest_hash: Option<&str>,
    revision_manifest_hash: Option<&str>,
    target_root_tree_id: Option<&str>,
    revision_root_tree_id: Option<&str>,
) -> JsonMap<String, JsonValue> {
    let normalized_target = normalize_optional_text(target_line_head);
    let normalized_revision = normalize_optional_text(revision_snapshot_id);
    let target_matches_revision_snapshot = normalized_target.as_deref().is_some()
        && normalized_revision.as_deref().is_some()
        && normalized_target == normalized_revision;

    let (
        normalized_target_manifest_hash,
        normalized_revision_manifest_hash,
        normalized_target_root_tree_id,
        normalized_revision_root_tree_id,
    ) = if target_matches_revision_snapshot {
        (None, None, None, None)
    } else {
        (
            normalize_optional_text(target_manifest_hash),
            normalize_optional_text(revision_manifest_hash),
            normalize_optional_text(target_root_tree_id),
            normalize_optional_text(revision_root_tree_id),
        )
    };
    let root_trees_match = normalized_target_root_tree_id.as_deref().is_some()
        && normalized_revision_root_tree_id.as_deref().is_some()
        && normalized_target_root_tree_id == normalized_revision_root_tree_id;
    let target_matches_revision_tree = normalized_target.as_deref().is_some()
        && normalized_revision.as_deref().is_some()
        && (target_matches_revision_snapshot || root_trees_match);

    JsonMap::from_iter([
        (
            "target_line_head".to_string(),
            json_string_or_null(normalized_target),
        ),
        (
            "revision_snapshot_id".to_string(),
            json_string_or_null(normalized_revision),
        ),
        (
            "target_manifest_hash".to_string(),
            json_string_or_null(normalized_target_manifest_hash),
        ),
        (
            "revision_manifest_hash".to_string(),
            json_string_or_null(normalized_revision_manifest_hash),
        ),
        (
            "target_root_tree_id".to_string(),
            json_string_or_null(normalized_target_root_tree_id),
        ),
        (
            "revision_root_tree_id".to_string(),
            json_string_or_null(normalized_revision_root_tree_id),
        ),
        (
            "target_matches_revision_snapshot".to_string(),
            JsonValue::Bool(target_matches_revision_snapshot),
        ),
        (
            "target_matches_revision_tree".to_string(),
            JsonValue::Bool(target_matches_revision_tree),
        ),
        (
            "already_aligned_equivalent".to_string(),
            JsonValue::Bool(target_matches_revision_tree),
        ),
    ])
}

fn alignment_bool_or_fallback(
    alignment: Option<&JsonMap<String, JsonValue>>,
    key: &str,
    fallback: bool,
) -> bool {
    match alignment {
        Some(payload) if !payload.is_empty() => payload
            .get(key)
            .and_then(JsonValue::as_bool)
            .unwrap_or(false),
        _ => fallback,
    }
}
