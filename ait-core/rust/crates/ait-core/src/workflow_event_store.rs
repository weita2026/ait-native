use crate::json_support::JsonValue;

pub type WorkflowEventStoreResult<T> = Result<T, String>;

#[derive(Clone, Debug, PartialEq)]
pub struct WorkflowEventRecord {
    pub event_type: String,
    pub entity_type: String,
    pub entity_id: String,
    pub payload: JsonValue,
    pub created_at: String,
}

pub trait WorkflowEventStore {
    fn record_event(&self, event: &WorkflowEventRecord) -> WorkflowEventStoreResult<bool>;
}

pub fn record_workflow_event_with_store<S>(
    store: &S,
    event: &WorkflowEventRecord,
) -> WorkflowEventStoreResult<bool>
where
    S: WorkflowEventStore + ?Sized,
{
    store.record_event(event)
}
