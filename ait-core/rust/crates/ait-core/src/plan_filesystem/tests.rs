use super::*;
use crate::external::lockfile::{ExternalLockCodec, ExternalLockfile, TomlExternalLockCodec};
use crate::external::manifest::{
    ExternalDeclaration, ExternalManifest, ExternalManifestCodec, TomlExternalManifestCodec,
};
use crate::external::materializer::{
    ExternalMaterializationOptions, ExternalMaterializer, FilesystemExternalMaterializer,
    FixtureExternalContentSource,
};
use crate::file_io::FileIoResult;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::{Cursor, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Default)]
struct FakePlanFilesystemFileIoStore {
    files: RefCell<BTreeMap<PathBuf, Vec<u8>>>,
    reads: RefCell<Vec<PathBuf>>,
}

impl FakePlanFilesystemFileIoStore {
    fn insert_file(&self, path: &str, bytes: &[u8]) {
        self.files
            .borrow_mut()
            .insert(PathBuf::from(path), bytes.to_vec());
    }
}

impl FileIoStore for FakePlanFilesystemFileIoStore {
    fn home_dir(&self) -> Option<PathBuf> {
        Some(PathBuf::from("/home/ait"))
    }

    fn path_exists(&self, path: &Path) -> bool {
        self.files.borrow().contains_key(path)
    }

    fn read_bytes(&self, path: &Path) -> FileIoResult<Vec<u8>> {
        self.reads.borrow_mut().push(path.to_path_buf());
        self.files
            .borrow()
            .get(path)
            .cloned()
            .ok_or_else(|| FileIoError::new(FileIoErrorKind::NotFound, "missing fake file"))
    }

    fn read_to_string(&self, path: &Path) -> FileIoResult<String> {
        let bytes = self.read_bytes(path)?;
        String::from_utf8(bytes)
            .map_err(|err| FileIoError::new(FileIoErrorKind::Utf8, err.to_string()))
    }

    fn write_string(&self, _path: &Path, _text: &str) -> FileIoResult<()> {
        Ok(())
    }

    fn write_string_atomically(
        &self,
        _path: &Path,
        _text: &str,
        _publish_label: &str,
    ) -> FileIoResult<()> {
        Ok(())
    }
}

fn temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("ait-{}-{}", prefix, nanos));
    fs::create_dir_all(&path).unwrap();
    path
}

fn direct_external_declaration(
    name: &str,
    repo_name: &str,
    snapshot: &str,
    materialize_to: &str,
) -> ExternalDeclaration {
    ExternalDeclaration {
        name: name.to_string(),
        repo_name: repo_name.to_string(),
        repository_index: 0,
        remote: "origin".to_string(),
        line: "main".to_string(),
        snapshot: snapshot.to_string(),
        materialize_to: materialize_to.to_string(),
        license: "Apache-2.0".to_string(),
        version: None,
        bindings: Default::default(),
    }
}

fn materialize_direct_external_fixture(root: &Path, materialize_to: &str) -> ExternalLockfile {
    let manifest = ExternalManifest {
        externals: vec![direct_external_declaration(
            "ait-db",
            "ait-db",
            "SNP-DB-DIRECT",
            materialize_to,
        )],
    };
    let lockfile = ExternalLockfile::direct_manifest_lock(&manifest).unwrap();
    fs::write(
        root.join("ait-external.toml"),
        TomlExternalManifestCodec
            .render_manifest(&manifest)
            .unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("ait-external.lock"),
        TomlExternalLockCodec.render_lockfile(&lockfile).unwrap(),
    )
    .unwrap();
    FilesystemExternalMaterializer::new(root, FixtureExternalContentSource)
        .unwrap()
        .materialize_lockfile(&lockfile, &ExternalMaterializationOptions::recursive())
        .unwrap();
    lockfile
}

