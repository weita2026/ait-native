pub type LineStoreResult<T> = Result<T, String>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineRecord {
    pub line_id: String,
    pub line_name: String,
    pub status: String,
    pub archived_at: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub head_snapshot_id: Option<String>,
}

pub trait LineStore {
    fn list_lines(&self) -> LineStoreResult<Vec<LineRecord>>;
    fn line_count(&self) -> LineStoreResult<usize> {
        self.list_lines().map(|lines| lines.len())
    }
    fn line_by_name(&self, line_name: &str) -> LineStoreResult<Option<LineRecord>>;
    fn create_line(
        &self,
        line_name: &str,
        head_snapshot_id: Option<&str>,
        created_at: &str,
    ) -> LineStoreResult<LineRecord>;
    fn archive_line(&self, line_name: &str, archived_at: &str) -> LineStoreResult<LineRecord>;
    fn rename_line(
        &self,
        old_line_name: &str,
        new_line_name: &str,
        updated_at: &str,
    ) -> LineStoreResult<LineRecord> {
        let _ = (old_line_name, new_line_name, updated_at);
        Err("Line rename is not supported by this store".to_string())
    }
    fn delete_line(&self, line_name: &str, deleted_at: &str) -> LineStoreResult<LineRecord> {
        let _ = (line_name, deleted_at);
        Err("Line delete is not supported by this store".to_string())
    }
    fn set_line_head(
        &self,
        line_name: &str,
        head_snapshot_id: Option<&str>,
        updated_at: &str,
    ) -> LineStoreResult<LineRecord>;
    fn compare_and_swap_line_head(
        &self,
        line_name: &str,
        expected_head_snapshot_id: Option<&str>,
        head_snapshot_id: Option<&str>,
        updated_at: &str,
    ) -> LineStoreResult<LineRecord> {
        let current = self
            .line_by_name(line_name)?
            .ok_or_else(|| format!("Unknown line: {line_name}"))?;
        if current.head_snapshot_id.as_deref() != expected_head_snapshot_id {
            return Err(format!(
                "Line {line_name} compare-and-swap expected head {} but found {}.",
                expected_head_snapshot_id.unwrap_or("none"),
                current.head_snapshot_id.as_deref().unwrap_or("none")
            ));
        }
        self.set_line_head(line_name, head_snapshot_id, updated_at)
    }
    fn line_updated_at(&self, line_name: &str) -> LineStoreResult<Option<String>>;
    fn set_line_updated_at(&self, line_name: &str, updated_at: Option<&str>)
        -> LineStoreResult<()>;
    fn touch_line_updated_at(&self, line_name: &str, updated_at: &str) -> LineStoreResult<()>;
}

pub fn line_by_name_with_line_store<S>(
    store: &S,
    line_name: &str,
) -> LineStoreResult<Option<LineRecord>>
where
    S: LineStore + ?Sized,
{
    store.line_by_name(line_name)
}
