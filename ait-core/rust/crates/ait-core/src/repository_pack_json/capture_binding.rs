use crate::json_support::{json, JsonValue};
use crate::workflow_binary_db::SnapshotWorkflowBinding;

pub const SNAPSHOT_CAPTURE_BINDING_CONTRACT: &str = "ait.snapshot_capture_binding.v2";

/// A capture request, never a substitute for a persisted server receipt.
/// The repository index scopes every workflow and content physical index.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotCaptureBinding {
    pub repository_index: u32,
    pub snapshot_id: String,
    pub owner: SnapshotWorkflowBinding,
}

impl SnapshotCaptureBinding {
    pub fn to_value(&self) -> JsonValue {
        json!({
            "contract": SNAPSHOT_CAPTURE_BINDING_CONTRACT,
            "repository_index": self.repository_index,
            "snapshot_id": self.snapshot_id,
            "task_id": self.owner.task_id,
            "change_ref": (!self.owner.change_id.is_empty()).then(|| format!("{}/{}", self.owner.task_id, self.owner.change_id)),
            "line_name": self.owner.line_name,
        })
    }

    /// Existing disposable pending requests may predate v2. Only their exact
    /// immutable identity is comparable; server readback still proves ownership.
    pub fn matches_recovery_value(&self, value: &JsonValue) -> bool {
        let mut value = value.clone();
        if value["contract"].as_str() == Some("ait.snapshot_capture_binding.v1") {
            value["contract"] = JsonValue::String(SNAPSHOT_CAPTURE_BINDING_CONTRACT.into());
            if let Some(object) = value.as_object_mut() {
                for field in ["worktree_name", "author_mode", "model_name"] {
                    object.remove(field);
                }
            }
        }
        Self::from_value(&value).is_ok_and(|binding| binding == *self)
    }

    pub fn from_value(value: &JsonValue) -> Result<Self, String> {
        let text = |key: &str| -> Result<String, String> {
            value[key]
                .as_str()
                .filter(|s| !s.trim().is_empty() && s.len() <= 4096)
                .map(str::to_string)
                .ok_or_else(|| format!("Capture binding requires {key}"))
        };
        if ["worktree_name", "author_mode", "model_name"]
            .iter()
            .any(|field| value.get(*field).is_some())
        {
            return Err("Capture v2 has no historical worktree/author/model fields".into());
        }
        if text("contract")? != SNAPSHOT_CAPTURE_BINDING_CONTRACT {
            return Err("Unsupported Snapshot capture binding contract".to_string());
        }
        let task_id = text("task_id")?;
        let change_id = match value.get("change_ref").filter(|value| !value.is_null()) {
            None => String::new(),
            Some(value) => {
                let reference = value
                    .as_str()
                    .ok_or("Capture Change reference must be text or null")?;
                let (owner, change) = reference
                    .rsplit_once('/')
                    .ok_or("Capture binding requires a complete Change reference")?;
                if owner != task_id
                    || crate::change_json::ChangeJson::stateless().canonical_change_id(change)?
                        != change
                {
                    return Err("Capture binding Change does not belong to its Task".into());
                }
                change.to_string()
            }
        };
        let snapshot_id = text("snapshot_id")?;
        crate::content_binary_db::snapshot_hash48_from_id(&snapshot_id)?;
        Ok(Self {
            repository_index: value["repository_index"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or("Capture binding requires a numeric repository index")?,
            snapshot_id,
            owner: SnapshotWorkflowBinding {
                task_id,
                change_id,
                line_name: text("line_name")?,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v2_optional_change_and_exact_pending_retry_never_use_historical_names() {
        let mut binding = SnapshotCaptureBinding {
            repository_index: 0,
            snapshot_id: "SNP-000000000001".into(),
            owner: SnapshotWorkflowBinding {
                task_id: "RCT-0001".into(),
                change_id: String::new(),
                line_name: "feature/original-author".into(),
            },
        };
        let task_only = binding.to_value();
        assert!(task_only["change_ref"].is_null());
        for field in ["worktree_name", "author_mode", "model_name"] {
            assert!(task_only.get(field).is_none());
        }
        assert_eq!(
            SnapshotCaptureBinding::from_value(&task_only).unwrap(),
            binding
        );
        binding.owner.change_id = "C-01".into();
        let value = binding.to_value();
        assert_eq!(value["change_ref"], "RCT-0001/C-01");
        assert_eq!(
            SnapshotCaptureBinding::from_value(&value)
                .unwrap()
                .owner
                .change_id,
            "C-01"
        );
        let mut legacy = value.clone();
        legacy["contract"] = json!("ait.snapshot_capture_binding.v1");
        legacy["worktree_name"] = json!("old-name-3");
        legacy["author_mode"] = json!("old-author");
        assert!(SnapshotCaptureBinding::from_value(&legacy).is_err());
        assert!(binding.matches_recovery_value(&legacy));
        let mut other_repository = legacy.clone();
        other_repository["repository_index"] = json!(1);
        assert!(!binding.matches_recovery_value(&other_repository));
        for field in ["worktree_name", "author_mode", "model_name"] {
            let mut obsolete = value.clone();
            obsolete[field] = json!("obsolete");
            assert!(
                SnapshotCaptureBinding::from_value(&obsolete).is_err(),
                "{field}"
            );
        }
        for field in ["task_id", "snapshot_id", "line_name", "change_ref"] {
            let mut foreign = legacy.clone();
            foreign[field] = json!("foreign");
            assert!(!binding.matches_recovery_value(&foreign), "{field}");
        }
        let mut foreign = value;
        foreign["change_ref"] = json!("RCT-0002/C-01");
        assert!(SnapshotCaptureBinding::from_value(&foreign).is_err());
    }
}
