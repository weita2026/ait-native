use super::*;

/// Exact caller-provided identity of one package CI execution. This is a
/// process-boundary input/output value, never a persisted workflow record.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageWorkflowExpectedIdentity {
    pub repository_index: u32,
    pub package_hash: String,
    pub worker_job_index: u32,
    pub base_snapshot_id: String,
    pub revision_snapshot_id: String,
}

impl PackageWorkflowExpectedIdentity {
    pub fn validate(&self) -> Result<(), String> {
        if self.package_hash.len() != 64
            || !self
                .package_hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(
                "Expected package_hash must be exactly 64 lowercase SHA-256 hex characters".into(),
            );
        }
        for (field, id) in [
            ("base_snapshot_id", &self.base_snapshot_id),
            ("revision_snapshot_id", &self.revision_snapshot_id),
        ] {
            if !id.is_ascii()
                || id.trim() != id
                || crate::content_binary_db::snapshot_hash48_from_id(id).is_err()
            {
                return Err(format!(
                    "Expected package {field} must use existing Snapshot ID syntax"
                ));
            }
        }
        Ok(())
    }

    pub fn from_value(value: &Value) -> Result<Self, String> {
        let identity: Self = serde_json::from_value(value.clone())
            .map_err(|error| format!("Invalid expected package identity: {error}"))?;
        identity.validate()?;
        Ok(identity)
    }

    pub fn to_value(&self) -> Value {
        serde_json::json!(self)
    }
}

/// Keep the existing strict formal-Patchset form separate. In particular, a
/// package pin cannot be downgraded into an unguarded or Patchset-only request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WorkflowExpectedIdentityInput {
    Package(PackageWorkflowExpectedIdentity),
    Patchset(WorkflowExpectedIdentity),
}

impl WorkflowExpectedIdentityInput {
    pub fn from_value(value: &Value) -> Result<Self, String> {
        let identity: Self = serde_json::from_value(value.clone())
            .map_err(|error| format!("Invalid expected candidate identity: {error}"))?;
        identity.validate()?;
        Ok(identity)
    }

    /// Decode the file directly so duplicate keys cannot silently overwrite an
    /// earlier pin while being converted to a generic JSON object.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, String> {
        let identity: Self = serde_json::from_slice(bytes)
            .map_err(|error| format!("Invalid expected candidate identity JSON: {error}"))?;
        identity.validate()?;
        Ok(identity)
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Package(identity) => identity.validate(),
            Self::Patchset(identity) => identity.validate(),
        }
    }
}

/// Fresh ready may select a Job once. After selection, every retry is exact
/// recovery of that same tuple, including Job index zero. Missing recovery is
/// handled by the adapter as required action; it cannot reset this selection.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PackageReadySelection {
    selected: Option<PackageWorkflowExpectedIdentity>,
}

impl PackageReadySelection {
    pub fn fresh() -> Self {
        Self::default()
    }

    pub fn recover(identity: PackageWorkflowExpectedIdentity) -> Result<Self, String> {
        identity.validate()?;
        Ok(Self {
            selected: Some(identity),
        })
    }

    pub fn may_select_job(&self) -> bool {
        self.selected.is_none()
    }

    pub fn expected_identity(&self) -> Option<&PackageWorkflowExpectedIdentity> {
        self.selected.as_ref()
    }

    pub fn select(&mut self, identity: PackageWorkflowExpectedIdentity) -> Result<(), String> {
        identity.validate()?;
        if self
            .selected
            .as_ref()
            .is_some_and(|expected| expected != &identity)
        {
            return Err(
                "EXPECTED_CANDIDATE_MISMATCH: ready must recover the original package/Job identity"
                    .into(),
            );
        }
        self.selected = Some(identity);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
