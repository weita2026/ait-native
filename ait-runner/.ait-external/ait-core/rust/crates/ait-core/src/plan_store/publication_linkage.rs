use std::collections::BTreeSet;

use super::*;

/// Resolve a direct Plan publication receipt, or the one exact published
/// adoption of an archived revision after mixed-lineage reconciliation.
///
/// The fallback is deliberately read-only and narrow. It requires a non-empty
/// artifact Blob identity, exact stable publication metadata, and one unique
/// Remote Plan/revision tuple. The Task's stored local Plan linkage remains
/// unchanged.
pub fn resolve_reconciled_plan_publish_linkage_with_plan_store<S>(
    store: &S,
    plan_id: Option<&str>,
    plan_revision_id: Option<&str>,
) -> PlanStoreResult<PlanPublishLinkage>
where
    S: PlanReadStore + ?Sized,
{
    let direct = PlanReadStore::resolve_plan_publish_linkage(store, plan_id, plan_revision_id)?;
    if direct.published_plan_revision_id.is_some() {
        return Ok(direct);
    }
    let Some(plan_revision_id) = plan_revision_id else {
        return Ok(direct);
    };
    let source_revision = PlanReadStore::get_plan_revision_by_id(store, plan_revision_id)?;
    let source_plan = PlanReadStore::get_plan(store, &source_revision.plan_id)?;
    if source_plan.status != "archived"
        || source_revision
            .artifact_blob_id
            .as_deref()
            .is_none_or(str::is_empty)
    {
        return Ok(direct);
    }

    let mut remote_targets = BTreeSet::new();
    for candidate_plan in PlanReadStore::list_plans(store)? {
        if candidate_plan.plan_id == source_plan.plan_id
            || candidate_plan.publication_state != "published"
        {
            continue;
        }
        let Some(published_plan_id) = candidate_plan.published_plan_id else {
            continue;
        };
        for candidate_revision in
            PlanReadStore::list_plan_revisions(store, &candidate_plan.plan_id)?
        {
            if candidate_revision.publication_state != "published"
                || !plan_revision_publication_metadata_matches(
                    &source_revision,
                    &candidate_revision,
                )
            {
                continue;
            }
            let Some(published_revision_id) = candidate_revision.published_plan_revision_id else {
                continue;
            };
            remote_targets.insert((published_plan_id.clone(), published_revision_id));
        }
    }

    match remote_targets.len() {
        0 => Ok(direct),
        1 => {
            let (published_plan_id, published_plan_revision_id) =
                remote_targets.into_iter().next().unwrap_or_default();
            Ok(PlanPublishLinkage {
                published_plan_id: Some(published_plan_id),
                published_plan_revision_id: Some(published_plan_revision_id),
                ..direct
            })
        }
        count => Err(PlanStoreError::Invalid(format!(
            "Archived local Plan revision {plan_revision_id} exactly matches {count} distinct published Remote Plan/revision tuples; refusing ambiguous Task publication linkage."
        ))),
    }
}

