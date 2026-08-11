pub fn attestation_id_for_patchset(patchset_id: &str) -> Result<String, String> {
    let text = patchset_id.trim();
    if text.is_empty() {
        return Err("patchset_id is required".to_string());
    }
    Ok(format!("AT-{text}"))
}

pub fn land_submission_id_for_change(
    change_id: &str,
    prior_request_count: i64,
) -> Result<String, String> {
    let text = change_id.trim();
    if text.is_empty() {
        return Err("change_id is required".to_string());
    }
    if prior_request_count < 0 {
        return Err("prior_request_count must be non-negative".to_string());
    }
    Ok(format!("LAND-{text}-{:04}", prior_request_count + 1))
}
