#![allow(unused_imports)]

use crate::foundation::db::ensure_postgres_schema_name;
use crate::foundation::policy_gate::{
    active_waiver_rules, policy_gate_evaluation, policy_input_fingerprint, policy_waiver_request,
};
use crate::foundation::server_context::{DEFAULT_CONTENT_SCHEMA, DEFAULT_CONTROL_SCHEMA};
use crate::foundation::workflow_artifacts::{
    effective_policy_status, patchset_changed_paths, requires_code_review_summary,
    review_summary_from_rows,
};
use ::postgres::{Client, NoTls, Row};
use chrono::Utc;
use serde_json::{json, Map as JsonMap, Value as JsonValue};

mod helpers;
mod policy_context;
mod postgres;
mod resolution;
mod rows;
mod runtime;
mod status;

use self::helpers::*;
use self::policy_context::{normalize_policy, policy_context_for_patchset};
use self::postgres::{PolicyInputs, PostgresPolicyStore};
use self::rows::*;
use self::runtime::PolicyStoreRuntime;
use self::status::{enrich_status_with_policy_context, ensure_effective_requirements};

pub use self::runtime::{server_policy_store_json, SERVER_POLICY_STORE_CONTRACT};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_declares_postgres_policy_store_operations() {
        let value = server_policy_store_json("contract", &json!({})).expect("contract");
        assert_eq!(value["contract"], json!(SERVER_POLICY_STORE_CONTRACT));
        assert_eq!(value["backend"], json!("postgres"));
        assert_eq!(
            value["migration_status"],
            json!("rust_owned_no_python_reference")
        );
        assert!(value.get("previous_reference_module").is_none());
        assert_eq!(value["mutates_state"], json!(true));
        assert!(value["operations"]
            .as_array()
            .unwrap()
            .contains(&json!("get-policy")));
        assert!(value["operations"]
            .as_array()
            .unwrap()
            .contains(&json!("evaluate-policy")));
        assert!(value["operations"]
            .as_array()
            .unwrap()
            .contains(&json!("create-waiver")));
    }

    #[test]
    fn runtime_rejects_non_postgres_and_fake_postgres() {
        let err = server_policy_store_json(
            "get-policy",
            &json!({"backend": "local-file", "patchset_id": "PS-1", "dsn": "postgresql://demo"}),
        )
        .expect_err("non-postgres backend should be rejected");
        assert!(err.contains("Only PostgreSQL is supported"));

        let err = server_policy_store_json(
            "get-policy",
            &json!({"backend": "postgres", "patchset_id": "PS-1", "dsn": "fake-postgres:///tmp/x"}),
        )
        .expect_err("fake postgres should be rejected");
        assert!(err.contains("fake-postgres is not supported"));
    }

    #[test]
    fn policy_context_resolves_defaults_and_doc_override_in_rust() {
        let mut patchset = JsonMap::new();
        patchset.insert("patchset_id".to_string(), json!("PS-1"));
        patchset.insert("author_mode".to_string(), json!("human_only"));
        patchset.insert(
            "diff_stats_json".to_string(),
            json!(r#"{"paths":{"modified":["README.md"]}}"#),
        );
        let context =
            policy_context_for_patchset(&JsonMap::new(), &patchset, None).expect("context");
        assert_eq!(context["content_class"], json!("docs_only"));
        assert_eq!(context["author_class"], json!("human_only"));
        assert_eq!(
            context["effective_requirements"]["require_tests"],
            json!(false)
        );
        assert_eq!(context["matched_overrides"][0]["index"], json!(1));
    }
}
