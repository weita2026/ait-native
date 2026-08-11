use crate::activation::{canonical_real_directory, read_regular_file, sha256};
use crate::domain::RepositoryDomain;
use crate::types::SourceJobRow;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

const LEGACY_GENERATION_SCHEMA: &str = "ait.server.postgres-remote-binary-db-migration.v1";
const RECOVERY_ALIAS_SCHEMA: &str = "ait.server.legacy_patchset_alias_source.v1";
const LEGACY_ID_INDEX_SCHEMA: &str = "ait.server.legacy_patchset_id_index_source.v1";
const EXPLICIT_WORKFLOW_MARKER: &str = "offline-explicit-workflow-schema-restoration";
const LEGACY_ID_INDEX_MARKER: &str = "schema1-final-legacy-patchset-id-index";
const LEGACY_PATCHSET_RECORD_SIZE: usize = 24;
const EXPLICIT_PATCHSET_RECORD_SIZE: usize = 43;
const EXPLICIT_POLICY_RECORD_SIZE: usize = 24;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LegacyPatchsetIdentity {
    pub physical_index: u32,
    pub canonical_patchset_id: String,
    pub base_snapshot_id: String,
    pub revision_snapshot_id: String,
    pub patchset_number: u32,
}

#[derive(Clone, Debug)]
struct LegacyRepositoryPatchsets {
    repo_name: String,
    patchsets: BTreeMap<String, LegacyPatchsetIdentity>,
    aliases: BTreeMap<String, String>,
    v0_identity_prefix_patchset_count: u32,
    target_patchset_indexes: BTreeMap<u32, u32>,
}

#[derive(Clone, Debug)]
pub(crate) struct LegacyPatchsetCatalog {
    manifest_sha256: String,
    id_index_manifest_sha256: Option<String>,
    production_cutover: Option<LegacyProductionCutover>,
    repositories: BTreeMap<String, LegacyRepositoryPatchsets>,
}

#[derive(Clone, Debug)]
struct LegacyProductionCutover {
    last_pre_activation_worker_job_id: i64,
    last_pre_activation_worker_job_created_at: DateTime<Utc>,
    v0_restart_started_at: DateTime<Utc>,
    v0_ready_at: DateTime<Utc>,
    first_post_activation_worker_job_id: i64,
    first_post_activation_worker_job_created_at: DateTime<Utc>,
}

impl LegacyPatchsetCatalog {
    pub(crate) fn load(path: &Path, source_root: Option<&Path>) -> Result<Self, String> {
        let path = std::fs::canonicalize(path).map_err(|error| {
            format!(
                "failed to canonicalize legacy alias manifest {}: {error}",
                path.display()
            )
        })?;
        let manifest_bytes = read_regular_file(&path)?;
        let manifest: LegacyGenerationManifest =
            serde_json::from_slice(&manifest_bytes).map_err(|error| {
                format!(
                    "failed to parse legacy alias manifest {}: {error}",
                    path.display()
                )
            })?;
        let published_generation = manifest.schema == LEGACY_GENERATION_SCHEMA
            && manifest.status == "complete"
            && manifest
                .source_backend
                .split('+')
                .any(|value| value == EXPLICIT_WORKFLOW_MARKER);
        let frozen_recovery_source = manifest.schema == RECOVERY_ALIAS_SCHEMA
            && manifest.status == "frozen_for_recovery"
            && manifest.source_backend == EXPLICIT_WORKFLOW_MARKER;
        if (!published_generation && !frozen_recovery_source) || manifest.repositories.is_empty() {
            return Err(
                "legacy alias manifest is not an admitted explicit-workflow source".to_string(),
            );
        }
        if published_generation && manifest.production_cutover.is_some() {
            return Err(
                "published explicit-workflow source must not carry a recovery production cutover"
                    .to_string(),
            );
        }
        let production_cutover = match manifest.production_cutover {
            Some(document) if frozen_recovery_source => {
                Some(LegacyProductionCutover::try_from(document)?)
            }
            None if frozen_recovery_source => {
                return Err(
                    "frozen legacy alias source lacks the required production_cutover evidence"
                        .to_string(),
                )
            }
            None => None,
            Some(_) => {
                return Err(
                    "legacy production cutover is only valid for a frozen source".to_string(),
                )
            }
        };
        if published_generation && manifest.v0_identity_prefix_source.is_some() {
            return Err(
                "published explicit-workflow source must not carry a v0 identity prefix source"
                    .to_string(),
            );
        }
        let mut v0_identity_prefixes = match manifest.v0_identity_prefix_source {
            Some(document) if frozen_recovery_source => parse_v0_identity_prefix_source(document)?,
            None if frozen_recovery_source => return Err(
                "frozen legacy alias source lacks the required v0_identity_prefix_source evidence"
                    .to_string(),
            ),
            None => BTreeMap::new(),
            Some(_) => {
                return Err(
                    "v0 identity prefix source is only valid for a frozen source".to_string(),
                )
            }
        };
        let default_root = path
            .parent()
            .ok_or_else(|| "legacy alias manifest has no parent".to_string())?;
        let generation_root = canonical_real_directory(source_root.unwrap_or(default_root))?;
        let mut repositories = BTreeMap::new();
        let mut authority_roots = BTreeSet::new();
        for repository in manifest.repositories {
            validate_non_empty_exact(&repository.repo_id, "legacy Repository ID")?;
            validate_non_empty_exact(&repository.repo_name, "legacy Repository name")?;
            validate_relative_path(&repository.authority_relative_path)?;
            let authority_root = canonical_real_directory(
                &generation_root.join(&repository.authority_relative_path),
            )?;
            if !authority_root.starts_with(&generation_root) {
                return Err(format!(
                    "legacy alias authority escaped generation root: {}",
                    authority_root.display()
                ));
            }
            if !authority_roots.insert(authority_root.clone()) {
                return Err(format!(
                    "duplicate legacy alias authority root {}",
                    authority_root.display()
                ));
            }
            let fixed_evidence =
                optional_exact_file_evidence(&repository.files, "remote/patchset.bin")?;
            let payload_evidence =
                optional_exact_file_evidence(&repository.files, "remote/patchset_payload.bin")?;
            let patchsets = match (fixed_evidence, payload_evidence) {
                (None, None) => BTreeMap::new(),
                (Some(fixed_evidence), Some(payload_evidence)) => {
                    let fixed =
                        verified_file(&authority_root.join("patchset.bin"), fixed_evidence)?;
                    let payload = verified_file(
                        &authority_root.join("patchset_payload.bin"),
                        payload_evidence,
                    )?;
                    parse_explicit_patchsets(&fixed, &payload).map_err(|error| {
                        format!(
                            "legacy alias Repository {} ({}) is invalid: {error}",
                            repository.repo_id, repository.repo_name
                        )
                    })?
                }
                _ => {
                    return Err(format!(
                        "legacy alias Repository {} has an incomplete Patchset file pair",
                        repository.repo_id
                    ))
                }
            };
            let policy_fixed_evidence =
                optional_exact_file_evidence(&repository.files, "remote/policy.bin")?;
            let policy_payload_evidence =
                optional_exact_file_evidence(&repository.files, "remote/policy_payload.bin")?;
            let aliases = match (policy_fixed_evidence, policy_payload_evidence) {
                (None, None) => BTreeMap::new(),
                (Some(policy_fixed_evidence), Some(policy_payload_evidence)) => {
                    let policy_fixed =
                        verified_file(&authority_root.join("policy.bin"), policy_fixed_evidence)?;
                    let policy_payload = verified_file(
                        &authority_root.join("policy_payload.bin"),
                        policy_payload_evidence,
                    )?;
                    parse_explicit_policy_aliases(&policy_fixed, &policy_payload, &patchsets)
                        .map_err(|error| {
                            format!(
                                "legacy alias Repository {} ({}) Policy authority is invalid: {error}",
                                repository.repo_id, repository.repo_name
                            )
                        })?
                }
                _ => {
                    return Err(format!(
                        "legacy alias Repository {} has an incomplete Policy file pair",
                        repository.repo_id
                    ))
                }
            };
            let v0_identity_prefix_patchset_count = if frozen_recovery_source {
                let evidence = v0_identity_prefixes
                    .remove(&repository.repo_id)
                    .ok_or_else(|| {
                        format!(
                            "v0 identity prefix source has no Repository ID {:?}",
                            repository.repo_id
                        )
                    })?;
                if evidence.repo_name != repository.repo_name {
                    return Err(format!(
                        "v0 identity prefix Repository {:?} name disagrees",
                        repository.repo_id
                    ));
                }
                evidence.patchset_count
            } else {
                0
            };
            if repositories
                .insert(
                    repository.repo_id.clone(),
                    LegacyRepositoryPatchsets {
                        repo_name: repository.repo_name,
                        patchsets,
                        aliases,
                        v0_identity_prefix_patchset_count,
                        target_patchset_indexes: BTreeMap::new(),
                    },
                )
                .is_some()
            {
                return Err(format!(
                    "duplicate legacy alias Repository ID {}",
                    repository.repo_id
                ));
            }
        }
        if !v0_identity_prefixes.is_empty() {
            return Err(format!(
                "v0 identity prefix source has {} unexpected Repository entries",
                v0_identity_prefixes.len()
            ));
        }
        Ok(Self {
            manifest_sha256: sha256(&manifest_bytes),
            id_index_manifest_sha256: None,
            production_cutover,
            repositories,
        })
    }

