use super::*;

fn write_release_exec_fixture(root: &Path) {
    fs::create_dir_all(root.join("src/ait/cli/commands")).unwrap();
    fs::create_dir_all(root.join("ci")).unwrap();
    fs::write(
        root.join("pyproject.toml"),
        "[project.scripts]\nait = \"ait.cli_entrypoint:main\"\n",
    )
    .unwrap();
    fs::write(
        root.join("src/ait/cli_entrypoint.py"),
        "NATIVE = {\"release\"}\nos.execvpe(binary, argv, env)\nfrom .cli import app\n",
    )
    .unwrap();
    fs::write(
        root.join("src/ait/cli/app_surfaces.py"),
        "# no release route\n",
    )
    .unwrap();
    fs::write(
        root.join("src/ait/cli/native_namespace_command.py"),
        "NATIVE_WORKFLOW_GATE_NAMESPACES = set()\n",
    )
    .unwrap();
    fs::write(
        root.join("src/ait/cli/commands/bootstrap.py"),
        "PRIMARY_COMMAND_MODULES = {}\n",
    )
    .unwrap();
    fs::write(
        root.join("ci/patch_ci.json"),
        "AIT_SHARED_CARGO_TARGET_DIR/debug/ait-cli test patchset-ci release-artifact-smoke --json\n",
    )
    .unwrap();
}

#[test]
fn release_authority_guard_requires_console_exec_and_native_smoke() {
    let temp = TempDir::new().unwrap();
    write_release_exec_fixture(temp.path());

    assert_release_python_authority_retired(temp.path()).unwrap();
}

#[test]
fn release_authority_guard_rejects_returning_python_namespace_bridge() {
    let temp = TempDir::new().unwrap();
    write_release_exec_fixture(temp.path());
    fs::write(
        temp.path().join("src/ait/cli/app_surfaces.py"),
        "_register_native_namespace_command(\"release\")\n",
    )
    .unwrap();

    let error = assert_release_python_authority_retired(temp.path()).unwrap_err();
    assert!(error.contains("still registered in the Python CLI application"));
}

#[test]
fn release_authority_guard_rejects_retired_python_module() {
    let temp = TempDir::new().unwrap();
    write_release_exec_fixture(temp.path());
    fs::write(
        temp.path().join("src/ait/cli/commands/release.py"),
        "# retired\n",
    )
    .unwrap();

    let error = assert_release_python_authority_retired(temp.path()).unwrap_err();
    assert!(error.contains("src/ait/cli/commands/release.py"));
}

#[test]
fn fake_remote_survives_fixture_setup_idle_window() {
    let mut remote = spawn_fake_remote();

    thread::sleep(Duration::from_millis(2200));
    let response = fake_remote_get(&remote.base_url, "/healthz");

    assert!(response.starts_with("HTTP/1.1 200"));
    assert!(response.contains("patchset_run_ci_route"));
    remote.stop().unwrap();
}

fn fake_remote_get(base_url: &str, path: &str) -> String {
    let address = base_url
        .strip_prefix("http://")
        .expect("fake remote base url should be http");
    let mut stream =
        std::net::TcpStream::connect(address).expect("fake remote should accept connections");
    let request = format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
    std::io::Write::write_all(&mut stream, request.as_bytes())
        .expect("fake remote request should be written");
    let mut response = String::new();
    std::io::Read::read_to_string(&mut stream, &mut response)
        .expect("fake remote response should be read");
    response
}

#[test]
fn markdown_link_checker_detects_missing_local_links() {
    let temp = TempDir::new().unwrap();
    let docs = temp.path().join("docs");
    fs::create_dir_all(&docs).unwrap();
    fs::write(docs.join("source.md"), "[missing](./missing.md)\n").unwrap();

    let issues = find_broken_links(temp.path()).unwrap();

    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].path, PathBuf::from("docs/source.md"));
    assert_eq!(issues[0].line_number, 1);
}

