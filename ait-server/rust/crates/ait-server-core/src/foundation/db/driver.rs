use super::*;

pub trait PostgresPoolDriver: Send + Sync + 'static {
    type Raw: Send + 'static;
    type Error: ToString + Send + 'static;

    fn connect(&self, dsn: &str) -> Result<Self::Raw, Self::Error>;
    fn commit(&self, raw: &mut Self::Raw) -> Result<(), Self::Error>;
    fn rollback(&self, raw: &mut Self::Raw) -> Result<(), Self::Error>;
    fn configure(
        &self,
        raw: &mut Self::Raw,
        schema: &str,
        ensure_schema: bool,
        timeouts: &PostgresTimeoutScope,
    ) -> Result<(), Self::Error>;
    fn configure_autocommit(
        &self,
        raw: &mut Self::Raw,
        schema: &str,
        ensure_schema: bool,
        timeouts: &PostgresTimeoutScope,
    ) -> Result<(), Self::Error> {
        self.configure(raw, schema, ensure_schema, timeouts)?;
        self.commit(raw)
    }
    fn advisory_lock(
        &self,
        raw: &mut Self::Raw,
        key_hi: i64,
        key_lo: i64,
    ) -> Result<(), Self::Error>;
    fn advisory_unlock(
        &self,
        raw: &mut Self::Raw,
        key_hi: i64,
        key_lo: i64,
    ) -> Result<(), Self::Error>;
    fn close(&self, raw: Self::Raw) -> Result<(), Self::Error>;
}

#[derive(Debug)]
pub enum NativePostgresDriverError {
    Postgres(postgres::Error),
    InvalidAdvisoryLockKey(String),
}

impl fmt::Display for NativePostgresDriverError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Postgres(error) => write!(formatter, "{error}"),
            Self::InvalidAdvisoryLockKey(message) => write!(formatter, "{message}"),
        }
    }
}

impl From<postgres::Error> for NativePostgresDriverError {
    fn from(error: postgres::Error) -> Self {
        Self::Postgres(error)
    }
}

#[derive(Debug, Default)]
pub struct NativePostgresDriver;

impl NativePostgresDriver {
    fn advisory_key_part(value: i64, name: &str) -> Result<i32, NativePostgresDriverError> {
        i32::try_from(value).map_err(|_| {
            NativePostgresDriverError::InvalidAdvisoryLockKey(format!(
                "{name} advisory lock key is outside PostgreSQL int4 range: {value}"
            ))
        })
    }
}

impl PostgresPoolDriver for NativePostgresDriver {
    type Raw = Client;
    type Error = NativePostgresDriverError;

    fn connect(&self, dsn: &str) -> Result<Self::Raw, Self::Error> {
        Client::connect(dsn, NoTls).map_err(Into::into)
    }

    fn commit(&self, raw: &mut Self::Raw) -> Result<(), Self::Error> {
        raw.batch_execute("commit").map_err(Into::into)
    }

    fn rollback(&self, raw: &mut Self::Raw) -> Result<(), Self::Error> {
        raw.batch_execute("rollback").map_err(Into::into)
    }

    fn configure(
        &self,
        raw: &mut Self::Raw,
        schema: &str,
        ensure_schema: bool,
        timeouts: &PostgresTimeoutScope,
    ) -> Result<(), Self::Error> {
        for statement in configure_postgres_checkout_sql(schema, ensure_schema, timeouts) {
            raw.batch_execute(&statement)?;
        }
        Ok(())
    }

    fn configure_autocommit(
        &self,
        raw: &mut Self::Raw,
        schema: &str,
        ensure_schema: bool,
        timeouts: &PostgresTimeoutScope,
    ) -> Result<(), Self::Error> {
        for statement in configure_postgres_session_sql(schema, ensure_schema, timeouts) {
            raw.batch_execute(&statement)?;
        }
        Ok(())
    }

    fn advisory_lock(
        &self,
        raw: &mut Self::Raw,
        key_hi: i64,
        key_lo: i64,
    ) -> Result<(), Self::Error> {
        let key_hi = Self::advisory_key_part(key_hi, "high")?;
        let key_lo = Self::advisory_key_part(key_lo, "low")?;
        raw.execute("select pg_advisory_lock($1, $2)", &[&key_hi, &key_lo])?;
        Ok(())
    }

    fn advisory_unlock(
        &self,
        raw: &mut Self::Raw,
        key_hi: i64,
        key_lo: i64,
    ) -> Result<(), Self::Error> {
        let key_hi = Self::advisory_key_part(key_hi, "high")?;
        let key_lo = Self::advisory_key_part(key_lo, "low")?;
        raw.execute("select pg_advisory_unlock($1, $2)", &[&key_hi, &key_lo])?;
        Ok(())
    }

    fn close(&self, raw: Self::Raw) -> Result<(), Self::Error> {
        raw.close().map_err(Into::into)
    }
}
