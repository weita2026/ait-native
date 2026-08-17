use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::foundation::transport::normalize_async_job_payload;

const DEFAULT_LOCAL_RESERVED_CPU_CORES: usize = 0;
const DEFAULT_DEDICATED_NON_CI_TOKENS: usize = 0;
const DEFAULT_CI_JOB_CPU_TOKENS: usize = 10;
const NORMAL_CI_PRIORITY: i64 = 30;
const FULL_TEST_PRIORITY: i64 = 20;
const MAIN_SEED_PRIORITY: i64 = 80;
const MAINTENANCE_PRIORITY: i64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerDeploymentPosture {
    LocalCoResident,
    DedicatedServer,
}

impl SchedulerDeploymentPosture {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "local" | "local_co_resident" | "local_coresident" => Some(Self::LocalCoResident),
            "dedicated" | "dedicated_server" | "server" => Some(Self::DedicatedServer),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerPolicy {
    pub host_cpu_cores: usize,
    pub reserved_local_cpu_cores: usize,
    pub global_cpu_tokens: usize,
    pub ci_full_shared_cpu_tokens: usize,
    pub full_test_cpu_tokens: usize,
    pub full_test_job_cpu_tokens: usize,
    pub interactive_reserved_tokens: usize,
    pub sync_cpu_tokens: usize,
    pub maintenance_cpu_tokens: usize,
}

impl SchedulerPolicy {
    pub fn detected_host_default() -> Self {
        Self::for_detected_host(SchedulerDeploymentPosture::LocalCoResident)
    }

    pub fn for_detected_host(posture: SchedulerDeploymentPosture) -> Self {
        Self::for_detected_host_with_full_test_job_cpu_tokens(posture, None)
    }

    pub fn for_detected_host_with_full_test_job_cpu_tokens(
        posture: SchedulerDeploymentPosture,
        full_test_job_cpu_tokens_override: Option<usize>,
    ) -> Self {
        Self::for_host_cpu_cores_with_full_test_job_cpu_tokens(
            detected_host_cpu_cores(),
            posture,
            full_test_job_cpu_tokens_override,
        )
    }

    pub fn for_host_cpu_cores(host_cpu_cores: usize, posture: SchedulerDeploymentPosture) -> Self {
        Self::for_host_cpu_cores_with_full_test_job_cpu_tokens(host_cpu_cores, posture, None)
    }

    pub fn for_host_cpu_cores_with_full_test_job_cpu_tokens(
        host_cpu_cores: usize,
        posture: SchedulerDeploymentPosture,
        full_test_job_cpu_tokens_override: Option<usize>,
    ) -> Self {
        let host_cpu_cores = host_cpu_cores.max(1);
        let reserved_local_cpu_cores = match posture {
            SchedulerDeploymentPosture::LocalCoResident => {
                DEFAULT_LOCAL_RESERVED_CPU_CORES.min(host_cpu_cores.saturating_sub(1))
            }
            SchedulerDeploymentPosture::DedicatedServer => 0,
        };
        let global_cpu_tokens = host_cpu_cores
            .saturating_sub(reserved_local_cpu_cores)
            .max(1);
        let ci_full_shared_cpu_tokens = match posture {
            SchedulerDeploymentPosture::LocalCoResident => global_cpu_tokens,
            SchedulerDeploymentPosture::DedicatedServer => global_cpu_tokens
                .saturating_sub(DEFAULT_DEDICATED_NON_CI_TOKENS)
                .max(1)
                .min(global_cpu_tokens),
        };
        let full_test_cpu_tokens = ci_full_shared_cpu_tokens;
        let full_test_job_cpu_tokens = DEFAULT_CI_JOB_CPU_TOKENS.min(ci_full_shared_cpu_tokens);
        let full_test_job_cpu_tokens = full_test_job_cpu_tokens_override
            .unwrap_or(full_test_job_cpu_tokens)
            .max(1)
            .min(ci_full_shared_cpu_tokens);

        Self {
            host_cpu_cores,
            reserved_local_cpu_cores,
            global_cpu_tokens,
            ci_full_shared_cpu_tokens,
            full_test_cpu_tokens,
            full_test_job_cpu_tokens,
            interactive_reserved_tokens: 2.min(global_cpu_tokens),
            sync_cpu_tokens: 2.min(global_cpu_tokens),
            maintenance_cpu_tokens: 1.min(global_cpu_tokens),
        }
    }

