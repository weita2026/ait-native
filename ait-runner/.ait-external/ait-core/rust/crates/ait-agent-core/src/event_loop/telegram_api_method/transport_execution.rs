use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use crate::file_store::agent_file_store_read_bytes_json;
use crate::transport::{
    agent_transport_http_execute_bytes_request,
    agent_transport_http_execute_multipart_json_request_with_bytes,
    agent_transport_retry_delay_seconds, AgentTransportHttpBytesExecution,
};

use super::execution::{
    execute_with_telegram_api_json_ports, TelegramApiJsonHttpExecutor, TelegramApiRetrySleeper,
    ThreadTelegramApiRetrySleeper,
};
use super::planning::agent_telegram_api_method_execution_plan_json;

const CONTRACT: &str = "ait_agent_core.event_loop.TelegramApiTransportExecution.v1";
const MIGRATION_STAGE: &str = "rust_agent_telegram_transport_execution";
const API_METHOD_EXECUTION_KIND: &str = "telegram_api_method";
const DELIVERY_MAX_ATTEMPTS: u64 = 3;
const RETRY_BASE_DELAY_SECONDS: f64 = 1.0;
const MAX_URL_BYTES: usize = 4_096;
const MAX_LOCAL_PATH_BYTES: usize = 4_096;
const MAX_MULTIPART_FIELDS: usize = 32;
const MAX_MULTIPART_FIELD_BYTES: usize = 32 * 1_024;
const MAX_FILE_NAME_BYTES: usize = 255;
const MAX_MIME_TYPE_BYTES: usize = 255;
const MAX_ATTACHMENT_BYTES: usize = 50 * 1024 * 1024;
static MULTIPART_BOUNDARY_COUNTER: AtomicU64 = AtomicU64::new(1);

pub trait TelegramApiTransportExecutor {
    fn execute_json_request(&self, request: &JsonValue) -> Result<JsonValue, String>;

    fn execute_multipart_request(
        &self,
        request: &JsonValue,
        file_bytes: &[u8],
    ) -> Result<JsonValue, String>;

    fn execute_bytes_request(
        &self,
        request: &JsonValue,
    ) -> Result<AgentTransportHttpBytesExecution, String>;

    fn read_attachment_bytes(&self, path: &Path) -> Result<Vec<u8>, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NativeTelegramApiTransportExecutor;

impl TelegramApiTransportExecutor for NativeTelegramApiTransportExecutor {
    fn execute_json_request(&self, request: &JsonValue) -> Result<JsonValue, String> {
        crate::transport::agent_transport_http_execute_json_request_json(request)
    }

    fn execute_multipart_request(
        &self,
        request: &JsonValue,
        file_bytes: &[u8],
    ) -> Result<JsonValue, String> {
        agent_transport_http_execute_multipart_json_request_with_bytes(request, file_bytes)
    }

    fn execute_bytes_request(
        &self,
        request: &JsonValue,
    ) -> Result<AgentTransportHttpBytesExecution, String> {
        agent_transport_http_execute_bytes_request(request)
    }

    fn read_attachment_bytes(&self, path: &Path) -> Result<Vec<u8>, String> {
        agent_file_store_read_bytes_json(&json!({"path": path.to_string_lossy()}))
            .map(|(_, payload)| payload)
    }
}

#[derive(Clone, PartialEq)]
pub struct TelegramApiTransportExecution {
    metadata: JsonValue,
    downloaded_bytes: Option<Vec<u8>>,
}

impl fmt::Debug for TelegramApiTransportExecution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TelegramApiTransportExecution")
            .field("operation", &self.metadata.get("operation"))
            .field("state", &self.metadata.get("telegram_api_state"))
            .field("ok", &self.metadata.get("ok"))
            .field(
                "downloaded_byte_count",
                &self.downloaded_bytes.as_ref().map(Vec::len),
            )
            .finish()
    }
}

impl TelegramApiTransportExecution {
    pub fn metadata(&self) -> &JsonValue {
        &self.metadata
    }

    pub fn downloaded_bytes(&self) -> Option<&[u8]> {
        self.downloaded_bytes.as_deref()
    }