#[cfg(unix)]
#[test]
fn markdown_link_checker_does_not_follow_directory_symlinks() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("workspace");
    let docs = workspace.join("docs");
    fs::create_dir_all(&docs).unwrap();
    fs::write(docs.join("source.md"), "[missing](./missing.md)\n").unwrap();

    let external_docs = temp.path().join("external-docs");
    fs::create_dir_all(&external_docs).unwrap();
    fs::write(
        external_docs.join("external.md"),
        "[external missing](./missing.md)\n",
    )
    .unwrap();
    symlink(&external_docs, workspace.join("linked-docs")).unwrap();

    let expected = workspace.canonicalize().unwrap().join("docs/source.md");
    assert_eq!(iter_markdown_files(&workspace).unwrap(), vec![expected]);
    let issues = find_broken_links(&workspace).unwrap();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].path, PathBuf::from("docs/source.md"));
}

#[test]
fn markdown_link_checker_ignores_external_anchors_and_fences() {
    let temp = TempDir::new().unwrap();
    let docs = temp.path().join("docs");
    fs::create_dir_all(&docs).unwrap();
    fs::write(docs.join("target.md"), "# Target\n").unwrap();
    fs::write(
        docs.join("source.md"),
        "[target](./target.md)\n[anchor](#local-anchor)\n[external](https://example.com/path)\n```\n[fenced](./missing.md)\n```\n",
    )
    .unwrap();

    assert!(find_broken_links(temp.path()).unwrap().is_empty());
}

#[test]
fn markdown_link_checker_skips_missing_sprint_targets_when_surface_is_absent() {
    let temp = TempDir::new().unwrap();
    let docs = temp.path().join("docs");
    fs::create_dir_all(&docs).unwrap();
    fs::write(docs.join("index.md"), "[card](./sprints/card.md)\n").unwrap();

    assert!(find_broken_links(temp.path()).unwrap().is_empty());
}

#[test]
fn markdown_link_checker_uses_source_root_targets_when_worktree_omits_authored_root_files() {
    let temp = TempDir::new().unwrap();
    let source_root = temp.path().join("source");
    fs::create_dir_all(source_root.join("release")).unwrap();
    fs::write(source_root.join("README.md"), "# Root README\n").unwrap();
    fs::write(
        source_root.join("release/HOMEBREW_TAP.md"),
        "# Homebrew tap\n",
    )
    .unwrap();

    let worktree_release = temp.path().join("release/guides");
    fs::create_dir_all(&worktree_release).unwrap();
    fs::write(
        temp.path().join(".ait-worktree.json"),
        format!(
            "{{\"repo_root\":\"{}\"}}\n",
            source_root.display().to_string().replace('\\', "\\\\")
        ),
    )
    .unwrap();
    fs::write(
        worktree_release.join("LOCAL_QUICKSTART.md"),
        "[homebrew](../HOMEBREW_TAP.md)\n[readme](../../README.md)\n",
    )
    .unwrap();

    assert!(find_broken_links(temp.path()).unwrap().is_empty());
}

#[test]
fn markdown_link_checker_rebases_sibling_repo_links_against_source_root() {
    let temp = TempDir::new().unwrap();
    let source_root = temp.path().join("source");
    fs::create_dir_all(source_root.join("../ait-core")).unwrap();
    fs::create_dir_all(source_root.join("docs/rust")).unwrap();
    fs::write(
        source_root.join("docs/rust/plan.md"),
        "[kernel](../../../ait-core/kernel.rs)\n",
    )
    .unwrap();
    fs::write(
        source_root.join("../ait-core/kernel.rs"),
        "pub fn ok() {}\n",
    )
    .unwrap();

    let worktree_docs = temp.path().join("docs/rust");
    fs::create_dir_all(&worktree_docs).unwrap();
    fs::write(
        temp.path().join(".ait-worktree.json"),
        format!("{{\"repo_root\":\"{}\"}}\n", source_root.display()),
    )
    .unwrap();
    fs::write(
        worktree_docs.join("plan.md"),
        "[kernel](../../../ait-core/kernel.rs)\n",
    )
    .unwrap();

    assert!(find_broken_links(temp.path()).unwrap().is_empty());
}
