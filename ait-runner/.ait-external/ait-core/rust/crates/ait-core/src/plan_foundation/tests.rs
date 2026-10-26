use super::{compute_sync_prune_decisions, extract_plan_refs, parse_plan_markdown};

const SAMPLE_MARKDOWN: &str = "# Runtime Stability\n\n## Stabilize Runtime Execution Tasks [plan-ref: runtime/tasks]\n\n- [ ] Move init to startup only [ref: runtime/startup-only-init]\n- [x] Tune busy timeout [ref: runtime/busy-timeout]\n\n### Follow-up Work [plan-ref: runtime/follow-up]\n\n1. Capture retry behavior [ref: runtime/retry-behavior]\n";

#[test]
fn parse_plan_markdown_collects_refs_and_items() {
    let parsed = parse_plan_markdown(Some(SAMPLE_MARKDOWN));
    assert_eq!(parsed.plan_ref_count, 2);
    assert_eq!(parsed.item_count, 3);
    assert_eq!(parsed.plan_refs[0].plan_ref, "runtime/tasks".to_string());
    assert_eq!(
        parsed.items[0].plan_item_ref,
        "runtime/startup-only-init".to_string()
    );
}

#[test]
fn extract_plan_refs_returns_identity_payload() {
    let parsed = parse_plan_markdown(Some(SAMPLE_MARKDOWN));
    let refs = extract_plan_refs(&parsed);
    assert_eq!(refs.plan_ref_count, 2);
    assert_eq!(refs.plan_refs[1].plan_ref, "runtime/follow-up".to_string());
}

#[test]
fn parse_plan_markdown_ignores_fenced_plan_examples() {
    let markdown = "# Actual [plan-ref: actual/root]\n\
- [ ] Actual item [ref: actual/item]\n\
```markdown\n\
## Example [plan-ref: example/root]\n\
- [ ] Example item [ref: example/item]\n\
```\n";
    let parsed = parse_plan_markdown(Some(markdown));
    assert_eq!(parsed.plan_ref_count, 1);
    assert_eq!(parsed.item_count, 1);
    assert_eq!(parsed.plan_refs[0].plan_ref, "actual/root");
    assert_eq!(parsed.items[0].plan_item_ref, "actual/item");
}

#[test]
fn compute_sync_prune_decisions_reports_prune_and_retained_paths() {
    let payload = compute_sync_prune_decisions(
        Some("directory"),
        &[
            "docs/sprints/a.md".to_string(),
            "docs/sprints/b.md".to_string(),
            "docs/sprints/b.md".to_string(),
        ],
        &["docs/sprints/b.md".to_string()],
    )
    .unwrap();
    assert_eq!(payload.scope, "directory".to_string());
    assert_eq!(payload.tracked_artifact_count, 2);
    assert_eq!(payload.synced_artifact_count, 1);
    assert_eq!(
        payload.retained_paths,
        vec!["docs/sprints/b.md".to_string()]
    );
    assert_eq!(payload.prune_paths, vec!["docs/sprints/a.md".to_string()]);
    assert_eq!(payload.prune_count, 1);
}

#[test]
fn compute_sync_prune_decisions_rejects_invalid_scope() {
    let error =
        compute_sync_prune_decisions(Some("remote"), &["docs/sprints/a.md".to_string()], &[])
            .unwrap_err();
    assert!(error.contains("Unsupported sync prune scope"));
}

#[test]
fn compute_sync_prune_decisions_rejects_empty_artifact_paths() {
    let error = compute_sync_prune_decisions(Some("file"), &["".to_string()], &[]).unwrap_err();
    assert_eq!(
        error,
        "Artifact paths must be non-empty strings.".to_string()
    );
}
