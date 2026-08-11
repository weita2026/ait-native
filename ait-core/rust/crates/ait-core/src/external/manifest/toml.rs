use crate::external::manifest::{ExternalManifest, ExternalManifestCodec};
use crate::external::{ExternalError, ExternalResult};

#[derive(Debug, Default, Clone, Copy)]
pub struct TomlExternalManifestCodec;

impl ExternalManifestCodec for TomlExternalManifestCodec {
    fn parse_manifest(&self, bytes: &[u8]) -> ExternalResult<ExternalManifest> {
        let text = std::str::from_utf8(bytes).map_err(|err| {
            ExternalError::with_code(
                "external_manifest_utf8",
                format!("ait-external.toml must be valid UTF-8: {err}"),
            )
        })?;
        let manifest: ExternalManifest = toml::from_str(text).map_err(|err| {
            ExternalError::with_code(
                "external_manifest_parse",
                format!("failed to parse ait-external.toml: {err}"),
            )
        })?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn render_manifest(&self, manifest: &ExternalManifest) -> ExternalResult<Vec<u8>> {
        manifest.validate()?;
        let text = toml::to_string_pretty(manifest).map_err(|err| {
            ExternalError::with_code(
                "external_manifest_render",
                format!("failed to render ait-external.toml: {err}"),
            )
        })?;
        Ok(text.into_bytes())
    }
}