    pub fn local_co_resident_10_core_default() -> Self {
        Self::for_host_cpu_cores(10, SchedulerDeploymentPosture::LocalCoResident)
    }

    pub fn dedicated_server_10_core_default() -> Self {
        Self::for_host_cpu_cores(10, SchedulerDeploymentPosture::DedicatedServer)
    }
}

impl Default for SchedulerPolicy {
    fn default() -> Self {
        Self::detected_host_default()
    }
}

pub fn detected_host_cpu_cores() -> usize {
    std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchedulerJobClass {
    NormalCi,
    FullTest,
    Maintenance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerJobSpec {
    pub job_kind: String,
    pub job_class: SchedulerJobClass,
    pub read_keys: Vec<String>,
    pub write_keys: Vec<String>,
    pub singleflight_key: Option<String>,
    pub cpu_tokens: usize,
    pub token_pools: Vec<String>,
    pub priority: i64,
    pub queue_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerQueuedJob {
    pub job_id: String,
    pub spec: SchedulerJobSpec,
    pub queued_ordinal: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerRunningJob {
    pub job_id: String,
    pub spec: SchedulerJobSpec,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchedulerAdmissionDecision {
    Admit {
        job_id: String,
    },
    Attach {
        job_id: String,
        active_job_id: String,
        singleflight_key: String,
    },
    Wait {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TokenUsage {
    global_cpu_tokens: usize,
    ci_full_shared_cpu_tokens: usize,
    full_test_cpu_tokens: usize,
}

pub fn scheduler_job_spec_from_async_job<'a, P>(
    job_type: &str,
    payload: P,
) -> Result<SchedulerJobSpec, String>
where
    P: AsyncSchedulerPayloadInput<'a>,
{
    let policy = SchedulerPolicy::default();
    scheduler_job_spec_from_async_job_with_policy(job_type, payload, &policy)
}

pub fn scheduler_job_spec_from_async_job_with_policy<'a, P>(
    job_type: &str,
    payload: P,
    policy: &SchedulerPolicy,
) -> Result<SchedulerJobSpec, String>
where
    P: AsyncSchedulerPayloadInput<'a>,
{
    let normalized = normalize_async_job_payload(job_type, payload.into_payload())?;
    match job_type {
        "patchset.ci" => patchset_ci_spec(&normalized, policy),
        "patchset.ci.aggregate" => patchset_ci_aggregate_spec(&normalized, policy),
        "repo.ci" => repo_ci_spec(&normalized, policy),
        "main-seed.refresh" => main_seed_refresh_spec(&normalized, policy),
        "content.gc" => content_gc_spec(&normalized),
        _ => Err(format!(
            "{job_type} is not registered with the scheduler contract."
        )),
    }
}

pub fn scheduler_queued_job_from_async_job_with_policy<'a, P>(
    job_id: impl Into<String>,
    queued_ordinal: usize,
    job_type: &str,
    payload: P,
    policy: &SchedulerPolicy,
) -> Result<SchedulerQueuedJob, String>
where
    P: AsyncSchedulerPayloadInput<'a>,
{
    Ok(SchedulerQueuedJob {
        job_id: job_id.into(),
        spec: scheduler_job_spec_from_async_job_with_policy(job_type, payload, policy)?,
        queued_ordinal,
    })
}

pub fn scheduler_running_job_from_async_job_with_policy<'a, P>(
    job_id: impl Into<String>,
    job_type: &str,
    payload: P,
    policy: &SchedulerPolicy,
) -> Result<SchedulerRunningJob, String>
where
    P: AsyncSchedulerPayloadInput<'a>,
{
    Ok(SchedulerRunningJob {
        job_id: job_id.into(),
        spec: scheduler_job_spec_from_async_job_with_policy(job_type, payload, policy)?,
    })
}

pub fn admit_next(
    queued: &[SchedulerQueuedJob],
    running: &[SchedulerRunningJob],
    policy: &SchedulerPolicy,
) -> SchedulerAdmissionDecision {
    for queued_job in queued {
        if let Some(singleflight_key) = &queued_job.spec.singleflight_key {
            if let Some(active) = running
                .iter()
                .find(|active| active.spec.singleflight_key.as_ref() == Some(singleflight_key))
            {
                return SchedulerAdmissionDecision::Attach {
                    job_id: queued_job.job_id.clone(),
                    active_job_id: active.job_id.clone(),
                    singleflight_key: singleflight_key.clone(),
                };
            }
        }
    }

    let usage = token_usage(running);
    let mut candidates: Vec<&SchedulerQueuedJob> = queued.iter().collect();
    candidates.sort_by(|left, right| {
        right
            .spec
            .priority
            .cmp(&left.spec.priority)
            .then_with(|| left.queued_ordinal.cmp(&right.queued_ordinal))
    });

    let mut blocked_reasons = Vec::new();
    for candidate in candidates {
        if let Some(conflict) = first_conflicting_job(&candidate.spec, running) {
            blocked_reasons.push(format!(
                "{} conflicts with running job {}",
                candidate.job_id, conflict
            ));
            continue;
        }
        if let Some(reason) = token_block_reason(&candidate.spec, &usage, policy) {
            blocked_reasons.push(format!("{} {reason}", candidate.job_id));
            continue;
        }
        return SchedulerAdmissionDecision::Admit {
            job_id: candidate.job_id.clone(),
        };
    }

    SchedulerAdmissionDecision::Wait {
        reason: if blocked_reasons.is_empty() {
            "no queued jobs".to_string()
        } else {
            blocked_reasons.join("; ")
        },
    }
}

pub trait AsyncSchedulerPayloadInput<'a> {
    fn into_payload(self) -> Option<&'a JsonMap<String, JsonValue>>;
}

impl<'a> AsyncSchedulerPayloadInput<'a> for &'a JsonMap<String, JsonValue> {
    fn into_payload(self) -> Option<&'a JsonMap<String, JsonValue>> {
        Some(self)
    }
}

impl<'a> AsyncSchedulerPayloadInput<'a> for Option<&'a JsonMap<String, JsonValue>> {
    fn into_payload(self) -> Option<&'a JsonMap<String, JsonValue>> {
        self
    }
}

