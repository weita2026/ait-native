use super::*;

/// A request identity validated before Repository or Actor registration effects.
#[derive(Clone, Debug)]
pub struct UserActorIdentity {
    payload: V0ActorPayload,
}

impl UserActorIdentity {
    pub fn new(user_name: &str, user_email: &str) -> Result<Self, String> {
        let payload = V0ActorPayload {
            user_name: user_name.trim().to_string(),
            user_id: String::new(),
            email: user_email.trim().to_string(),
            memo: String::new(),
        };
        if payload.user_name.is_empty() || payload.email.is_empty() {
            return Err("user_name and user_email must both be non-empty".into());
        }
        WorkflowBinaryV0Codec::encode_actor_payload(&payload).map_err(|error| {
            format!("user_name and user_email must fit the existing Actor payload: {error}")
        })?;
        Ok(Self { payload })
    }
}

impl<D> BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    pub fn resolve_user_actor(&self, identity: &UserActorIdentity) -> Result<u32, String> {
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerWorkflow)
                .map_err(|error| Self::error("Actor registration", error))?;
        let index =
            self.find_or_create_actor_payload(&mut tx, &identity.payload, 0, Self::now_s()?)?;
        index
            .checked_add(1)
            .ok_or("actor_key_index must fit plus-one bounds")?;
        tx.commit()
            .map_err(|error| Self::error("Actor registration commit", error))?;
        Ok(index)
    }
}

impl<D> BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone,
{
    pub fn validate_workflow_package_actor(&self, actor_key_index: u32) -> Result<u32, String> {
        self.request_actor(
            json!({"actor_key_index":actor_key_index})
                .as_object()
                .expect("Actor object"),
        )
    }

    pub(crate) fn request_actor(
        &self,
        payload: &JsonMap<String, JsonValue>,
    ) -> Result<u32, String> {
        let key = request_actor_key(payload)?;
        let read = BinaryDbReadTxn::new_bounded_for_scope(
            &self.db,
            crate::foundation::remote_binary_db::BinaryDbReadScope::WORKFLOW,
        );
        validate_actor_bytes(
            read.read_record(WorkflowBinaryV0Codec::actor_file(), key)
                .map_err(|error| {
                    format!(
                        "actor_key_index must reference a live Actor in this Repository: {error}"
                    )
                })?,
            key,
        )
    }

    pub(crate) fn request_actor_in_write<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
        payload: &JsonMap<String, JsonValue>,
    ) -> Result<u32, String> {
        let key = request_actor_key(payload)?;
        validate_actor_bytes(
            tx.read_record(WorkflowBinaryV0Codec::actor_file(), key)
                .map_err(|error| {
                    format!(
                        "actor_key_index must reference a live Actor in this Repository: {error}"
                    )
                })?,
            key,
        )
    }
}

fn request_actor_key(payload: &JsonMap<String, JsonValue>) -> Result<u32, String> {
    let key = payload
        .get("actor_key_index")
        .and_then(JsonValue::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or("actor_key_index must be a non-null u32")?;
    key.checked_add(1)
        .ok_or("actor_key_index must fit plus-one bounds")?;
    Ok(key)
}

pub(super) fn domain_request(payload: &JsonValue) -> Result<JsonValue, String> {
    let mut domain = payload
        .as_object()
        .ok_or("request payload must be an object")?
        .clone();
    domain.remove("actor_key_index");
    Ok(JsonValue::Object(domain))
}

fn validate_actor_bytes(bytes: Vec<u8>, key: u32) -> Result<u32, String> {
    let record = WorkflowBinaryV0Codec::decode_actor(&bytes).map_err(|error| error.to_string())?;
    if record.actor_meta & 0x80 != 0 {
        return Err("actor_key_index must reference a live Actor in this Repository".into());
    }
    Ok(key + 1)
}