    pub fn into_parts(self) -> (JsonValue, Option<Vec<u8>>) {
        (self.metadata, self.downloaded_bytes)
    }

    fn metadata_only(metadata: JsonValue) -> Self {
        Self {
            metadata,
            downloaded_bytes: None,
        }
    }

    fn downloaded(metadata: JsonValue, payload: Vec<u8>) -> Self {
        Self {
            metadata,
            downloaded_bytes: Some(payload),
        }
    }
}

pub fn agent_telegram_api_execute(
    request: &JsonValue,
) -> Result<TelegramApiTransportExecution, String> {
    execute_with_telegram_api_transport_ports(
        &NativeTelegramApiTransportExecutor,
        &ThreadTelegramApiRetrySleeper,
        request,
    )
}

pub fn execute_with_telegram_api_transport_ports<E, S>(
    executor: &E,
    sleeper: &S,
    request: &JsonValue,
) -> Result<TelegramApiTransportExecution, String>
where
    E: TelegramApiTransportExecutor + ?Sized,
    S: TelegramApiRetrySleeper + ?Sized,
{
    let object = request
        .as_object()
        .ok_or_else(|| "Telegram API transport execution request must be an object.".to_string())?;
    let requested_progress = ExecutionProgress::from_request(object);
    let planned_request = match planned_execution_request(object) {
        Ok(request) => request,
        Err(failure) => {
            return Ok(TelegramApiTransportExecution::metadata_only(
                failure_payload(
                    &requested_progress,
                    failure.state,
                    failure.error_kind,
                    failure.error,
                    false,
                ),
            ))
        }
    };
    let transport = clean_text(planned_request.get("transport")).unwrap_or_default();
    match transport.as_str() {
        "json" => execute_json_transport(executor, sleeper, request, requested_progress),
        "bytes" => {
            let plan = match BytesPlan::parse(&planned_request) {
                Ok(plan) => plan,
                Err(failure) => {
                    return Ok(TelegramApiTransportExecution::metadata_only(
                        failure_payload(
                            &requested_progress,
                            failure.state,
                            failure.error_kind,
                            failure.error,
                            false,
                        ),
                    ))
                }
            };
            execute_bytes_transport(executor, sleeper, plan)
        }
        "multipart" => {
            let plan = match MultipartPlan::parse(&planned_request) {
                Ok(plan) => plan,
                Err(failure) => {
                    return Ok(TelegramApiTransportExecution::metadata_only(
                        failure_payload(
                            &requested_progress,
                            failure.state,
                            failure.error_kind,
                            failure.error,
                            false,
                        ),
                    ))
                }
            };
            execute_multipart_transport(executor, sleeper, plan)
        }
        _ => Ok(TelegramApiTransportExecution::metadata_only(
            failure_payload(
                &requested_progress,
                "unsupported_operation_or_transport",
                "unsupported",
                "Telegram API execution does not support this planned transport.",
                false,
            ),
        )),
    }
}

struct JsonExecutorAdapter<'a, E: ?Sized>(&'a E);

impl<E> TelegramApiJsonHttpExecutor for JsonExecutorAdapter<'_, E>
where
    E: TelegramApiTransportExecutor + ?Sized,
{
    fn execute_json_request(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.0.execute_json_request(request)
    }
}

fn execute_json_transport<E, S>(
    executor: &E,
    sleeper: &S,
    request: &JsonValue,
    progress: ExecutionProgress,
) -> Result<TelegramApiTransportExecution, String>
where
    E: TelegramApiTransportExecutor + ?Sized,
    S: TelegramApiRetrySleeper + ?Sized,
{
    let outcome = match execute_with_telegram_api_json_ports(
        &JsonExecutorAdapter(executor),
        sleeper,
        request,
    ) {
        Ok(outcome) => outcome,
        Err(_) => {
            return Ok(TelegramApiTransportExecution::metadata_only(
                failure_payload(
                    &progress,
                    "executor_failed",
                    "executor",
                    "Telegram JSON API execution failed.",
                    false,
                ),
            ))
        }
    };
    let Some(mut metadata) = outcome.as_object().cloned() else {
        return Ok(TelegramApiTransportExecution::metadata_only(
            failure_payload(
                &progress,
                "result_contract_failed",
                "contract",
                "Telegram JSON API execution returned an invalid result.",
                false,
            ),
        ));
    };
    metadata.insert("contract".to_string(), json!(CONTRACT));
    metadata.insert("migration_stage".to_string(), json!(MIGRATION_STAGE));
    metadata.insert("transport".to_string(), json!("json"));
    metadata.insert("downloaded".to_string(), json!(false));
    metadata.insert("byte_count".to_string(), JsonValue::Null);
    add_safety_flags(&mut metadata);
    Ok(TelegramApiTransportExecution::metadata_only(
        JsonValue::Object(metadata),
    ))
}

