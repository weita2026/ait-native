use crate::external::manifest::ExternalManifest;
use crate::external::ExternalResult;

pub trait ExternalManifestCodec {
    fn parse_manifest(&self, bytes: &[u8]) -> ExternalResult<ExternalManifest>;
    fn render_manifest(&self, manifest: &ExternalManifest) -> ExternalResult<Vec<u8>>;
}
