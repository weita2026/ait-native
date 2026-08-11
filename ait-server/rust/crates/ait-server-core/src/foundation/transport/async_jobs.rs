use super::*;

#[derive(Clone, Copy)]
enum AsyncPayloadFieldKind {
    Str,
    StrOrNone,
    StrListOrNone,
    JsonObject,
    JsonObjectOrNone,
    Bool,
    PositiveIntOrNone,
    #[allow(dead_code)]
    PositiveInt,
}

#[derive(Clone, Copy)]
enum AsyncPayloadDefault {
    Null,
    Bool(bool),
}

struct AsyncJobSpec {
    job_type: &'static str,
    required: &'static [(&'static str, AsyncPayloadFieldKind)],
    optional: &'static [(&'static str, AsyncPayloadFieldKind, AsyncPayloadDefault)],
    max_attempts: i64,
    retry_delay_seconds: i64,
}

const ASYNC_JOB_SPECS: &[AsyncJobSpec] = &[
    AsyncJobSpec {
        job_type: "agent.turn.submit",
        required: &[
            ("repo_name", AsyncPayloadFieldKind::Str),
            ("idempotency_key", AsyncPayloadFieldKind::Str),
            ("payload", AsyncPayloadFieldKind::JsonObject),
        ],
        optional: &[(
            "transport",
            AsyncPayloadFieldKind::StrOrNone,
            AsyncPayloadDefault::Null,
        )],
        max_attempts: 8,
        retry_delay_seconds: 3,
    },
    AsyncJobSpec {
        job_type: "content.gc",
        required: &[("repo_name", AsyncPayloadFieldKind::Str)],
        optional: &[
            (
                "prune_unreferenced",
                AsyncPayloadFieldKind::Bool,
                AsyncPayloadDefault::Bool(true),
            ),
            (
                "prune_orphan_packs",
                AsyncPayloadFieldKind::Bool,
                AsyncPayloadDefault::Bool(true),
            ),
        ],
        max_attempts: 3,
        retry_delay_seconds: 3,
    },
    AsyncJobSpec {
        job_type: "content.optimize",
        required: &[("repo_name", AsyncPayloadFieldKind::Str)],
        optional: &[(
            "repair",
            AsyncPayloadFieldKind::Bool,
            AsyncPayloadDefault::Bool(true),
        )],
        max_attempts: 3,
        retry_delay_seconds: 3,
    },
    AsyncJobSpec {
        job_type: "content.pack",
        required: &[("repo_name", AsyncPayloadFieldKind::Str)],
        optional: &[
            (
                "repack",
                AsyncPayloadFieldKind::Bool,
                AsyncPayloadDefault::Bool(false),
            ),
            (
                "max_members",
                AsyncPayloadFieldKind::PositiveIntOrNone,
                AsyncPayloadDefault::Null,
            ),
        ],
        max_attempts: 3,
        retry_delay_seconds: 3,
    },
    AsyncJobSpec {
        job_type: "land.process",
        required: &[("submission_id", AsyncPayloadFieldKind::Str)],
        optional: &[
            (
                "repo_name",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "repo_id",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "change_id",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "change_seq",
                AsyncPayloadFieldKind::PositiveIntOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "patchset_id",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "land_seq",
                AsyncPayloadFieldKind::PositiveIntOrNone,
                AsyncPayloadDefault::Null,
            ),
        ],
        max_attempts: 5,
        retry_delay_seconds: 3,
    },
    AsyncJobSpec {
        job_type: "main-seed.refresh",
        required: &[
            ("repo_name", AsyncPayloadFieldKind::Str),
            ("snapshot_id", AsyncPayloadFieldKind::Str),
            ("patchset_id", AsyncPayloadFieldKind::Str),
        ],
        optional: &[
            (
                "repo_id",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "change_id",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "previous_snapshot_id",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "target_line",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "trigger",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
        ],
        max_attempts: 5,
        retry_delay_seconds: 3,
    },
    AsyncJobSpec {
        job_type: "patchset.ci",
        required: &[("patchset_id", AsyncPayloadFieldKind::Str)],
        optional: &[
            (
                "repo_name",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "repo_id",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "change_id",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "change_seq",
                AsyncPayloadFieldKind::PositiveIntOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "patchset_number",
                AsyncPayloadFieldKind::PositiveIntOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "trigger",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "execution_profile",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "suite_id",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "suite_ids",
                AsyncPayloadFieldKind::StrListOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "revision_snapshot_id",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "snapshot_id",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "runtime_payload",
                AsyncPayloadFieldKind::JsonObjectOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "runner_context",
                AsyncPayloadFieldKind::JsonObjectOrNone,
                AsyncPayloadDefault::Null,
            ),
        ],
        max_attempts: 3,
        retry_delay_seconds: 3,
    },
    AsyncJobSpec {
        job_type: "patchset.ci.aggregate",
        required: &[("patchset_id", AsyncPayloadFieldKind::Str)],
        optional: &[
            (
                "repo_name",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "repo_id",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "change_id",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "change_seq",
                AsyncPayloadFieldKind::PositiveIntOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "patchset_number",
                AsyncPayloadFieldKind::PositiveIntOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "suite_ids",
                AsyncPayloadFieldKind::StrListOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "stage",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "revision_snapshot_id",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "snapshot_id",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
        ],
        max_attempts: 3,
        retry_delay_seconds: 3,
    },
    AsyncJobSpec {
        job_type: "policy.evaluate",
        required: &[("patchset_id", AsyncPayloadFieldKind::Str)],
        optional: &[
            (
                "repo_name",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "repo_id",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "change_id",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "change_seq",
                AsyncPayloadFieldKind::PositiveIntOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "patchset_number",
                AsyncPayloadFieldKind::PositiveIntOrNone,
                AsyncPayloadDefault::Null,
            ),
        ],
        max_attempts: 5,
        retry_delay_seconds: 3,
    },
    AsyncJobSpec {
        job_type: "reconcile.repo",
        required: &[("repo_name", AsyncPayloadFieldKind::Str)],
        optional: &[(
            "repair",
            AsyncPayloadFieldKind::Bool,
            AsyncPayloadDefault::Bool(false),
        )],
        max_attempts: 3,
        retry_delay_seconds: 3,
    },
    AsyncJobSpec {
        job_type: "repo.ci",
        required: &[("repo_name", AsyncPayloadFieldKind::Str)],
        optional: &[
            (
                "repo_id",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "suite_ids",
                AsyncPayloadFieldKind::StrListOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "plane",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "target_line",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "trigger",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "selector",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "snapshot_id",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "task_ids",
                AsyncPayloadFieldKind::StrListOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "curated_corpus",
                AsyncPayloadFieldKind::StrOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "count",
                AsyncPayloadFieldKind::PositiveIntOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "window_days",
                AsyncPayloadFieldKind::PositiveIntOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "dependency_evidence",
                AsyncPayloadFieldKind::StrListOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "compliance_evidence",
                AsyncPayloadFieldKind::StrListOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "runtime_payload",
                AsyncPayloadFieldKind::JsonObjectOrNone,
                AsyncPayloadDefault::Null,
            ),
            (
                "runner_context",
                AsyncPayloadFieldKind::JsonObjectOrNone,
                AsyncPayloadDefault::Null,
            ),
        ],
        max_attempts: 3,
        retry_delay_seconds: 3,
    },
];

fn sort_async_job_types(mut types: Vec<String>) -> Vec<String> {
    types.sort_unstable();
    types
}

pub fn supported_async_job_types() -> Vec<String> {
    AsyncJobJson::stateless().supported_async_job_types()
}

pub(crate) fn supported_async_job_types_impl() -> Vec<String> {
    let types = ASYNC_JOB_SPECS
        .iter()
        .map(|spec| spec.job_type.to_string())
        .collect();
    sort_async_job_types(types)
}

pub fn async_job_contract() -> Vec<JsonMap<String, JsonValue>> {
    AsyncJobJson::stateless().async_job_contract()
}

pub(crate) fn async_job_contract_impl() -> Vec<JsonMap<String, JsonValue>> {
    supported_async_job_types_impl()
        .into_iter()
        .map(|job_type| {
            let spec = async_job_spec(&job_type).expect("known async job type");
            let mut required = JsonMap::new();
            for (field, kind) in spec.required.iter() {
                required.insert((*field).to_string(), kind_name(kind).into());
            }
            let mut optional = JsonMap::new();
            for (field, kind, default) in spec.optional.iter() {
                let mut entry = JsonMap::new();
                entry.insert("type".to_string(), kind_name(kind).into());
                entry.insert("default".to_string(), default_value(default));
                optional.insert((*field).to_string(), JsonValue::Object(entry));
            }
            JsonMap::from_iter([
                ("job_type".to_string(), JsonValue::String(job_type)),
                ("required".to_string(), JsonValue::Object(required)),
                ("optional".to_string(), JsonValue::Object(optional)),
                (
                    "max_attempts".to_string(),
                    JsonValue::Number(spec.max_attempts.into()),
                ),
                (
                    "retry_delay_seconds".to_string(),
                    JsonValue::Number(spec.retry_delay_seconds.into()),
                ),
            ])
        })
        .collect()
}

pub fn normalize_async_job_payload<'a, P>(
    job_type: &str,
    payload: P,
) -> Result<JsonMap<String, JsonValue>, String>
where
    P: AsyncJobPayloadInput<'a>,
{
    AsyncJobJson::stateless().normalize_async_job_payload(job_type, payload)
}

pub(crate) fn normalize_async_job_payload_impl<'a, P>(
    job_type: &str,
    payload: P,
) -> Result<JsonMap<String, JsonValue>, String>
where
    P: AsyncJobPayloadInput<'a>,
{
    let spec = async_job_spec(job_type)?;
    let empty_payload = JsonMap::new();
    let payload = payload.into_payload().unwrap_or(&empty_payload);

    let mut allowed_fields = Vec::new();
    for (field, _) in spec.required {
        allowed_fields.push(*field);
    }
    for (field, _, _) in spec.optional {
        allowed_fields.push(*field);
    }

    let mut extra_fields: Vec<String> = payload
        .keys()
        .filter(|key| !allowed_fields.contains(&key.as_str()))
        .cloned()
        .collect();
    if !extra_fields.is_empty() {
        extra_fields.sort_unstable();
        return Err(format!(
            "{job_type} payload has unsupported field(s): {}",
            extra_fields.join(", ")
        ));
    }

    let mut normalized = JsonMap::new();
    for (field, kind) in spec.required {
        if !payload.contains_key(*field) {
            return Err(format!("{job_type} requires payload field `{field}`."));
        }
        let value = payload
            .get(*field)
            .ok_or_else(|| format!("{job_type} requires payload field `{field}`."))?;
        normalized.insert(
            (*field).to_string(),
            coerce_payload_value(job_type, field, *kind, value)?,
        );
    }
    for (field, kind, default) in spec.optional {
        if let Some(value) = payload.get(*field) {
            normalized.insert(
                (*field).to_string(),
                coerce_payload_value(job_type, field, *kind, value)?,
            );
            continue;
        }
        let fallback = default_to_json_value(*default);
        normalized.insert(
            (*field).to_string(),
            coerce_payload_value(job_type, field, *kind, &fallback)?,
        );
    }
    Ok(normalized)
}

