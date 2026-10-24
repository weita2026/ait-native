use super::actor_plan::plan_only_source;
use super::*;

#[test]
fn actor_upgrade_reports_real_work_and_excludes_unmanaged_repositories() {
    let (root, mut config, active) = plan_only_source("progress-actor");
    config.repository_scope =
        crate::foundation::server_repository_scope::ServerRepositoryScope::parse(Some("3"))
            .unwrap();
    let plan = active.generation_root.join("repositories/0/plan.bin");
    let mut events = Vec::new();
    let mut staged_before_publish = false;
    config
        .with_exclusive_admission(|| {
            admit_workflow_generation_with_progress(&config, &active, |event| {
                if event.phase == WorkflowUpgradePhase::Stage && event.processed > 0 {
                    // Staged progress is observed before any source replacement.
                    assert_eq!(&fs::read(&plan).unwrap()[..4], &1u32.to_le_bytes());
                    staged_before_publish = true;
                }
                events.push(event);
            })
        })
        .unwrap();
    assert!(staged_before_publish);
    for phase in [
        WorkflowUpgradePhase::Convert,
        WorkflowUpgradePhase::Stage,
        WorkflowUpgradePhase::Publish,
    ] {
        let observed = events
            .iter()
            .filter(|event| event.phase == phase)
            .collect::<Vec<_>>();
        assert!(observed.len() > 1);
        assert_eq!(observed.first().unwrap().processed, 0);
        assert_eq!(observed.last().unwrap().processed, 12);
        assert_eq!(observed.last().unwrap().total, Some(12));
        assert!(observed
            .windows(2)
            .all(|events| events[0].processed <= events[1].processed && events[0] != events[1]));
    }
    assert!(events.iter().any(
        |event| event.phase == WorkflowUpgradePhase::AwaitPublication && event.total.is_none()
    ));
    assert_eq!(events.last().unwrap().phase, WorkflowUpgradePhase::Complete);
    assert_eq!(&fs::read(plan).unwrap()[..4], &2u32.to_le_bytes());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_conversion_never_reports_staging_or_completion() {
    let (root, config, active) = plan_only_source("progress-failed");
    fs::write(
        active.generation_root.join("repositories/3/land.bin"),
        [2, 0, 0, 0, 255],
    )
    .unwrap();
    let mut events = Vec::new();
    assert!(config
        .with_exclusive_admission(|| {
            admit_workflow_generation_with_progress(&config, &active, |event| events.push(event))
        })
        .is_err());
    assert!(events
        .iter()
        .any(|event| event.phase == WorkflowUpgradePhase::Convert && event.processed > 0));
    assert!(!events.iter().any(|event| matches!(
        event.phase,
        WorkflowUpgradePhase::Stage
            | WorkflowUpgradePhase::Publish
            | WorkflowUpgradePhase::Complete
    )));
    let last = events.last().unwrap();
    assert_eq!(last.phase, WorkflowUpgradePhase::Convert);
    assert_eq!(last.total, None);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn current_profile_reports_inspection_without_inventing_conversion_work() {
    let (root, config, active) = plan_only_source("progress-current");
    admit(&config, &active).unwrap();
    let mut events = Vec::new();
    config
        .with_exclusive_admission(|| {
            admit_workflow_generation_with_progress(&config, &active, |event| events.push(event))
        })
        .unwrap();
    assert!(events
        .iter()
        .any(|event| event.phase == WorkflowUpgradePhase::Inspect && event.processed == 4));
    assert!(!events.iter().any(|event| matches!(
        event.phase,
        WorkflowUpgradePhase::Convert | WorkflowUpgradePhase::Stage | WorkflowUpgradePhase::Publish
    )));
    assert_eq!(
        *events.last().unwrap(),
        WorkflowUpgradeProgress {
            phase: WorkflowUpgradePhase::Complete,
            processed: 0,
            total: Some(0)
        }
    );
    fs::remove_dir_all(root).unwrap();
}
