use super::*;
use crate::workflow_package::{WORKFLOW_PACKAGE_MEDIA_TYPE, WorkflowPackageReference};

pub(super) struct ClaimedSourceProvider<'a> {
    pub(super) client: &'a ServerClient,
    pub(super) claim: &'a BinaryClaim,
    pub(super) control: &'a ExecutionControl,
}

impl RemoteSnapshotProvider for ClaimedSourceProvider<'_> {
    fn fetch_import_manifest(
        &self,
        source: &RemoteSnapshotReference,
    ) -> Result<Vec<u8>, RunnerError> {
        self.control.check()?;
        self.client.fetch_import_manifest(source)
    }

    fn download_pack(
        &self,
        source: &RemoteSnapshotReference,
        kind: RemotePackKind,
        id: &str,
        destination: &Path,
        maximum: u64,
    ) -> Result<u64, RunnerError> {
        self.control.check()?;
        self.client
            .download_pack(source, kind, id, destination, maximum)
    }

    fn download_workflow_package(
        &self,
        source: &WorkflowPackageReference,
        destination: &Path,
        maximum: u64,
    ) -> Result<u64, RunnerError> {
        let proof = &self.claim.proof;
        proof.validate().map_err(RunnerError::InvalidRequest)?;
        let matches_claim = matches!(&self.claim.request.source,
            SourceSpec::WorkflowPackage { repository_index, package_hash, base_snapshot_id, revision_snapshot_id }
            if *repository_index == source.key.repository_index && package_hash == &source.package_hash
                && base_snapshot_id == &source.base_snapshot_id && revision_snapshot_id == &source.revision_snapshot_id);
        if source.key != proof.key() || !matches_claim {
            return Err(RunnerError::InvalidRequest(
                "workflow_package source does not match the original claimed Job".into(),
            ));
        }
        self.control.check()?;
        let url = format!(
            "{}/v1/native/repository-authorities/{}/worker-jobs/{}",
            self.client.base_url, proof.repository_index, proof.worker_job_index
        );
        let mut response = self
            .client
            .auth(
                self.client
                    .client
                    .get(url)
                    .header(reqwest::header::ACCEPT, WORKFLOW_PACKAGE_MEDIA_TYPE)
                    .header("X-Ait-Package-Sha256", &source.package_hash)
                    .header(
                        "X-Ait-Worker-Attempt-Count",
                        proof.attempt_count.to_string(),
                    )
                    .header("X-Ait-Worker-Lease-Token", &proof.lease_token),
            )
            .send()
            .map_err(|error| {
                RunnerError::ServerUnavailable(format!("workflow package request failed: {error}"))
            })?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(
                decode_error_response(response, "workflow package").redact(&proof.lease_token)
            );
        }
        let types: Vec<_> = response
            .headers()
            .get_all(reqwest::header::CONTENT_TYPE)
            .iter()
            .collect();
        let hashes: Vec<_> = response
            .headers()
            .get_all("X-Ait-Package-Sha256")
            .iter()
            .collect();
        if types.len() != 1
            || types[0]
                .to_str()
                .ok()
                .and_then(|text| text.split(';').next())
                .map(str::trim)
                != Some(WORKFLOW_PACKAGE_MEDIA_TYPE)
            || hashes.len() != 1
            || hashes[0].to_str().ok() != Some(source.package_hash.as_str())
            || response
                .headers()
                .get(reqwest::header::CONTENT_ENCODING)
                .is_some_and(|value| value != "identity")
        {
            return Err(RunnerError::Server(
                "workflow package returned invalid media type, hash or content encoding".into(),
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > maximum)
        {
            return Err(RunnerError::Server(
                "workflow package exceeds download bound".into(),
            ));
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .map_err(|error| RunnerError::fs("create downloaded package", destination, error))?;
        let mut total = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            self.control.check()?;
            let read = response
                .read(&mut buffer)
                .map_err(|error| RunnerError::fs("read workflow package", destination, error))?;
            if read == 0 {
                break;
            }
            total = total
                .checked_add(read as u64)
                .ok_or_else(|| RunnerError::Server("workflow package size overflow".into()))?;
            if total > maximum {
                return Err(RunnerError::Server(
                    "workflow package exceeds download bound".into(),
                ));
            }
            output
                .write_all(&buffer[..read])
                .map_err(|error| RunnerError::fs("write workflow package", destination, error))?;
        }
        self.control.check()?;
        Ok(total)
    }
}

#[cfg(test)]
mod tests;