#[test]
fn markdown_path_helpers_match_expected_shape() {
    assert_eq!(
        normalize_markdown_artifact_path(r"docs\plan.md"),
        "docs/plan.md"
    );
    assert!(is_markdown_artifact_path("docs/PLAN.MD"));
    assert!(is_lineage_only_markdown_artifact_path("docs/plan.md"));
    assert!(is_lineage_only_markdown_artifact_path("README.md"));
    assert!(is_lineage_only_markdown_artifact_path(
        "release/guides/LOCAL_QUICKSTART.md"
    ));
    assert!(path_is_projected_out_for_workspace(
        ".",
        "docs/plan.md",
        false
    ));
    assert!(path_is_projected_out_for_workspace(".", "docs", true));
    assert!(path_is_projected_out_for_workspace(".", "README.md", false));
    assert!(path_is_projected_out_for_workspace(
        ".",
        "release/guides/LOCAL_QUICKSTART.md",
        false
    ));
    assert!(path_is_projected_out_for_workspace(
        ".",
        "docs/sprints/card.task_graph.json",
        false
    ));
    assert!(path_is_projected_out_for_workspace(
        ".",
        "docs/sprints/card.task_graph.json",
        true
    ));
}

#[test]
fn workspace_ignore_and_visible_paths_follow_expected_rules() {
    let root = temp_dir("plan-filesystem-visible");
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::create_dir_all(root.join("generated")).unwrap();
    fs::create_dir_all(root.join(".ait-runtime")).unwrap();
    fs::write(root.join(".aitignore"), "generated/\n").unwrap();
    fs::write(root.join("docs/plan.md"), "# Plan\n").unwrap();
    fs::write(root.join("generated/skip.md"), "skip\n").unwrap();
    fs::write(root.join(".ait-runtime/runtime.txt"), "runtime\n").unwrap();
    fs::write(root.join("alpha.txt"), "alpha\n").unwrap();

    let visible = list_visible_workspace_paths(
        root.to_str().unwrap(),
        None,
        Some(root.join(".ait-runtime").to_str().unwrap()),
    )
    .unwrap();
    assert_eq!(visible, vec![".aitignore", "alpha.txt", "docs/plan.md"]);
    let markdown = list_visible_markdown_artifact_paths(
        root.to_str().unwrap(),
        None,
        Some(root.join(".ait-runtime").to_str().unwrap()),
    )
    .unwrap();
    assert_eq!(markdown, vec!["docs/plan.md"]);
    assert!(workspace_path_is_ignored(root.to_str().unwrap(), "generated/skip.md", None).unwrap());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workspace_ignore_directory_rules_prune_directory_entries() {
    let root = temp_dir("plan-filesystem-ignore-dir-prune");
    fs::create_dir_all(root.join("generated/nested")).unwrap();
    fs::write(root.join(".aitignore"), "generated/\n").unwrap();
    fs::write(root.join("generated/nested/skip.txt"), "skip\n").unwrap();
    fs::write(root.join("keep.txt"), "keep\n").unwrap();

    let visible = list_visible_workspace_entries(root.to_str().unwrap(), None, None).unwrap();
    assert_eq!(visible.files, vec![".aitignore", "keep.txt"]);
    assert!(!visible.directories.iter().any(|path| path == "generated"));
    assert!(!visible
        .directories
        .iter()
        .any(|path| path.starts_with("generated/")));
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn visible_workspace_ignores_operational_directory_symlinks_but_keeps_user_symlinks() {
    use std::os::unix::fs::symlink;

    let root = temp_dir("plan-filesystem-operational-symlink");
    let metadata_root = temp_dir("plan-filesystem-shared-ait-metadata");
    fs::write(metadata_root.join("runtime.json"), "{}\n").unwrap();
    symlink(&metadata_root, root.join(".ait")).unwrap();
    fs::write(root.join("target.txt"), "tracked\n").unwrap();
    symlink("target.txt", root.join("tracked.link")).unwrap();

    let visible = list_visible_workspace_entries(root.to_str().unwrap(), None, None).unwrap();
    assert_eq!(visible.files, vec!["target.txt", "tracked.link"]);

    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(metadata_root).unwrap();
}

#[test]
fn workspace_ignore_directory_negation_keeps_descendant_file() {
    let root = temp_dir("plan-filesystem-ignore-dir-negation");
    fs::create_dir_all(root.join("generated")).unwrap();
    fs::write(root.join(".aitignore"), "generated/\n!generated/keep.md\n").unwrap();
    fs::write(root.join("generated/skip.md"), "skip\n").unwrap();
    fs::write(root.join("generated/keep.md"), "keep\n").unwrap();
    fs::write(root.join("README.md"), "readme\n").unwrap();

    let visible = list_visible_workspace_paths(root.to_str().unwrap(), None, None).unwrap();
    assert_eq!(
        visible,
        vec![".aitignore", "README.md", "generated/keep.md"]
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn generated_worktree_cargo_config_stays_out_of_visible_workspace_paths() {
    let root = temp_dir("plan-filesystem-worktree-cargo-config");
    fs::create_dir_all(root.join(".ait")).unwrap();
    fs::create_dir_all(root.join(".cargo")).unwrap();
    fs::write(root.join(WORKTREE_CONFIG_NAME), "{}\n").unwrap();
    fs::write(root.join("README.md"), "base\n").unwrap();
    fs::write(
        root.join(WORKTREE_CARGO_CONFIG_RELATIVE_PATH),
        generated_worktree_cargo_config_text(&root),
    )
    .unwrap();

    let visible = list_visible_workspace_paths(root.to_str().unwrap(), None, None).unwrap();
    assert_eq!(visible, vec!["README.md"]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn exact_task_cargo_config_with_aliases_stays_out_of_visible_workspace_paths() {
    let root = temp_dir("plan-filesystem-exact-task-cargo-config");
    fs::create_dir_all(root.join(".ait")).unwrap();
    fs::create_dir_all(root.join(".cargo")).unwrap();
    fs::write(
        root.join(WORKTREE_CONFIG_NAME),
        r#"{"worktree_name":"task-one"}"#,
    )
    .unwrap();
    fs::write(root.join("README.md"), "base\n").unwrap();
    let mut generated = generated_worktree_cargo_config_text(&root);
    generated.push_str("\n[alias]\nmanaged-test = [\"test\", \"--profile\", \"ait-ci\"]\n");
    fs::write(root.join(WORKTREE_CARGO_CONFIG_RELATIVE_PATH), generated).unwrap();

    assert!(worktree_cargo_build_dir(&root).ends_with("task-workspaces/task-one"));
    assert!(worktree_cargo_target_dir(&root).ends_with("task-workspaces/task-one"));
    let visible = list_visible_workspace_paths(root.to_str().unwrap(), None, None).unwrap();
    assert_eq!(visible, vec!["README.md"]);

    fs::write(
        root.join(WORKTREE_CARGO_CONFIG_RELATIVE_PATH),
        format!(
            "{SHARED_FINAL_ARTIFACT_GENERATED_CARGO_CONFIG_HEADER}\n[build]\ntarget-dir = \".ait/cargo-target\"\nbuild-dir = {}\n",
            encoded_cargo_path(&worktree_cargo_build_dir(&root))
        ),
    )
    .unwrap();
    let legacy_visible = list_visible_workspace_paths(root.to_str().unwrap(), None, None).unwrap();
    assert_eq!(legacy_visible, vec!["README.md"]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn canonical_source_cargo_config_remains_visible_in_a_worktree() {
    let root = temp_dir("plan-filesystem-source-cargo-config");
    fs::create_dir_all(root.join(".ait")).unwrap();
    fs::create_dir_all(root.join(".cargo")).unwrap();
    fs::write(
        root.join(WORKTREE_CONFIG_NAME),
        r#"{"worktree_name":"task-one"}"#,
    )
    .unwrap();
    fs::write(
        root.join(WORKTREE_CARGO_CONFIG_RELATIVE_PATH),
        "# AIT source policy: canonical Cargo settings; task worktrees receive a managed projection.\n[build]\ntarget-dir = \".ait/cargo-target\"\nbuild-dir = \".ait/cargo-build/canonical\"\n",
    )
    .unwrap();

    let visible = list_visible_workspace_paths(root.to_str().unwrap(), None, None).unwrap();
    assert_eq!(
        visible,
        vec![WORKTREE_CARGO_CONFIG_RELATIVE_PATH.to_string()]
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn repository_shared_worktree_cargo_config_stays_out_of_visible_workspace_paths() {
    let root = temp_dir("plan-filesystem-repository-shared-cargo-config");
    fs::create_dir_all(root.join(".ait")).unwrap();
    fs::create_dir_all(root.join(".cargo")).unwrap();
    fs::write(root.join(WORKTREE_CONFIG_NAME), "{}\n").unwrap();
    fs::write(root.join("README.md"), "base\n").unwrap();
    let shared_ait_dir = fs::canonicalize(root.join(".ait")).unwrap();
    fs::write(
        root.join(WORKTREE_CARGO_CONFIG_RELATIVE_PATH),
        format!(
            "{REPOSITORY_SHARED_GENERATED_CARGO_CONFIG_HEADER}\n[build]\ntarget-dir = {}\nbuild-dir = {}\n",
            encoded_cargo_path(&shared_ait_dir.join(SHARED_CARGO_TARGET_DIRNAME)),
            encoded_cargo_path(&shared_ait_dir.join(SHARED_CARGO_BUILD_DIRNAME)),
        ),
    )
    .unwrap();

    let visible = list_visible_workspace_paths(root.to_str().unwrap(), None, None).unwrap();
    assert_eq!(visible, vec!["README.md"]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn authored_cargo_config_with_managed_prefix_remains_visible_when_paths_do_not_match() {
    let root = temp_dir("plan-filesystem-authored-cargo-config");
    fs::create_dir_all(root.join(".ait")).unwrap();
    fs::create_dir_all(root.join(".cargo")).unwrap();
    fs::write(root.join(WORKTREE_CONFIG_NAME), "{}\n").unwrap();
    fs::write(
        root.join(WORKTREE_CARGO_CONFIG_RELATIVE_PATH),
        format!(
            "{GENERATED_CARGO_CONFIG_HEADER}\n[build]\ntarget-dir = \"custom-target\"\nbuild-dir = \"custom-build\"\n"
        ),
    )
    .unwrap();

    let visible = list_visible_workspace_paths(root.to_str().unwrap(), None, None).unwrap();
    assert_eq!(
        visible,
        vec![WORKTREE_CARGO_CONFIG_RELATIVE_PATH.to_string()]
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn clean_materialized_external_roots_are_projected_out_of_visible_workspace_entries() {
    let root = temp_dir("plan-filesystem-clean-external-projection");
    materialize_direct_external_fixture(&root, ".ait-external/ait-db");
    fs::write(root.join("authored.txt"), "authored\n").unwrap();

    let visible = list_visible_workspace_entries(root.to_str().unwrap(), None, None).unwrap();

    assert_eq!(
        visible.operational_external_roots,
        vec![".ait-external/ait-db"]
    );
    assert_eq!(
        visible.files,
        vec!["ait-external.lock", "ait-external.toml", "authored.txt"]
    );
    assert!(!visible
        .files
        .iter()
        .any(|path| path.starts_with(".ait-external/ait-db/")));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dirty_materialized_external_roots_and_orphans_remain_visible() {
    let root = temp_dir("plan-filesystem-dirty-external-visibility");
    materialize_direct_external_fixture(&root, ".ait-external/ait-db");
    fs::write(
        root.join(".ait-external/ait-db/AIT_EXTERNAL_SNAPSHOT"),
        "dirty\n",
    )
    .unwrap();
    fs::create_dir_all(root.join(".ait-external/orphan")).unwrap();
    fs::write(root.join(".ait-external/orphan/orphan.txt"), "orphan\n").unwrap();

    let visible = list_visible_workspace_entries(root.to_str().unwrap(), None, None).unwrap();

    assert!(visible.operational_external_roots.is_empty());
    assert!(visible
        .files
        .iter()
        .any(|path| path.starts_with(".ait-external/ait-db/")));
    assert!(visible
        .files
        .iter()
        .any(|path| path == ".ait-external/orphan/orphan.txt"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn file_reads_and_zip_bridge_work() {
    let root = temp_dir("plan-filesystem-read");
    let text_path = root.join("plan.md");
    let json_path = root.join("payload.json");
    let binary_path = root.join("blob.bin");
    fs::write(&text_path, "line one\nline two\n").unwrap();
    fs::write(&json_path, "{\"plan_id\":\"PL-1\"}").unwrap();
    fs::write(&binary_path, b"\x00\x01\x02").unwrap();
    assert_eq!(
        read_utf8_text_file(text_path.to_str().unwrap()).unwrap(),
        "line one\nline two\n"
    );
    assert_eq!(
        read_json_file(json_path.to_str().unwrap()).unwrap()["plan_id"],
        JsonValue::String("PL-1".to_string())
    );
    assert_eq!(
        read_binary_file(binary_path.to_str().unwrap()).unwrap(),
        b"\x00\x01\x02"
    );

    let invalid_utf8_path = root.join("invalid.txt");
    fs::write(&invalid_utf8_path, vec![0xff, 0xfe]).unwrap();
    assert!(matches!(
        read_utf8_text_file(invalid_utf8_path.to_str().unwrap()),
        Err(PlanFilesystemError::Invalid(_))
    ));

    let archive_path = root.join("sample.zip");
    let file = File::create(&archive_path).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    writer
        .start_file("docs/plan.md", zip::write::FileOptions::default())
        .unwrap();
    writer.write_all(b"# Plan\n").unwrap();
    writer.finish().unwrap();
    assert!(zip_archive_has_member(archive_path.to_str().unwrap(), "docs/plan.md").unwrap());
    assert!(!zip_archive_has_member(archive_path.to_str().unwrap(), "missing.md").unwrap());
    assert_eq!(
        read_zip_archive_member(archive_path.to_str().unwrap(), "docs/plan.md").unwrap(),
        b"# Plan\n"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn file_reads_use_file_io_store_entrypoints() {
    let store = FakePlanFilesystemFileIoStore::default();
    store.insert_file("/home/ait/docs/plan.md", b"line one\r\nline two\r");
    store.insert_file("/home/ait/payload.json", br#"{"plan_id":"PL-STORE"}"#);
    store.insert_file("/home/ait/blob.bin", b"\x00\x01\x02");
    let mut zip_buffer = Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut zip_buffer);
        writer
            .start_file("docs/plan.md", zip::write::FileOptions::default())
            .unwrap();
        writer.write_all(b"# Plan\n").unwrap();
        writer.finish().unwrap();
    }
    store.insert_file("/home/ait/archive.zip", &zip_buffer.into_inner());

    assert_eq!(
        read_utf8_text_file_with_file_io_store(&store, "~/docs/plan.md").unwrap(),
        "line one\nline two\n"
    );
    assert_eq!(
        read_json_file_with_file_io_store(&store, "~/payload.json").unwrap()["plan_id"],
        JsonValue::String("PL-STORE".to_string())
    );
    assert_eq!(
        read_binary_file_with_file_io_store(&store, "~/blob.bin").unwrap(),
        b"\x00\x01\x02"
    );
    assert!(
        zip_archive_has_member_with_file_io_store(&store, "~/archive.zip", "docs/plan.md").unwrap()
    );
    assert_eq!(
        read_zip_archive_member_with_file_io_store(&store, "~/archive.zip", "docs/plan.md")
            .unwrap(),
        b"# Plan\n"
    );
    assert_eq!(
        *store.reads.borrow(),
        vec![
            PathBuf::from("/home/ait/docs/plan.md"),
            PathBuf::from("/home/ait/payload.json"),
            PathBuf::from("/home/ait/blob.bin"),
            PathBuf::from("/home/ait/archive.zip"),
            PathBuf::from("/home/ait/archive.zip"),
        ]
    );
}

#[test]
fn resolve_repo_artifact_path_enforces_root_and_runtime_boundaries() {
    let root = temp_dir("plan-filesystem-resolve");
    fs::create_dir_all(root.join(".ait")).unwrap();
    fs::write(root.join("docs.md"), "doc\n").unwrap();

    let payload = resolve_repo_artifact_path(root.to_str().unwrap(), "docs.md", false).unwrap();
    assert_eq!(
        payload["artifact_path"],
        JsonValue::String("docs.md".to_string())
    );
    assert!(resolve_repo_artifact_path(root.to_str().unwrap(), ".ait/config.json", true).is_err());
    assert!(resolve_repo_artifact_path(root.to_str().unwrap(), "../outside.txt", true).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn resolve_repo_artifact_path_canonicalizes_symlinked_absolute_targets() {
    let root = temp_dir("plan-filesystem-resolve-symlink");
    fs::create_dir_all(root.join(".ait")).unwrap();
    fs::write(root.join("docs.md"), "doc\n").unwrap();
    let alias = root.with_file_name(format!(
        "{}-alias",
        root.file_name().unwrap().to_string_lossy()
    ));
    std::os::unix::fs::symlink(&root, &alias).unwrap();
    let canonical_root = root.canonicalize().unwrap();

    let payload = resolve_repo_artifact_path(
        root.to_str().unwrap(),
        alias.join("docs.md").to_str().unwrap(),
        false,
    )
    .unwrap();
    assert_eq!(
        payload["artifact_path"],
        JsonValue::String("docs.md".to_string())
    );
    assert_eq!(
        payload["resolved_path"],
        JsonValue::String(canonical_root.join("docs.md").to_string_lossy().to_string())
    );

    let missing = resolve_repo_artifact_path(
        root.to_str().unwrap(),
        alias.join("missing.md").to_str().unwrap(),
        true,
    )
    .unwrap();
    assert_eq!(
        missing["artifact_path"],
        JsonValue::String("missing.md".to_string())
    );
    assert_eq!(
        missing["resolved_path"],
        JsonValue::String(
            canonical_root
                .join("missing.md")
                .to_string_lossy()
                .to_string()
        )
    );

    fs::remove_file(alias).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workspace_ignore_parser_supports_negation_and_anchored_patterns() {
    let rules = parse_workspace_ignore_rules("generated/\n!generated/keep.txt\n/docs/*.md\n");
    let ignored = BTreeSet::from([
        workspace_path_is_ignored_for_rules(Path::new("generated/skip.txt"), &rules),
        workspace_path_is_ignored_for_rules(Path::new("docs/plan.md"), &rules),
    ]);
    assert!(ignored.contains(&true));
    assert!(!workspace_path_is_ignored_for_rules(
        Path::new("generated/keep.txt"),
        &rules
    ));
}

#[test]
fn workspace_ignore_matcher_matches_one_shot_ignore_decisions() {
    let matcher = parse_workspace_ignore_matcher("generated/\n!generated/keep.txt\n/docs/*.md\n");
    assert!(workspace_relative_path_is_ignored_with_matcher(
        "generated/skip.txt",
        &matcher
    ));
    assert!(!workspace_relative_path_is_ignored_with_matcher(
        "generated/keep.txt",
        &matcher
    ));
    assert!(workspace_relative_path_is_ignored_with_matcher(
        "docs/plan.md",
        &matcher
    ));
}