    pub(crate) fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }

    pub(crate) fn id_index_manifest_sha256(&self) -> Option<&str> {
        self.id_index_manifest_sha256.as_deref()
    }

    pub(crate) fn validate_production_cutover(&self, jobs: &[SourceJobRow]) -> Result<(), String> {
        let Some(cutover) = self.production_cutover.as_ref() else {
            return Ok(());
        };
        let last_pre_activation = jobs
            .iter()
            .find(|job| job.job_id == cutover.last_pre_activation_worker_job_id)
            .ok_or_else(|| {
                format!(
                    "production cutover has no source Job {}",
                    cutover.last_pre_activation_worker_job_id
                )
            })?;
        let first_post_activation = jobs
            .iter()
            .find(|job| job.job_id == cutover.first_post_activation_worker_job_id)
            .ok_or_else(|| {
                format!(
                    "production cutover has no source Job {}",
                    cutover.first_post_activation_worker_job_id
                )
            })?;
        if last_pre_activation.created_at != cutover.last_pre_activation_worker_job_created_at
            || first_post_activation.created_at
                != cutover.first_post_activation_worker_job_created_at
        {
            return Err("production cutover Job timestamps disagree with PostgreSQL".to_string());
        }
        Ok(())
    }

    pub(crate) fn is_post_production_cutover(&self, job_id: i64) -> Option<bool> {
        self.production_cutover
            .as_ref()
            .map(|cutover| job_id >= cutover.first_post_activation_worker_job_id)
    }

    pub(crate) fn has_repository(&self, repo_id: &str) -> bool {
        self.repositories.contains_key(repo_id)
    }

    pub(crate) fn bind_v0_repository(
        &mut self,
        repo_id: &str,
        repo_name: &str,
        domain: &RepositoryDomain,
    ) -> Result<(), String> {
        let repository = self
            .repositories
            .get_mut(repo_id)
            .ok_or_else(|| format!("legacy alias manifest has no Repository ID {repo_id:?}"))?;
        if repository.repo_name != repo_name {
            return Err(format!(
                "legacy alias Repository {repo_id:?} name disagrees while binding v0 authority"
            ));
        }
        if !repository.target_patchset_indexes.is_empty() {
            return Err(format!(
                "legacy alias Repository {repo_id:?} was already bound to v0 authority"
            ));
        }
        let prefix_count = usize::try_from(repository.v0_identity_prefix_patchset_count)
            .map_err(|_| "v0 identity prefix count exceeds usize".to_string())?;
        if prefix_count == 0 || prefix_count > domain.patchsets_by_index.len() {
            return Err(format!(
                "v0 identity prefix Repository {repo_id:?} count is zero or exceeds current authority"
            ));
        }
        let source_by_index = repository
            .patchsets
            .values()
            .map(|identity| (identity.physical_index, identity))
            .collect::<BTreeMap<_, _>>();
        if source_by_index.len() != repository.patchsets.len()
            || source_by_index
                .keys()
                .copied()
                .ne(0..u32::try_from(source_by_index.len())
                    .map_err(|_| "explicit Patchset count exceeds u32".to_string())?)
        {
            return Err(format!(
                "explicit Patchset Repository {repo_id:?} physical indexes are not dense"
            ));
        }
        let source = source_by_index.values().copied().collect::<Vec<_>>();
        let mut source_cursor = 0_usize;
        for target_index in 0..prefix_count {
            let target = domain.patchsets_by_index[target_index];
            let target_base = domain.snapshot_id(target.base_snapshot_index)?;
            let target_revision = domain.snapshot_id(target.revision_snapshot_index)?;
            let mut matched = None;
            while let Some(candidate) = source.get(source_cursor).copied() {
                source_cursor += 1;
                if candidate.base_snapshot_id == target_base
                    && candidate.revision_snapshot_id == target_revision
                    && candidate.patchset_number == u32::from(target.patch_ordinal) + 1
                {
                    matched = Some(candidate);
                    break;
                }
            }
            let matched = matched.ok_or_else(|| {
                format!(
                    "v0 identity prefix Repository {repo_id:?} target Patchset {target_index} has no monotonic explicit source"
                )
            })?;
            repository.target_patchset_indexes.insert(
                matched.physical_index,
                u32::try_from(target_index)
                    .map_err(|_| "v0 Patchset target index exceeds u32".to_string())?,
            );
        }
        if repository.target_patchset_indexes.len() != prefix_count {
            return Err(format!(
                "v0 identity prefix Repository {repo_id:?} did not bind exactly {prefix_count} Patchsets"
            ));
        }
        Ok(())
    }

    pub(crate) fn target_patchset_index(
        &self,
        repo_id: &str,
        repo_name: &str,
        explicit_physical_index: u32,
    ) -> Result<Option<u32>, String> {
        Ok(self
            .repository(repo_id, repo_name)?
            .target_patchset_indexes
            .get(&explicit_physical_index)
            .copied())
    }

    pub(crate) fn validate_v0_bindings_complete(&self) -> Result<(), String> {
        let incomplete = self
            .repositories
            .iter()
            .filter_map(|(repo_id, repository)| {
                (repository.target_patchset_indexes.len()
                    != repository.v0_identity_prefix_patchset_count as usize)
                    .then_some(repo_id.as_str())
            })
            .collect::<Vec<_>>();
        if incomplete.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "legacy alias v0 identity prefix was not bound for Repository IDs: {}",
                incomplete.join(", ")
            ))
        }
    }

    pub(crate) fn exact_patchset(
        &self,
        repo_id: &str,
        repo_name: &str,
        source_patchset_id: &str,
    ) -> Result<Option<LegacyPatchsetIdentity>, String> {
        let repository = self.repository(repo_id, repo_name)?;
        Ok(repository.patchsets.get(source_patchset_id).cloned())
    }

    pub(crate) fn apply_id_index_source(
        &mut self,
        path: &Path,
        source_root: Option<&Path>,
    ) -> Result<(), String> {
        if self.id_index_manifest_sha256.is_some() {
            return Err("legacy Patchset ID index source was already applied".to_string());
        }
        let path = std::fs::canonicalize(path).map_err(|error| {
            format!(
                "failed to canonicalize legacy Patchset ID index manifest {}: {error}",
                path.display()
            )
        })?;
        let manifest_bytes = read_regular_file(&path)?;
        let manifest: LegacyGenerationManifest =
            serde_json::from_slice(&manifest_bytes).map_err(|error| {
                format!(
                    "failed to parse legacy Patchset ID index manifest {}: {error}",
                    path.display()
                )
            })?;
        if manifest.schema != LEGACY_ID_INDEX_SCHEMA
            || manifest.status != "frozen_for_recovery"
            || manifest.source_backend != LEGACY_ID_INDEX_MARKER
            || manifest.repositories.is_empty()
        {
            return Err(
                "legacy Patchset ID index manifest is not an admitted frozen source".to_string(),
            );
        }
        let default_root = path
            .parent()
            .ok_or_else(|| "legacy Patchset ID index manifest has no parent".to_string())?;
        let generation_root = canonical_real_directory(source_root.unwrap_or(default_root))?;
        let mut seen_repositories = BTreeSet::new();
        let mut authority_roots = BTreeSet::new();
        for source in manifest.repositories {
            validate_non_empty_exact(&source.repo_id, "legacy Repository ID")?;
            validate_non_empty_exact(&source.repo_name, "legacy Repository name")?;
            validate_relative_path(&source.authority_relative_path)?;
            if !seen_repositories.insert(source.repo_id.clone()) {
                return Err(format!(
                    "duplicate legacy Patchset ID index Repository {}",
                    source.repo_id
                ));
            }
            let repository = self.repositories.get_mut(&source.repo_id).ok_or_else(|| {
                format!(
                    "legacy Patchset ID index source has unexpected Repository ID {:?}",
                    source.repo_id
                )
            })?;
            if repository.repo_name != source.repo_name {
                return Err(format!(
                    "legacy Patchset ID index Repository {:?} name disagrees",
                    source.repo_id
                ));
            }
            let authority_root =
                canonical_real_directory(&generation_root.join(&source.authority_relative_path))?;
            if !authority_root.starts_with(&generation_root)
                || !authority_roots.insert(authority_root.clone())
            {
                return Err(format!(
                    "legacy Patchset ID index authority root is unsafe or duplicated: {}",
                    authority_root.display()
                ));
            }
            let fixed = verified_file(
                &authority_root.join("patchset.bin"),
                exact_file_evidence(&source.files, "remote/patchset.bin")?,
            )?;
            let payload = verified_file(
                &authority_root.join("patchset_payload.bin"),
                exact_file_evidence(&source.files, "remote/patchset_payload.bin")?,
            )?;
            let index = verified_file(
                &authority_root.join("patchset_id.idx"),
                exact_file_evidence(&source.files, "remote/patchset_id.idx")?,
            )?;
            let mappings =
                parse_legacy_patchset_id_source(&fixed, &payload, &index, &repository.patchsets)
                    .map_err(|error| {
                        format!(
                            "legacy Patchset ID index Repository {} ({}) is invalid: {error}",
                            source.repo_id, source.repo_name
                        )
                    })?;
            for (source_patchset_id, canonical_patchset_id) in mappings {
                match repository
                    .aliases
                    .insert(source_patchset_id.clone(), canonical_patchset_id.clone())
                {
                    Some(previous) if previous != canonical_patchset_id => {
                        return Err(format!(
                            "legacy Patchset alias {source_patchset_id:?} maps to both {previous:?} and {canonical_patchset_id:?}"
                        ));
                    }
                    _ => {}
                }
            }
        }
        if seen_repositories.len() != self.repositories.len()
            || self
                .repositories
                .keys()
                .any(|repo_id| !seen_repositories.contains(repo_id))
        {
            return Err(
                "legacy Patchset ID index source does not cover every alias Repository".to_string(),
            );
        }
        self.id_index_manifest_sha256 = Some(sha256(&manifest_bytes));
        Ok(())
    }

    pub(crate) fn patchset(
        &self,
        repo_id: &str,
        repo_name: &str,
        source_patchset_id: &str,
        canonical_fallback_id: &str,
        patchset_number: u32,
        revision_snapshot_hint: Option<&str>,
    ) -> Result<LegacyPatchsetIdentity, String> {
        let repository = self.repository(repo_id, repo_name)?;
        let validate_hints = |identity: LegacyPatchsetIdentity| {
            if revision_snapshot_hint.is_some_and(|hint| hint != identity.revision_snapshot_id) {
                return Err(format!(
                    "legacy Patchset {source_patchset_id:?} revision Snapshot disagrees with explicit Patchset {:?}",
                    identity.canonical_patchset_id
                ));
            }
            Ok(identity)
        };
        if let Some(canonical_patchset_id) = repository.aliases.get(source_patchset_id) {
            let identity = repository
                .patchsets
                .get(canonical_patchset_id)
                .cloned()
                .ok_or_else(|| {
                    format!(
                        "legacy alias authority has no canonical Patchset {canonical_patchset_id:?}"
                    )
                })?;
            return validate_hints(identity);
        }

        if let Some(revision_snapshot_id) = revision_snapshot_hint {
            let candidates = repository
                .patchsets
                .values()
                .filter(|identity| {
                    identity.patchset_number == patchset_number
                        && identity.revision_snapshot_id == revision_snapshot_id
                })
                .cloned()
                .collect::<Vec<_>>();
            if candidates.len() == 1 {
                return validate_hints(candidates[0].clone());
            }
            return Err(format!(
                "legacy Patchset {source_patchset_id:?} revision Snapshot {revision_snapshot_id:?} and ordinal resolved to {} explicit candidates",
                candidates.len()
            ));
        }

        let identity = repository
            .patchsets
            .get(canonical_fallback_id)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "legacy alias authority has no canonical Patchset {canonical_fallback_id:?}"
                )
            })?;
        validate_hints(identity)
    }

    fn repository(
        &self,
        repo_id: &str,
        repo_name: &str,
    ) -> Result<&LegacyRepositoryPatchsets, String> {
        let repository = self
            .repositories
            .get(repo_id)
            .ok_or_else(|| format!("legacy alias manifest has no Repository ID {repo_id:?}"))?;
        if repository.repo_name != repo_name {
            return Err(format!(
                "legacy alias Repository {repo_id:?} name disagrees: expected {repo_name:?}, got {:?}",
                repository.repo_name
            ));
        }
        Ok(repository)
    }
}

