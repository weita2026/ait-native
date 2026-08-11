#![allow(unused_imports)]

use crate::foundation::workflow_artifacts::review_summary_from_rows;
use crate::middle::read_model_contract::{
    json_value_to_text, object_text_field, optional_text_field, read_model_payload_object,
    ReadModelContract, ReadModelRowSetSpec, ReadModelRows,
};
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[path = "secondary_read_model/authority_map.rs"]
mod authority_map;
#[path = "secondary_read_model/contracts.rs"]
mod contracts;
#[path = "secondary_read_model/documents.rs"]
mod documents;
#[path = "secondary_read_model/filters.rs"]
mod filters;
#[path = "secondary_read_model/helpers.rs"]
mod helpers;
#[path = "secondary_read_model/inputs.rs"]
mod inputs;
#[path = "secondary_read_model/markdown.rs"]
mod markdown;
#[path = "secondary_read_model/reviewer_inbox.rs"]
mod reviewer_inbox;

pub use authority_map::authority_map_read_model;
pub use contracts::{
    authority_map_read_model_contract, reviewer_inbox_read_model_contract,
    AUTHORITY_MAP_READ_MODEL_CONTRACT, AUTHORITY_MAP_ROW_SETS, REVIEWER_INBOX_READ_MODEL_CONTRACT,
    REVIEWER_INBOX_ROW_SETS,
};
pub use inputs::{AuthorityMapInput, ReviewerInboxInput};
pub use reviewer_inbox::reviewer_inbox_read_model;

use documents::{
    add_related_documents, authority_doc, authority_doc_or_missing, authority_missing_doc,
    authority_node_layer, authority_parent_path, document_short_title, merge_node_fields,
    sort_docs, sync_related_documents,
};
use filters::{
    effective_validation_state, matches_author_class, matches_filter, matches_review_filter,
    missing_requirements, normalize_filter, repo_matches,
};
use helpers::{
    filename, filename_stem, insert_string, int_field, int_value, object_text, optional_text,
    parse_json_field, patchset_number, string_list, value_int, value_text,
};
use markdown::{body_markdown, markdown_link_targets, markdown_metadata, markdown_title};
