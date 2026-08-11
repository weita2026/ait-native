use super::*;

#[derive(Clone)]
pub struct PostgresConnectionPoolRegistry<D: PostgresPoolDriver> {
    driver: Arc<D>,
    max_size: usize,
    pools: Arc<Mutex<HashMap<(String, String), PostgresConnectionPool<D>>>>,
}

impl<D: PostgresPoolDriver> PostgresConnectionPoolRegistry<D> {
    pub fn new(driver: Arc<D>, max_size: usize) -> Self {
        Self {
            driver,
            max_size: max_size.max(1),
            pools: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn pool(&self, dsn: &str, schema: &str) -> PostgresConnectionPool<D> {
        let mut pools = self
            .pools
            .lock()
            .expect("postgres pool registry mutex poisoned");
        pools
            .entry((dsn.to_string(), schema.to_string()))
            .or_insert_with(|| {
                PostgresConnectionPool::new(
                    dsn.to_string(),
                    schema.to_string(),
                    self.max_size,
                    Arc::clone(&self.driver),
                )
            })
            .clone()
    }

    pub fn close_all(&self) {
        let pools = {
            let mut registry = self
                .pools
                .lock()
                .expect("postgres pool registry mutex poisoned");
            std::mem::take(&mut *registry)
        };
        for pool in pools.into_values() {
            pool.close();
        }
    }
}
