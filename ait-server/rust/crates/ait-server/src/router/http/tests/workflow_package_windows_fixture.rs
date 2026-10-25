use std::fs;
use std::os::windows::{fs::MetadataExt, process::CommandExt};
use std::path::Path;
use std::process::Command;

pub(super) fn junction(link: &Path, target: &Path) {
    // PathBuf::join can retain '/' inside a segment. mklink parses those as
    // switches, so resolve the existing parent before appending the link name.
    let link = link
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap()
        .join(link.file_name().unwrap());
    let target = target.canonicalize().unwrap();
    // cmd does not use CRT argv quoting. Keep its command fixed and expand each
    // native path once inside quotes; /v:off preserves literal '!' in paths.
    let mut command = Command::new("cmd.exe");
    command
        .args(["/d", "/e:on", "/v:off", "/c"])
        .raw_arg(r#"mklink /J "%AIT_FIXTURE_JUNCTION_LINK%" "%AIT_FIXTURE_JUNCTION_TARGET%""#)
        .env("AIT_FIXTURE_JUNCTION_LINK", &link)
        .env("AIT_FIXTURE_JUNCTION_TARGET", &target);
    let result = command
        .output()
        .unwrap_or_else(|error| panic!("junction fixture {command:?}: {error}"));
    assert!(
        result.status.success(),
        "junction fixture {command:?}: status={}; stdout={}; stderr={}",
        result.status,
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    assert_ne!(
        fs::symlink_metadata(&link).unwrap().file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT,
        0,
        "junction fixture is not a reparse point: {link:?}"
    );
    assert_eq!(link.canonicalize().unwrap(), target);
}
