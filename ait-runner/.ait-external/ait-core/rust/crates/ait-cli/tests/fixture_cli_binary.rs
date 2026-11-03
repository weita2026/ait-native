#[path = "support/cli_binary.rs"]
mod cli_binary;
#[path = "support/native_home.rs"]
mod native_home;
#[path = "support/workflow_package_fixture.rs"]
mod package_fixture;
#[path = "support/workflow_process.rs"]
mod workflow_process;

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

const PROBE: &str = ".cli-selection-probe";

fn probe(root: &Path, supplied: Option<&OsStr>, package: bool) -> Output {
    fs::write(
        root.join(PROBE),
        if package { "package" } else { "process" },
    )
    .unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["cli_selection_probe", "--exact", "--nocapture"])
        .current_dir(root);
    if let Some(value) = supplied {
        command.env("AIT_TEST_CLI_BIN", value);
    } else {
        command.env_remove("AIT_TEST_CLI_BIN");
    }
    if package {
        let not_a_directory = root.join("invalid-temp-root");
        fs::write(&not_a_directory, "not a directory").unwrap();
        command
            .env("TMPDIR", &not_a_directory)
            .env("TMP", &not_a_directory)
            .env("TEMP", &not_a_directory);
    }
    command.output().unwrap()
}

#[test]
fn cli_selection_probe() {
    let root = std::env::current_dir().unwrap();
    let Ok(mode) = fs::read_to_string(root.join(PROBE)) else {
        return;
    };
    if mode == "package" {
        let _fixture = package_fixture::Fixture::new();
    } else {
        println!(
            "selected={}",
            cli_binary::selected(env!("CARGO_BIN_EXE_ait-cli")).display()
        );
        let output = workflow_process::cli(&root, &["init", "--json"], None);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn installed_cli_selection_preserves_unset_and_runs_the_supplied_executable() {
    let cargo = Path::new(env!("CARGO_BIN_EXE_ait-cli"));
    assert_eq!(cli_binary::select(cargo, None).unwrap(), cargo);
    let temp = tempfile::tempdir().unwrap();
    let supplied = temp.path().join(if cfg!(windows) {
        "installed-cli.exe"
    } else {
        "installed-cli"
    });
    fs::copy(cargo, &supplied).unwrap();
    assert_eq!(
        cli_binary::select(cargo, Some(supplied.as_os_str())).unwrap(),
        supplied
    );
    for input in [None, Some(supplied.as_os_str())] {
        let root = tempfile::tempdir().unwrap();
        let output = probe(root.path(), input, false);
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let selected = input.map(Path::new).unwrap_or(cargo);
        assert!(String::from_utf8_lossy(&output.stdout)
            .contains(&format!("selected={}", selected.display())));
        assert!(root.path().join(".ait").is_dir());
    }
}

#[test]
fn invalid_installed_cli_inputs_reject_before_mutation_without_fallback() {
    let fixtures = tempfile::tempdir().unwrap();
    let non_executable = fixtures.path().join("not-executable.exe");
    fs::write(&non_executable, "not an executable image").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&non_executable, fs::Permissions::from_mode(0o644)).unwrap();
    }
    let inputs = [
        OsString::new(),
        OsString::from("relative-cli"),
        fixtures.path().join("missing-cli").into_os_string(),
        fixtures.path().as_os_str().to_os_string(),
        non_executable.into_os_string(),
    ];
    for input in inputs {
        let root = tempfile::tempdir().unwrap();
        let output = probe(root.path(), Some(&input), false);
        assert!(!output.status.success(), "{input:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("AIT_TEST_CLI_BIN"),
            "{input:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !root.path().join(".ait").exists(),
            "invalid input fell back: {input:?}"
        );
        assert_eq!(
            fs::read_to_string(root.path().join(PROBE)).unwrap(),
            "process"
        );
    }
}

#[test]
fn invalid_installed_cli_is_checked_before_package_fixture_authoring() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing-cli");
    let output = probe(root.path(), Some(missing.as_os_str()), true);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("AIT_TEST_CLI_BIN"),
        "package fixture attempted setup before selection: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!root.path().join(".ait").exists());
}