#[derive(Debug, Deserialize)]
struct LegacyGenerationManifest {
    schema: String,
    status: String,
    source_backend: String,
    #[serde(default)]
    production_cutover: Option<LegacyProductionCutoverDocument>,
    #[serde(default)]
    v0_identity_prefix_source: Option<V0IdentityPrefixSourceDocument>,
    repositories: Vec<LegacyManifestRepository>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyProductionCutoverDocument {
    last_pre_activation_worker_job_id: i64,
    last_pre_activation_worker_job_created_at: String,
    v0_restart_started_at: String,
    v0_ready_at: String,
    first_post_activation_worker_job_id: i64,
    first_post_activation_worker_job_created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct V0IdentityPrefixSourceDocument {
    generation_name: String,
    manifest_sha256: String,
    repositories: Vec<V0IdentityPrefixRepositoryDocument>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct V0IdentityPrefixRepositoryDocument {
    repo_id: String,
    repo_name: String,
    patchset_count: u32,
    patchset_byte_size: u64,
    patchset_sha256: String,
}

#[derive(Debug)]
struct V0IdentityPrefixEvidence {
    repo_name: String,
    patchset_count: u32,
}

fn parse_v0_identity_prefix_source(
    document: V0IdentityPrefixSourceDocument,
) -> Result<BTreeMap<String, V0IdentityPrefixEvidence>, String> {
    validate_non_empty_exact(
        &document.generation_name,
        "v0 identity prefix generation name",
    )?;
    validate_sha256_text(
        &document.manifest_sha256,
        "v0 identity prefix manifest SHA-256",
    )?;
    if document.repositories.is_empty() {
        return Err("v0 identity prefix source has no Repository evidence".to_string());
    }
    let mut repositories = BTreeMap::new();
    for source in document.repositories {
        validate_non_empty_exact(&source.repo_id, "v0 identity prefix Repository ID")?;
        validate_non_empty_exact(&source.repo_name, "v0 identity prefix Repository name")?;
        validate_sha256_text(
            &source.patchset_sha256,
            "v0 identity prefix Patchset SHA-256",
        )?;
        let expected_size = u64::from(source.patchset_count)
            .checked_mul(57)
            .and_then(|size| size.checked_add(4))
            .ok_or_else(|| "v0 identity prefix Patchset byte size overflows".to_string())?;
        if source.patchset_count == 0 || source.patchset_byte_size != expected_size {
            return Err(format!(
                "v0 identity prefix Repository {:?} count and byte size disagree",
                source.repo_id
            ));
        }
        let repo_id = source.repo_id.clone();
        if repositories
            .insert(
                source.repo_id,
                V0IdentityPrefixEvidence {
                    repo_name: source.repo_name,
                    patchset_count: source.patchset_count,
                },
            )
            .is_some()
        {
            return Err(format!(
                "duplicate v0 identity prefix Repository ID {repo_id:?}"
            ));
        }
    }
    Ok(repositories)
}

impl TryFrom<LegacyProductionCutoverDocument> for LegacyProductionCutover {
    type Error = String;

    fn try_from(document: LegacyProductionCutoverDocument) -> Result<Self, Self::Error> {
        if document.last_pre_activation_worker_job_id <= 0
            || document.first_post_activation_worker_job_id
                != document
                    .last_pre_activation_worker_job_id
                    .checked_add(1)
                    .ok_or_else(|| "production cutover Job ID overflows".to_string())?
        {
            return Err(
                "production cutover must name two positive consecutive Worker Job IDs".to_string(),
            );
        }
        let parse = |value: &str, label: &str| {
            DateTime::parse_from_rfc3339(value)
                .map(|value| value.with_timezone(&Utc))
                .map_err(|error| format!("production cutover {label} is invalid: {error}"))
        };
        let cutover = Self {
            last_pre_activation_worker_job_id: document.last_pre_activation_worker_job_id,
            last_pre_activation_worker_job_created_at: parse(
                &document.last_pre_activation_worker_job_created_at,
                "last pre-activation Job timestamp",
            )?,
            v0_restart_started_at: parse(&document.v0_restart_started_at, "v0 restart timestamp")?,
            v0_ready_at: parse(&document.v0_ready_at, "v0 ready timestamp")?,
            first_post_activation_worker_job_id: document.first_post_activation_worker_job_id,
            first_post_activation_worker_job_created_at: parse(
                &document.first_post_activation_worker_job_created_at,
                "first post-activation Job timestamp",
            )?,
        };
        if !(cutover.last_pre_activation_worker_job_created_at < cutover.v0_restart_started_at
            && cutover.v0_restart_started_at < cutover.v0_ready_at
            && cutover.v0_ready_at < cutover.first_post_activation_worker_job_created_at)
        {
            return Err(
                "production cutover timestamps do not prove an idle PostgreSQL Job interval"
                    .to_string(),
            );
        }
        Ok(cutover)
    }
}

#[derive(Debug, Deserialize)]
struct LegacyManifestRepository {
    repo_name: String,
    repo_id: String,
    authority_relative_path: String,
    files: Vec<LegacyFileEvidence>,
}

#[derive(Debug, Deserialize)]
struct LegacyFileEvidence {
    relative_path: String,
    byte_size: u64,
    sha256: String,
}

fn exact_file_evidence<'a>(
    files: &'a [LegacyFileEvidence],
    relative_path: &str,
) -> Result<&'a LegacyFileEvidence, String> {
    let matches = files
        .iter()
        .filter(|file| file.relative_path == relative_path)
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(format!(
            "legacy alias manifest requires exactly one {relative_path:?} file entry"
        ));
    }
    let evidence = matches[0];
    validate_sha256_text(
        &evidence.sha256,
        &format!("legacy alias file {relative_path:?} SHA-256"),
    )?;
    Ok(evidence)
}