fn patchset_ci_spec(
    payload: &JsonMap<String, JsonValue>,
    policy: &SchedulerPolicy,
) -> Result<SchedulerJobSpec, String> {
    let patchset_id = required_text(payload, "patchset_id")?;
    let explicit_suite_id = optional_text(payload, "suite_id");
    let suite_id = explicit_suite_id.unwrap_or_else(|| "default".to_string());
    let suite_ids = suite_ids(payload);
    let suite_key = suite_ids.join("+");
    let snapshot_id = optional_text(payload, "revision_snapshot_id")
        .or_else(|| optional_text(payload, "snapshot_id"))
        .unwrap_or_else(|| "unknown-snapshot".to_string());
    let repo_scope = repo_scope(payload);
    let full_test = is_full_test_suite(&suite_id)
        || suite_ids
            .iter()
            .any(|suite_id| is_full_test_suite(suite_id));
    let tg1_required = is_tg1_required_suite(&suite_id)
        || suite_ids
            .iter()
            .any(|suite_id| is_tg1_required_suite(suite_id));

    let mut read_keys = Vec::from([format!("patchset:{patchset_id}")]);
    if let Some(repo_scope) = &repo_scope {
        read_keys.push(format!("{repo_scope}:snapshot:{snapshot_id}"));
    }

    let mut write_keys: Vec<String> = suite_ids
        .iter()
        .map(|suite_id| format!("patchset:{patchset_id}:ci:{suite_id}"))
        .collect();
    if let Some(repo_scope) = &repo_scope {
        write_keys.extend(
            suite_ids
                .iter()
                .map(|suite_id| format!("{repo_scope}:ci-shard-pool:patchset-ci:{suite_id}")),
        );
    }

    Ok(ci_spec(
        "patchset.ci",
        if full_test {
            SchedulerJobClass::FullTest
        } else {
            SchedulerJobClass::NormalCi
        },
        read_keys,
        write_keys,
        Some(format!(
            "patchset.ci:{patchset_id}:{suite_key}:{snapshot_id}"
        )),
        format!("patchset:{patchset_id}:ci:{suite_key}"),
        policy,
        if full_test {
            None
        } else if tg1_required {
            Some(tg1_required_cpu_tokens(policy))
        } else {
            Some(patchset_ci_cpu_tokens(policy))
        },
    ))
}

