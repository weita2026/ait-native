#![allow(dead_code)]
use ait_core::json_support::{JsonCodec, JsonValue};
use assert_cmd::prelude::*;
use std::{fs, path::PathBuf, process::Command};
use tempfile::TempDir;
pub struct Fixture {
    pub temp: TempDir,
    pub repo: PathBuf,
    pub home: PathBuf,
}
impl Fixture {
    pub fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("native-agent-teams");
        let home = temp.path().join("native-home");
        fs::create_dir(&repo).unwrap();
        fs::create_dir(&home).unwrap();
        let f = Self { temp, repo, home };
        let init = f.run(&["init", "--json"], true);
        assert_eq!(init["agent_team_bootstrap"]["status"], "ready", "{init}");
        f
    }
    pub fn run(&self, args: &[&str], success: bool) -> JsonValue {
        let o = Command::cargo_bin("ait-cli")
            .unwrap()
            .current_dir(&self.repo)
            .env_remove("CODEX_THREAD_ID")
            .env_remove("CODEX_SESSION_ID")
            .env("CODEX_HOME", &self.home)
            .args(args)
            .output()
            .unwrap();
        assert_eq!(
            o.status.success(),
            success,
            "{} {}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        JsonCodec::parse_slice_with_error_prefix(&o.stdout, "Native CLI JSON").unwrap()
    }
}