fn validate_sha256_text(value: &str, label: &str) -> Result<(), String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        Ok(())
    } else {
        Err(format!("{label} is invalid"))
    }
}

fn optional_exact_file_evidence<'a>(
    files: &'a [LegacyFileEvidence],
    relative_path: &str,
) -> Result<Option<&'a LegacyFileEvidence>, String> {
    if files.iter().all(|file| file.relative_path != relative_path) {
        Ok(None)
    } else {
        exact_file_evidence(files, relative_path).map(Some)
    }
}

fn verified_file(path: &Path, evidence: &LegacyFileEvidence) -> Result<Vec<u8>, String> {
    let bytes = read_regular_file(path)?;
    if bytes.len() as u64 != evidence.byte_size || sha256(&bytes) != evidence.sha256 {
        return Err(format!(
            "legacy alias authority file disagrees with manifest: {}",
            path.display()
        ));
    }
    Ok(bytes)
}

fn parse_legacy_patchset_id_source(
    fixed: &[u8],
    payload: &[u8],
    index: &[u8],
    explicit_patchsets: &BTreeMap<String, LegacyPatchsetIdentity>,
) -> Result<BTreeMap<String, String>, String> {
    validate_layout_file(fixed, LEGACY_PATCHSET_RECORD_SIZE, "legacy patchset.bin")?;
    validate_layout_file(payload, 1, "legacy patchset_payload.bin")?;
    let index_entries = parse_legacy_exact_index(index)?;
    let record_count = (fixed.len() - 4) / LEGACY_PATCHSET_RECORD_SIZE;
    if index_entries.len() != record_count {
        return Err(format!(
            "legacy Patchset ID index has {} entries for {record_count} records",
            index_entries.len()
        ));
    }
    let explicit_by_index = explicit_patchsets
        .values()
        .map(|identity| (identity.physical_index, identity))
        .collect::<BTreeMap<_, _>>();
    if explicit_by_index.len() != explicit_patchsets.len() {
        return Err("explicit Patchset authority repeats a physical index".to_string());
    }
    let mut seen_indexes = BTreeSet::new();
    let mut aliases = BTreeMap::new();
    for (source_patchset_id, physical_index) in index_entries {
        let index_usize = usize::try_from(physical_index)
            .map_err(|_| "legacy Patchset physical index exceeds usize".to_string())?;
        if index_usize >= record_count || !seen_indexes.insert(physical_index) {
            return Err(format!(
                "legacy Patchset ID {source_patchset_id:?} has an out-of-range or duplicate physical index {physical_index}"
            ));
        }
        let record_start = 4 + index_usize * LEGACY_PATCHSET_RECORD_SIZE;
        let record = &fixed[record_start..record_start + LEGACY_PATCHSET_RECORD_SIZE];
        let payload_offset = u64::from_le_bytes(record[0..8].try_into().unwrap());
        let payload_len = u32::from_le_bytes(record[8..12].try_into().unwrap());
        let entity_index = u32::from_le_bytes(record[12..16].try_into().unwrap());
        let payload_end = payload_offset
            .checked_add(u64::from(payload_len))
            .ok_or_else(|| format!("legacy Patchset {physical_index} payload range overflows"))?;
        let start = usize::try_from(payload_offset)
            .map_err(|_| format!("legacy Patchset {physical_index} payload offset overflows"))?;
        let end = usize::try_from(payload_end)
            .map_err(|_| format!("legacy Patchset {physical_index} payload end overflows"))?;
        let slice = payload.get(start..end).ok_or_else(|| {
            format!("legacy Patchset {physical_index} payload range is out of bounds")
        })?;
        if payload_len == 0
            || payload_offset < 4
            || entity_index != physical_index
            || legacy_payload_patchset_id(slice)? != source_patchset_id
        {
            return Err(format!(
                "legacy Patchset {physical_index} fixed record, payload, and ID index disagree"
            ));
        }
        if looks_like_legacy_patchset_alias(&source_patchset_id) {
            let patchset_number = source_patchset_id
                .rsplit_once('-')
                .and_then(|(_, value)| parse_positive_decimal(value))
                .ok_or_else(|| {
                    format!("legacy Patchset ID has no ordinal: {source_patchset_id:?}")
                })?;
            let explicit = explicit_by_index.get(&physical_index).ok_or_else(|| {
                format!(
                    "legacy Patchset {source_patchset_id:?} physical index {physical_index} is absent from explicit authority"
                )
            })?;
            if explicit.patchset_number != patchset_number {
                return Err(format!(
                    "legacy Patchset {source_patchset_id:?} ordinal disagrees with explicit physical index {physical_index}"
                ));
            }
            aliases.insert(source_patchset_id, explicit.canonical_patchset_id.clone());
        }
    }
    if seen_indexes.len() != record_count {
        return Err("legacy Patchset ID index is not a dense record projection".to_string());
    }
    Ok(aliases)
}