fn patchset_ci_aggregate_spec(
    payload: &JsonMap<String, JsonValue>,
    policy: &SchedulerPolicy,
) -> Result<SchedulerJobSpec, String> {
    let patchset_id = required_text(payload, "patchset_id")?;
    let suite_ids = suite_ids(payload);
    let suite_key = suite_ids.join("+");
    let stage = optional_text(payload, "stage").unwrap_or_else(|| "ready_blocking".to_string());
    let snapshot_id = optional_text(payload, "revision_snapshot_id")
        .or_else(|| optional_text(payload, "snapshot_id"))
        .unwrap_or_else(|| "unknown-snapshot".to_string());

    let mut read_keys = Vec::from([format!("patchset:{patchset_id}")]);
    read_keys.extend(
        suite_ids
            .iter()
            .map(|suite_id| format!("patchset:{patchset_id}:ci:{suite_id}")),
    );
    let write_keys = Vec::from([format!("patchset:{patchset_id}:attestation")]);

    Ok(ci_spec(
        "patchset.ci.aggregate",
        SchedulerJobClass::NormalCi,
        read_keys,
        write_keys,
        Some(format!(
            "patchset.ci.aggregate:{patchset_id}:{stage}:{suite_key}:{snapshot_id}"
        )),
        format!("patchset:{patchset_id}:ci.aggregate:{stage}:{suite_key}"),
        policy,
        None,
    ))
}

fn repo_ci_spec(
    payload: &JsonMap<String, JsonValue>,
    policy: &SchedulerPolicy,
) -> Result<SchedulerJobSpec, String> {
    let repo_name = required_text(payload, "repo_name")?;
    let repo_scope = repo_scope(payload).unwrap_or_else(|| format!("repo:{repo_name}"));
    let plane = optional_text(payload, "plane").unwrap_or_else(|| "default".to_string());
    let target_line = optional_text(payload, "target_line").unwrap_or_else(|| "main".to_string());
    let snapshot_id =
        optional_text(payload, "snapshot_id").unwrap_or_else(|| "unknown-snapshot".to_string());
    let suite_ids = suite_ids(payload);
    let full_test = suite_ids
        .iter()
        .any(|suite_id| is_full_test_suite(suite_id));
    let suite_key = suite_ids.join("+");

    let read_keys = Vec::from([
        format!("{repo_scope}:line:{target_line}"),
        format!("{repo_scope}:snapshot:{snapshot_id}"),
    ]);
    let mut write_keys: Vec<String> = suite_ids
        .iter()
        .map(|suite_id| format!("{repo_scope}:ci:{plane}:{suite_id}"))
        .collect();
    write_keys.extend(suite_ids.iter().map(|suite_id| {
        format!("{repo_scope}:ci-shard-pool:repo-ci:{plane}:{target_line}:{suite_id}")
    }));

    Ok(ci_spec(
        "repo.ci",
        if full_test {
            SchedulerJobClass::FullTest
        } else {
            SchedulerJobClass::NormalCi
        },
        read_keys,
        write_keys,
        Some(format!(
            "repo.ci:{repo_scope}:{plane}:{suite_key}:{snapshot_id}"
        )),
        format!("{repo_scope}:ci:{plane}:{suite_key}"),
        policy,
        if full_test {
            None
        } else {
            Some(patchset_ci_cpu_tokens(policy))
        },
    ))
}

