use crate::json_support::JsonValue as Value;

pub trait TaskWorkflowMutationReceiptBuilder {
    fn mutation_receipt(
        &self,
        action: &str,
        source_action: &str,
        delivery: &str,
        response_recovery: Option<&Value>,
        result: Option<&Value>,
    ) -> Result<Value, String>;
}

pub trait TaskWorkflowActionMutationReceiptsBuilder {
    fn action_mutation_receipts(&self, code: &str, result: &Value) -> Result<Value, String>;
}

pub trait TaskWorkflowMutationReceiptRemote:
    TaskWorkflowMutationReceiptBuilder + TaskWorkflowActionMutationReceiptsBuilder
{
}

impl<R> TaskWorkflowMutationReceiptRemote for R where
    R: TaskWorkflowMutationReceiptBuilder + TaskWorkflowActionMutationReceiptsBuilder + ?Sized
{
}
