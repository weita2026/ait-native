use std::collections::BTreeMap;

use crate::json_support::JsonValue as Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanHttpRequestSpec {
    pub method: String,
    pub path: String,
    pub url: String,
    pub query_pairs: Vec<(String, String)>,
    pub headers: BTreeMap<String, String>,
    pub body: Option<Value>,
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanHttpBytesRequestSpec {
    pub method: String,
    pub path: String,
    pub url: String,
    pub query_pairs: Vec<(String, String)>,
    pub headers: BTreeMap<String, String>,
    pub body: Option<Vec<u8>>,
    pub timeout_ms: u64,
}

pub trait PlanHttpClientLifecycle {
    type Stats;

    fn inspect(&self) -> Self::Stats;
    fn close(&mut self) -> Self::Stats;
}

pub trait PlanHttpTransport {
    type Error;

    fn execute_json(&mut self, spec: PlanHttpRequestSpec) -> Result<Option<Value>, Self::Error>;
    fn execute_bytes(&mut self, spec: PlanHttpBytesRequestSpec) -> Result<Vec<u8>, Self::Error>;
}

pub fn inspect_with_plan_http_client_lifecycle<C>(client: &C) -> C::Stats
where
    C: PlanHttpClientLifecycle + ?Sized,
{
    client.inspect()
}

pub fn close_with_plan_http_client_lifecycle<C>(client: &mut C) -> C::Stats
where
    C: PlanHttpClientLifecycle + ?Sized,
{
    client.close()
}

pub fn execute_json_with_plan_http_transport<T>(
    transport: &mut T,
    spec: PlanHttpRequestSpec,
) -> Result<Option<Value>, T::Error>
where
    T: PlanHttpTransport + ?Sized,
{
    transport.execute_json(spec)
}

pub fn execute_bytes_with_plan_http_transport<T>(
    transport: &mut T,
    spec: PlanHttpBytesRequestSpec,
) -> Result<Vec<u8>, T::Error>
where
    T: PlanHttpTransport + ?Sized,
{
    transport.execute_bytes(spec)
}
