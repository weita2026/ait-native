use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    Json, Router,
};
use serde_json::json;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

#[derive(Clone, Default)]
pub struct StartupRouterHandle {
    ready_router: Arc<Mutex<Option<Router>>>,
}

impl StartupRouterHandle {
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
                Json(json!({
                    "error": "ait-server startup router lock is poisoned",
                    "ready": false,
                    "status": "failed",
                })),
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
            Json(json!({
                "ready": false,
                "status": "starting",
            })),
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
    use axum::routing::get;
    use hyper::body::to_bytes;

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
            json!({"ready": false, "status": "starting"})
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
