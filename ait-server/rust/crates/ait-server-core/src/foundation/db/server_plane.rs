use super::*;

pub fn connect_postgres_runtime<D: PostgresPoolDriver>(
    registry: &PostgresConnectionPoolRegistry<D>,
    dsn: &str,
    schema: &str,
    timeouts: &PostgresTimeoutScope,
) -> Result<PostgresDbConnection<D>, String> {
    let schema = ensure_postgres_schema_name(schema)?;
    let pooled = registry
        .pool(dsn, schema)
        .checkout_with_timeouts(timeouts)?;
    Ok(PostgresDbConnection::new(pooled))
}

pub fn connect_postgres_runtime_autocommit<D: PostgresPoolDriver>(
    registry: &PostgresConnectionPoolRegistry<D>,
    dsn: &str,
    schema: &str,
    timeouts: &PostgresTimeoutScope,
) -> Result<PostgresDbConnection<D>, String> {
    let schema = ensure_postgres_schema_name(schema)?;
    let pooled = registry
        .pool(dsn, schema)
        .checkout_autocommit_with_timeouts(timeouts)?;
    Ok(PostgresDbConnection::new(pooled))
}

pub fn connect_server_plane<D: PostgresPoolDriver>(
    registry: &PostgresConnectionPoolRegistry<D>,
    backend: &str,
    dsn: Option<&str>,
    content_schema: &str,
    control_schema: &str,
    plane: &str,
    timeouts: &PostgresTimeoutScope,
) -> Result<PostgresDbConnection<D>, String> {
    let (config, plane) =
        resolve_server_plane_runtime(backend, dsn, content_schema, control_schema, plane)?;
    connect_postgres_runtime(registry, &config.dsn, config.schema_for(plane), timeouts)
}

pub fn connect_server_plane_autocommit<D: PostgresPoolDriver>(
    registry: &PostgresConnectionPoolRegistry<D>,
    backend: &str,
    dsn: Option<&str>,
    content_schema: &str,
    control_schema: &str,
    plane: &str,
    timeouts: &PostgresTimeoutScope,
) -> Result<PostgresDbConnection<D>, String> {
    let (config, plane) =
        resolve_server_plane_runtime(backend, dsn, content_schema, control_schema, plane)?;
    connect_postgres_runtime_autocommit(registry, &config.dsn, config.schema_for(plane), timeouts)
}

pub fn run_server_plane<D, F, T, E>(
    registry: &PostgresConnectionPoolRegistry<D>,
    backend: &str,
    dsn: Option<&str>,
    content_schema: &str,
    control_schema: &str,
    plane: &str,
    write: bool,
    timeouts: &PostgresTimeoutScope,
    callback: F,
) -> Result<T, String>
where
    D: PostgresPoolDriver,
    F: FnOnce(&mut PostgresDbConnection<D>) -> Result<T, E>,
    E: ToString,
{
    let mut conn = connect_server_plane(
        registry,
        backend,
        dsn,
        content_schema,
        control_schema,
        plane,
        timeouts,
    )?;
    let result = callback(&mut conn).map_err(|err| err.to_string())?;
    if write {
        conn.commit()?;
    }
    Ok(result)
}

pub fn read_server_plane<D, F, T, E>(
    registry: &PostgresConnectionPoolRegistry<D>,
    backend: &str,
    dsn: Option<&str>,
    content_schema: &str,
    control_schema: &str,
    plane: &str,
    timeouts: &PostgresTimeoutScope,
    callback: F,
) -> Result<T, String>
where
    D: PostgresPoolDriver,
    F: FnOnce(&mut PostgresDbConnection<D>) -> Result<T, E>,
    E: ToString,
{
    run_server_plane(
        registry,
        backend,
        dsn,
        content_schema,
        control_schema,
        plane,
        false,
        timeouts,
        callback,
    )
}

pub fn read_server_plane_autocommit<D, F, T, E>(
    registry: &PostgresConnectionPoolRegistry<D>,
    backend: &str,
    dsn: Option<&str>,
    content_schema: &str,
    control_schema: &str,
    plane: &str,
    timeouts: &PostgresTimeoutScope,
    callback: F,
) -> Result<T, String>
where
    D: PostgresPoolDriver,
    F: FnOnce(&mut PostgresDbConnection<D>) -> Result<T, E>,
    E: ToString,
{
    let mut conn = connect_server_plane_autocommit(
        registry,
        backend,
        dsn,
        content_schema,
        control_schema,
        plane,
        timeouts,
    )?;
    callback(&mut conn).map_err(|err| err.to_string())
}

pub fn write_server_plane<D, F, T, E>(
    registry: &PostgresConnectionPoolRegistry<D>,
    backend: &str,
    dsn: Option<&str>,
    content_schema: &str,
    control_schema: &str,
    plane: &str,
    timeouts: &PostgresTimeoutScope,
    callback: F,
) -> Result<T, String>
where
    D: PostgresPoolDriver,
    F: FnOnce(&mut PostgresDbConnection<D>) -> Result<T, E>,
    E: ToString,
{
    run_server_plane(
        registry,
        backend,
        dsn,
        content_schema,
        control_schema,
        plane,
        true,
        timeouts,
        callback,
    )
}