fn plan_revision_publication_metadata_matches(
    left: &PlanRevisionRecord,
    right: &PlanRevisionRecord,
) -> bool {
    left.artifact_path == right.artifact_path
        && left.artifact_selector == right.artifact_selector
        && left.artifact_heading == right.artifact_heading
        && left.title_snapshot == right.title_snapshot
        && left.artifact_blob_id == right.artifact_blob_id
        && left.items.len() == right.items.len()
        && left.items.iter().zip(&right.items).all(|(left, right)| {
            left.plan_item_ref == right.plan_item_ref
                && left.text == right.text
                && left.checkbox_state == right.checkbox_state
                && left.heading_path == right.heading_path
                && left.line_number == right.line_number
        })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[derive(Default)]
    struct FakePlanReadStore {
        plans: Vec<PlanRecord>,
        revisions: BTreeMap<String, Vec<PlanRevisionRecord>>,
    }

    impl PlanReadStore for FakePlanReadStore {
        fn list_plans(&self) -> PlanStoreResult<Vec<PlanRecord>> {
            Ok(self.plans.clone())
        }

        fn get_plan(&self, plan_id: &str) -> PlanStoreResult<PlanRecord> {
            self.plans
                .iter()
                .find(|plan| plan.plan_id == plan_id)
                .cloned()
                .ok_or_else(|| PlanStoreError::NotFound(format!("missing Plan {plan_id}")))
        }

        fn list_plan_revisions(&self, plan_id: &str) -> PlanStoreResult<Vec<PlanRevisionRecord>> {
            Ok(self.revisions.get(plan_id).cloned().unwrap_or_default())
        }

        fn get_plan_revision(
            &self,
            plan_id: &str,
            plan_revision_id: &str,
        ) -> PlanStoreResult<PlanRevisionRecord> {
            self.list_plan_revisions(plan_id)?
                .into_iter()
                .find(|revision| revision.plan_revision_id == plan_revision_id)
                .ok_or_else(|| {
                    PlanStoreError::NotFound(format!("missing Plan revision {plan_revision_id}"))
                })
        }

        fn get_plan_revision_by_id(
            &self,
            plan_revision_id: &str,
        ) -> PlanStoreResult<PlanRevisionRecord> {
            self.revisions
                .values()
                .flatten()
                .find(|revision| revision.plan_revision_id == plan_revision_id)
                .cloned()
                .ok_or_else(|| {
                    PlanStoreError::NotFound(format!("missing Plan revision {plan_revision_id}"))
                })
        }

        fn resolve_plan_publish_linkage(
            &self,
            plan_id: Option<&str>,
            plan_revision_id: Option<&str>,
        ) -> PlanStoreResult<PlanPublishLinkage> {
            let revision = plan_revision_id
                .map(|revision_id| self.get_plan_revision_by_id(revision_id))
                .transpose()?;
            let resolved_plan_id = plan_id
                .map(str::to_string)
                .or_else(|| revision.as_ref().map(|revision| revision.plan_id.clone()))
                .ok_or_else(|| PlanStoreError::Invalid("missing Plan linkage".to_string()))?;
            let plan = self.get_plan(&resolved_plan_id)?;
            Ok(PlanPublishLinkage {
                plan_id: resolved_plan_id,
                published_plan_id: plan.published_plan_id,
                plan_revision_id: revision
                    .as_ref()
                    .map(|revision| revision.plan_revision_id.clone()),
                published_plan_revision_id: revision
                    .and_then(|revision| revision.published_plan_revision_id),
            })
        }
    }

    fn item() -> PlanItemRecord {
        PlanItemRecord {
            plan_item_ref: Some("card/implement".to_string()),
            text: Some("Implement the exact repair".to_string()),
            checkbox_state: Some("open".to_string()),
            heading_path: vec!["Card".to_string(), "Work".to_string()],
            line_number: Some(12),
            payload: Map::new(),
        }
    }

    fn revision(
        plan_id: &str,
        revision_id: &str,
        artifact_blob_id: Option<&str>,
        publication: Option<&str>,
    ) -> PlanRevisionRecord {
        PlanRevisionRecord {
            plan_revision_id: revision_id.to_string(),
            plan_id: plan_id.to_string(),
            revision_number: 1,
            parent_plan_revision_id: None,
            title_snapshot: "Card".to_string(),
            summary: None,
            artifact_path: "docs/sprints/card.md".to_string(),
            artifact_selector: Some("card/root".to_string()),
            artifact_heading: "Card".to_string(),
            artifact_blob_id: artifact_blob_id.map(str::to_string),
            items: vec![item()],
            source_kind: "binary_db".to_string(),
            created_by: None,
            actor_type: "system".to_string(),
            publication_state: if publication.is_some() {
                "published".to_string()
            } else {
                "local_draft".to_string()
            },
            published_plan_revision_id: publication.map(str::to_string),
            published_at: publication.map(|_| "1".to_string()),
            created_at: "1".to_string(),
        }
    }

    fn plan(plan_id: &str, status: &str, published_plan_id: Option<&str>) -> PlanRecord {
        PlanRecord {
            plan_id: plan_id.to_string(),
            repo_name: "repo".to_string(),
            title: "Card".to_string(),
            status: status.to_string(),
            head_revision_id: None,
            publication_state: if published_plan_id.is_some() {
                "published".to_string()
            } else {
                "local_draft".to_string()
            },
            published_remote_name: None,
            published_plan_id: published_plan_id.map(str::to_string),
            published_head_revision_id: None,
            published_at: published_plan_id.map(|_| "1".to_string()),
            created_by: None,
            created_at: "1".to_string(),
            updated_at: "1".to_string(),
            head_revision: None,
            head_summary: None,
        }
    }

    fn store_with_adoption() -> FakePlanReadStore {
        FakePlanReadStore {
            plans: vec![
                plan("PR-OLD", "archived", Some("PR-OTHER")),
                plan("PR-ADOPTED", "draft", Some("PR-REMOTE")),
            ],
            revisions: BTreeMap::from([
                (
                    "PR-OLD".to_string(),
                    vec![revision("PR-OLD", "REV-OLD", Some("BLB-1"), None)],
                ),
                (
                    "PR-ADOPTED".to_string(),
                    vec![revision(
                        "PR-ADOPTED",
                        "REV-ADOPTED",
                        Some("BLB-1"),
                        Some("RREV-1"),
                    )],
                ),
            ]),
        }
    }

    #[test]
    fn archived_revision_resolves_one_exact_published_adoption() {
        let linkage = resolve_reconciled_plan_publish_linkage_with_plan_store(
            &store_with_adoption(),
            Some("PR-OLD"),
            Some("REV-OLD"),
        )
        .unwrap();

        assert_eq!(linkage.plan_id, "PR-OLD");
        assert_eq!(linkage.plan_revision_id.as_deref(), Some("REV-OLD"));
        assert_eq!(linkage.published_plan_id.as_deref(), Some("PR-REMOTE"));
        assert_eq!(
            linkage.published_plan_revision_id.as_deref(),
            Some("RREV-1")
        );
    }

    #[test]
    fn direct_revision_receipt_remains_authoritative() {
        let mut store = store_with_adoption();
        store.revisions.get_mut("PR-OLD").unwrap()[0].publication_state = "published".to_string();
        store.revisions.get_mut("PR-OLD").unwrap()[0].published_plan_revision_id =
            Some("RREV-DIRECT".to_string());

        let linkage = resolve_reconciled_plan_publish_linkage_with_plan_store(
            &store,
            Some("PR-OLD"),
            Some("REV-OLD"),
        )
        .unwrap();

        assert_eq!(linkage.published_plan_id.as_deref(), Some("PR-OTHER"));
        assert_eq!(
            linkage.published_plan_revision_id.as_deref(),
            Some("RREV-DIRECT")
        );
    }

    #[test]
    fn active_or_contentless_source_revision_does_not_fallback() {
        for (status, blob_id) in [("draft", Some("BLB-1")), ("archived", None)] {
            let mut store = store_with_adoption();
            store.plans[0].status = status.to_string();
            store.revisions.get_mut("PR-OLD").unwrap()[0].artifact_blob_id =
                blob_id.map(str::to_string);

            let linkage = resolve_reconciled_plan_publish_linkage_with_plan_store(
                &store,
                Some("PR-OLD"),
                Some("REV-OLD"),
            )
            .unwrap();
            assert_eq!(linkage.published_plan_id.as_deref(), Some("PR-OTHER"));
            assert_eq!(linkage.published_plan_revision_id, None);
        }
    }

    #[test]
    fn distinct_matching_remote_tuples_fail_closed() {
        let mut store = store_with_adoption();
        store
            .plans
            .push(plan("PR-SECOND", "draft", Some("PR-REMOTE-2")));
        store.revisions.insert(
            "PR-SECOND".to_string(),
            vec![revision(
                "PR-SECOND",
                "REV-SECOND",
                Some("BLB-1"),
                Some("RREV-2"),
            )],
        );

        let error = resolve_reconciled_plan_publish_linkage_with_plan_store(
            &store,
            Some("PR-OLD"),
            Some("REV-OLD"),
        )
        .unwrap_err();

        assert!(error.to_string().contains("2 distinct published Remote"));
    }
}
