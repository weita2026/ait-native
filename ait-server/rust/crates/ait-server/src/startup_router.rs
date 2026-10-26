use ait_server_core::foundation::server_workflow_upgrade::WorkflowUpgradeProgress;
use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    Json, Router,
};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tower::ServiceExt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartupUpgradeProgress {
    pub work: WorkflowUpgradeProgress,
    pub sequence: u64,
    pub last_progress_at: SystemTime,
}

#[derive(Clone, Default)]
pub struct StartupProgressTracker {
    upgrade_progress: Arc<Mutex<Option<StartupUpgradeProgress>>>,
}

impl StartupProgressTracker {
    pub fn observe_upgrade_progress(&self, work: WorkflowUpgradeProgress) {
        self.record_upgrade_at(work, SystemTime::now());
    }

    pub fn upgrade_progress(&self) -> Option<StartupUpgradeProgress> {
        self.upgrade_progress
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn record_upgrade_at(&self, work: WorkflowUpgradeProgress, now: SystemTime) {
        if work.total.is_some_and(|total| work.processed > total) {
            return;
        }
        let mut current = self
            .upgrade_progress
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(previous) = current.as_mut() {
            if previous.work.phase == work.phase {
                if work.processed < previous.work.processed {
                    return;
                }
                if work.processed == previous.work.processed {
                    if work.total.is_some() {
                        previous.work.total = work.total;
                    }
                    return;
                }
            }
        }
        let sequence = current
            .as_ref()
            .map_or(1, |previous| previous.sequence.saturating_add(1));
        *current = Some(StartupUpgradeProgress {
            work,
            sequence,
            last_progress_at: now,
        });
    }

    pub(crate) fn health_projection(&self, mut value: serde_json::Value) -> serde_json::Value {
        use ait_server_core::foundation::server_workflow_upgrade::WorkflowUpgradePhase;
        let progress = self.upgrade_progress();
        let phase = progress.as_ref().map(|progress| match progress.work.phase {
            WorkflowUpgradePhase::Inspect => "inspect",
            WorkflowUpgradePhase::ReadMetadata => "read_metadata",
            WorkflowUpgradePhase::Convert => "convert",
            WorkflowUpgradePhase::Stage => "stage",
            WorkflowUpgradePhase::Validate => "validate",
            WorkflowUpgradePhase::AwaitPublication => "await_publication",
            WorkflowUpgradePhase::Publish => "publish",
            WorkflowUpgradePhase::Verify => "verify",
            WorkflowUpgradePhase::Complete => "complete",
        });
        let fields = json!({
            "phase": phase,
            "processed": progress.as_ref().map(|progress| progress.work.processed),
            "total": progress.as_ref().and_then(|progress| progress.work.total),
            "last_progress_at": progress.as_ref().map(|progress| chrono::DateTime::<chrono::Utc>::from(progress.last_progress_at).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        });
        value
            .as_object_mut()
            .expect("health response is an object")
            .extend(
                fields
                    .as_object()
                    .expect("progress fields are an object")
                    .clone(),
            );
        value
    }
}

#[derive(Clone, Default)]
pub struct StartupRouterHandle {
    ready_router: Arc<Mutex<Option<Router>>>,
    progress: StartupProgressTracker,
}

impl StartupRouterHandle {
    pub fn progress_tracker(&self) -> StartupProgressTracker {
        self.progress.clone()
    }
    pub fn observe_upgrade_progress(&self, work: WorkflowUpgradeProgress) {
        self.progress.observe_upgrade_progress(work);
    }
    pub fn upgrade_progress(&self) -> Option<StartupUpgradeProgress> {
        self.progress.upgrade_progress()
    }
    #[cfg(test)]
    fn record_upgrade_at(&self, work: WorkflowUpgradeProgress, now: SystemTime) {
        self.progress.record_upgrade_at(work, now);
    }

    pub fn activate(&self, ready_router: Router) -> Result<(), String> {
        let mut current = self
            .ready_router
            .lock()
            .map_err(|_| "ait-server startup router lock is poisoned".to_string())?;
        if current.is_some() {
            return Err("ait-server startup router is already active".to_string());
        }
        *current = Some(ready_router);
        Ok(())
    }
}

pub fn build_startup_router() -> (Router, StartupRouterHandle) {
    let handle = StartupRouterHandle::default();
    let router = Router::new()
        .fallback(proxy_startup_request)
        .with_state(handle.clone());
    (router, handle)
}

async fn proxy_startup_request(
    State(handle): State<StartupRouterHandle>,
    request: Request<Body>,
) -> Response {
    let ready_router = match handle.ready_router.lock() {
        Ok(router) => router.clone(),
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(handle.progress.health_projection(json!({
                    "error": "ait-server startup router lock is poisoned",
                    "ready": false,
                    "status": "failed",
                }))),
            )
                .into_response();
        }
    };
    if let Some(router) = ready_router {
        return router
            .oneshot(request)
            .await
            .unwrap_or_else(|error| match error {});
    }