fn parse_legacy_exact_index(bytes: &[u8]) -> Result<BTreeMap<String, u32>, String> {
    if bytes.len() < 4 || bytes[..4] != 1_u32.to_le_bytes() {
        return Err("legacy Patchset ID index lacks layout-1 header".to_string());
    }
    let mut offset = 4_usize;
    let mut entries = BTreeMap::new();
    while offset < bytes.len() {
        let length_end = offset
            .checked_add(4)
            .ok_or_else(|| "legacy Patchset ID index length offset overflow".to_string())?;
        let key_len = u32::from_le_bytes(
            bytes
                .get(offset..length_end)
                .ok_or_else(|| "legacy Patchset ID index truncates key length".to_string())?
                .try_into()
                .unwrap(),
        );
        offset = length_end;
        if key_len == 0 || key_len > 4096 {
            return Err("legacy Patchset ID index has an invalid key length".to_string());
        }
        let key_end = offset
            .checked_add(key_len as usize)
            .ok_or_else(|| "legacy Patchset ID index key range overflow".to_string())?;
        let key = std::str::from_utf8(
            bytes
                .get(offset..key_end)
                .ok_or_else(|| "legacy Patchset ID index truncates key".to_string())?,
        )
        .map_err(|_| "legacy Patchset ID index key is not UTF-8".to_string())?;
        if key.trim() != key {
            return Err("legacy Patchset ID index key requires exact Text".to_string());
        }
        offset = key_end;
        let value_end = offset
            .checked_add(4)
            .ok_or_else(|| "legacy Patchset ID index value offset overflow".to_string())?;
        let value = u32::from_le_bytes(
            bytes
                .get(offset..value_end)
                .ok_or_else(|| "legacy Patchset ID index truncates value".to_string())?
                .try_into()
                .unwrap(),
        );
        offset = value_end;
        if entries.insert(key.to_string(), value).is_some() {
            return Err(format!("duplicate legacy Patchset ID index key {key:?}"));
        }
    }
    Ok(entries)
}