struct PlanFailure {
    state: &'static str,
    error_kind: &'static str,
    error: &'static str,
}

impl PlanFailure {
    fn contract(error: &'static str) -> Self {
        Self {
            state: "planning_contract_failed",
            error_kind: "contract",
            error,
        }
    }

    fn rejected() -> Self {
        Self {
            state: "planning_rejected",
            error_kind: "planning",
            error: "Telegram API request planning was rejected.",
        }
    }
}

fn planned_execution_request(
    object: &Map<String, JsonValue>,
) -> Result<Map<String, JsonValue>, PlanFailure> {
    let mut planner_request = object.clone();
    planner_request.insert("stage".to_string(), json!("request"));
    let planned =
        agent_telegram_api_method_execution_plan_json(&JsonValue::Object(planner_request))
            .map_err(|_| PlanFailure::contract("Telegram API request planning failed."))?;
    let planned = planned.as_object().ok_or_else(|| {
        PlanFailure::contract("Telegram API request planning returned an invalid contract.")
    })?;
    if clean_text(planned.get("stage")).as_deref() != Some("request")
        || clean_text(planned.get("execution_kind")).as_deref() != Some(API_METHOD_EXECUTION_KIND)
    {
        return Err(PlanFailure::contract(
            "Telegram API request planning returned an invalid identity.",
        ));
    }
    match planned.get("should_execute").and_then(JsonValue::as_bool) {
        Some(false) => return Err(PlanFailure::rejected()),
        Some(true) => {}
        None => {
            return Err(PlanFailure::contract(
                "Telegram API request planning returned an invalid execution flag.",
            ))
        }
    }
    planned
        .get("request")
        .and_then(JsonValue::as_object)
        .cloned()
        .ok_or_else(|| {
            PlanFailure::contract("Telegram API request planning omitted its execution request.")
        })
}

struct BytesPlan {
    progress: ExecutionProgress,
    http_request: JsonValue,
}

impl BytesPlan {
    fn parse(request: &Map<String, JsonValue>) -> Result<Self, PlanFailure> {
        validate_plan_identity(request)?;
        if clean_text(request.get("operation")).as_deref() != Some("download_file")
            || clean_text(request.get("transport")).as_deref() != Some("bytes")
            || clean_text(request.get("method")).as_deref() != Some("GET")
            || clean_text(request.get("telegram_method")).as_deref() != Some("downloadFile")
            || clean_text(request.get("result_kind")).as_deref() != Some("bytes")
            || clean_text(request.get("retry_family")).as_deref() != Some("delivery")
        {
            return Err(PlanFailure::contract(
                "Telegram bytes execution received an invalid request contract.",
            ));
        }
        let url = validated_url(request.get("url"))?;
        let timeout = validated_timeout(request.get("timeout"))?;
        Ok(Self {
            progress: ExecutionProgress::planned("download_file", "downloadFile", "bytes"),
            http_request: json!({
                "method": "GET",
                "url": url,
                "timeout_seconds": timeout,
            }),
        })
    }
}

struct MultipartPlan {
    progress: ExecutionProgress,
    execution_request: JsonValue,
    url: String,
    timeout: JsonValue,
    fields: Map<String, JsonValue>,
    file_field: &'static str,
    file_name: String,
    mime_type: String,
    local_path: String,
}

