use super::*;

pub(super) const QUEUE_PROJECTION_MUTATION_QUIET_PERIOD: Duration = Duration::from_secs(1);
const QUEUE_PROJECTION_DEBOUNCE_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Clone)]
struct QueueProjectionRows {
    repo_name: String,
    tasks: Arc<Vec<JsonMap<String, JsonValue>>>,
    changes: Arc<Vec<JsonMap<String, JsonValue>>>,
    patchsets: Arc<Vec<JsonMap<String, JsonValue>>>,
    reviews: Arc<Vec<JsonMap<String, JsonValue>>>,
    review_requests: Arc<Vec<JsonMap<String, JsonValue>>>,
    attestations: Arc<Vec<JsonMap<String, JsonValue>>>,
    policy_decisions: Arc<Vec<JsonMap<String, JsonValue>>>,
}

impl QueueProjectionRows {
    fn read_model_input(
        &self,
        status: Option<&str>,
        include_all_changes: bool,
    ) -> QueueReadModelInput {
        QueueReadModelInput {
            repo_name: Some(self.repo_name.clone()),
            status: status.unwrap_or("active").to_string(),
            include_all_changes,
            tasks: self.tasks.clone(),
            changes: self.changes.clone(),
            patchsets: self.patchsets.clone(),
            reviews: self.reviews.clone(),
            review_requests: self.review_requests.clone(),
            attestations: self.attestations.clone(),
            policy_decisions: self.policy_decisions.clone(),
            refs: Arc::default(),
            ci_statuses: Arc::default(),
        }
    }
}

struct CachedQueueProjection {
    rows: QueueProjectionRows,
}

#[derive(Default)]
struct QueueProjectionCacheState {
    current: Option<Arc<CachedQueueProjection>>,
    last_refresh_attempt: Option<Instant>,
    last_refresh_error: Option<String>,
}

struct QueueProjectionCache {
    state: RwLock<QueueProjectionCacheState>,
    refresh_in_flight: AtomicBool,
    refresh_pending: AtomicBool,
    refresh_immediate: AtomicBool,
    refresh_request_generation: std::sync::atomic::AtomicU64,
    #[cfg(test)]
    refresh_attempt_count: std::sync::atomic::AtomicU64,
}

