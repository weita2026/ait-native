use super::*;

pub(super) struct PostgresPatchsetStore {
    pub(super) client: Client,
    pub(super) content_schema: String,
    pub(super) control_schema: String,
    pub(super) root: Option<PathBuf>,
}

impl PostgresPatchsetStore {
    pub(super) fn connect(runtime: PatchsetStoreRuntime) -> Result<Self, String> {
        let client = Client::connect(&runtime.dsn, NoTls).map_err(|exc| exc.to_string())?;
        Ok(Self {
            client,
            content_schema: runtime.content_schema,
            control_schema: runtime.control_schema,
            root: runtime.root,
        })
    }
    pub(super) fn transaction<F>(&mut self, f: F) -> Result<JsonMap<String, JsonValue>, String>
    where
        F: FnOnce(&mut Self) -> Result<JsonMap<String, JsonValue>, String>,
    {
        self.client
            .batch_execute("begin")
            .map_err(|exc| exc.to_string())?;
        let result = f(self);
        match result {
            Ok(value) => {
                self.client
                    .batch_execute("commit")
                    .map_err(|exc| exc.to_string())?;
                Ok(value)
            }
            Err(err) => {
                let _ = self.client.batch_execute("rollback");
                Err(err)
            }
        }
    }
    pub(super) fn content_table(&self, name: &str) -> String {
        schema_table(&self.content_schema, name)
    }
    pub(super) fn control_table(&self, name: &str) -> String {
        schema_table(&self.control_schema, name)
    }
}
