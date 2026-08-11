use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostgresConnectionPoolStats {
    pub idle_count: usize,
    pub total_count: usize,
    pub closed: bool,
}

struct PostgresConnectionPoolState<Raw> {
    idle: Vec<Raw>,
    total: usize,
    closed: bool,
}

impl<Raw> Default for PostgresConnectionPoolState<Raw> {
    fn default() -> Self {
        Self {
            idle: Vec::new(),
            total: 0,
            closed: false,
        }
    }
}

pub(super) struct PostgresConnectionPoolInner<D: PostgresPoolDriver> {
    dsn: String,
    schema: String,
    max_size: usize,
    pub(super) driver: Arc<D>,
    condition: Condvar,
    state: Mutex<PostgresConnectionPoolState<D::Raw>>,
}

pub struct PostgresConnectionPool<D: PostgresPoolDriver> {
    inner: Arc<PostgresConnectionPoolInner<D>>,
}

impl<D: PostgresPoolDriver> Clone for PostgresConnectionPool<D> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

pub struct PooledPostgresConnection<D: PostgresPoolDriver> {
    pub(super) pool: Arc<PostgresConnectionPoolInner<D>>,
    pub(super) raw: Option<D::Raw>,
    pub(super) transactional: bool,
}

impl<D: PostgresPoolDriver> PostgresConnectionPool<D> {
    pub fn new(
        dsn: impl Into<String>,
        schema: impl Into<String>,
        max_size: usize,
        driver: Arc<D>,
    ) -> Self {
        Self {
            inner: Arc::new(PostgresConnectionPoolInner {
                dsn: dsn.into(),
                schema: schema.into(),
                max_size: max_size.max(1),
                driver,
                condition: Condvar::new(),
                state: Mutex::new(PostgresConnectionPoolState::default()),
            }),
        }
    }

    pub fn checkout(&self) -> Result<PooledPostgresConnection<D>, String> {
        self.checkout_with_timeouts(&PostgresTimeoutScope::default())
    }

    pub fn checkout_with_timeouts(
        &self,
        timeouts: &PostgresTimeoutScope,
    ) -> Result<PooledPostgresConnection<D>, String> {
        self.checkout_with_mode(timeouts, true)
    }

    pub fn checkout_autocommit_with_timeouts(
        &self,
        timeouts: &PostgresTimeoutScope,
    ) -> Result<PooledPostgresConnection<D>, String> {
        self.checkout_with_mode(timeouts, false)
    }

    fn checkout_with_mode(
        &self,
        timeouts: &PostgresTimeoutScope,
        transactional: bool,
    ) -> Result<PooledPostgresConnection<D>, String> {
        let mut raw = None;
        let mut needs_create = false;
        loop {
            {
                let mut state = self
                    .inner
                    .state
                    .lock()
                    .expect("postgres pool mutex poisoned");
                loop {
                    if state.closed {
                        return Err("PostgreSQL connection pool is closed".to_string());
                    }
                    if let Some(existing) = state.idle.pop() {
                        raw = Some(existing);
                        break;
                    }
                    if state.total < self.inner.max_size {
                        state.total += 1;
                        needs_create = true;
                        break;
                    }
                    state = self
                        .inner
                        .condition
                        .wait(state)
                        .expect("postgres pool wait should succeed");
                }
            }

            if needs_create {
                let mut created = match self.inner.driver.connect(&self.inner.dsn) {
                    Ok(raw) => raw,
                    Err(err) => {
                        let mut state = self
                            .inner
                            .state
                            .lock()
                            .expect("postgres pool mutex poisoned");
                        state.total = state.total.saturating_sub(1);
                        self.inner.condition.notify_all();
                        return Err(err.to_string());
                    }
                };
                let configured = if transactional {
                    self.inner
                        .driver
                        .configure(&mut created, &self.inner.schema, true, timeouts)
                } else {
                    self.inner.driver.configure_autocommit(
                        &mut created,
                        &self.inner.schema,
                        true,
                        timeouts,
                    )
                };
                if let Err(err) = configured {
                    self.discard_created_raw(created);
                    return Err(err.to_string());
                }
                return Ok(PooledPostgresConnection {
                    pool: Arc::clone(&self.inner),
                    raw: Some(created),
                    transactional,
                });
            }

            if let Some(mut existing) = raw.take() {
                let _ = self.inner.driver.rollback(&mut existing);
                let configured = if transactional {
                    self.inner
                        .driver
                        .configure(&mut existing, &self.inner.schema, false, timeouts)
                } else {
                    self.inner.driver.configure_autocommit(
                        &mut existing,
                        &self.inner.schema,
                        false,
                        timeouts,
                    )
                };
                match configured {
                    Ok(()) => {
                        return Ok(PooledPostgresConnection {
                            pool: Arc::clone(&self.inner),
                            raw: Some(existing),
                            transactional,
                        })
                    }
                    Err(_) => {
                        self.discard_raw(existing);
                        needs_create = false;
                        continue;
                    }
                }
            }
        }
    }

    pub fn close(&self) {
        let idle = {
            let mut state = self
                .inner
                .state
                .lock()
                .expect("postgres pool mutex poisoned");
            state.closed = true;
            let idle = std::mem::take(&mut state.idle);
            self.inner.condition.notify_all();
            idle
        };
        for raw in idle {
            let _ = self.inner.driver.close(raw);
            let mut state = self
                .inner
                .state
                .lock()
                .expect("postgres pool mutex poisoned");
            state.total = state.total.saturating_sub(1);
            self.inner.condition.notify_all();
        }
    }

