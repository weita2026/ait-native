#![cfg(feature = "legacy-postgres-runtime")]

use ait_server_core::foundation::db::{
    configure_postgres_checkout_sql, configure_postgres_session_sql, connect_postgres_runtime,
    connect_server_plane, read_server_plane, read_server_plane_autocommit,
    resolve_server_plane_runtime, with_postgres_advisory_lock, write_server_plane,
    PostgresConnectionPoolRegistry, PostgresPoolDriver, PostgresTimeoutScope,
};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
struct FakeRaw {
    id: usize,
}

#[derive(Default)]
struct FakeDriverState {
    connect_calls: usize,
    commit_calls: Vec<usize>,
    rollback_calls: Vec<usize>,
    configure_calls: Vec<(usize, String, bool, Option<i64>, Option<i64>)>,
    configure_autocommit_calls: Vec<(usize, String, bool, Option<i64>, Option<i64>)>,
    advisory_lock_calls: Vec<(usize, i64, i64)>,
    advisory_unlock_calls: Vec<(usize, i64, i64)>,
    close_calls: Vec<usize>,
    next_id: usize,
    fail_rollback_ids: HashSet<usize>,
    fail_unlock_ids: HashSet<usize>,
}

#[derive(Clone, Default)]
struct FakeDriver {
    state: Arc<Mutex<FakeDriverState>>,
}

impl FakeDriver {
    fn fail_rollback_for(&self, raw_id: usize) {
        self.state
            .lock()
            .expect("fake driver mutex poisoned")
            .fail_rollback_ids
            .insert(raw_id);
    }

    fn fail_unlock_for(&self, raw_id: usize) {
        self.state
            .lock()
            .expect("fake driver mutex poisoned")
            .fail_unlock_ids
            .insert(raw_id);
    }