pub fn retry_delay_seconds_for_job(job_type: &str) -> i64 {
    AsyncJobJson::stateless().retry_delay_seconds_for_job(job_type)
}

pub(crate) fn retry_delay_seconds_for_job_impl(job_type: &str) -> i64 {
    async_job_spec(job_type).map_or(3, |spec| spec.retry_delay_seconds)
}

pub fn max_attempts_for_job(job_type: &str) -> i64 {
    AsyncJobJson::stateless().max_attempts_for_job(job_type)
}

pub(crate) fn max_attempts_for_job_impl(job_type: &str) -> i64 {
    async_job_spec(job_type).map_or(5, |spec| spec.max_attempts)
}

fn async_job_spec(job_type: &str) -> Result<&'static AsyncJobSpec, String> {
    ASYNC_JOB_SPECS
        .iter()
        .find(|spec| spec.job_type == job_type)
        .ok_or_else(|| {
            let known = supported_async_job_types_impl().join(", ");
            format!("Unsupported async job type: {job_type}. Expected one of: {known}")
        })
}

fn kind_name(kind: &AsyncPayloadFieldKind) -> &'static str {
    match kind {
        AsyncPayloadFieldKind::Str => "str",
        AsyncPayloadFieldKind::StrOrNone => "str_or_none",
        AsyncPayloadFieldKind::StrListOrNone => "str_list_or_none",
        AsyncPayloadFieldKind::JsonObject => "json_object",
        AsyncPayloadFieldKind::JsonObjectOrNone => "json_object_or_none",
        AsyncPayloadFieldKind::Bool => "bool",
        AsyncPayloadFieldKind::PositiveIntOrNone => "positive_int_or_none",
        AsyncPayloadFieldKind::PositiveInt => "positive_int",
    }
}