impl Default for QueueProjectionCache {
    fn default() -> Self {
        Self {
            state: RwLock::new(QueueProjectionCacheState::default()),
            refresh_in_flight: AtomicBool::new(false),
            refresh_pending: AtomicBool::new(false),
            refresh_immediate: AtomicBool::new(false),
            refresh_request_generation: std::sync::atomic::AtomicU64::new(0),
            #[cfg(test)]
            refresh_attempt_count: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

#[derive(Clone)]
pub struct BinaryDbServerWorkflowReadModelService<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    db: D,
    workflow: Arc<dyn ServerWorkflowStore>,
    queue_projection_cache: Arc<QueueProjectionCache>,
}

impl<D> BinaryDbServerWorkflowReadModelService<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    pub fn new(db: D, workflow: Arc<dyn ServerWorkflowStore>) -> Self {
        let db = workflow
            .as_any()
            .downcast_ref::<BinaryDbServerWorkflowV0Store<D>>()
            .map(|store| store.db().clone())
            .unwrap_or(db);
        Self {
            db,
            workflow,
            queue_projection_cache: Arc::new(QueueProjectionCache::default()),
        }
    }

    pub fn read_queue_summary(
        &self,
        repo_name: Option<&str>,
        status: Option<&str>,
        include_all_changes: bool,
    ) -> Result<JsonValue, String> {
        let input = self.queue_read_model_input(repo_name, status, include_all_changes)?;
        queue_summary_read_model(&input)
    }

    pub fn read_task_queue(
        &self,
        repo_name: Option<&str>,
        status: Option<&str>,
    ) -> Result<JsonValue, String> {
        Ok(self
            .read_queue_summary(repo_name, status, false)?
            .get("task_queue")
            .cloned()
            .unwrap_or_else(|| json!({"items": [], "count": 0})))
    }

    pub fn read_reviewer_inbox(&self, repo_name: Option<&str>) -> Result<JsonValue, String> {
        Ok(self
            .read_queue_summary(repo_name, Some("active"), false)?
            .get("reviewer_inbox")
            .cloned()
            .unwrap_or_else(|| json!({"items": [], "count": 0})))
    }

    fn queue_read_model_input(
        &self,
        repo_name: Option<&str>,
        status: Option<&str>,
        include_all_changes: bool,
    ) -> Result<QueueReadModelInput, String> {
        let repo_name = repo_name.unwrap_or_else(|| self.db.repo_name().as_str());
        if repo_name != self.db.repo_name().as_str() {
            return Err(format!(
                "Binary DB queue projection is bound to repository {}, not {repo_name}.",
                self.db.repo_name().as_str()
            ));
        }
        let (cached, last_refresh_error) = self
            .queue_projection_cache
            .state
            .read()
            .map(|state| (state.current.clone(), state.last_refresh_error.clone()))
            .map_err(|_| "Binary DB queue projection cache read lock is poisoned".to_string())?;
        if cached.is_none() {
            self.request_queue_projection_refresh();
            return Err(last_refresh_error.unwrap_or_else(|| {
                binary_db_runtime_error(
                    "Binary DB queue projection",
                    BinaryDbError::retryable_busy(format!(
                        "projection for {repo_name} is warming in the background; retry shortly"
                    )),
                )
            }));
        }
        let cached = cached.expect("cached queue projection was checked above");
        if last_refresh_error.is_some() {
            self.request_queue_projection_refresh();
        }
        Ok(cached.rows.read_model_input(status, include_all_changes))
    }

    #[cfg(test)]
    pub(crate) fn queue_projection_refresh_attempted(&self) -> bool {
        self.queue_projection_cache
            .state
            .read()
            .map(|state| state.last_refresh_attempt.is_some())
            .unwrap_or(false)
    }

    #[cfg(test)]
    pub(crate) fn queue_projection_refresh_attempt_count(&self) -> u64 {
        self.queue_projection_cache
            .refresh_attempt_count
            .load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn set_queue_projection_refresh_in_flight_for_test(&self, value: bool) {
        self.queue_projection_cache
            .refresh_in_flight
            .store(value, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn queue_projection_refresh_request_generation(&self) -> u64 {
        self.queue_projection_cache
            .refresh_request_generation
            .load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn queue_projection_refresh_pending(&self) -> bool {
        self.queue_projection_cache
            .refresh_pending
            .load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn queue_projection_refresh_immediate(&self) -> bool {
        self.queue_projection_cache
            .refresh_immediate
            .load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn queue_projection_inputs_share_row_storage(&self) -> Result<bool, String> {
        let cached = self
            .queue_projection_cache
            .state
            .read()
            .map_err(|_| "Binary DB queue projection cache read lock is poisoned".to_string())?
            .current
            .clone()
            .ok_or_else(|| "Binary DB queue projection cache is empty".to_string())?;
        let first = cached.rows.read_model_input(Some("active"), false);
        let second = cached.rows.read_model_input(Some("all"), true);
        Ok(Arc::ptr_eq(&first.tasks, &second.tasks)
            && Arc::ptr_eq(&first.changes, &second.changes)
            && Arc::ptr_eq(&first.patchsets, &second.patchsets)
            && Arc::ptr_eq(&first.reviews, &second.reviews)
            && Arc::ptr_eq(&first.review_requests, &second.review_requests)
            && Arc::ptr_eq(&first.attestations, &second.attestations)
            && Arc::ptr_eq(&first.policy_decisions, &second.policy_decisions))
    }

    /// Requests a refresh and returns without reading workflow rows. Repeated
    /// read retries join an in-flight scan instead of scheduling another full
    /// pass. Mutation requests use the separate path below and still guarantee
    /// a subsequent pass when they race with an active scan.
    pub fn request_queue_projection_refresh(&self) {
        #[cfg(feature = "perfetto-tracing")]
        let _trace =
            crate::perfetto_trace::PerfettoRange::new("ait.server.queue_projection.request.read");
        if self
            .queue_projection_cache
            .refresh_in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            if self
                .queue_projection_cache
                .refresh_pending
                .load(Ordering::Acquire)
            {
                self.queue_projection_cache
                    .refresh_immediate
                    .store(true, Ordering::Release);
            }
            return;
        }
        self.queue_projection_cache
            .refresh_immediate
            .store(true, Ordering::Release);
        self.queue_projection_cache
            .refresh_request_generation
            .fetch_add(1, Ordering::AcqRel);
        self.queue_projection_cache
            .refresh_pending
            .store(true, Ordering::Release);
        self.spawn_reserved_queue_projection_refresh();
    }

    /// Mutation bursts must not start a broad WORKFLOW read transaction in the
    /// gap between review, policy, land, and task-completion writers. The
    /// background worker waits until mutations have been quiet for one bounded
    /// period, while an explicit queue read can still override the delay.
    pub fn request_queue_projection_refresh_after_mutation(&self) {
        #[cfg(feature = "perfetto-tracing")]
        let _trace = crate::perfetto_trace::PerfettoRange::new(
            "ait.server.queue_projection.request.mutation",
        );
        self.request_queue_projection_refresh_with_mode();
    }

    fn request_queue_projection_refresh_with_mode(&self) {
        self.queue_projection_cache
            .refresh_request_generation
            .fetch_add(1, Ordering::AcqRel);
        self.queue_projection_cache
            .refresh_pending
            .store(true, Ordering::Release);
        self.start_queue_projection_refresh_if_idle();
    }

    fn refresh_queue_projection_once(&self) {
        #[cfg(feature = "perfetto-tracing")]
        let _trace =
            crate::perfetto_trace::PerfettoRange::new("ait.server.queue_projection.refresh_once");
        #[cfg(test)]
        self.queue_projection_cache
            .refresh_attempt_count
            .fetch_add(1, Ordering::AcqRel);
        if let Ok(mut state) = self.queue_projection_cache.state.write() {
            state.last_refresh_attempt = Some(Instant::now());
        }
        let refresh = self.load_queue_projection_rows();
        if let Ok(mut state) = self.queue_projection_cache.state.write() {
            match refresh {
                Ok(rows) => {
                    state.current = Some(Arc::new(CachedQueueProjection { rows }));
                    state.last_refresh_error = None;
                }
                Err(error) => state.last_refresh_error = Some(error),
            }
        }
    }

    fn start_queue_projection_refresh_if_idle(&self) {
        if self
            .queue_projection_cache
            .refresh_in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        self.spawn_reserved_queue_projection_refresh();
    }

    fn spawn_reserved_queue_projection_refresh(&self) {
        let service = self.clone();
        let spawn = std::thread::Builder::new()
            .name("ait-queue-projection-refresh".to_string())
            .spawn(move || service.run_queue_projection_refresh_worker());
        if spawn.is_err() {
            self.queue_projection_cache
                .refresh_in_flight
                .store(false, Ordering::Release);
        }
    }

    fn run_queue_projection_refresh_worker(&self) {
        #[cfg(feature = "perfetto-tracing")]
        let _trace =
            crate::perfetto_trace::PerfettoRange::new("ait.server.queue_projection.refresh_worker");
        loop {
            self.wait_for_queue_projection_refresh_admission();
            // Consume all requests that happened before this scan. A request
            // that arrives during the scan sets the flag again and causes one
            // more pass.
            self.queue_projection_cache
                .refresh_pending
                .store(false, Ordering::Release);
            self.refresh_queue_projection_once();
            if self
                .queue_projection_cache
                .refresh_pending
                .load(Ordering::Acquire)
            {
                continue;
            }

            self.queue_projection_cache
                .refresh_in_flight
                .store(false, Ordering::Release);
            if !self
                .queue_projection_cache
                .refresh_pending
                .load(Ordering::Acquire)
            {
                return;
            }
            if self
                .queue_projection_cache
                .refresh_in_flight
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return;
            }
        }
    }

    fn wait_for_queue_projection_refresh_admission(&self) {
        let mut observed_generation = self
            .queue_projection_cache
            .refresh_request_generation
            .load(Ordering::Acquire);
        let mut quiet_started = Instant::now();
        loop {
            if self
                .queue_projection_cache
                .refresh_immediate
                .swap(false, Ordering::AcqRel)
            {
                return;
            }
            let current_generation = self
                .queue_projection_cache
                .refresh_request_generation
                .load(Ordering::Acquire);
            if current_generation != observed_generation {
                observed_generation = current_generation;
                quiet_started = Instant::now();
            }
            let remaining =
                QUEUE_PROJECTION_MUTATION_QUIET_PERIOD.saturating_sub(quiet_started.elapsed());
            if remaining.is_zero() {
                return;
            }
            std::thread::sleep(remaining.min(QUEUE_PROJECTION_DEBOUNCE_POLL_INTERVAL));
        }
    }

    fn load_queue_projection_rows(&self) -> Result<QueueProjectionRows, String> {
        #[cfg(feature = "perfetto-tracing")]
        let _trace =
            crate::perfetto_trace::PerfettoRange::new("ait.server.queue_projection.load_rows");
        let repo_name = self.db.repo_name().as_str();
        if let Some(store) = self
            .workflow
            .as_any()
            .downcast_ref::<BinaryDbServerWorkflowV0Store<D>>()
        {
            return queue_projection_rows_from_bulk_values(
                repo_name,
                store.queue_projection_values_nonblocking()?,
            );
        }
        self.load_queue_projection_rows_from_workflow(repo_name)
    }

    fn load_queue_projection_rows_from_workflow(
        &self,
        repo_name: &str,
    ) -> Result<QueueProjectionRows, String> {
        let tasks = object_rows(self.workflow.list_tasks(repo_name)?, "task list")?;
        let changes = object_rows(self.workflow.list_changes(repo_name)?, "change list")?;
        let mut patchsets = Vec::new();
        let mut reviews = Vec::new();
        let mut review_requests = Vec::new();
        let mut attestations = Vec::new();
        let mut policy_decisions = Vec::new();

        for change in &changes {
            let Some(change_ref) = text_field(change, "change_ref") else {
                continue;
            };
            for patchset in object_rows(
                self.workflow.list_patchsets(Some(repo_name), &change_ref)?,
                "patchset list",
            )? {
                if let Some(patchset_id) = text_field(&patchset, "patchset_id") {
                    if let Ok(attestation) = self.workflow.get_attestation(&patchset_id) {
                        attestations.push(normalize_attestation_row(attestation)?);
                    }
                    if let Ok(policy) = self.workflow.get_policy(&patchset_id) {
                        policy_decisions.push(normalize_policy_row(policy)?);
                    }
                }
                patchsets.push(patchset);
            }

            let review_payload = self.workflow.list_reviews(&change_ref)?;
            let change_reviews = object_rows(
                review_payload
                    .get("reviews")
                    .cloned()
                    .unwrap_or_else(|| JsonValue::Array(Vec::new())),
                "review list",
            )?;
            for review in change_reviews {
                review_requests.extend(review_request_rows_from_review(&review));
                reviews.push(review);
            }
        }

        Ok(QueueProjectionRows {
            repo_name: repo_name.to_string(),
            tasks: Arc::new(tasks),
            changes: Arc::new(changes),
            patchsets: Arc::new(patchsets),
            reviews: Arc::new(reviews),
            review_requests: Arc::new(review_requests),
            attestations: Arc::new(attestations),
            policy_decisions: Arc::new(policy_decisions),
        })
    }
}

fn queue_projection_rows_from_bulk_values(
    repo_name: &str,
    mut values: JsonValue,
) -> Result<QueueProjectionRows, String> {
    #[cfg(feature = "perfetto-tracing")]
    let _trace =
        crate::perfetto_trace::PerfettoRange::new("ait.server.queue_projection.normalize_rows");
    let values = values
        .as_object_mut()
        .ok_or_else(|| "Binary DB queue projection must be a JSON object".to_string())?;
    let tasks = take_projection_object_rows(values, "tasks")?
        .into_iter()
        .filter(|row| queue_row_matches_repo(row, repo_name))
        .collect::<Vec<_>>();
    let changes = take_projection_object_rows(values, "changes")?
        .into_iter()
        .filter(|row| queue_row_matches_repo(row, repo_name))
        .collect::<Vec<_>>();
    let change_refs = changes
        .iter()
        .filter_map(|row| text_field(row, "change_ref"))
        .collect::<BTreeSet<_>>();
    let patchsets = take_projection_object_rows(values, "patchsets")?
        .into_iter()
        .filter(|row| {
            text_field(row, "change_ref")
                .map(|change_ref| change_refs.contains(&change_ref))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    let patchset_ids = patchsets
        .iter()
        .filter_map(|row| text_field(row, "patchset_id"))
        .collect::<BTreeSet<_>>();
    let reviews = take_projection_object_rows(values, "reviews")?
        .into_iter()
        .filter(|row| {
            text_field(row, "change_ref")
                .map(|change_ref| change_refs.contains(&change_ref))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    let review_requests = reviews
        .iter()
        .flat_map(review_request_rows_from_review)
        .collect::<Vec<_>>();

    let mut attestation_by_patchset = BTreeMap::new();
    for value in take_projection_json_values(values, "attestations")? {
        let Some(patchset_id) = value
            .get("patchset_id")
            .and_then(JsonValue::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        if patchset_ids.contains(&patchset_id) {
            attestation_by_patchset.insert(patchset_id, normalize_attestation_row(value)?);
        }
    }

    let mut policy_by_patchset = BTreeMap::new();
    for value in take_projection_json_values(values, "policy_decisions")? {
        let Some(patchset_id) = value
            .get("patchset_id")
            .and_then(JsonValue::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        if patchset_ids.contains(&patchset_id) {
            policy_by_patchset.insert(patchset_id, normalize_policy_row(value)?);
        }
    }
    for patchset_id in &patchset_ids {
        if !policy_by_patchset.contains_key(patchset_id) {
            policy_by_patchset.insert(
                patchset_id.clone(),
                normalize_policy_row(json!({
                    "patchset_id": patchset_id,
                    "decision": "pending",
                    "checks": [],
                }))?,
            );
        }
    }

    Ok(QueueProjectionRows {
        repo_name: repo_name.to_string(),
        tasks: Arc::new(tasks),
        changes: Arc::new(changes),
        patchsets: Arc::new(patchsets),
        reviews: Arc::new(reviews),
        review_requests: Arc::new(review_requests),
        attestations: Arc::new(attestation_by_patchset.into_values().collect()),
        policy_decisions: Arc::new(policy_by_patchset.into_values().collect()),
    })
}

fn take_projection_object_rows(
    values: &mut JsonMap<String, JsonValue>,
    field: &str,
) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
    object_rows(
        values
            .remove(field)
            .unwrap_or_else(|| JsonValue::Array(Vec::new())),
        &format!("Binary DB queue projection {field}"),
    )
}

fn take_projection_json_values(
    values: &mut JsonMap<String, JsonValue>,
    field: &str,
) -> Result<Vec<JsonValue>, String> {
    match values.remove(field) {
        Some(JsonValue::Array(rows)) => Ok(rows),
        _ => Err(format!(
            "Binary DB queue projection {field} must be a JSON array"
        )),
    }
}

fn queue_row_matches_repo(row: &JsonMap<String, JsonValue>, repo_name: &str) -> bool {
    text_field(row, "repo_name")
        .or_else(|| text_field(row, "repository"))
        .map(|row_repo| row_repo == repo_name)
        .unwrap_or(true)
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    #[test]
    fn bulk_queue_projection_joins_task_local_short_changes_by_contextual_ref() {
        let rows = queue_projection_rows_from_bulk_values(
            "repo",
            json!({
                "tasks": [],
                "changes": [
                    {"repo_name":"repo", "change_id":"C-01", "change_ref":"TASK-A/C-01"},
                    {"repo_name":"repo", "change_id":"C-01", "change_ref":"TASK-B/C-01"}
                ],
                "patchsets": [
                    {"patchset_id":"TASK-A/C-01/P-01", "change_id":"C-01", "change_ref":"TASK-A/C-01"},
                    {"patchset_id":"TASK-X/C-01/P-01", "change_id":"C-01", "change_ref":"TASK-X/C-01"}
                ],
                "reviews": [
                    {"review_id":"TASK-B/C-01/RR-01", "change_id":"C-01", "change_ref":"TASK-B/C-01", "patchset_id":"TASK-B/C-01/P-01", "action":"request", "reviewer_groups":["maintainers"]},
                    {"review_id":"TASK-X/C-01/REV-01", "change_id":"C-01", "change_ref":"TASK-X/C-01", "patchset_id":"TASK-X/C-01/P-01", "action":"approve"}
                ],
                "attestations": [],
                "policy_decisions": []
            }),
        )
        .expect("queue projection");

        assert_eq!(rows.patchsets.len(), 1);
        assert_eq!(
            text_field(&rows.patchsets[0], "patchset_id").as_deref(),
            Some("TASK-A/C-01/P-01")
        );
        assert_eq!(rows.reviews.len(), 1);
        assert_eq!(rows.review_requests.len(), 1);
        assert_eq!(
            text_field(&rows.review_requests[0], "change_ref").as_deref(),
            Some("TASK-B/C-01")
        );
    }
}