fn main_seed_refresh_spec(
    payload: &JsonMap<String, JsonValue>,
    policy: &SchedulerPolicy,
) -> Result<SchedulerJobSpec, String> {
    let repo_name = required_text(payload, "repo_name")?;
    let snapshot_id = required_text(payload, "snapshot_id")?;
    let repo_scope = repo_scope(payload).unwrap_or_else(|| format!("repo:{repo_name}"));
    Ok(SchedulerJobSpec {
        job_kind: "main-seed.refresh".to_string(),
        job_class: SchedulerJobClass::FullTest,
        read_keys: Vec::from([format!("{repo_scope}:snapshot:{snapshot_id}")]),
        write_keys: Vec::from([format!("{repo_scope}:main-seed")]),
        singleflight_key: Some(format!("main-seed.refresh:{repo_scope}:{snapshot_id}")),
        cpu_tokens: full_test_job_cpu_tokens(policy),
        token_pools: Vec::from([
            "global_cpu_tokens".to_string(),
            "ci_full_shared_cpu_tokens".to_string(),
            "full_test_cpu_tokens".to_string(),
        ]),
        priority: MAIN_SEED_PRIORITY,
        queue_key: format!("{repo_scope}:main-seed.refresh"),
    })
}

fn content_gc_spec(payload: &JsonMap<String, JsonValue>) -> Result<SchedulerJobSpec, String> {
    let repo_name = required_text(payload, "repo_name")?;
    let repo_scope = format!("repo:{repo_name}");
    Ok(SchedulerJobSpec {
        job_kind: "content.gc".to_string(),
        job_class: SchedulerJobClass::Maintenance,
        read_keys: Vec::from([format!("{repo_scope}:content")]),
        write_keys: Vec::from([format!("{repo_scope}:content")]),
        singleflight_key: Some(format!("content.gc:{repo_scope}")),
        cpu_tokens: 1,
        token_pools: Vec::from([
            "global_cpu_tokens".to_string(),
            "maintenance_cpu_tokens".to_string(),
        ]),
        priority: MAINTENANCE_PRIORITY,
        queue_key: format!("{repo_scope}:content.gc"),
    })
}

fn ci_spec(
    job_kind: &str,
    job_class: SchedulerJobClass,
    read_keys: Vec<String>,
    write_keys: Vec<String>,
    singleflight_key: Option<String>,
    queue_key: String,
    policy: &SchedulerPolicy,
    cpu_tokens_override: Option<usize>,
) -> SchedulerJobSpec {
    let full_test = job_class == SchedulerJobClass::FullTest;
    SchedulerJobSpec {
        job_kind: job_kind.to_string(),
        job_class,
        read_keys,
        write_keys,
        singleflight_key,
        cpu_tokens: cpu_tokens_override.unwrap_or_else(|| {
            if full_test {
                full_test_job_cpu_tokens(policy)
            } else {
                1
            }
        }),
        token_pools: if full_test {
            Vec::from([
                "global_cpu_tokens".to_string(),
                "ci_full_shared_cpu_tokens".to_string(),
                "full_test_cpu_tokens".to_string(),
            ])
        } else {
            Vec::from([
                "global_cpu_tokens".to_string(),
                "ci_full_shared_cpu_tokens".to_string(),
            ])
        },
        priority: if full_test {
            FULL_TEST_PRIORITY
        } else {
            NORMAL_CI_PRIORITY
        },
        queue_key,
    }
}

fn full_test_job_cpu_tokens(policy: &SchedulerPolicy) -> usize {
    let pool_cap = [
        policy.global_cpu_tokens,
        policy.ci_full_shared_cpu_tokens,
        policy.full_test_cpu_tokens,
    ]
    .into_iter()
    .filter(|value| *value > 0)
    .min()
    .unwrap_or(1);
    policy.full_test_job_cpu_tokens.max(1).min(pool_cap)
}

fn tg1_required_cpu_tokens(policy: &SchedulerPolicy) -> usize {
    patchset_ci_cpu_tokens(policy)
}

fn patchset_ci_cpu_tokens(policy: &SchedulerPolicy) -> usize {
    DEFAULT_CI_JOB_CPU_TOKENS
        .min(policy.global_cpu_tokens.max(1))
        .min(policy.ci_full_shared_cpu_tokens.max(1))
}

fn first_conflicting_job(
    candidate: &SchedulerJobSpec,
    running: &[SchedulerRunningJob],
) -> Option<String> {
    running
        .iter()
        .find(|active| specs_conflict(candidate, &active.spec))
        .map(|active| active.job_id.clone())
}

fn specs_conflict(left: &SchedulerJobSpec, right: &SchedulerJobSpec) -> bool {
    any_key_conflicts(&left.write_keys, &right.write_keys)
        || any_key_conflicts(&left.write_keys, &right.read_keys)
        || any_key_conflicts(&right.write_keys, &left.read_keys)
}

