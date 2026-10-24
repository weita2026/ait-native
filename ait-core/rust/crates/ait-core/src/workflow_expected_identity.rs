//! Optional caller pin for a sealed remote workflow candidate. No persisted schema.
use serde::{Deserialize, Serialize};
use serde_json::Value;

mod package;
pub use package::{
    PackageReadySelection, PackageWorkflowExpectedIdentity, WorkflowExpectedIdentityInput,
};

pub const GUARDED_TASK_LAND_CONTRACT: &str = "task-land-atomic-guarded/v2";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowExpectedIdentity {
    pub repository_index: u32,
    pub patchset_id: String,
    pub revision_snapshot_id: String,
    pub base_snapshot_id: String,
}

pub fn canonical_patchset_id(id: &str) -> String {
    id.to_string()
}

pub fn validate_patchset_owner_fields(
    patchset: &Value,
    task_id: &str,
) -> Result<Option<String>, String> {
    if patchset["task_id"].as_str() != Some(task_id) {
        return Err("Patchset belongs to another Task".into());
    }
    let optional = |field: &str| -> Result<Option<String>, String> {
        match patchset.get(field) {
            Some(Value::Null) => Ok(None),
            Some(Value::String(text)) if !text.is_empty() && text.trim() == text => {
                Ok(Some(text.clone()))
            }
            _ => Err(format!("Patchset requires nullable exact {field}")),
        }
    };
    let change = optional("change_ref")?;
    let child = optional("change_id")?;
    if change
        .as_deref()
        .and_then(|id| id.rsplit_once('/').map(|p| p.1))
        != child.as_deref()
    {
        return Err("Patchset optional Change fields disagree".into());
    }
    validate_patchset_reference_owner(
        patchset["patchset_id"]
            .as_str()
            .ok_or("Patchset identity missing")?,
        task_id,
        change.as_deref(),
    )?;
    Ok(change)
}

pub fn validate_patchset_reference_owner(
    patchset_id: &str,
    task_id: &str,
    change_ref: Option<&str>,
) -> Result<(), String> {
    if let Some(change) = change_ref {
        let (owner, child) = change.rsplit_once('/').ok_or("Change has no Task owner")?;
        if owner != task_id
            || crate::change_json::ChangeJson::stateless().canonical_change_id(child)? != child
        {
            return Err("Patchset Change belongs to another Task".into());
        }
    }
    let (owner, suffix) = patchset_id
        .rsplit_once("/P-")
        .ok_or("Patchset has no exact public owner")?;
    let ordinal: u8 = suffix.parse().map_err(|_| "Invalid Patchset ordinal")?;
    if !(1..=64).contains(&ordinal)
        || suffix != format!("{ordinal:02}")
        || (owner != task_id && Some(owner) != change_ref)
    {
        return Err("Patchset reference disagrees with actual Task/optional Change".into());
    }
    Ok(())
}