impl MultipartPlan {
    fn parse(request: &Map<String, JsonValue>) -> Result<Self, PlanFailure> {
        validate_plan_identity(request)?;
        let telegram_method =
            attachment_method(request.get("telegram_method")).ok_or_else(|| {
                PlanFailure::contract("Telegram multipart execution received an invalid method.")
            })?;
        let expected_file_field = attachment_file_field(telegram_method);
        if clean_text(request.get("operation")).as_deref() != Some("send_attachment")
            || clean_text(request.get("transport")).as_deref() != Some("multipart")
            || clean_text(request.get("method")).as_deref() != Some("POST")
            || clean_text(request.get("result_kind")).as_deref() != Some("unit")
            || clean_text(request.get("retry_family")).as_deref() != Some("delivery")
            || clean_text(request.get("file_field")).as_deref() != Some(expected_file_field)
        {
            return Err(PlanFailure::contract(
                "Telegram multipart execution received an invalid request contract.",
            ));
        }
        let url = validated_url(request.get("url"))?;
        if !url.ends_with(&format!("/{telegram_method}")) {
            return Err(PlanFailure::contract(
                "Telegram multipart execution received an invalid method URL.",
            ));
        }
        let timeout = validated_timeout(request.get("timeout"))?;
        let fields = request
            .get("fields")
            .and_then(JsonValue::as_object)
            .cloned()
            .ok_or_else(|| {
                PlanFailure::contract("Telegram multipart execution requires bounded fields.")
            })?;
        validate_multipart_fields(&fields)?;
        let file_name = clean_text(request.get("file_name")).ok_or_else(|| {
            PlanFailure::contract("Telegram multipart execution requires a file name.")
        })?;
        if !valid_file_name(&file_name) {
            return Err(PlanFailure::contract(
                "Telegram multipart execution received an invalid file name.",
            ));
        }
        let mime_type = clean_text(request.get("mime_type")).ok_or_else(|| {
            PlanFailure::contract("Telegram multipart execution requires a MIME type.")
        })?;
        if !valid_mime_type(&mime_type) {
            return Err(PlanFailure::contract(
                "Telegram multipart execution received an invalid MIME type.",
            ));
        }
        let local_path = clean_text(request.get("local_path")).ok_or_else(|| {
            PlanFailure::contract("Telegram multipart execution requires a local path.")
        })?;
        if local_path.len() > MAX_LOCAL_PATH_BYTES
            || local_path
                .chars()
                .any(|ch| ch == '\0' || ch == '\r' || ch == '\n')
        {
            return Err(PlanFailure::contract(
                "Telegram multipart execution received an invalid local path.",
            ));
        }
        Ok(Self {
            progress: ExecutionProgress::planned("send_attachment", telegram_method, "multipart"),
            execution_request: JsonValue::Object(request.clone()),
            url,
            timeout,
            fields,
            file_field: expected_file_field,
            file_name,
            mime_type,
            local_path,
        })
    }
}

fn validate_plan_identity(request: &Map<String, JsonValue>) -> Result<(), PlanFailure> {
    if clean_text(request.get("execution_kind")).as_deref() != Some(API_METHOD_EXECUTION_KIND)
        || request.get("ok").and_then(JsonValue::as_bool) != Some(true)
    {
        return Err(PlanFailure::contract(
            "Telegram API execution received an invalid planner identity.",
        ));
    }
    Ok(())
}

fn validated_url(value: Option<&JsonValue>) -> Result<String, PlanFailure> {
    let url = clean_text(value).ok_or_else(|| {
        PlanFailure::contract("Telegram API execution requires an HTTP or HTTPS URL.")
    })?;
    if url.len() > MAX_URL_BYTES
        || !(url.starts_with("http://") || url.starts_with("https://"))
        || url.chars().any(|ch| ch == '\r' || ch == '\n' || ch == '\0')
    {
        return Err(PlanFailure::contract(
            "Telegram API execution received an invalid URL.",
        ));
    }
    Ok(url)
}

fn validated_timeout(value: Option<&JsonValue>) -> Result<JsonValue, PlanFailure> {
    match value {
        None | Some(JsonValue::Null) => Ok(JsonValue::Null),
        Some(JsonValue::Number(value)) => Ok(JsonValue::Number(value.clone())),
        _ => Err(PlanFailure::contract(
            "Telegram API execution received an invalid timeout.",
        )),
    }
}