    fn snapshot(&self) -> FakeDriverSnapshot {
        let state = self.state.lock().expect("fake driver mutex poisoned");
        FakeDriverSnapshot {
            connect_calls: state.connect_calls,
            commit_calls: state.commit_calls.clone(),
            rollback_calls: state.rollback_calls.clone(),
            configure_calls: state.configure_calls.clone(),
            configure_autocommit_calls: state.configure_autocommit_calls.clone(),
            advisory_lock_calls: state.advisory_lock_calls.clone(),
            advisory_unlock_calls: state.advisory_unlock_calls.clone(),
            close_calls: state.close_calls.clone(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct FakeDriverSnapshot {
    connect_calls: usize,
    commit_calls: Vec<usize>,
    rollback_calls: Vec<usize>,
    configure_calls: Vec<(usize, String, bool, Option<i64>, Option<i64>)>,
    configure_autocommit_calls: Vec<(usize, String, bool, Option<i64>, Option<i64>)>,
    advisory_lock_calls: Vec<(usize, i64, i64)>,
    advisory_unlock_calls: Vec<(usize, i64, i64)>,
    close_calls: Vec<usize>,
}

impl PostgresPoolDriver for FakeDriver {
    type Raw = FakeRaw;
    type Error = String;

    fn connect(&self, _dsn: &str) -> Result<Self::Raw, Self::Error> {
        let mut state = self.state.lock().expect("fake driver mutex poisoned");
        state.next_id += 1;
        state.connect_calls += 1;
        Ok(FakeRaw { id: state.next_id })
    }

    fn commit(&self, raw: &mut Self::Raw) -> Result<(), Self::Error> {
        self.state
            .lock()
            .expect("fake driver mutex poisoned")
            .commit_calls
            .push(raw.id);
        Ok(())
    }

    fn rollback(&self, raw: &mut Self::Raw) -> Result<(), Self::Error> {
        let mut state = self.state.lock().expect("fake driver mutex poisoned");
        state.rollback_calls.push(raw.id);
        if state.fail_rollback_ids.remove(&raw.id) {
            return Err(format!("rollback failed for {}", raw.id));
        }
        Ok(())
    }

    fn configure(
        &self,
        raw: &mut Self::Raw,
        schema: &str,
        ensure_schema: bool,
        timeouts: &PostgresTimeoutScope,
    ) -> Result<(), Self::Error> {
        self.state
            .lock()
            .expect("fake driver mutex poisoned")
            .configure_calls
            .push((
                raw.id,
                schema.to_string(),
                ensure_schema,
                timeouts.lock_timeout_ms,
                timeouts.statement_timeout_ms,
            ));
        Ok(())
    }

    fn configure_autocommit(
        &self,
        raw: &mut Self::Raw,
        schema: &str,
        ensure_schema: bool,
        timeouts: &PostgresTimeoutScope,
    ) -> Result<(), Self::Error> {
        self.state
            .lock()
            .expect("fake driver mutex poisoned")
            .configure_autocommit_calls
            .push((
                raw.id,
                schema.to_string(),
                ensure_schema,
                timeouts.lock_timeout_ms,
                timeouts.statement_timeout_ms,
            ));
        Ok(())
    }

    fn advisory_lock(
        &self,
        raw: &mut Self::Raw,
        key_hi: i64,
        key_lo: i64,
    ) -> Result<(), Self::Error> {
        self.state
            .lock()
            .expect("fake driver mutex poisoned")
            .advisory_lock_calls
            .push((raw.id, key_hi, key_lo));
        Ok(())
    }

    fn advisory_unlock(
        &self,
        raw: &mut Self::Raw,
        key_hi: i64,
        key_lo: i64,
    ) -> Result<(), Self::Error> {
        let mut state = self.state.lock().expect("fake driver mutex poisoned");
        state.advisory_unlock_calls.push((raw.id, key_hi, key_lo));
        if state.fail_unlock_ids.remove(&raw.id) {
            return Err(format!("unlock failed for {}", raw.id));
        }
        Ok(())
    }

    fn close(&self, raw: Self::Raw) -> Result<(), Self::Error> {
        self.state
            .lock()
            .expect("fake driver mutex poisoned")
            .close_calls
            .push(raw.id);
        Ok(())
    }
}

#[test]
fn resolve_server_plane_runtime_validates_contract() {
    let (config, plane) = resolve_server_plane_runtime(
        "postgres",
        Some("postgresql://demo"),
        "ait_content",
        "ait_control",
        "content",
    )
    .expect("runtime should resolve");
    assert_eq!(plane.as_str(), "content");
    assert_eq!(config.schema_for(plane), "ait_content");
    assert_eq!(
        resolve_server_plane_runtime(
            "postgres",
            Some("postgresql://demo"),
            "ait_content",
            "ait_control",
            "unknown",
        )
        .expect_err("unknown plane should fail"),
        "Unknown plane: unknown"
    );
    assert_eq!(
        resolve_server_plane_runtime(
            "local-file",
            Some("postgresql://demo"),
            "ait_content",
            "ait_control",
            "content",
        )
        .expect_err("unsupported backend should fail"),
        "Unsupported AIT server database backend for content plane: 'local-file'. Only PostgreSQL is supported for ait-server runtime state."
    );
    assert_eq!(
        resolve_server_plane_runtime("postgres", None, "ait_content", "ait_control", "content")
            .expect_err("missing dsn should fail"),
        "PostgreSQL backend requested but AIT_NATIVE_SERVER_POSTGRES_DSN is not configured"
    );
    assert_eq!(
        resolve_server_plane_runtime(
            "postgres",
            Some("postgresql://demo"),
            "ait-content",
            "ait_control",
            "content",
        )
        .expect_err("invalid content schema should fail"),
        "Invalid schema name: ait-content"
    );
}

#[test]
fn native_driver_checkout_sql_configures_session_then_starts_transaction() {
    assert_eq!(
        configure_postgres_checkout_sql(
            "ait_content",
            true,
            &PostgresTimeoutScope {
                lock_timeout_ms: Some(0),
                statement_timeout_ms: Some(25),
            },
        ),
        vec![
            "create schema if not exists \"ait_content\"".to_string(),
            "set search_path to \"ait_content\", public".to_string(),
            "set lock_timeout = '1ms'".to_string(),
            "set statement_timeout = '25ms'".to_string(),
            "begin".to_string(),
        ]
    );
    assert_eq!(
        configure_postgres_checkout_sql("ait_control", false, &PostgresTimeoutScope::default()),
        vec![
            "set search_path to \"ait_control\", public".to_string(),
            "reset lock_timeout".to_string(),
            "reset statement_timeout".to_string(),
            "begin".to_string(),
        ]
    );
    assert_eq!(
        configure_postgres_session_sql("ait_control", false, &PostgresTimeoutScope::default()),
        vec![
            "set search_path to \"ait_control\", public".to_string(),
            "reset lock_timeout".to_string(),
            "reset statement_timeout".to_string(),
        ]
    );
}

#[test]
fn connect_server_plane_passes_plane_schema_and_timeouts() {
    let driver = Arc::new(FakeDriver::default());
    let registry = PostgresConnectionPoolRegistry::new(driver.clone(), 2);
    let mut conn = connect_server_plane(
        &registry,
        "postgres",
        Some("postgresql://demo"),
        "ait_content",
        "ait_control",
        "control",
        &PostgresTimeoutScope {
            lock_timeout_ms: Some(0),
            statement_timeout_ms: Some(25),
        },
    )
    .expect("connect_server_plane should succeed");
    assert_eq!(conn.raw().id, 1);
    conn.close();
    assert_eq!(
        driver.snapshot(),
        FakeDriverSnapshot {
            connect_calls: 1,
            commit_calls: vec![],
            rollback_calls: vec![1],
            configure_calls: vec![(1, "ait_control".to_string(), true, Some(0), Some(25))],
            configure_autocommit_calls: vec![],
            advisory_lock_calls: vec![],
            advisory_unlock_calls: vec![],
            close_calls: vec![],
        }
    );
}

#[test]
fn connect_postgres_runtime_rejects_invalid_schema_name() {
    let driver = Arc::new(FakeDriver::default());
    let registry = PostgresConnectionPoolRegistry::new(driver, 2);
    match connect_postgres_runtime(
        &registry,
        "postgresql://demo",
        "ait-content",
        &PostgresTimeoutScope::default(),
    ) {
        Err(error) => assert_eq!(error, "Invalid schema name: ait-content"),
        Ok(_) => panic!("invalid schema should fail"),
    }
}

#[test]
fn write_server_plane_commits_and_read_server_plane_rolls_back() {
    let driver = Arc::new(FakeDriver::default());
    let registry = PostgresConnectionPoolRegistry::new(driver.clone(), 1);

    let write_id = write_server_plane(
        &registry,
        "postgres",
        Some("postgresql://demo"),
        "ait_content",
        "ait_control",
        "content",
        &PostgresTimeoutScope::default(),
        |conn| Ok::<_, String>(conn.raw().id),
    )
    .expect("write_server_plane should succeed");
    assert_eq!(write_id, 1);
    assert_eq!(
        driver.snapshot(),
        FakeDriverSnapshot {
            connect_calls: 1,
            commit_calls: vec![1],
            rollback_calls: vec![1],
            configure_calls: vec![(1, "ait_content".to_string(), true, None, None)],
            configure_autocommit_calls: vec![],
            advisory_lock_calls: vec![],
            advisory_unlock_calls: vec![],
            close_calls: vec![],
        }
    );

    let read_id = read_server_plane(
        &registry,
        "postgres",
        Some("postgresql://demo"),
        "ait_content",
        "ait_control",
        "content",
        &PostgresTimeoutScope::default(),
        |conn| Ok::<_, String>(conn.raw().id),
    )
    .expect("read_server_plane should succeed");
    assert_eq!(read_id, 1);
    assert_eq!(
        driver.snapshot(),
        FakeDriverSnapshot {
            connect_calls: 1,
            commit_calls: vec![1],
            rollback_calls: vec![1, 1, 1],
            configure_calls: vec![
                (1, "ait_content".to_string(), true, None, None),
                (1, "ait_content".to_string(), false, None, None),
            ],
            configure_autocommit_calls: vec![],
            advisory_lock_calls: vec![],
            advisory_unlock_calls: vec![],
            close_calls: vec![],
        }
    );
}

#[test]
fn autocommit_read_server_plane_does_not_open_or_close_a_transaction() {
    let driver = Arc::new(FakeDriver::default());
    let registry = PostgresConnectionPoolRegistry::new(driver.clone(), 1);

    let read_id = read_server_plane_autocommit(
        &registry,
        "postgres",
        Some("postgresql://demo"),
        "ait_content",
        "ait_control",
        "control",
        &PostgresTimeoutScope::default(),
        |conn| {
            assert!(!conn.is_transactional());
            Ok::<_, String>(conn.raw().id)
        },
    )
    .expect("autocommit read should succeed");
    assert_eq!(read_id, 1);
    assert_eq!(
        driver.snapshot(),
        FakeDriverSnapshot {
            connect_calls: 1,
            commit_calls: vec![],
            rollback_calls: vec![],
            configure_calls: vec![],
            configure_autocommit_calls: vec![(1, "ait_control".to_string(), true, None, None,)],
            advisory_lock_calls: vec![],
            advisory_unlock_calls: vec![],
            close_calls: vec![],
        }
    );
}

#[test]
fn callback_error_discards_connection_when_close_rollback_fails() {
    let driver = Arc::new(FakeDriver::default());
    let registry = PostgresConnectionPoolRegistry::new(driver.clone(), 1);
    driver.fail_rollback_for(1);

    let result = read_server_plane(
        &registry,
        "postgres",
        Some("postgresql://demo"),
        "ait_content",
        "ait_control",
        "content",
        &PostgresTimeoutScope::default(),
        |_conn| Err::<(), _>("boom"),
    );
    assert_eq!(result.expect_err("callback error should surface"), "boom");
    assert_eq!(
        driver.snapshot(),
        FakeDriverSnapshot {
            connect_calls: 1,
            commit_calls: vec![],
            rollback_calls: vec![1],
            configure_calls: vec![(1, "ait_content".to_string(), true, None, None)],
            configure_autocommit_calls: vec![],
            advisory_lock_calls: vec![],
            advisory_unlock_calls: vec![],
            close_calls: vec![1],
        }
    );
}

#[test]
fn advisory_lock_wrapper_uses_key_and_ignores_unlock_failure() {
    let driver = Arc::new(FakeDriver::default());
    let registry = PostgresConnectionPoolRegistry::new(driver.clone(), 1);
    let mut conn = connect_postgres_runtime(
        &registry,
        "postgresql://demo",
        "ait_content",
        &PostgresTimeoutScope::default(),
    )
    .expect("connect_postgres_runtime should succeed");
    driver.fail_unlock_for(1);

    let result = with_postgres_advisory_lock(&mut conn, "repo:main", |locked| {
        Ok::<_, String>(locked.raw().id)
    })
    .expect("unlock failure should be ignored");
    assert_eq!(result, 1);
    conn.close();

    let snapshot = driver.snapshot();
    assert_eq!(snapshot.advisory_lock_calls.len(), 1);
    assert_eq!(snapshot.advisory_unlock_calls.len(), 1);
    assert_eq!(snapshot.advisory_lock_calls[0].0, 1);
    assert_eq!(
        snapshot.advisory_lock_calls[0].1,
        snapshot.advisory_unlock_calls[0].1
    );
    assert_eq!(
        snapshot.advisory_lock_calls[0].2,
        snapshot.advisory_unlock_calls[0].2
    );
}