impl WorkflowExpectedIdentity {
    pub fn validate_patchset_reference(&self, patchset: &Value) -> Result<(), String> {
        let actual = patchset["patchset_id"]
            .as_str()
            .ok_or("Patchset identity is missing")?;
        if actual == self.patchset_id {
            return Ok(());
        }
        let task = patchset["task_id"]
            .as_str()
            .ok_or("Patchset alias requires its actual Task owner")?;
        let change = match patchset.get("change_ref") {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) => Some(value.as_str()),
            _ => return Err("Patchset Change must be exact text or null".into()),
        };
        validate_patchset_reference_owner(actual, task, change)?;
        validate_patchset_reference_owner(&self.patchset_id, task, change)?;
        if actual.rsplit_once("/P-").map(|p| p.1)
            != self.patchset_id.rsplit_once("/P-").map(|p| p.1)
        {
            return Err("EXPECTED_CANDIDATE_MISMATCH: Patchset ordinal differs".into());
        }
        Ok(())
    }
    pub fn from_value(value: &Value) -> Result<Self, String> {
        let identity: Self = serde_json::from_value(value.clone())
            .map_err(|error| format!("Invalid expected candidate identity: {error}"))?;
        identity.validate()?;
        Ok(identity)
    }

    pub fn validate(&self) -> Result<(), String> {
        for (field, text) in [
            ("patchset_id", self.patchset_id.as_str()),
            ("revision_snapshot_id", self.revision_snapshot_id.as_str()),
            ("base_snapshot_id", self.base_snapshot_id.as_str()),
        ] {
            if text.trim().is_empty() || text != text.trim() {
                return Err(format!(
                    "Expected candidate {field} must be nonempty without surrounding whitespace."
                ));
            }
        }
        Ok(())
    }

    pub fn validate_patchset(&self, repository_index: u32, patchset: &Value) -> Result<(), String> {
        self.validate()?;
        if repository_index != self.repository_index {
            return Err("EXPECTED_CANDIDATE_MISMATCH: repository_index differs.".to_string());
        }
        self.validate_patchset_reference(patchset)?;
        for (field, expected) in [
            ("revision_snapshot_id", self.revision_snapshot_id.as_str()),
            ("base_snapshot_id", self.base_snapshot_id.as_str()),
        ] {
            if patchset.get(field).and_then(Value::as_str) != Some(expected) {
                return Err(format!("EXPECTED_CANDIDATE_MISMATCH: {field} differs."));
            }
        }
        if ["withdrawn", "invalidated"]
            .iter()
            .any(|field| patchset.get(field).and_then(Value::as_bool) == Some(true))
            || matches!(
                patchset.get("status").and_then(Value::as_str),
                Some("withdrawn" | "invalidated")
            )
        {
            return Err(
                "EXPECTED_CANDIDATE_MISMATCH: Patchset is withdrawn or invalidated.".to_string(),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn patchset_owner_validation_never_rewrites_an_initial_change_alias() {
        assert!(validate_patchset_reference_owner("RT-1/P-01", "RT-1", None).is_ok());
        assert!(
            validate_patchset_reference_owner("RT-1/C-02/P-01", "RT-1", Some("RT-1/C-02")).is_ok()
        );
        assert!(validate_patchset_reference_owner("RT-1/C-01/P-01", "RT-1", None).is_err());
        assert!(
            validate_patchset_reference_owner("RT-1/C-01/P-01", "RT-1", Some("RT-1/C-02")).is_err()
        );
        assert!(validate_patchset_reference_owner("RT-2/P-01", "RT-1", None).is_err());
    }

    #[test]
    fn public_task_ordinal_matches_machine_reference_only_with_actual_owner_evidence() {
        let expected = WorkflowExpectedIdentity {
            repository_index: 7,
            patchset_id: "RT-1/P-02".into(),
            base_snapshot_id: "B".into(),
            revision_snapshot_id: "R".into(),
        };
        let mut actual = json!({"patchset_id":"RT-1/C-02/P-02","task_id":"RT-1","change_ref":"RT-1/C-02","base_snapshot_id":"B","revision_snapshot_id":"R"});
        expected.validate_patchset(7, &actual).unwrap();
        actual["change_ref"] = json!("RT-1/C-01");
        assert!(expected.validate_patchset(7, &actual).is_err());
        actual["change_ref"] = Value::Null;
        assert!(expected.validate_patchset(7, &actual).is_err());
    }
    #[test]
    fn identity_requires_complete_tuple_and_rejects_collisions_or_revision_change() {
        let value = json!({"repository_index":7,"patchset_id":"RT-1/P-01","revision_snapshot_id":"SNP-R","base_snapshot_id":"SNP-B"});
        let expected = WorkflowExpectedIdentity::from_value(&value).unwrap();
        let mut patchset = value;
        patchset["patchset_id"] = json!("RT-1/C-01/P-01");
        assert!(expected.validate_patchset(7, &patchset).is_err());
        patchset["patchset_id"] = json!("RT-1/P-01");
        expected.validate_patchset(7, &patchset).unwrap();
        assert!(expected.validate_patchset(8, &patchset).is_err());
        patchset["revision_snapshot_id"] = json!("SNP-OTHER");
        assert!(expected.validate_patchset(7, &patchset).is_err());
        assert!(WorkflowExpectedIdentity::from_value(&json!({"repository_index":7})).is_err());
    }

    #[test]
    fn identity_rejects_every_mixed_field_and_non_live_patchset() {
        let value = json!({"repository_index":7,"patchset_id":"RT-1/P-01","revision_snapshot_id":"R","base_snapshot_id":"B"});
        let expected = WorkflowExpectedIdentity::from_value(&value).unwrap();
        for (field, replacement) in [
            ("patchset_id", json!("RT-2/P-01")),
            ("revision_snapshot_id", json!("R2")),
            ("base_snapshot_id", json!("B2")),
            ("invalidated", json!(true)),
            ("withdrawn", json!(true)),
        ] {
            let mut mixed = value.clone();
            mixed[field] = replacement;
            assert!(expected.validate_patchset(7, &mixed).is_err(), "{field}");
        }
        let mut malformed = expected;
        malformed.patchset_id.clear();
        assert!(malformed.validate().is_err());
        assert!(WorkflowExpectedIdentity::from_value(&json!({"repository_index":"7","patchset_id":"P","revision_snapshot_id":"R","base_snapshot_id":"B"})).is_err());
    }
}