    if request.uri().path() == "/healthz" {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(handle.progress.health_projection(json!({
                "ready": false,
                "status": "starting",
            }))),
        )
            .into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "error": "ait-server is starting",
                "ready": false,
                "status": "starting",
            })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ait_server_core::foundation::server_workflow_upgrade::WorkflowUpgradePhase;
    use axum::routing::get;
    use hyper::body::to_bytes;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn progress_time_only_advances_with_observed_work() {
        let handle = StartupRouterHandle::default();
        let work = WorkflowUpgradeProgress {
            phase: WorkflowUpgradePhase::Convert,
            processed: 0,
            total: Some(4),
        };
        handle.record_upgrade_at(work, UNIX_EPOCH + Duration::from_secs(1));
        let initial = handle.upgrade_progress().unwrap();
        handle.record_upgrade_at(work, UNIX_EPOCH + Duration::from_secs(2));
        assert_eq!(handle.upgrade_progress().unwrap(), initial);
        handle.record_upgrade_at(
            WorkflowUpgradeProgress {
                total: Some(5),
                ..work
            },
            UNIX_EPOCH + Duration::from_secs(2),
        );
        let total_only = handle.upgrade_progress().unwrap();
        assert_eq!(total_only.work.total, Some(5));
        assert_eq!(total_only.sequence, initial.sequence);
        assert_eq!(total_only.last_progress_at, initial.last_progress_at);
        handle.record_upgrade_at(
            WorkflowUpgradeProgress {
                processed: 1,
                ..work
            },
            UNIX_EPOCH + Duration::from_secs(3),
        );
        let advanced = handle.upgrade_progress().unwrap();
        assert_eq!(advanced.sequence, initial.sequence + 1);
        assert_eq!(
            advanced.last_progress_at,
            UNIX_EPOCH + Duration::from_secs(3)
        );
        handle.record_upgrade_at(work, UNIX_EPOCH + Duration::from_secs(4));
        handle.record_upgrade_at(
            WorkflowUpgradeProgress {
                processed: 5,
                ..work
            },
            UNIX_EPOCH + Duration::from_secs(5),
        );
        assert_eq!(handle.upgrade_progress().unwrap(), advanced);
    }

    #[tokio::test]
    async fn observed_conversion_completion_does_not_admit_business_writes() {
        let (router, handle) = build_startup_router();
        handle.observe_upgrade_progress(WorkflowUpgradeProgress {
            phase: WorkflowUpgradePhase::Complete,
            processed: 4,
            total: Some(4),
        });
        let health = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(health.status(), StatusCode::SERVICE_UNAVAILABLE);
        let health = response_json(health).await;
        assert_eq!(health["phase"], "complete");
        assert_eq!(health["processed"], 4);
        assert_eq!(health["total"], 4);
        assert!(health["last_progress_at"].as_str().is_some());
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/native/repository-authorities/0/task-land")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    async fn response_json(response: Response) -> serde_json::Value {
        let bytes = to_bytes(response.into_body())
            .await
            .expect("response body should read");
        serde_json::from_slice(&bytes).expect("response body should be JSON")
    }

    #[tokio::test]
    async fn startup_router_reports_503_then_atomically_delegates() {
        let (startup_router, handle) = build_startup_router();
        let starting_health = startup_router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .expect("starting health request"),
            )
            .await
            .expect("starting health response");
        assert_eq!(starting_health.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response_json(starting_health).await,
            json!({"ready": false, "status": "starting", "phase":null, "processed":null, "total":null, "last_progress_at":null})
        );

        let starting_route = startup_router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/ready-only")
                    .body(Body::empty())
                    .expect("starting route request"),
            )
            .await
            .expect("starting route response");
        assert_eq!(starting_route.status(), StatusCode::SERVICE_UNAVAILABLE);

        let ready_router = Router::new()
            .route("/healthz", get(|| async { Json(json!({"ready": true})) }))
            .route("/ready-only", get(|| async { StatusCode::NO_CONTENT }));
        handle
            .activate(ready_router)
            .expect("ready router should activate exactly once");

        let ready_health = startup_router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .expect("ready health request"),
            )
            .await
            .expect("ready health response");
        assert_eq!(ready_health.status(), StatusCode::OK);
        assert_eq!(response_json(ready_health).await, json!({"ready": true}));

        let ready_route = startup_router
            .oneshot(
                Request::builder()
                    .uri("/ready-only")
                    .body(Body::empty())
                    .expect("ready route request"),
            )
            .await
            .expect("ready route response");
        assert_eq!(ready_route.status(), StatusCode::NO_CONTENT);
        assert!(handle.activate(Router::new()).is_err());
    }
}
