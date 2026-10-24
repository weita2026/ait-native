use super::PreparedPackageEvidence;
use crate::foundation::workflow_package_contract::PackageEvidenceOperation;
use std::collections::BTreeMap;

// These are the same principals materialized by the existing formal Review
// writer. Request Actor scopes authorization/idempotency, not a bot's identity.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Principal {
    Human(u32),
    Bot(String),
}

#[derive(Default)]
struct General {
    blocked: bool,
    summary: bool,
    user_approval: bool,
}

#[derive(Default)]
struct Task {
    blocked: bool,
    approval: bool,
}

#[derive(Default)]
struct Lanes {
    general: General,
    task: Task,
}

#[derive(Default)]
pub(crate) struct PackageReviews {
    principals: BTreeMap<Principal, Lanes>,
    invalid: bool,
}

impl PackageReviews {
    pub(crate) fn has_objection(&self) -> bool {
        self.invalid
            || self
                .principals
                .values()
                .any(|p| p.general.blocked || p.task.blocked)
    }

    pub(crate) fn require_complete(&self, require_human: bool) -> Result<(), String> {
        if self.has_objection() {
            return Err("PACKAGE_REVIEW_BLOCKED: an exact reviewer lane is unresolved".into());
        }
        if !self.principals.values().any(|p| p.general.summary) {
            return Err(
                "PACKAGE_CODE_REVIEW_REQUIRED: current nonblocking code_review_summary is missing"
                    .into(),
            );
        }
        if !self.principals.values().any(|p| p.task.approval) {
            return Err("PACKAGE_TASK_APPROVAL_REQUIRED: current task_approve is missing".into());
        }
        if require_human && !self.principals.values().any(|p| p.general.user_approval) {
            return Err(
                "PACKAGE_HUMAN_REVIEW_REQUIRED: current General user approval is missing".into(),
            );
        }
        Ok(())
    }
}