fn default_to_json_value(default: AsyncPayloadDefault) -> JsonValue {
    match default {
        AsyncPayloadDefault::Null => JsonValue::Null,
        AsyncPayloadDefault::Bool(value) => JsonValue::Bool(value),
    }
}

fn default_value(default: &AsyncPayloadDefault) -> JsonValue {
    default_to_json_value(*default)
}

fn coerce_payload_value(
    job_type: &str,
    field: &str,
    kind: AsyncPayloadFieldKind,
    value: &JsonValue,
) -> Result<JsonValue, String> {
    match kind {
        AsyncPayloadFieldKind::Str => {
            let normalized = value_to_string(value);
            if normalized.is_empty() {
                return Err(format!(
                    "{job_type} requires non-empty payload field `{field}`."
                ));
            }
            Ok(JsonValue::String(normalized))
        }
        AsyncPayloadFieldKind::StrOrNone => {
            if value.is_null() {
                Ok(JsonValue::Null)
            } else {
                let normalized = value_to_string(value);
                Ok(if normalized.is_empty() {
                    JsonValue::Null
                } else {
                    JsonValue::String(normalized)
                })
            }
        }
        AsyncPayloadFieldKind::StrListOrNone => {
            if value.is_null() {
                return Ok(JsonValue::Null);
            }
            match value {
                JsonValue::String(text) => {
                    let trimmed = text.trim();
                    if trimmed.is_empty() {
                        Ok(JsonValue::Array(Vec::new()))
                    } else {
                        Ok(JsonValue::Array(vec![JsonValue::String(
                            trimmed.to_string(),
                        )]))
                    }
                }
                JsonValue::Array(values) => {
                    let normalized = values
                        .iter()
                        .filter_map(|item| {
                            let normalized = value_to_string(item);
                            if normalized.is_empty() {
                                None
                            } else {
                                Some(JsonValue::String(normalized))
                            }
                        })
                        .collect();
                    Ok(JsonValue::Array(normalized))
                }
                _ => Err(format!(
                    "{job_type} payload field `{field}` must be a list of strings or null."
                )),
            }
        }
        AsyncPayloadFieldKind::JsonObject => match value {
            JsonValue::Object(map) => Ok(JsonValue::Object(map.clone())),
            _ => Err(format!(
                "{job_type} payload field `{field}` must be a JSON object."
            )),
        },
        AsyncPayloadFieldKind::JsonObjectOrNone => {
            if value.is_null() {
                Ok(JsonValue::Null)
            } else {
                match value {
                    JsonValue::Object(map) => Ok(JsonValue::Object(map.clone())),
                    _ => Err(format!(
                        "{job_type} payload field `{field}` must be a JSON object or null."
                    )),
                }
            }
        }
        AsyncPayloadFieldKind::Bool => match value {
            JsonValue::Bool(value) => Ok(JsonValue::Bool(*value)),
            JsonValue::Number(number) => {
                if number.as_i64() == Some(0) || number.as_i64() == Some(1) {
                    Ok(JsonValue::Bool(number.as_i64() == Some(1)))
                } else if let Some(u) = number.as_u64() {
                    if u == 0 || u == 1 {
                        Ok(JsonValue::Bool(u == 1))
                    } else {
                        Err(format!(
                            "{job_type} payload field `{field}` must be a boolean."
                        ))
                    }
                } else if number.is_f64() {
                    Err(format!(
                        "{job_type} payload field `{field}` must be a boolean."
                    ))
                } else {
                    Err(format!(
                        "{job_type} payload field `{field}` must be a boolean."
                    ))
                }
            }
            JsonValue::String(text) => {
                let normalized = text.trim().to_lowercase();
                match normalized.as_str() {
                    "1" | "true" | "yes" | "on" => Ok(JsonValue::Bool(true)),
                    "0" | "false" | "no" | "off" => Ok(JsonValue::Bool(false)),
                    _ => Err(format!(
                        "{job_type} payload field `{field}` must be a boolean."
                    )),
                }
            }
            _ => Err(format!(
                "{job_type} payload field `{field}` must be a boolean."
            )),
        },
        AsyncPayloadFieldKind::PositiveIntOrNone => {
            if value.is_null() {
                return Ok(JsonValue::Null);
            }
            let converted = parse_positive_int(job_type, field, value, true)?;
            Ok(JsonValue::Number(converted.into()))
        }
        AsyncPayloadFieldKind::PositiveInt => {
            let converted = parse_positive_int(job_type, field, value, false)?;
            Ok(JsonValue::Number(converted.into()))
        }
    }
}

