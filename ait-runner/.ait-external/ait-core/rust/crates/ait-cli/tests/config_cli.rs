#[path = "support/native_home.rs"]
mod native_home;
#[cfg(unix)]
#[path = "support/agent_teams_cli.rs"]
mod team_fixture;
use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::process::Command;
use tempfile::TempDir;

fn cargo_bin() -> Command {
    let mut c = Command::cargo_bin("ait-cli").unwrap();
    c.env("CODEX_HOME", native_home::path());
    c
}

#[test]
fn plan_language_and_style_cli_keep_both_agent_contracts_english() {
    let temp = TempDir::new().unwrap();
    cargo_bin()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    for language in ["zh-Hant-TW", "zh-CN", "en", "ja-JP", "ko-KR"] {
        cargo_bin()
            .current_dir(temp.path())
            .args([
                "config",
                "set",
                "--plan-language",
                language,
                "--plan-style",
                "concise",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains(format!(
                "plan-language: {language}"
            )));
        for name in ["AGENTS.md", "CLAUDE.md"] {
            let text = std::fs::read_to_string(temp.path().join(name)).unwrap();
            assert!(text.is_ascii(), "{language}/{name}");
            assert!(text.contains(&format!("language=`{language}` (BCP 47); style=`concise`")));
            assert!(text.contains("acceptance criteria, and verification"));
        }
    }
    cargo_bin()
        .current_dir(temp.path())
        .args(["config", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "plan_language: ko-KR (repo_config)",
        ))
        .stdout(predicate::str::contains(
            "plan_style: concise (repo_config)",
        ));
    cargo_bin()
        .current_dir(temp.path())
        .args(["config", "set", "--plan-style", "detailed"])
        .assert()
        .success();
    for key in ["plan-language", "plan-style"] {
        cargo_bin()
            .current_dir(temp.path())
            .args(["config", "unset", key])
            .assert()
            .success();
    }
    cargo_bin()
        .current_dir(temp.path())
        .args(["config", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("plan_language: en (built_in)"))
        .stdout(predicate::str::contains("plan_style: concise (built_in)"));
    cargo_bin()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("plan=en/concise"));
}

#[test]
fn invalid_plan_preference_cli_options_preserve_files() {
    let temp = TempDir::new().unwrap();
    cargo_bin()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    let paths = [".ait/config.json", "AGENTS.md", "CLAUDE.md"];
    let before = paths.map(|path| std::fs::read(temp.path().join(path)).unwrap());
    for args in [
        vec!["--plan-language", "zh_TW.UTF-8"],
        vec!["--plan-language", "ja-JP", "--plan-style", "verbose"],
        vec!["--plan-language", ""],
        vec!["--plan-language", "en", "--plan-style", "Concise"],
    ] {
        cargo_bin()
            .current_dir(temp.path())
            .args(["config", "set"])
            .args(args)
            .assert()
            .failure();
        assert_eq!(
            paths.map(|path| std::fs::read(temp.path().join(path)).unwrap()),
            before
        );
    }
}

#[test]
fn compact_config_show_exposes_automatic_task_review_policy_and_reviewer() {
    let temp = TempDir::new().unwrap();
    cargo_bin()
        .current_dir(temp.path())
        .args(["init", "--json"])
        .assert()
        .success();
    cargo_bin()
        .current_dir(temp.path())
        .args(["config", "set", "--user-name", "Alice Example"])
        .assert()
        .success();

    cargo_bin()
        .current_dir(temp.path())
        .args(["config", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("task_review: automatic"))
        .stdout(predicate::str::contains("task_review_source: built_in"))
        .stdout(predicate::str::contains(
            "automatic_task_reviewer: Alice Example",
        ));
}

#[test]
fn compact_config_show_exposes_missing_automatic_task_reviewer() {
    let temp = TempDir::new().unwrap();
    cargo_bin()
        .current_dir(temp.path())
        .args(["init", "--json"])
        .assert()
        .success();

    cargo_bin()
        .current_dir(temp.path())
        .args(["config", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("task_review: automatic"))
        .stdout(predicate::str::contains("automatic_task_reviewer: <unset>"));
}

#[test]
fn config_show_json_keeps_required_mode_explicit_without_automatic_reviewer() {
    let temp = TempDir::new().unwrap();
    cargo_bin()
        .current_dir(temp.path())
        .args(["init", "--json"])
        .assert()
        .success();
    cargo_bin()
        .current_dir(temp.path())
        .args([
            "config",
            "set",
            "--user-name",
            "Alice Example",
            "--task-review",
            "required",
        ])
        .assert()
        .success();

    cargo_bin()
        .current_dir(temp.path())
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"value\": \"required\""))
        .stdout(predicate::str::contains("\"automatic_reviewer\": null"));
}

#[test]
fn init_and_config_regenerate_stale_identity_reminders_without_configuring_identity() {
    use std::fs;
    for policy in ["automatic", "required"] {
        #[cfg(unix)]
        let temp = team_fixture::fixture();
        #[cfg(not(unix))]
        let temp = {
            let temp = TempDir::new().unwrap();
            fs::create_dir(temp.path().join("native-home")).unwrap();
            temp
        };
        let run_fixture = |args: &[&str]| {
            #[cfg(unix)]
            {
                team_fixture::run(&temp, args, true)
            }
            #[cfg(not(unix))]
            {
                let output = cargo_bin()
                    .current_dir(temp.path())
                    .args(args)
                    .env_remove("CODEX_THREAD_ID")
                    .env_remove("CODEX_SESSION_ID")
                    .env("CODEX_HOME", temp.path().join("native-home"))
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{} {}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                ait_core::json_support::JsonCodec::parse_slice_with_error_prefix(
                    &output.stdout,
                    "CLI JSON",
                )
                .unwrap()
            }
        };
        #[cfg(not(unix))]
        {
            // Preserve config regeneration coverage without the POSIX RPC mock.
            run_fixture(&["init", "--json"]);
            run_fixture(&["config", "set", "--agent-teams", "disabled", "--json"]);
        }
        run_fixture(&["config", "set", "--task-review", policy, "--json"]);
        for command in [
            &["init", "--json"][..],
            &["config", "set", "--plan-style", "concise", "--json"][..],
        ] {
            for name in ["AGENTS.md", "CLAUDE.md"] {
                // Seed old generated output in this isolated fixture. Only the
                // native init/config convergence removes the obsolete reminder.
                let current = fs::read_to_string(temp.path().join(name)).unwrap();
                let stale=current.replacen("### Code-change path",&format!("Action required before mutation:\n\n- review=`{policy}`; reviewer=`unset` (configure user-name)\n\n### Code-change path"),1);
                assert!(stale.contains("configure user-name"));
                fs::write(
                    temp.path().join(name),
                    stale + "\nRepository-specific guidance is preserved.\n",
                )
                .unwrap();
            }
            run_fixture(command);
            for name in ["AGENTS.md", "CLAUDE.md"] {
                let updated = fs::read_to_string(temp.path().join(name)).unwrap();
                assert!(
                    updated.contains("Admission: ready."),
                    "{policy}/{name}: {updated}"
                );
                assert!(
                    !updated.contains("Action required before mutation:"),
                    "{policy}/{name}: {updated}"
                );
                assert!(
                    !updated.contains("configure user-name"),
                    "{policy}/{name}: {updated}"
                );
                assert!(
                    !updated.contains("reviewer=`unset`"),
                    "{policy}/{name}: {updated}"
                );
                assert!(updated.contains("Repository-specific guidance is preserved."));
            }
            let shown = run_fixture(&["config", "show", "--json"]);
            assert_eq!(shown["task_review"]["value"], policy);
            assert!(shown["user_name"].is_null());
            assert!(shown["user_email"].is_null());
        }
    }
}
