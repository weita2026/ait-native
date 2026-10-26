pub mod local;
pub mod remote;

pub use local::LocalContentBinaryDb;
pub use remote::{RemoteContentBinaryDb, RemoteFsContentBinaryDb};
