//! Contract: a refusal or stop message must carry one command the reader can
//! run next, written as `ait ...` in backticks. The tests here pin every
//! refusal renderer added by the workflow friction work to that rule.

/// True when `message` contains at least one backticked `ait ...` command.
pub(crate) fn contains_runnable_ait_command(message: &str) -> bool {
    let mut rest = message;
    while let Some(start) = rest.find("`ait") {
        let after = &rest[start + 1..];
        if let Some(end) = after.find('`') {
            let command = &after[..end];
            if command.starts_with("ait ") || command.starts_with("ait-runner ") {
                return true;
            }
            rest = &after[end + 1..];
        } else {
            break;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::post_finish_hooks::{parse_post_finish_hooks, PostFinishHook};
    use crate::primitives::{
        planning_only_artifact_sync_command, render_planning_only_artifact_drift_error,
        review_message_required_error, review_summary_missing_sections_error,
        QUICK_TASK_REQUIRES_SPRINT_MESSAGE,
    };
    use crate::runner_hint::{derive_runner_hint, runner_hint_text};
    use ait_core::json_support::json;

    fn assert_actionable(label: &str, message: &str) {
        assert!(
            contains_runnable_ait_command(message),
            "{label} must name a runnable `ait ...` command:\n{message}"
        );
    }

    #[test]
    fn detector_requires_a_backticked_ait_command() {
        assert!(contains_runnable_ait_command(
            "run `ait plan sync a.md --local` now"
        ));
        assert!(contains_runnable_ait_command("`ait-runner serve --once`"));
        assert!(!contains_runnable_ait_command("run ait plan sync a.md"));
        assert!(!contains_runnable_ait_command("see `docs/plan.md`"));
        assert!(!contains_runnable_ait_command("`aitk`"));
    }

    #[test]
    fn every_workflow_refusal_renderer_is_actionable() {
        assert_actionable(
            "markdown drift (modified)",
            &render_planning_only_artifact_drift_error(
                "ait task start",
                &[("docs/a.md".to_string(), false)],
            ),
        );
        assert_actionable(
            "markdown drift (deleted)",
            &render_planning_only_artifact_drift_error(
                "ait snapshot create",
                &[("docs/gone.md".to_string(), true)],
            ),
        );
        assert!(planning_only_artifact_sync_command("docs/x.md", true).ends_with("--prune --local"));
        assert_actionable("quick requires sprint", QUICK_TASK_REQUIRES_SPRINT_MESSAGE);
        assert_actionable("review message required", &review_message_required_error());
        assert_actionable(
            "review sections missing",
            &review_summary_missing_sections_error(&["Risks".to_string()]),
        );
        let hint = derive_runner_hint(&json!({"jobs": [{"state": "queued"}]}), "http://x", Some(3))
            .unwrap();
        assert_actionable(
            "runner hint",
            &runner_hint_text(&json!({crate::runner_hint::RUNNER_HINT_PAYLOAD_KEY: hint})).unwrap(),
        );
        let _: Vec<PostFinishHook> = parse_post_finish_hooks(&json!([])).unwrap();
        assert_actionable(
            "plan sync missing path",
            &crate::main_app::plan_sync_missing_path_hint_text(
                "docs/gone.md",
                false,
                true,
                None,
                "Plan sync failed: Path does not exist: docs/gone.md".to_string(),
            ),
        );
    }
}
