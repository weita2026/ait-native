use super::*;

const POSTGRES_ADVISORY_LOCK_PERSON: &[u8] = b"ait-lock";

pub fn postgres_advisory_lock_key(scope: &str) -> Result<(i64, i64), String> {
    let text = scope.trim();
    if text.is_empty() {
        return Err("scope is required".to_string());
    }
    let digest = Params::new()
        .hash_length(8)
        .personal(POSTGRES_ADVISORY_LOCK_PERSON)
        .hash(text.as_bytes());
    let bytes: [u8; 8] = digest
        .as_bytes()
        .try_into()
        .expect("digest length should stay at 8");
    let value = u64::from_be_bytes(bytes);
    Ok((
        ((value >> 32) & 0x7FFF_FFFF) as i64,
        (value & 0x7FFF_FFFF) as i64,
    ))
}

pub fn with_postgres_advisory_lock<D, F, T, E>(
    conn: &mut PostgresDbConnection<D>,
    scope: &str,
    callback: F,
) -> Result<T, String>
where
    D: PostgresPoolDriver,
    F: FnOnce(&mut PostgresDbConnection<D>) -> Result<T, E>,
    E: ToString,
{
    let (key_hi, key_lo) = postgres_advisory_lock_key(scope)?;
    {
        let pooled = conn
            .pooled
            .as_mut()
            .expect("postgres db connection should still hold pooled raw");
        pooled
            .pool
            .driver
            .advisory_lock(
                pooled
                    .raw
                    .as_mut()
                    .expect("pooled connection should still hold raw"),
                key_hi,
                key_lo,
            )
            .map_err(|err| err.to_string())?;
    }
    let result = callback(conn).map_err(|err| err.to_string());
    if let Some(pooled) = conn.pooled.as_mut() {
        let _ = pooled.pool.driver.advisory_unlock(
            pooled
                .raw
                .as_mut()
                .expect("pooled connection should still hold raw"),
            key_hi,
            key_lo,
        );
    }
    result
}
