use super::*;

fn local_request() -> TaskCloseRequest {
    TaskCloseRequest {
        requested_status: None,
        abandoned: false,
        exclude_later_promotion: false,
        scope: TaskCloseScope::Local,
    }
}

#[test]
fn defaults_to_abandoned() {
    let result = resolve_task_close(local_request()).unwrap();
    assert_eq!(result.status(), TASK_STATUS_ABANDONED);
    assert_eq!(result.scope().as_str(), "local");
    assert_eq!(result.display_label(), "abandoned");
}

#[test]
fn allows_local_later_promotion_excluded() {
    let mut request = local_request();
    request.exclude_later_promotion = true;
    let result = resolve_task_close(request).unwrap();
    assert_eq!(result.status(), TASK_STATUS_LATER_PROMOTION_EXCLUDED);
    assert_eq!(result.display_label(), "later-promotion-excluded");
}

#[test]
fn rejects_conflicting_flags() {
    let mut request = local_request();
    request.abandoned = true;
    request.exclude_later_promotion = true;
    assert_eq!(
        resolve_task_close(request).unwrap_err(),
        TaskCloseError::ConflictingImplicitFlags
    );
}

#[test]
fn rejects_remote_later_promotion_excluded() {
    let request = TaskCloseRequest {
        requested_status: None,
        abandoned: false,
        exclude_later_promotion: true,
        scope: TaskCloseScope::Remote,
    };
    assert_eq!(
        resolve_task_close(request).unwrap_err(),
        TaskCloseError::LaterPromotionExcludedRequiresLocal
    );
}

#[test]
fn normalizes_explicit_remote_completed_status() {
    let request = TaskCloseRequest {
        requested_status: Some(" completed ".to_string()),
        abandoned: false,
        exclude_later_promotion: false,
        scope: TaskCloseScope::Remote,
    };
    let result = resolve_task_close(request).unwrap();
    assert_eq!(result.status(), TASK_STATUS_COMPLETED);
    assert_eq!(result.display_label(), "completed");
}

#[test]
fn rejects_unsupported_remote_status() {
    let request = TaskCloseRequest {
        requested_status: Some(TASK_STATUS_LATER_PROMOTION_EXCLUDED.to_string()),
        abandoned: false,
        exclude_later_promotion: false,
        scope: TaskCloseScope::Remote,
    };
    assert_eq!(
        resolve_task_close(request).unwrap_err(),
        TaskCloseError::UnsupportedStatus {
            scope: TaskCloseScope::Remote,
            status: TASK_STATUS_LATER_PROMOTION_EXCLUDED.to_string(),
        }
    );
}
