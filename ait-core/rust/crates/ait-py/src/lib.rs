use pyo3::prelude::*;

mod exports;
mod json_support;

#[pymodule(name = "ait_py")]
fn ait_py(py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    exports::register(py, module)
}

#[cfg(test)]
mod dependency_security_tests {
    const MINIMUM_SAFE_PYO3: [u64; 3] = [0, 29, 0];

    fn parse_plain_version(raw: &str) -> [u64; 3] {
        let numeric = raw
            .split(['-', '+'])
            .next()
            .expect("version must contain a numeric component");
        assert_eq!(
            numeric, raw,
            "security floor must resolve to a stable PyO3 release"
        );
        let mut parts = numeric.split('.');
        let version = [
            parts
                .next()
                .expect("version must contain a major component")
                .parse()
                .expect("major version must be numeric"),
            parts
                .next()
                .expect("version must contain a minor component")
                .parse()
                .expect("minor version must be numeric"),
            parts
                .next()
                .expect("version must contain a patch component")
                .parse()
                .expect("patch version must be numeric"),
        ];
        assert!(parts.next().is_none(), "version must have three components");
        version
    }

    #[test]
    fn direct_pyo3_floor_excludes_vulnerable_releases() {
        let manifest = include_str!("../Cargo.toml");
        let line = manifest
            .lines()
            .map(str::trim)
            .find(|line| line.starts_with("pyo3 ="))
            .expect("ait-py must declare a direct pyo3 dependency");
        let lower_bound = line
            .split_once("version = \"")
            .and_then(|(_, rest)| rest.split_once('"'))
            .map(|(version, _)| version)
            .expect("pyo3 dependency must keep an explicit plain version floor");

        assert!(
            parse_plain_version(lower_bound) >= MINIMUM_SAFE_PYO3,
            "pyo3 dependency floor must remain at or above 0.29.0"
        );
    }

    #[test]
    fn locked_pyo3_versions_exclude_vulnerable_releases() {
        let lockfile = include_str!("../../../Cargo.lock");
        let locked_versions: Vec<_> = lockfile
            .split("[[package]]")
            .filter_map(|package| {
                let mut name = None;
                let mut version = None;
                for line in package.lines().map(str::trim) {
                    if let Some(value) = line.strip_prefix("name = \"") {
                        name = value.strip_suffix('"');
                    } else if let Some(value) = line.strip_prefix("version = \"") {
                        version = value.strip_suffix('"');
                    }
                }
                (name == Some("pyo3")).then(|| version.expect("pyo3 must have a locked version"))
            })
            .collect();

        assert!(!locked_versions.is_empty(), "Cargo.lock must contain pyo3");
        for version in locked_versions {
            assert!(
                parse_plain_version(version) >= MINIMUM_SAFE_PYO3,
                "locked pyo3 version {version} is vulnerable; require 0.29.0 or newer"
            );
        }
    }
}
