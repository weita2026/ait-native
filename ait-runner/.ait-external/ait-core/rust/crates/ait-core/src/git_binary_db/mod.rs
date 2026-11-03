//! Optional Git interoperability codecs and native object storage.
//!
//! These kernels are not runtime admission: CLI activation also requires
//! document lineage, generation/mapping validation and typed GC roots.

mod admission;
mod bundle;
mod delta_hints;
mod documents;
mod gc;
mod history;
mod records;
mod store;

pub use admission::validate_git_content_closure;
pub(crate) use admission::validate_git_content_closure_with_session;
pub use bundle::*;
pub use delta_hints::GitObjectDeltaHints;
pub use documents::*;
pub use gc::*;
pub use history::*;
pub use records::*;
pub use store::*;

#[cfg(test)]
mod tests;
