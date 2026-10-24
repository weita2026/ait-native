use crate::json_support::JsonValue as Value;

use super::http_client_types::TaskWorkflowHttpClientResult;

pub trait TaskWorkflowHistoryPromotionPreparer {
    fn prepare_history_promotion(
        &mut self,
        repo_name: &str,
        payload: &Value,
    ) -> TaskWorkflowHttpClientResult<Value>;
}