fn validate_multipart_fields(fields: &Map<String, JsonValue>) -> Result<(), PlanFailure> {
    if fields.len() > MAX_MULTIPART_FIELDS {
        return Err(PlanFailure::contract(
            "Telegram multipart execution received too many fields.",
        ));
    }
    let mut encoded_bytes = 0usize;
    for (key, value) in fields {
        if key.is_empty()
            || key.len() > 128
            || key
                .chars()
                .any(|ch| ch.is_control() || matches!(ch, '"' | '\\'))
        {
            return Err(PlanFailure::contract(
                "Telegram multipart execution received an invalid field name.",
            ));
        }
        encoded_bytes = encoded_bytes
            .saturating_add(key.len())
            .saturating_add(value.to_string().len());
        if encoded_bytes > MAX_MULTIPART_FIELD_BYTES {
            return Err(PlanFailure::contract(
                "Telegram multipart execution fields exceeded their size limit.",
            ));
        }
    }
    Ok(())
}

fn valid_file_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_FILE_NAME_BYTES
        && !value
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '"' | '/' | '\\'))
}

fn valid_mime_type(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_MIME_TYPE_BYTES || value.matches('/').count() != 1 {
        return false;
    }
    value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'!' | b'#' | b'$' | b'&' | b'^' | b'_' | b'.' | b'+' | b'-' | b'/'
            )
    })
}

fn execute_bytes_transport<E, S>(
    executor: &E,
    sleeper: &S,
    plan: BytesPlan,
) -> Result<TelegramApiTransportExecution, String>
where
    E: TelegramApiTransportExecutor + ?Sized,
    S: TelegramApiRetrySleeper + ?Sized,
{
    let mut progress = plan.progress;
    loop {
        progress.attempts += 1;
        match executor.execute_bytes_request(&plan.http_request) {
            Err(_) => {
                return Ok(TelegramApiTransportExecution::metadata_only(
                    failure_payload(
                        &progress,
                        "executor_failed",
                        "executor",
                        "Telegram bytes HTTP executor failed.",
                        false,
                    ),
                ))
            }
            Ok(AgentTransportHttpBytesExecution::Success {
                method,
                status_code,
                payload,
                ..
            }) => {
                progress.status_code = Some(status_code);
                if !method.eq_ignore_ascii_case("GET") || !(200..300).contains(&status_code) {
                    return Ok(TelegramApiTransportExecution::metadata_only(
                        failure_payload(
                            &progress,
                            "http_contract_failed",
                            "contract",
                            "Telegram bytes HTTP executor returned an invalid success result.",
                            false,
                        ),
                    ));
                }
                let metadata = success_payload(&progress, true, payload.len(), false);
                return Ok(TelegramApiTransportExecution::downloaded(metadata, payload));
            }
            Ok(AgentTransportHttpBytesExecution::Error(raw)) => {
                match failed_http_disposition(&mut progress, sleeper, &raw) {
                    HttpDisposition::Retry => continue,
                    HttpDisposition::Finished(outcome) => {
                        return Ok(TelegramApiTransportExecution::metadata_only(outcome))
                    }
                }
            }
        }
    }
}