fn any_key_conflicts(left: &[String], right: &[String]) -> bool {
    left.iter().any(|left_key| {
        right
            .iter()
            .any(|right_key| key_conflicts(left_key, right_key))
    })
}

fn key_conflicts(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    wildcard_prefix(left)
        .map(|prefix| right.starts_with(prefix))
        .unwrap_or(false)
        || wildcard_prefix(right)
            .map(|prefix| left.starts_with(prefix))
            .unwrap_or(false)
}

fn wildcard_prefix(key: &str) -> Option<&str> {
    key.strip_suffix('*')
}

fn token_usage(running: &[SchedulerRunningJob]) -> TokenUsage {
    let mut usage = TokenUsage {
        global_cpu_tokens: 0,
        ci_full_shared_cpu_tokens: 0,
        full_test_cpu_tokens: 0,
    };
    for job in running {
        usage.global_cpu_tokens += job.spec.cpu_tokens;
        if job
            .spec
            .token_pools
            .iter()
            .any(|pool| pool == "ci_full_shared_cpu_tokens")
        {
            usage.ci_full_shared_cpu_tokens += job.spec.cpu_tokens;
        }
        if job
            .spec
            .token_pools
            .iter()
            .any(|pool| pool == "full_test_cpu_tokens")
        {
            usage.full_test_cpu_tokens += job.spec.cpu_tokens;
        }
    }
    usage
}

fn token_block_reason(
    spec: &SchedulerJobSpec,
    usage: &TokenUsage,
    policy: &SchedulerPolicy,
) -> Option<String> {
    if usage.global_cpu_tokens + spec.cpu_tokens > policy.global_cpu_tokens {
        return Some("waits for global_cpu_tokens".to_string());
    }
    if spec
        .token_pools
        .iter()
        .any(|pool| pool == "ci_full_shared_cpu_tokens")
        && usage.ci_full_shared_cpu_tokens + spec.cpu_tokens > policy.ci_full_shared_cpu_tokens
    {
        return Some("waits for ci_full_shared_cpu_tokens".to_string());
    }
    if spec
        .token_pools
        .iter()
        .any(|pool| pool == "full_test_cpu_tokens")
        && usage.full_test_cpu_tokens + spec.cpu_tokens > policy.full_test_cpu_tokens
    {
        return Some("waits for full_test_cpu_tokens".to_string());
    }
    None
}

fn required_text(payload: &JsonMap<String, JsonValue>, field: &str) -> Result<String, String> {
    optional_text(payload, field).ok_or_else(|| format!("scheduler requires `{field}`."))
}

fn optional_text(payload: &JsonMap<String, JsonValue>, field: &str) -> Option<String> {
    payload
        .get(field)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn repo_scope(payload: &JsonMap<String, JsonValue>) -> Option<String> {
    optional_text(payload, "repo_id")
        .or_else(|| optional_text(payload, "repo_name"))
        .map(|value| format!("repo:{value}"))
}

fn suite_ids(payload: &JsonMap<String, JsonValue>) -> Vec<String> {
    match payload.get("suite_ids") {
        Some(JsonValue::Array(values)) => {
            let mut out: Vec<String> = values
                .iter()
                .filter_map(JsonValue::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect();
            if out.is_empty() {
                out.push("default".to_string());
            }
            out
        }
        Some(JsonValue::String(value)) if !value.trim().is_empty() => {
            Vec::from([value.trim().to_string()])
        }
        _ => {
            Vec::from([optional_text(payload, "suite_id").unwrap_or_else(|| "default".to_string())])
        }
    }
}

fn is_full_test_suite(suite_id: &str) -> bool {
    matches!(
        suite_id.trim().to_ascii_lowercase().as_str(),
        "full"
            | "full-test"
            | "full_test"
            | "full-repo"
            | "full_repo"
            | "full_repo_zstd_only"
            | "all"
    )
}

fn is_tg1_required_suite(suite_id: &str) -> bool {
    matches!(
        suite_id.trim().to_ascii_lowercase().as_str(),
        "tg1" | "tg-1" | "tg1_required" | "tg1_required_zstd_only" | "tg-1-required"
    )
}
