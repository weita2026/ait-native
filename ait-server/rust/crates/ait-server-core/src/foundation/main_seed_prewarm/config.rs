use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::path::PathBuf;

use super::helpers::{
    optional_bool, optional_object, optional_text, path_string, positive_u64, positive_usize,
};
use super::paths::{default_copy_excludes, relative_path_array_from_either};
use super::steps::{prewarm_steps_from_request, PrewarmStep};
use super::DEFAULT_LOCK_TIMEOUT_MS;
use crate::foundation::ci_process_stream::validated_ci_process_timeout_seconds;

pub(super) struct PrewarmConfig {
    pub(super) repo_name: String,
    pub(super) main_seed_path: PathBuf,
    pub(super) source_repo_path: Option<PathBuf>,
    pub(super) generation_key: String,
    pub(super) parallelism: usize,
    pub(super) force: bool,
    pub(super) reuse_only: bool,
    pub(super) dry_run: bool,
    pub(super) lock_timeout_ms: u64,
    pub(super) timeout_seconds: u64,
    pub(super) copy_excludes: Vec<PathBuf>,
    pub(super) steps: Vec<PrewarmStep>,
    pub(super) required_paths: Vec<PathBuf>,
}

impl PrewarmConfig {
    pub(super) fn from_request(request: &JsonMap<String, JsonValue>) -> Result<Self, String> {
        let prewarm = optional_object(request, "main_seed_prewarm")
            .or_else(|| optional_object(request, "prewarm"));
        let repo_name = optional_text(request, "repo_name")
            .or_else(|| prewarm.and_then(|value| optional_text(value, "repo_name")))
            .or_else(|| {
                request
                    .get("payload")
                    .and_then(JsonValue::as_object)
                    .and_then(|payload| optional_text(payload, "repo_name"))
            })
            .unwrap_or_else(|| "unknown-repo".to_string());
        let main_seed_path = optional_text(request, "main_seed_path")
            .or_else(|| prewarm.and_then(|value| optional_text(value, "main_seed_path")))
            .map(PathBuf::from)
            .ok_or_else(|| {
                "Field `main_seed_path` is required for main-seed prewarm.".to_string()
            })?;
        let source_repo_path = optional_text(request, "source_repo_path")
            .or_else(|| prewarm.and_then(|value| optional_text(value, "source_repo_path")))
            .map(PathBuf::from);
        let generation_key = optional_text(request, "generation_key")
            .or_else(|| prewarm.and_then(|value| optional_text(value, "generation_key")))
            .or_else(|| {
                request
                    .get("payload")
                    .and_then(JsonValue::as_object)
                    .and_then(|payload| {
                        optional_text(payload, "revision_snapshot_id")
                            .or_else(|| optional_text(payload, "snapshot_id"))
                            .or_else(|| optional_text(payload, "target_line"))
                    })
            })
            .unwrap_or_else(|| "unknown-generation".to_string());
        let parallelism = positive_usize(request, "parallelism")?
            .or(match prewarm {
                Some(value) => positive_usize(value, "parallelism")?,
                None => None,
            })
            .unwrap_or(1)
            .max(1);
        let force = optional_bool(request, "force")?
            .or(match prewarm {
                Some(value) => optional_bool(value, "force")?,
                None => None,
            })
            .unwrap_or(false);
        let reuse_only = optional_bool(request, "reuse_only")?
            .or(match prewarm {
                Some(value) => optional_bool(value, "reuse_only")?,
                None => None,
            })
            .unwrap_or(false);
        let dry_run = optional_bool(request, "dry_run")?
            .or(match prewarm {
                Some(value) => optional_bool(value, "dry_run")?,
                None => None,
            })
            .unwrap_or(false);
        let lock_timeout_ms = positive_u64(request, "lock_timeout_ms")?
            .or(match prewarm {
                Some(value) => positive_u64(value, "lock_timeout_ms")?,
                None => None,
            })
            .unwrap_or(DEFAULT_LOCK_TIMEOUT_MS);
        let configured_timeout_seconds =
            positive_u64(request, "timeout_seconds")?.or(match prewarm {
                Some(value) => positive_u64(value, "timeout_seconds")?,
                None => None,
            });
        let configured_timeout_seconds = configured_timeout_seconds
            .map(|value| {
                i64::try_from(value)
                    .map_err(|_| "Field `timeout_seconds` is too large.".to_string())
            })
            .transpose()?;
        let timeout_seconds =
            validated_ci_process_timeout_seconds(configured_timeout_seconds, "timeout_seconds")?;
        let copy_excludes = relative_path_array_from_either(request, prewarm, "copy_excludes")?
            .unwrap_or_else(default_copy_excludes);
        let required_paths = relative_path_array_from_either(request, prewarm, "required_paths")?
            .unwrap_or_default();
        let steps = prewarm_steps_from_request(request, prewarm, parallelism)?;

        Ok(Self {
            repo_name,
            main_seed_path,
            source_repo_path,
            generation_key,
            parallelism,
            force,
            reuse_only,
            dry_run,
            lock_timeout_ms,
            timeout_seconds,
            copy_excludes,
            steps,
            required_paths,
        })
    }

    pub(super) fn fingerprint(&self) -> String {
        json!({
            "generation_key": self.generation_key,
            "parallelism": self.parallelism,
            "timeout_seconds": self.timeout_seconds,
            "copy_excludes": self.copy_excludes.iter().map(|path| path_string(path)).collect::<Vec<_>>(),
            "steps": self.steps.iter().map(PrewarmStep::fingerprint_json).collect::<Vec<_>>(),
            "required_paths": self.required_paths.iter().map(|path| path_string(path)).collect::<Vec<_>>(),
        })
        .to_string()
    }
}
