//! Non-persisted serving/conversion exclusions. Native records still prove
//! historical membership independently of this runtime selection.
use crate::environment_contract::names::AIT_NATIVE_SERVER_EXCLUDED_REPOSITORY_INDEXES;
use crate::foundation::server_operational_repository_registry::OperationalRepositoryEntry;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ServerRepositoryScope {
    excluded: BTreeSet<u32>,
}

impl ServerRepositoryScope {
    pub fn from_environment() -> Result<Self, String> {
        match std::env::var(AIT_NATIVE_SERVER_EXCLUDED_REPOSITORY_INDEXES) {
            Ok(value) => Self::parse(Some(&value)),
            Err(std::env::VarError::NotPresent) => Ok(Self::default()),
            Err(_) => Err(format!("{AIT_NATIVE_SERVER_EXCLUDED_REPOSITORY_INDEXES} must be UTF-8 canonical decimal indexes")),
        }
    }

    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        let mut scope = Self::default();
        let Some(value) = value else { return Ok(scope) };
        for token in value.split(',') {
            let index = token.parse::<u32>().ok().filter(|index| index.to_string() == token)
                .ok_or_else(|| format!("{AIT_NATIVE_SERVER_EXCLUDED_REPOSITORY_INDEXES} requires comma-separated canonical u32 indexes"))?;
            if !scope.excluded.insert(index) {
                return Err(format!("{AIT_NATIVE_SERVER_EXCLUDED_REPOSITORY_INDEXES} repeats Repository index {index}"));
            }
        }
        Ok(scope)
    }

    pub fn validate_known(&self, entries: &[OperationalRepositoryEntry]) -> Result<(), String> {
        let known = entries
            .iter()
            .map(|entry| entry.repository_index)
            .collect::<BTreeSet<_>>();
        if let Some(index) = self.excluded.difference(&known).next() {
            return Err(format!("{AIT_NATIVE_SERVER_EXCLUDED_REPOSITORY_INDEXES} names unknown Repository index {index}"));
        }
        Ok(())
    }

    pub fn excludes(&self, index: u32) -> bool {
        self.excluded.contains(&index)
    }

    pub fn has_exclusions(&self) -> bool {
        !self.excluded.is_empty()
    }

    pub fn includes(&self, entry: &OperationalRepositoryEntry) -> bool {
        entry.record.is_serving() && !self.excludes(entry.repository_index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_exclusion_configuration_has_no_implicit_default() {
        assert_eq!(
            ServerRepositoryScope::parse(None).unwrap(),
            ServerRepositoryScope::default()
        );
        let scope = ServerRepositoryScope::parse(Some("4294967295,0,4")).unwrap();
        assert!(scope.excludes(0) && scope.excludes(4) && scope.excludes(u32::MAX));
        assert!(!scope.excludes(1));
        for invalid in [
            "",
            " ",
            "04",
            "+4",
            "-1",
            "4,",
            ",4",
            "4,,5",
            "4, 5",
            "4,4",
            "4294967296",
            "x",
        ] {
            assert!(
                ServerRepositoryScope::parse(Some(invalid)).is_err(),
                "{invalid:?}"
            );
        }
    }
}
