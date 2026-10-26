use super::*;

impl RepositoryPackInventory {
    pub fn new(repo_name: impl Into<String>) -> Self {
        Self {
            repo_name: repo_name.into(),
            ..Self::default()
        }
    }
}