    pub fn stats(&self) -> PostgresConnectionPoolStats {
        let state = self
            .inner
            .state
            .lock()
            .expect("postgres pool mutex poisoned");
        PostgresConnectionPoolStats {
            idle_count: state.idle.len(),
            total_count: state.total,
            closed: state.closed,
        }
    }

    fn discard_created_raw(&self, raw: D::Raw) {
        let _ = self.inner.driver.close(raw);
        let mut state = self
            .inner
            .state
            .lock()
            .expect("postgres pool mutex poisoned");
        state.total = state.total.saturating_sub(1);
        self.inner.condition.notify_all();
    }

    fn discard_raw(&self, raw: D::Raw) {
        let _ = self.inner.driver.close(raw);
        let mut state = self
            .inner
            .state
            .lock()
            .expect("postgres pool mutex poisoned");
        state.total = state.total.saturating_sub(1);
        self.inner.condition.notify_all();
    }
}

impl<D: PostgresPoolDriver> PooledPostgresConnection<D> {
    pub fn raw(&self) -> &D::Raw {
        self.raw
            .as_ref()
            .expect("pooled connection should still hold raw")
    }

    pub fn raw_mut(&mut self) -> &mut D::Raw {
        self.raw
            .as_mut()
            .expect("pooled connection should still hold raw")
    }

    pub fn release(mut self) {
        if let Some(raw) = self.raw.take() {
            let mut state = self
                .pool
                .state
                .lock()
                .expect("postgres pool mutex poisoned");
            if state.closed {
                drop(state);
                let _ = self.pool.driver.close(raw);
                let mut state = self
                    .pool
                    .state
                    .lock()
                    .expect("postgres pool mutex poisoned");
                state.total = state.total.saturating_sub(1);
                self.pool.condition.notify_all();
            } else {
                state.idle.push(raw);
                self.pool.condition.notify_all();
            }
        }
    }

    pub fn discard(mut self) {
        if let Some(raw) = self.raw.take() {
            let _ = self.pool.driver.close(raw);
            let mut state = self
                .pool
                .state
                .lock()
                .expect("postgres pool mutex poisoned");
            state.total = state.total.saturating_sub(1);
            self.pool.condition.notify_all();
        }
    }
}

pub struct PostgresDbConnection<D: PostgresPoolDriver> {
    pub(super) pooled: Option<PooledPostgresConnection<D>>,
}

impl<D: PostgresPoolDriver> PostgresDbConnection<D> {
    pub fn new(pooled: PooledPostgresConnection<D>) -> Self {
        Self {
            pooled: Some(pooled),
        }
    }

    pub fn raw(&self) -> &D::Raw {
        self.pooled
            .as_ref()
            .expect("postgres db connection should still hold pooled raw")
            .raw()
    }

    pub fn raw_mut(&mut self) -> &mut D::Raw {
        self.pooled
            .as_mut()
            .expect("postgres db connection should still hold pooled raw")
            .raw_mut()
    }

    pub fn is_transactional(&self) -> bool {
        self.pooled
            .as_ref()
            .expect("postgres db connection should still hold pooled raw")
            .transactional
    }

    pub fn commit(&mut self) -> Result<(), String> {
        let pooled = self
            .pooled
            .as_mut()
            .expect("postgres db connection should still hold pooled raw");
        pooled
            .pool
            .driver
            .commit(
                pooled
                    .raw
                    .as_mut()
                    .expect("pooled connection should still hold raw"),
            )
            .map_err(|err| err.to_string())
    }

    pub fn rollback(&mut self) -> Result<(), String> {
        let pooled = self
            .pooled
            .as_mut()
            .expect("postgres db connection should still hold pooled raw");
        pooled
            .pool
            .driver
            .rollback(
                pooled
                    .raw
                    .as_mut()
                    .expect("pooled connection should still hold raw"),
            )
            .map_err(|err| err.to_string())
    }

    pub fn close(&mut self) {
        if let Some(mut pooled) = self.pooled.take() {
            if !pooled.transactional {
                pooled.release();
                return;
            }
            let rollback_ok = pooled
                .pool
                .driver
                .rollback(
                    pooled
                        .raw
                        .as_mut()
                        .expect("pooled connection should still hold raw"),
                )
                .is_ok();
            if rollback_ok {
                pooled.release();
            } else {
                pooled.discard();
            }
        }
    }

    pub fn discard(&mut self) {
        if let Some(pooled) = self.pooled.take() {
            pooled.discard();
        }
    }
}

impl<D: PostgresPoolDriver> Drop for PostgresDbConnection<D> {
    fn drop(&mut self) {
        self.close();
    }
}

impl<D: PostgresPoolDriver> Drop for PooledPostgresConnection<D> {
    fn drop(&mut self) {
        if let Some(raw) = self.raw.take() {
            let mut state = self
                .pool
                .state
                .lock()
                .expect("postgres pool mutex poisoned");
            if state.closed {
                drop(state);
                let _ = self.pool.driver.close(raw);
                let mut state = self
                    .pool
                    .state
                    .lock()
                    .expect("postgres pool mutex poisoned");
                state.total = state.total.saturating_sub(1);
                self.pool.condition.notify_all();
            } else {
                state.idle.push(raw);
                self.pool.condition.notify_all();
            }
        }
    }
}
