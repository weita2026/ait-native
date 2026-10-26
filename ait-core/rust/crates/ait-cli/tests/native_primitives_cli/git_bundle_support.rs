fn native_git_bytes(repo: &Path, name: &str) -> Vec<u8> {
    let bundle=ait_core::git_binary_db::GitInteropBundle::open_for_recovery(repo,false).unwrap();
    fs::read(bundle.decoded_root().join(name)).unwrap()
}
fn native_git_mutate(repo: &Path, edit: impl FnOnce(&Path)) {
    use ait_core::git_binary_db::*;
    let bundle=GitInteropBundle::open_for_recovery(repo,true).unwrap();
    edit(bundle.decoded_root());
    let mut candidate=tempfile::NamedTempFile::new_in(repo.join(".ait/git")).unwrap();
    let mut encoder=zstd::stream::write::Encoder::new(candidate.as_file_mut(),3).unwrap();
    encoder.include_checksum(true).unwrap();
    let mut tar=tar::Builder::new(encoder);
    for name in GIT_OBJECT_STORE_BIN_PATHS.iter().chain(GIT_OBJECT_STORE_INDEX_PATHS) {
        let mut file=fs::File::open(bundle.decoded_root().join(name)).unwrap();
        let mut header=tar::Header::new_ustar();header.set_size(file.metadata().unwrap().len());header.set_mode(0o600);header.set_cksum();
        tar.append_data(&mut header,*name,&mut file).unwrap();
    }
    tar.into_inner().unwrap().finish().unwrap();candidate.as_file().sync_all().unwrap();
    candidate.persist(repo.join(GIT_INTEROP_BUNDLE_PATH)).unwrap();
}
fn native_git_write(repo:&Path,name:&str,bytes:&[u8]) {
    native_git_mutate(repo,|root|fs::write(root.join(name),bytes).unwrap());
}