pub(crate) fn fold_package_reviews(actions: &[PreparedPackageEvidence]) -> PackageReviews {
    let mut state = PackageReviews::default();
    for event in actions {
        if event.request.operation != PackageEvidenceOperation::Review {
            continue;
        }
        let payload = &event.request.payload;
        let reviewer = payload
            .get("reviewer")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let principal = match payload
            .get("reviewer_kind")
            .and_then(|v| v.as_str())
            .unwrap_or("user")
        {
            "user" if event.actor_index_plus1 != 0 && !reviewer.is_empty() => {
                Principal::Human(event.actor_index_plus1)
            }
            "bot" if !reviewer.is_empty() => Principal::Bot(reviewer.into()),
            _ => {
                state.invalid = true;
                continue;
            }
        };
        let human = matches!(principal, Principal::Human(_));
        let lanes = state.principals.entry(principal).or_default();
        let action = payload
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let blocking = payload
            .get("blocking")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        match action {
            "task_comment" | "task_approve" | "task_request_changes" | "task_defer" => {
                if blocking || matches!(action, "task_request_changes" | "task_defer") {
                    lanes.task.blocked = true;
                    lanes.task.approval = false;
                } else if action == "task_approve" {
                    lanes.task.blocked = false;
                    lanes.task.approval = true;
                }
            }
            "comment"
            | "code_review_summary"
            | "approve"
            | "request_changes"
            | "dismiss"
            | "defer" => {
                if blocking || matches!(action, "request_changes" | "dismiss" | "defer") {
                    lanes.general.blocked = true;
                    lanes.general.summary = false;
                    lanes.general.user_approval = false;
                } else if action == "code_review_summary" {
                    lanes.general.blocked = false;
                    lanes.general.summary = true;
                } else if action == "approve" {
                    lanes.general.blocked = false;
                    lanes.general.user_approval = human;
                }
            }
            _ => state.invalid = true,
        }
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::workflow_package_contract::{
        PackageEvidenceRequest, PackageExpectedIdentity,
    };
    use serde_json::json;

    fn event(
        actor: u32,
        reviewer: &str,
        kind: &str,
        action: &str,
        blocking: bool,
    ) -> PreparedPackageEvidence {
        PreparedPackageEvidence {
            request: PackageEvidenceRequest {
                contract: "workflow-package-evidence/v1".into(),
                operation: PackageEvidenceOperation::Review,
                idempotency_key: action.into(), actor_key_index: actor-1,
                expected_identity: PackageExpectedIdentity {
                    repository_index: 0, package_hash: "00".repeat(32), worker_job_index: 0,
                    base_snapshot_id: "SNP-000000000001".into(), revision_snapshot_id: "SNP-000000000002".into(),
                },
                payload: json!({"reviewer":reviewer,"reviewer_kind":kind,"action":action,"blocking":blocking}).as_object().unwrap().clone(),
            }, actor_index_plus1: actor, recorded_at_s: 1,
        }
    }

    fn bot(action: &str) -> PreparedPackageEvidence {
        event(1, "Reviewer", "bot", action, false)
    }

    #[test]
    fn summary_and_task_acceptance_are_independent_requirements() {
        for actions in [
            vec![],
            vec![bot("approve")],
            vec![bot("code_review_summary")],
            vec![bot("task_approve")],
        ] {
            assert!(fold_package_reviews(&actions)
                .require_complete(false)
                .is_err());
        }
        assert!(
            fold_package_reviews(&[bot("code_review_summary"), bot("task_approve")])
                .require_complete(false)
                .is_ok()
        );
    }

    #[test]
    fn general_objection_requires_fresh_summary_and_cannot_be_cleared_by_task_lane() {
        let mut actions = vec![
            bot("code_review_summary"),
            bot("task_approve"),
            bot("request_changes"),
            bot("task_approve"),
        ];
        assert!(fold_package_reviews(&actions).has_objection());
        actions.push(bot("approve"));
        assert!(!fold_package_reviews(&actions).has_objection());
        assert!(fold_package_reviews(&actions)
            .require_complete(false)
            .is_err());
        actions.push(bot("code_review_summary"));
        assert!(fold_package_reviews(&actions)
            .require_complete(false)
            .is_ok());
    }

    #[test]
    fn canonical_bot_principal_merges_request_actors_but_not_other_reviewers() {
        let mut actions = vec![
            bot("code_review_summary"),
            bot("task_approve"),
            event(1, " Reviewer ", "bot", "request_changes", false),
            event(2, "Other", "bot", "code_review_summary", false),
        ];
        assert!(fold_package_reviews(&actions).has_objection());
        actions.push(event(2, "Reviewer", "bot", "code_review_summary", false));
        assert!(fold_package_reviews(&actions)
            .require_complete(false)
            .is_ok());
    }

    #[test]
    fn blocking_positive_and_withdrawal_dominate_until_same_lane_decision() {
        for action in [
            "comment",
            "code_review_summary",
            "approve",
            "dismiss",
            "defer",
        ] {
            let mut actions = vec![
                bot("code_review_summary"),
                bot("task_approve"),
                event(1, "Reviewer", "bot", action, true),
                bot("comment"),
            ];
            assert!(fold_package_reviews(&actions).has_objection());
            actions.push(bot("code_review_summary"));
            assert!(fold_package_reviews(&actions)
                .require_complete(false)
                .is_ok());
        }
        let mut actions = vec![
            bot("code_review_summary"),
            bot("task_approve"),
            bot("task_defer"),
            bot("code_review_summary"),
        ];
        assert!(fold_package_reviews(&actions).has_objection());
        actions.push(bot("task_approve"));
        assert!(fold_package_reviews(&actions)
            .require_complete(false)
            .is_ok());
    }

    #[test]
    fn human_gate_uses_general_approval_of_exact_human_actor() {
        let mut actions = vec![bot("code_review_summary"), bot("task_approve")];
        assert!(fold_package_reviews(&actions)
            .require_complete(true)
            .is_err());
        actions.push(event(1, "Alice", "user", "approve", false));
        assert!(fold_package_reviews(&actions)
            .require_complete(true)
            .is_ok());
        actions.push(event(1, "Alice", "user", "request_changes", false));
        actions.push(event(2, "Alice", "user", "approve", false));
        assert!(fold_package_reviews(&actions).has_objection());
        actions.push(event(1, "Alice", "user", "code_review_summary", false));
        // Actor2's approval is genuine but cannot resolve Actor1's objection.
        assert!(fold_package_reviews(&actions)
            .require_complete(true)
            .is_ok());
    }
}