fn legacy_payload_patchset_id(bytes: &[u8]) -> Result<&str, String> {
    if bytes.len() < 10 || &bytes[..4] != b"AITW" || bytes[4] != 1 || bytes[5] != 5 {
        return Err("legacy Patchset payload lacks the exact AITW v1 envelope".to_string());
    }
    let exact_index_count = u32::from_le_bytes(bytes[6..10].try_into().unwrap());
    if exact_index_count == 0 || exact_index_count > 32 {
        return Err("legacy Patchset payload has an invalid exact-index count".to_string());
    }
    let mut offset = 10_usize;
    let mut patchset_id = None;
    for _ in 0..exact_index_count {
        let path = read_legacy_length_prefixed_text(bytes, &mut offset, "index path")?;
        let key = read_legacy_length_prefixed_text(bytes, &mut offset, "index key")?;
        if path == "patchset_id.idx" {
            if patchset_id.replace(key).is_some() {
                return Err(
                    "legacy Patchset payload repeats the patchset_id.idx exact key".to_string(),
                );
            }
        }
    }
    if offset >= bytes.len() {
        return Err("legacy Patchset payload has no typed body".to_string());
    }
    patchset_id
        .ok_or_else(|| "legacy Patchset payload lacks the patchset_id.idx exact key".to_string())
}

fn read_legacy_length_prefixed_text<'a>(
    bytes: &'a [u8],
    offset: &mut usize,
    label: &str,
) -> Result<&'a str, String> {
    let length_end = offset
        .checked_add(4)
        .ok_or_else(|| format!("legacy Patchset payload {label} length offset overflow"))?;
    let length = u32::from_le_bytes(
        bytes
            .get(*offset..length_end)
            .ok_or_else(|| format!("legacy Patchset payload truncates {label} length"))?
            .try_into()
            .unwrap(),
    );
    *offset = length_end;
    if length == 0 || length > 4096 {
        return Err(format!(
            "legacy Patchset payload {label} has an invalid length"
        ));
    }
    let end = offset
        .checked_add(length as usize)
        .ok_or_else(|| format!("legacy Patchset payload {label} range overflow"))?;
    let value = std::str::from_utf8(
        bytes
            .get(*offset..end)
            .ok_or_else(|| format!("legacy Patchset payload truncates {label}"))?,
    )
    .map_err(|_| format!("legacy Patchset payload {label} is not UTF-8"))?;
    *offset = end;
    if value.trim() != value {
        return Err(format!(
            "legacy Patchset payload {label} requires exact Text"
        ));
    }
    Ok(value)
}

fn parse_explicit_patchsets(
    fixed: &[u8],
    payload: &[u8],
) -> Result<BTreeMap<String, LegacyPatchsetIdentity>, String> {
    validate_layout_file(fixed, EXPLICIT_PATCHSET_RECORD_SIZE, "patchset.bin")?;
    validate_layout_file(payload, 1, "patchset_payload.bin")?;
    let mut expected_payload_offset = 4_u64;
    let mut patchsets = BTreeMap::new();
    for (index, record) in fixed[4..]
        .chunks_exact(EXPLICIT_PATCHSET_RECORD_SIZE)
        .enumerate()
    {
        let payload_offset = u64::from_le_bytes(record[0..8].try_into().unwrap());
        let payload_len = u32::from_le_bytes(record[8..12].try_into().unwrap());
        let entity_index = u32::from_le_bytes(record[12..16].try_into().unwrap());
        let patchset_number = u32::from_le_bytes(record[20..24].try_into().unwrap());
        if entity_index != index as u32
            || payload_len == 0
            || payload_offset != expected_payload_offset
            || patchset_number == 0
        {
            return Err(format!(
                "explicit Patchset record {index} has invalid index, sequence, or payload range"
            ));
        }
        let payload_end = payload_offset
            .checked_add(u64::from(payload_len))
            .ok_or_else(|| format!("explicit Patchset record {index} payload range overflow"))?;
        let payload_start = usize::try_from(payload_offset)
            .map_err(|_| format!("explicit Patchset record {index} payload offset overflow"))?;
        let payload_end_usize = usize::try_from(payload_end)
            .map_err(|_| format!("explicit Patchset record {index} payload end overflow"))?;
        let slice = payload
            .get(payload_start..payload_end_usize)
            .ok_or_else(|| {
                format!("explicit Patchset record {index} payload range is out of bounds")
            })?;
        let mut cursor = TextCursor::new(slice);
        let patchset_id = cursor.required_text("patchset_id")?.to_string();
        let change_id = cursor.required_text("change_id")?;
        let base_snapshot_id = cursor.required_text("base_snapshot_id")?.to_string();
        let revision_snapshot_id = cursor.required_text("revision_snapshot_id")?.to_string();
        validate_explicit_patchset_identity(&patchset_id, change_id, patchset_number)?;
        validate_snapshot_id(&base_snapshot_id, "base Snapshot")?;
        validate_snapshot_id(&revision_snapshot_id, "revision Snapshot")?;
        let identity = LegacyPatchsetIdentity {
            physical_index: u32::try_from(index)
                .map_err(|_| "explicit Patchset physical index exceeds u32".to_string())?,
            canonical_patchset_id: patchset_id.clone(),
            base_snapshot_id,
            revision_snapshot_id,
            patchset_number,
        };
        if patchsets.insert(patchset_id.clone(), identity).is_some() {
            return Err(format!(
                "duplicate explicit Patchset identity {patchset_id:?}"
            ));
        }
        expected_payload_offset = payload_end;
    }
    if expected_payload_offset != payload.len() as u64 {
        return Err(
            "explicit Patchset payload authority has unreferenced trailing bytes".to_string(),
        );
    }
    Ok(patchsets)
}

