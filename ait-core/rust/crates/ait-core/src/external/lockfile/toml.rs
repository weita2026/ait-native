use crate::external::lockfile::{ExternalLockCodec, ExternalLockfile};
use crate::external::{ExternalError, ExternalResult};

#[derive(Debug, Default, Clone, Copy)]
pub struct TomlExternalLockCodec;

impl ExternalLockCodec for TomlExternalLockCodec {
    fn parse_lockfile(&self, bytes: &[u8]) -> ExternalResult<ExternalLockfile> {
        let text = std::str::from_utf8(bytes).map_err(|err| {
            ExternalError::with_code(
                "external_lock_utf8",
                format!("ait-external.lock must be valid UTF-8: {err}"),
            )
        })?;
        let lockfile: ExternalLockfile = toml::from_str(text).map_err(|err| {
            ExternalError::with_code(
                "external_lock_parse",
                format!("failed to parse ait-external.lock: {err}"),
            )
        })?;
        lockfile.validate()?;
        Ok(lockfile)
    }

    fn render_lockfile(&self, lockfile: &ExternalLockfile) -> ExternalResult<Vec<u8>> {
        lockfile.validate()?;
        let normalized = lockfile.normalized();
        let text = toml::to_string_pretty(&normalized).map_err(|err| {
            ExternalError::with_code(
                "external_lock_render",
                format!("failed to render ait-external.lock: {err}"),
            )
        })?;
        Ok(text.into_bytes())
    }
}
