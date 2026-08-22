use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::foundation::transport::{self, AsyncJobPayloadInput};

pub struct AsyncJobJson<S> {
    store: S,
}

impl<S> AsyncJobJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl AsyncJobJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> AsyncJobJson<S> {
    pub fn supported_async_job_types(&self) -> Vec<String> {
        let _ = &self.store;
        transport::supported_async_job_types_impl()
    }

    pub fn async_job_contract(&self) -> Vec<JsonMap<String, JsonValue>> {
        let _ = &self.store;
        transport::async_job_contract_impl()
    }

    pub fn normalize_async_job_payload<'a, P>(
        &self,
        job_type: &str,
        payload: P,
    ) -> Result<JsonMap<String, JsonValue>, String>
    where
        P: AsyncJobPayloadInput<'a>,
    {
        let _ = &self.store;
        transport::normalize_async_job_payload_impl(job_type, payload)
    }

    pub fn retry_delay_seconds_for_job(&self, job_type: &str) -> i64 {
        let _ = &self.store;
        transport::retry_delay_seconds_for_job_impl(job_type)
    }

    pub fn max_attempts_for_job(&self, job_type: &str) -> i64 {
        let _ = &self.store;
        transport::max_attempts_for_job_impl(job_type)
    }

    pub fn row_to_job(
        &self,
        row: &JsonMap<String, JsonValue>,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let _ = &self.store;
        transport::row_to_job_impl(row)
    }

    pub fn land_request_payload(
        &self,
        row: &JsonMap<String, JsonValue>,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let _ = &self.store;
        transport::land_request_payload_impl(row)
    }

    pub fn phase_timings_from_result(
        &self,
        result: Option<&JsonValue>,
    ) -> JsonMap<String, JsonValue> {
        let _ = &self.store;
        transport::phase_timings_from_result_impl(result)
    }
}