fn parse_explicit_policy_aliases(
    fixed: &[u8],
    payload: &[u8],
    patchsets: &BTreeMap<String, LegacyPatchsetIdentity>,
) -> Result<BTreeMap<String, String>, String> {
    validate_layout_file(fixed, EXPLICIT_POLICY_RECORD_SIZE, "policy.bin")?;
    validate_layout_file(payload, 1, "policy_payload.bin")?;
    let mut expected_payload_offset = 4_u64;
    let mut aliases = BTreeMap::new();
    for (index, record) in fixed[4..]
        .chunks_exact(EXPLICIT_POLICY_RECORD_SIZE)
        .enumerate()
    {
        let payload_offset = u64::from_le_bytes(record[0..8].try_into().unwrap());
        let payload_len = u32::from_le_bytes(record[8..12].try_into().unwrap());
        let entity_index = u32::from_le_bytes(record[12..16].try_into().unwrap());
        if entity_index != index as u32
            || payload_len == 0
            || payload_offset != expected_payload_offset
        {
            return Err(format!(
                "explicit Policy record {index} has invalid index or payload range"
            ));
        }
        let payload_end = payload_offset
            .checked_add(u64::from(payload_len))
            .ok_or_else(|| format!("explicit Policy record {index} payload range overflow"))?;
        let start = usize::try_from(payload_offset)
            .map_err(|_| format!("explicit Policy record {index} payload offset overflow"))?;
        let end = usize::try_from(payload_end)
            .map_err(|_| format!("explicit Policy record {index} payload end overflow"))?;
        let slice = payload.get(start..end).ok_or_else(|| {
            format!("explicit Policy record {index} payload range is out of bounds")
        })?;
        let mut cursor = TextCursor::new(slice);
        cursor.required_text("policy_decision_id")?;
        let canonical_patchset_id = cursor.required_text("patchset_id")?;
        if !patchsets.contains_key(canonical_patchset_id) {
            return Err(format!(
                "explicit Policy record {index} references unknown Patchset {canonical_patchset_id:?}"
            ));
        }
        cursor.required_text("decision")?;
        let check_count = cursor.required_count("checks")?;
        for _ in 0..check_count {
            cursor.required_text("check.name")?;
            cursor.required_text("check.label")?;
            cursor.required_text("check.status")?;
            cursor.required_text("check.message")?;
        }
        let input_fingerprint = cursor.optional_text("input_fingerprint")?;
        cursor.required_text("evaluated_at")?;
        cursor.require_end("Policy")?;
        if let Some(fingerprint) = input_fingerprint {
            if let Some((candidate, _)) = fingerprint.split_once(':') {
                if looks_like_legacy_patchset_alias(candidate) {
                    match aliases.insert(candidate.to_string(), canonical_patchset_id.to_string()) {
                        Some(previous) if previous != canonical_patchset_id => {
                            return Err(format!(
                                "legacy Patchset alias {candidate:?} maps to both {previous:?} and {canonical_patchset_id:?}"
                            ));
                        }
                        _ => {}
                    }
                }
            }
        }
        expected_payload_offset = payload_end;
    }
    if expected_payload_offset != payload.len() as u64 {
        return Err(
            "explicit Policy payload authority has unreferenced trailing bytes".to_string(),
        );
    }
    Ok(aliases)
}

fn looks_like_legacy_patchset_alias(value: &str) -> bool {
    if value.contains("/P-") {
        return false;
    }
    let Some((prefix_and_task, patchset_number)) = value.rsplit_once('-') else {
        return false;
    };
    let Some((prefix, task_number)) = prefix_and_task.rsplit_once('-') else {
        return false;
    };
    !prefix.is_empty()
        && parse_positive_decimal(task_number).is_some()
        && parse_positive_decimal(patchset_number).is_some()
}

fn validate_layout_file(bytes: &[u8], record_size: usize, label: &str) -> Result<(), String> {
    if bytes.len() < 4
        || bytes[..4] != 1_u32.to_le_bytes()
        || (record_size > 1 && (bytes.len() - 4) % record_size != 0)
    {
        Err(format!(
            "legacy explicit {label} is not a layout-1 aligned file"
        ))
    } else {
        Ok(())
    }
}

fn validate_explicit_patchset_identity(
    patchset_id: &str,
    change_id: &str,
    patchset_number: u32,
) -> Result<(), String> {
    let (change_ref, patchset_ordinal) = patchset_id
        .rsplit_once("/P-")
        .ok_or_else(|| format!("explicit Patchset ID is not canonical: {patchset_id:?}"))?;
    let (_, embedded_change_id) = change_ref
        .rsplit_once('/')
        .ok_or_else(|| format!("explicit Patchset ID has no Change: {patchset_id:?}"))?;
    if embedded_change_id != change_id
        || !change_id.starts_with("C-")
        || parse_positive_decimal(patchset_ordinal) != Some(patchset_number)
    {
        return Err(format!(
            "explicit Patchset identity fields disagree: {patchset_id:?}"
        ));
    }
    Ok(())
}

fn validate_snapshot_id(value: &str, label: &str) -> Result<(), String> {
    let suffix = value
        .strip_prefix("SNP-")
        .ok_or_else(|| format!("explicit {label} ID is invalid: {value:?}"))?;
    if suffix.len() != 12
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'A'..=b'F'))
    {
        return Err(format!("explicit {label} ID is invalid: {value:?}"));
    }
    Ok(())
}

fn parse_positive_decimal(value: &str) -> Option<u32> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse::<u32>().ok().filter(|value| *value != 0)
}

fn validate_relative_path(value: &str) -> Result<(), String> {
    let path = PathBuf::from(value);
    if value.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "legacy alias authority_relative_path is unsafe: {value:?}"
        ));
    }
    Ok(())
}

fn validate_non_empty_exact(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty() || value.trim() != value {
        Err(format!("{label} must be non-empty without normalization"))
    } else {
        Ok(())
    }
}

