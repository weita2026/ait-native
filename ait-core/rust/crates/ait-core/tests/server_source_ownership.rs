use std::path::PathBuf;

#[test]
fn apache_core_does_not_own_server_binary_lifecycle_source() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    assert!(
        !crate_root.join("src/server_binary_lifecycle.rs").exists(),
        "server Binary lifecycle source belongs to ait-server-core, not Apache-licensed ait-core"
    );

    let lib_source =
        std::fs::read_to_string(crate_root.join("src/lib.rs")).expect("read ait-core lib.rs");
    assert!(
        !lib_source
            .lines()
            .any(|line| line.trim() == "pub mod server_binary_lifecycle;"),
        "ait-core must not export the server-only Binary lifecycle module"
    );
}
