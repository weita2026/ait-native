use crate::external::lockfile::ExternalLockfile;
use crate::external::ExternalResult;

pub trait ExternalLockCodec {
    fn parse_lockfile(&self, bytes: &[u8]) -> ExternalResult<ExternalLockfile>;
    fn render_lockfile(&self, lockfile: &ExternalLockfile) -> ExternalResult<Vec<u8>>;
}