fn execute_multipart_transport<E, S>(
    executor: &E,
    sleeper: &S,
    plan: MultipartPlan,
) -> Result<TelegramApiTransportExecution, String>
where
    E: TelegramApiTransportExecutor + ?Sized,
    S: TelegramApiRetrySleeper + ?Sized,
{
    let mut progress = plan.progress;
    let file_bytes = match executor.read_attachment_bytes(Path::new(&plan.local_path)) {
        Ok(payload) => payload,
        Err(_) => {
            return Ok(TelegramApiTransportExecution::metadata_only(
                failure_payload(
                    &progress,
                    "file_read_failed",
                    "file",
                    "Telegram attachment file could not be read.",
                    false,
                ),
            ))
        }
    };
    if file_bytes.len() > MAX_ATTACHMENT_BYTES {
        return Ok(TelegramApiTransportExecution::metadata_only(
            failure_payload(
                &progress,
                "attachment_too_large",
                "capacity",
                "Telegram attachment exceeded the Rust multipart size limit.",
                false,
            ),
        ));
    }
    let byte_count = file_bytes.len();
    let http_request = json!({
        "url": plan.url,
        "boundary": multipart_boundary(),
        "fields": JsonValue::Object(plan.fields),
        "file_field": plan.file_field,
        "file_name": plan.file_name,
        "mime_type": plan.mime_type,
        "timeout_seconds": plan.timeout,
    });

    loop {
        progress.attempts += 1;
        let raw = match executor.execute_multipart_request(&http_request, &file_bytes) {
            Ok(raw) => raw,
            Err(_) => {
                return Ok(TelegramApiTransportExecution::metadata_only(
                    failure_payload(
                        &progress,
                        "executor_failed",
                        "executor",
                        "Telegram multipart HTTP executor failed.",
                        false,
                    ),
                ))
            }
        };
        let Some(raw_object) = raw.as_object() else {
            return Ok(TelegramApiTransportExecution::metadata_only(
                failure_payload(
                    &progress,
                    "http_contract_failed",
                    "contract",
                    "Telegram multipart HTTP executor returned an invalid result.",
                    false,
                ),
            ));
        };
        let Some(http_ok) = raw_object.get("ok").and_then(JsonValue::as_bool) else {
            return Ok(TelegramApiTransportExecution::metadata_only(
                failure_payload(
                    &progress,
                    "http_contract_failed",
                    "contract",
                    "Telegram multipart HTTP executor returned an invalid status.",
                    false,
                ),
            ));
        };
        progress.status_code = optional_i64(raw_object.get("status_code"));
        if !http_ok {
            match failed_http_disposition(&mut progress, sleeper, &raw) {
                HttpDisposition::Retry => continue,
                HttpDisposition::Finished(outcome) => {
                    return Ok(TelegramApiTransportExecution::metadata_only(outcome))
                }
            }
        }
        if !progress
            .status_code
            .is_some_and(|status_code| (200..300).contains(&status_code))
            || clean_text(raw_object.get("response_kind")).as_deref() != Some("json")
        {
            return Ok(TelegramApiTransportExecution::metadata_only(
                failure_payload(
                    &progress,
                    "http_contract_failed",
                    "response",
                    "Telegram multipart HTTP executor returned an invalid success response.",
                    false,
                ),
            ));
        }
        let Some(telegram_payload) = raw_object.get("payload").filter(|value| value.is_object())
        else {
            return Ok(TelegramApiTransportExecution::metadata_only(
                failure_payload(
                    &progress,
                    "http_contract_failed",
                    "response",
                    "Telegram multipart HTTP response payload was invalid.",
                    false,
                ),
            ));
        };
        let outcome = normalize_attachment_result(
            &plan.execution_request,
            telegram_payload,
            &progress,
            byte_count,
        );
        return Ok(TelegramApiTransportExecution::metadata_only(outcome));
    }
}

enum HttpDisposition {
    Retry,
    Finished(JsonValue),
}

fn failed_http_disposition<S>(
    progress: &mut ExecutionProgress,
    sleeper: &S,
    raw: &JsonValue,
) -> HttpDisposition
where
    S: TelegramApiRetrySleeper + ?Sized,
{
    let Some(object) = raw.as_object() else {
        return HttpDisposition::Finished(failure_payload(
            progress,
            "http_contract_failed",
            "contract",
            "Telegram HTTP executor returned an invalid failure result.",
            false,
        ));
    };
    if object.get("ok").and_then(JsonValue::as_bool) != Some(false) {
        return HttpDisposition::Finished(failure_payload(
            progress,
            "http_contract_failed",
            "contract",
            "Telegram HTTP executor returned an invalid failure status.",
            false,
        ));
    }
    progress.status_code = optional_i64(object.get("status_code"));
    let error_kind = public_http_error_kind(object.get("error_kind"));
    let retryable = matches!(error_kind, "timeout" | "transport");
    if retryable && progress.attempts < progress.max_attempts {
        let delay = agent_transport_retry_delay_seconds(
            RETRY_BASE_DELAY_SECONDS,
            (progress.attempts - 1) as i64,
        );
        if sleeper.sleep_seconds(delay).is_err() {
            return HttpDisposition::Finished(failure_payload(
                progress,
                "retry_sleep_failed",
                "sleep",
                "Telegram API retry sleep failed.",
                false,
            ));
        }
        progress.retry_delays_seconds.push(delay);
        return HttpDisposition::Retry;
    }
    let exhausted = retryable && progress.attempts >= progress.max_attempts;
    HttpDisposition::Finished(failure_payload(
        progress,
        if exhausted {
            "retry_exhausted"
        } else {
            "http_failed"
        },
        error_kind,
        if exhausted {
            "Telegram API HTTP retries were exhausted."
        } else {
            "Telegram API HTTP request failed."
        },
        exhausted,
    ))
}

