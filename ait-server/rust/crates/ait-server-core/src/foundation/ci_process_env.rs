use serde_json::{json, Value as JsonValue};
use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

pub(crate) const CI_PROCESS_ENVIRONMENT_POLICY: &str =
    "safe_ambient_allowlist_with_explicit_overrides";

const SAFE_AMBIENT_CI_ENV_NAMES: [&str; 45] = [
    "ANDROID_HOME",
    "ANDROID_SDK_ROOT",
    "BUNDLE_PATH",
    "CARGO_HOME",
    "COMPOSER_HOME",
    "DENO_DIR",
    "DOTNET_ROOT",
    "FLUTTER_ROOT",
    "GEM_HOME",
    "GEM_PATH",
    "GOCACHE",
    "GOENV",
    "GOMODCACHE",
    "GOPATH",
    "GOROOT",
    "GRADLE_HOME",
    "GRADLE_USER_HOME",
    "HOME",
    "JAVA_HOME",
    "JDK_HOME",
    "KOTLIN_HOME",
    "LANG",
    "LANGUAGE",
    "LC_ALL",
    "LC_CTYPE",
    "LOGNAME",
    "M2_HOME",
    "MAVEN_HOME",
    "NODE_PATH",
    "NPM_CONFIG_CACHE",
    "NUGET_PACKAGES",
    "PATH",
    "PNPM_HOME",
    "RUSTUP_HOME",
    "SDKROOT",
    "SHELL",
    "SSL_CERT_DIR",
    "SSL_CERT_FILE",
    "SWIFT_EXEC",
    "TEMP",
    "TMP",
    "TMPDIR",
    "TZ",
    "USER",
    "YARN_CACHE_FOLDER",
];

pub(crate) fn clean_ci_process_env(
    explicit: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    clean_ci_process_env_from(env::vars(), explicit)
}

pub(crate) fn apply_clean_ci_process_env(
    command: &mut Command,
    environment: &BTreeMap<String, String>,
) {
    command.env_clear();
    command.envs(environment);
}

pub(crate) fn ci_process_environment_report() -> JsonValue {
    json!({
        "policy": CI_PROCESS_ENVIRONMENT_POLICY,
        "ambient_inheritance": "allowlist",
        "explicit_env_overrides": true,
        "ambient_values_reported": false,
        "ambient_secret_forwarding": false,
    })
}

fn clean_ci_process_env_from<I>(
    ambient: I,
    explicit: &BTreeMap<String, String>,
) -> BTreeMap<String, String>
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut environment = ambient
        .into_iter()
        .filter(|(name, _)| SAFE_AMBIENT_CI_ENV_NAMES.contains(&name.as_str()))
        .collect::<BTreeMap<_, _>>();
    add_user_cargo_bin_to_path(&mut environment);
    environment.extend(explicit.clone());
    environment
}

fn add_user_cargo_bin_to_path(environment: &mut BTreeMap<String, String>) {
    let Some(home) = environment.get("HOME") else {
        return;
    };
    let cargo_bin = PathBuf::from(home).join(".cargo").join("bin");
    if !cargo_bin.is_dir() {
        return;
    }
    let current = environment
        .get("PATH")
        .map(OsString::from)
        .unwrap_or_default();
    let mut paths = env::split_paths(&current).collect::<Vec<_>>();
    if paths.iter().any(|path| path == &cargo_bin) {
        return;
    }
    paths.insert(0, cargo_bin);
    if let Ok(path) = env::join_paths(paths) {
        environment.insert("PATH".to_string(), path.to_string_lossy().into_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ambient_fixture() -> Vec<(String, String)> {
        [
            ("PATH", "/usr/local/bin:/usr/bin"),
            ("HOME", "/srv/ci"),
            ("LANG", "en_US.UTF-8"),
            ("JAVA_HOME", "/opt/jdk"),
            ("GOMODCACHE", "/cache/go-mod"),
            ("NPM_CONFIG_CACHE", "/cache/npm"),
            ("AWS_SECRET_ACCESS_KEY", "ambient-aws-secret"),
            ("GITHUB_TOKEN", "ambient-github-token"),
            ("BASH_ENV", "/srv/credential-loader"),
            ("PYTHONPATH", "/srv/server-python"),
            ("DATABASE_URL", "postgres://server-secret"),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
    }

    #[test]
    fn clean_environment_inherits_language_baseline_without_ambient_secrets() {
        let environment =
            clean_ci_process_env_from(ambient_fixture(), &BTreeMap::<String, String>::new());

        assert_eq!(
            environment.get("PATH").map(String::as_str),
            Some("/usr/local/bin:/usr/bin")
        );
        assert_eq!(
            environment.get("JAVA_HOME").map(String::as_str),
            Some("/opt/jdk")
        );
        assert_eq!(
            environment.get("GOMODCACHE").map(String::as_str),
            Some("/cache/go-mod")
        );
        assert_eq!(
            environment.get("NPM_CONFIG_CACHE").map(String::as_str),
            Some("/cache/npm")
        );
        for forbidden in [
            "AWS_SECRET_ACCESS_KEY",
            "GITHUB_TOKEN",
            "BASH_ENV",
            "PYTHONPATH",
            "DATABASE_URL",
        ] {
            assert!(!environment.contains_key(forbidden), "{forbidden}");
        }
    }

    #[test]
    fn explicit_environment_is_the_only_escape_hatch_and_overrides_baseline() {
        let explicit = BTreeMap::from([
            ("PATH".to_string(), "/workspace/toolchain/bin".to_string()),
            (
                "CUSTOM_LANGUAGE_HOME".to_string(),
                "/workspace/custom".to_string(),
            ),
            (
                "GITHUB_TOKEN".to_string(),
                "authorized-explicit-token".to_string(),
            ),
        ]);
        let environment = clean_ci_process_env_from(ambient_fixture(), &explicit);

        assert_eq!(
            environment.get("PATH").map(String::as_str),
            Some("/workspace/toolchain/bin")
        );
        assert_eq!(
            environment.get("CUSTOM_LANGUAGE_HOME").map(String::as_str),
            Some("/workspace/custom")
        );
        assert_eq!(
            environment.get("GITHUB_TOKEN").map(String::as_str),
            Some("authorized-explicit-token")
        );
        assert!(!environment.contains_key("AWS_SECRET_ACCESS_KEY"));
    }

    #[test]
    fn environment_report_discloses_policy_but_no_names_or_values() {
        let report = ci_process_environment_report();
        let text = report.to_string();

        assert_eq!(report["ambient_inheritance"], json!("allowlist"));
        assert_eq!(report["ambient_values_reported"], json!(false));
        assert!(!text.contains("GITHUB_TOKEN"));
        assert!(!text.contains("JAVA_HOME"));
        assert!(!text.contains("authorized-explicit-token"));
    }
}
