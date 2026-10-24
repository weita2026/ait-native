//! Share the same full admission used by capture, activation and migration.
use super::*;
pub(super) fn validate(
    objects: &LocalGitObjectStore,
    history: &GitHistoryStore,
    documents: &RawGitDocumentStore,
    content: &LocalContentBinaryDb<1>,
) -> Result<(), String> {
    validate_git_content_closure(objects, history, documents, content)
}