fn normalize_attachment_result(
    execution_request: &JsonValue,
    telegram_payload: &JsonValue,
    progress: &ExecutionProgress,
    byte_count: usize,
) -> JsonValue {
    let planned = match agent_telegram_api_method_execution_plan_json(&json!({
        "stage": "result",
        "execution_request": execution_request,
        "payload": telegram_payload,
    })) {
        Ok(value) => value,
        Err(_) => {
            return failure_payload(
                progress,
                "result_contract_failed",
                "contract",
                "Telegram attachment result planning failed.",
                false,
            )
        }
    };
    let Some(planned_object) = planned.as_object() else {
        return failure_payload(
            progress,
            "result_contract_failed",
            "contract",
            "Telegram attachment result planning returned an invalid contract.",
            false,
        );
    };
    let Some(result) = planned_object.get("result").and_then(JsonValue::as_object) else {
        return failure_payload(
            progress,
            "result_contract_failed",
            "contract",
            "Telegram attachment result planning omitted its result.",
            false,
        );
    };
    let result_ok = result.get("ok").and_then(JsonValue::as_bool);
    if clean_text(planned_object.get("stage")).as_deref() != Some("result")
        || clean_text(planned_object.get("execution_kind")).as_deref()
            != Some(API_METHOD_EXECUTION_KIND)
        || clean_text(result.get("execution_kind")).as_deref() != Some(API_METHOD_EXECUTION_KIND)
        || clean_text(result.get("operation")).as_deref() != Some("send_attachment")
        || clean_text(result.get("telegram_method")).as_deref() != Some(progress.telegram_method)
        || planned_object.get("completed").and_then(JsonValue::as_bool) != result_ok
    {
        return failure_payload(
            progress,
            "result_contract_failed",
            "contract",
            "Telegram attachment result planning returned an invalid result contract.",
            false,
        );
    }
    if result_ok != Some(true) {
        return failure_payload(
            progress,
            "telegram_api_failed",
            "telegram_api",
            "Telegram API rejected the attachment request.",
            false,
        );
    }
    success_payload(progress, false, byte_count, true)
}

#[derive(Default)]
struct ExecutionProgress {
    operation: &'static str,
    telegram_method: &'static str,
    transport: &'static str,
    retry_family: &'static str,
    max_attempts: u64,
    attempts: u64,
    retry_delays_seconds: Vec<f64>,
    status_code: Option<i64>,
}

impl ExecutionProgress {
    fn from_request(request: &Map<String, JsonValue>) -> Self {
        let operation = operation_label(
            &clean_text(request.get("operation"))
                .or_else(|| clean_text(request.get("method_kind")))
                .unwrap_or_default(),
        );
        let telegram_method = match operation {
            "get_updates" => "getUpdates",
            "get_file" => "getFile",
            "download_file" => "downloadFile",
            "send_message" => "sendMessage",
            "send_attachment" => attachment_method(
                request
                    .get("telegram_method")
                    .or_else(|| request.get("method_name")),
            )
            .unwrap_or("sendAttachment"),
            _ => "unknown",
        };
        Self {
            operation,
            telegram_method,
            ..Self::default()
        }
    }

    fn planned(
        operation: &'static str,
        telegram_method: &'static str,
        transport: &'static str,
    ) -> Self {
        Self {
            operation,
            telegram_method,
            transport,
            retry_family: "delivery",
            max_attempts: DELIVERY_MAX_ATTEMPTS,
            ..Self::default()
        }
    }
}

fn success_payload(
    progress: &ExecutionProgress,
    downloaded: bool,
    byte_count: usize,
    sent: bool,
) -> JsonValue {
    execution_payload(
        progress,
        "completed",
        true,
        true,
        None,
        None,
        false,
        downloaded,
        Some(byte_count),
        sent,
    )
}

