use super::*;
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SetSecurityInfo,
};

fn private_acl(path: &Path, extra: &str) {
    let principals = Principals::current().unwrap();
    let mut text = null_mut();
    // SAFETY: the validated SID and output pointer remain live for this call.
    assert_ne!(
        unsafe { ConvertSidToStringSidW(principals.user.pointer(), &mut text) },
        0
    );
    let mut length = 0;
    // SAFETY: ConvertSidToStringSidW returns a NUL-terminated allocation.
    while unsafe { *text.add(length) } != 0 {
        length += 1;
    }
    let user = String::from_utf16(unsafe { std::slice::from_raw_parts(text, length) }).unwrap();
    unsafe { LocalFree(text.cast()) };
    let sddl = format!("D:P(A;OICI;FA;;;{user})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA){extra}");
    let sddl = OsStr::new(&sddl)
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut descriptor = null_mut();
    // SAFETY: sddl is terminated and descriptor receives a LocalAlloc buffer.
    assert_ne!(
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                null_mut(),
            )
        },
        0
    );
    let descriptor = Descriptor(descriptor);
    let mut present = 0;
    let mut defaulted = 0;
    let mut acl = null_mut();
    assert_ne!(
        unsafe { GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut acl, &mut defaulted) },
        0
    );
    assert_ne!(present, 0);
    let file = OpenOptions::new()
        .access_mode(READ_CONTROL | WRITE_DAC | SYNCHRONIZE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .unwrap();
    assert_eq!(
        unsafe {
            SetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                acl,
                null_mut(),
            )
        },
        ERROR_SUCCESS
    );
}

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    private_acl(&root, "");
    let job = root.join("1").join("ab".repeat(32)).join("7");
    fs::create_dir_all(job.join("source")).unwrap();
    fs::write(job.join("package.zst"), b"sealed bytes").unwrap();
    (temporary, root, job)
}

#[test]
fn windows_owned_source_pins_paths_and_archive_until_validation_finishes() {
    let (_temporary, root, job) = fixture();
    let source = verify_managed_runtime_source(&root, &job).unwrap();
    assert_eq!(source.read_package().unwrap(), b"sealed bytes");
    assert!(fs::rename(&job, job.with_file_name("renamed")).is_err());
    assert!(fs::write(job.join("package.zst"), b"changed").is_err());
    drop(source);
    fs::write(job.join("package.zst"), b"changed").unwrap();
}

#[test]
fn windows_acl_foreign_owner_and_reparse_paths_are_held() {
    let (_temporary, root, job) = fixture();
    private_acl(&root, "(A;OICI;FR;;;WD)");
    assert!(verify_managed_runtime_source(&root, &job).is_ok());
    private_acl(job.parent().unwrap(), "(A;OICI;FW;;;WD)");
    assert!(verify_managed_runtime_source(&root, &job).is_err());
    assert!(remove_empty(&root, &job.join("source"), || Ok(())).is_err());
    private_acl(job.parent().unwrap(), "");
    private_acl(&job.join("package.zst"), "(A;;FW;;;WD)");
    assert!(verify_managed_runtime_source(&root, &job).is_err());
    private_acl(&job.join("package.zst"), "");
    // A read-only lookup of a real foreign-owned directory; no owner/ACL change
    // or privilege escalation is needed to exercise the actual owner rejection.
    let foreign = PathBuf::from(std::env::var_os("SystemRoot").expect("native Windows SystemRoot"));
    assert!(Scope::open(&foreign).is_err());
    fs::remove_dir(job.join("source")).unwrap();
    let outside = root.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), b"keep").unwrap();
    let result = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(job.join("source"))
        .arg(&outside)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "native junction fixture creation failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(verify_managed_runtime_source(&root, &job).is_err());
    assert!(remove_empty(&root, &job.join("source"), || Ok(())).is_err());
    assert_eq!(fs::read(outside.join("keep")).unwrap(), b"keep");
    fs::remove_dir(job.join("source")).unwrap();
}

#[test]
fn windows_empty_cleanup_is_atomic_and_preserves_inserted_files_and_active_handles() {
    let (_temporary, root, job) = fixture();
    let child_path = job.join("source");
    assert!(!remove_empty(&root, &child_path, || {
        assert!(fs::rename(&job, job.with_file_name("moved")).is_err());
        fs::write(child_path.join("raced"), b"keep")
    })
    .unwrap());
    assert_eq!(fs::read(child_path.join("raced")).unwrap(), b"keep");
    fs::remove_file(child_path.join("raced")).unwrap();
    let active = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(&child_path)
        .unwrap();
    assert!(remove_empty(&root, &child_path, || Ok(())).is_err());
    drop(active);
    assert!(remove_empty(&root, &child_path, || Ok(())).unwrap());
    fs::create_dir(&child_path).unwrap();
    assert!(remove_empty(&root, &child_path, || Ok(())).unwrap());
    assert!(remove_empty(&root, &child_path, || Ok(())).unwrap());
    assert!(remove_empty(&root, &root, || Ok(())).is_err());
    assert!(remove_empty(&root, &root.join("1/../foreign"), || Ok(())).is_err());
}