fn parse_positive_int(
    job_type: &str,
    field: &str,
    value: &JsonValue,
    allow_null: bool,
) -> Result<i64, String> {
    if value.is_boolean() {
        let suffix = if allow_null { " or null." } else { "." };
        return Err(format!(
            "{job_type} payload field `{field}` must be a positive integer{suffix}"
        ));
    }

    let converted = if let Some(as_i64) = value.as_i64() {
        as_i64
    } else if let Some(as_u64) = value.as_u64() {
        i64::try_from(as_u64).map_err(|_| {
            format!("{job_type} payload field `{field}` must be a positive integer.")
        })?
    } else if let Some(as_f64) = value.as_f64() {
        if !as_f64.is_finite() || as_f64 < 0.0 {
            return Err(format!(
                "{job_type} payload field `{field}` must be a positive integer."
            ));
        }
        as_f64.trunc() as i64
    } else if let Some(as_text) = value.as_str() {
        let as_text = as_text.trim().to_string();
        if as_text.is_empty() {
            return Err(format!(
                "{job_type} payload field `{field}` must be a positive integer."
            ));
        }
        as_text.parse::<i64>().map_err(|_| {
            format!("{job_type} payload field `{field}` must be a positive integer.")
        })?
    } else {
        let as_text = value.to_string().trim().to_string();
        if as_text.is_empty() {
            return Err(format!(
                "{job_type} payload field `{field}` must be a positive integer."
            ));
        }
        as_text.parse::<i64>().map_err(|_| {
            format!("{job_type} payload field `{field}` must be a positive integer.")
        })?
    };
    if converted <= 0 {
        if allow_null {
            return Err(format!(
                "{job_type} payload field `{field}` must be greater than zero when set."
            ));
        }
        return Err(format!(
            "{job_type} payload field `{field}` must be greater than zero."
        ));
    }
    if converted > 0 {
        return Ok(converted);
    }
    if allow_null {
        Err(format!(
            "{job_type} payload field `{field}` must be greater than zero when set."
        ))
    } else {
        Err(format!(
            "{job_type} payload field `{field}` must be greater than zero."
        ))
    }
}

pub trait AsyncJobPayloadInput<'a> {
    fn into_payload(self) -> Option<&'a JsonMap<String, JsonValue>>;
}

impl<'a> AsyncJobPayloadInput<'a> for &'a JsonMap<String, JsonValue> {
    fn into_payload(self) -> Option<&'a JsonMap<String, JsonValue>> {
        Some(self)
    }
}

impl<'a> AsyncJobPayloadInput<'a> for Option<&'a JsonMap<String, JsonValue>> {
    fn into_payload(self) -> Option<&'a JsonMap<String, JsonValue>> {
        self
    }
}