fn failure_payload(
    progress: &ExecutionProgress,
    state: &str,
    error_kind: &str,
    error: &str,
    retry_exhausted: bool,
) -> JsonValue {
    execution_payload(
        progress,
        state,
        false,
        false,
        Some(error_kind),
        Some(error),
        retry_exhausted,
        false,
        None,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn execution_payload(
    progress: &ExecutionProgress,
    state: &str,
    ok: bool,
    completed: bool,
    error_kind: Option<&str>,
    error: Option<&str>,
    retry_exhausted: bool,
    downloaded: bool,
    byte_count: Option<usize>,
    sent: bool,
) -> JsonValue {
    let mut payload = json!({
        "contract": CONTRACT,
        "migration_stage": MIGRATION_STAGE,
        "stage": "execute",
        "telegram_api_state": state,
        "operation": progress.operation,
        "telegram_method": progress.telegram_method,
        "transport": if progress.transport.is_empty() { JsonValue::Null } else { json!(progress.transport) },
        "retry_family": if progress.retry_family.is_empty() { JsonValue::Null } else { json!(progress.retry_family) },
        "max_attempts": progress.max_attempts,
        "attempts": progress.attempts,
        "retry_count": progress.retry_delays_seconds.len(),
        "retry_delays_seconds": progress.retry_delays_seconds,
        "retry_exhausted": retry_exhausted,
        "http_status_code": progress.status_code.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "ok": ok,
        "completed": completed,
        "downloaded": downloaded,
        "byte_count": byte_count.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "sent": sent,
        "error_kind": error_kind.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "error": error.map(JsonValue::from).unwrap_or(JsonValue::Null),
    });
    if let Some(object) = payload.as_object_mut() {
        add_safety_flags(object);
    }
    payload
}

fn add_safety_flags(object: &mut Map<String, JsonValue>) {
    for key in [
        "python_telegram_api_allowed",
        "python_http_execution_allowed",
        "python_retry_allowed",
        "raw_telegram_payload_exposed",
        "token_bearing_url_exposed",
        "downloaded_bytes_exposed",
        "local_path_exposed",
        "multipart_fields_exposed",
        "file_name_exposed",
    ] {
        object.insert(key.to_string(), json!(false));
    }
}

fn attachment_method(value: Option<&JsonValue>) -> Option<&'static str> {
    match value.and_then(JsonValue::as_str).map(str::trim) {
        Some("sendAudio") => Some("sendAudio"),
        Some("sendPhoto") => Some("sendPhoto"),
        Some("sendDocument") => Some("sendDocument"),
        _ => None,
    }
}

fn attachment_file_field(method: &str) -> &'static str {
    match method {
        "sendAudio" => "audio",
        "sendPhoto" => "photo",
        _ => "document",
    }
}

fn multipart_boundary() -> String {
    let sequence = MULTIPART_BOUNDARY_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("aittelegram-{nanos:x}-{sequence:x}")
}

fn public_http_error_kind(value: Option<&JsonValue>) -> &'static str {
    match value.and_then(JsonValue::as_str).map(str::trim) {
        Some("timeout") => "timeout",
        Some("transport") => "transport",
        Some("http") => "http",
        Some("url") => "url",
        Some("invalid_timeout") => "invalid_timeout",
        Some("response") => "response",
        _ => "http",
    }
}

fn operation_label(value: &str) -> &'static str {
    match value {
        "get_updates" | "getUpdates" | "telegram.get_updates" => "get_updates",
        "get_file" | "getFile" | "telegram.get_file" => "get_file",
        "send_message" | "sendMessage" | "telegram.send_message" => "send_message",
        "download_file" | "downloadFile" | "download_file_bytes" | "telegram.download_file" => {
            "download_file"
        }
        "send_attachment" | "sendAudio" | "sendPhoto" | "sendDocument" => "send_attachment",
        _ => "unknown",
    }
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn optional_i64(value: Option<&JsonValue>) -> Option<i64> {
    match value? {
        JsonValue::Number(value) => value.as_i64(),
        JsonValue::String(value) => value.trim().parse::<i64>().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