struct TextCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> TextCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn required_text(&mut self, field: &str) -> Result<&'a str, String> {
        self.optional_text(field)?
            .ok_or_else(|| format!("explicit Patchset payload {field} must be non-null Text"))
    }

    fn optional_text(&mut self, field: &str) -> Result<Option<&'a str>, String> {
        let length_bytes = self
            .bytes
            .get(self.offset..self.offset + 2)
            .ok_or_else(|| format!("explicit Patchset payload truncates {field} length"))?;
        let length = u16::from_le_bytes(length_bytes.try_into().unwrap());
        self.offset += 2;
        if length == u16::MAX {
            return Ok(None);
        }
        if length == 0 {
            return Err(format!(
                "explicit Patchset payload {field} must be non-empty Text"
            ));
        }
        let end = self
            .offset
            .checked_add(usize::from(length))
            .ok_or_else(|| format!("explicit Patchset payload {field} length overflow"))?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| format!("explicit Patchset payload truncates {field}"))?;
        self.offset = end;
        let value = std::str::from_utf8(bytes)
            .map_err(|_| format!("explicit Patchset payload {field} is not UTF-8"))?;
        if value.trim() != value {
            return Err(format!(
                "explicit Patchset payload {field} requires exact text"
            ));
        }
        Ok(Some(value))
    }

    fn required_count(&mut self, field: &str) -> Result<u16, String> {
        let bytes = self
            .bytes
            .get(self.offset..self.offset + 2)
            .ok_or_else(|| format!("explicit payload truncates {field} count"))?;
        self.offset += 2;
        let count = u16::from_le_bytes(bytes.try_into().unwrap());
        if count == u16::MAX {
            Err(format!("explicit payload {field} count is reserved"))
        } else {
            Ok(count)
        }
    }

    fn require_end(&self, entity: &str) -> Result<(), String> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(format!("explicit {entity} payload has trailing bytes"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str, output: &mut Vec<u8>) {
        output.extend_from_slice(&(value.len() as u16).to_le_bytes());
        output.extend_from_slice(value.as_bytes());
    }

    fn explicit_fixture(patchset_number: u32) -> (Vec<u8>, Vec<u8>) {
        let mut payload_slice = Vec::new();
        text("RCT-0079/C-01/P-01", &mut payload_slice);
        text("C-01", &mut payload_slice);
        text("SNP-111111111111", &mut payload_slice);
        text("SNP-C925402BCB36", &mut payload_slice);
        payload_slice.push(0);

        let mut fixed = 1_u32.to_le_bytes().to_vec();
        let mut record = [0_u8; EXPLICIT_PATCHSET_RECORD_SIZE];
        record[0..8].copy_from_slice(&4_u64.to_le_bytes());
        record[8..12].copy_from_slice(&(payload_slice.len() as u32).to_le_bytes());
        record[12..16].copy_from_slice(&0_u32.to_le_bytes());
        record[20..24].copy_from_slice(&patchset_number.to_le_bytes());
        fixed.extend_from_slice(&record);
        let mut payload = 1_u32.to_le_bytes().to_vec();
        payload.extend_from_slice(&payload_slice);
        (fixed, payload)
    }

    #[test]
    fn legacy_exact_id_index_maps_preserved_physical_ordinal_to_explicit_identity() {
        let (explicit_fixed, explicit_payload) = explicit_fixture(1);
        let explicit_patchsets =
            parse_explicit_patchsets(&explicit_fixed, &explicit_payload).unwrap();

        let source_patchset_id = "RCP-0079-1";
        let mut legacy_slice = Vec::new();
        legacy_slice.extend_from_slice(b"AITW");
        legacy_slice.extend_from_slice(&[1, 5]);
        legacy_slice.extend_from_slice(&1_u32.to_le_bytes());
        legacy_slice.extend_from_slice(&15_u32.to_le_bytes());
        legacy_slice.extend_from_slice(b"patchset_id.idx");
        legacy_slice.extend_from_slice(&(source_patchset_id.len() as u32).to_le_bytes());
        legacy_slice.extend_from_slice(source_patchset_id.as_bytes());
        legacy_slice.push(1);

        let mut legacy_fixed = 1_u32.to_le_bytes().to_vec();
        let mut legacy_record = [0_u8; LEGACY_PATCHSET_RECORD_SIZE];
        legacy_record[0..8].copy_from_slice(&4_u64.to_le_bytes());
        legacy_record[8..12].copy_from_slice(&(legacy_slice.len() as u32).to_le_bytes());
        legacy_record[12..16].copy_from_slice(&0_u32.to_le_bytes());
        legacy_fixed.extend_from_slice(&legacy_record);
        let mut legacy_payload = 1_u32.to_le_bytes().to_vec();
        legacy_payload.extend_from_slice(&legacy_slice);
        let mut legacy_index = 1_u32.to_le_bytes().to_vec();
        legacy_index.extend_from_slice(&(source_patchset_id.len() as u32).to_le_bytes());
        legacy_index.extend_from_slice(source_patchset_id.as_bytes());
        legacy_index.extend_from_slice(&0_u32.to_le_bytes());

        let aliases = parse_legacy_patchset_id_source(
            &legacy_fixed,
            &legacy_payload,
            &legacy_index,
            &explicit_patchsets,
        )
        .unwrap();
        assert_eq!(aliases[source_patchset_id], "RCT-0079/C-01/P-01");
    }

    #[test]
    fn explicit_patchset_alias_source_reads_identity_and_revision_snapshot() {
        let (fixed, payload) = explicit_fixture(1);
        let patchsets = parse_explicit_patchsets(&fixed, &payload).unwrap();
        assert_eq!(
            patchsets["RCT-0079/C-01/P-01"],
            LegacyPatchsetIdentity {
                physical_index: 0,
                canonical_patchset_id: "RCT-0079/C-01/P-01".to_string(),
                base_snapshot_id: "SNP-111111111111".to_string(),
                revision_snapshot_id: "SNP-C925402BCB36".to_string(),
                patchset_number: 1,
            }
        );
    }

    #[test]
    fn explicit_patchset_alias_source_rejects_ordinal_disagreement() {
        let (fixed, payload) = explicit_fixture(2);
        assert!(parse_explicit_patchsets(&fixed, &payload)
            .unwrap_err()
            .contains("identity fields disagree"));
    }

    #[test]
    fn legacy_policy_fingerprint_recognition_is_narrow() {
        assert!(looks_like_legacy_patchset_alias("RSEP-0169-3"));
        assert!(looks_like_legacy_patchset_alias("P-LCC-0044-3"));
        assert!(!looks_like_legacy_patchset_alias("RSET-0170/C-01/P-03"));
        assert!(!looks_like_legacy_patchset_alias("opaque"));
    }
}
