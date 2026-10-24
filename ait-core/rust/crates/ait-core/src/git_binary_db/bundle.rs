//! The outer, checksummed container. Inner Binary DB bytes remain layout 1.
use super::{GIT_OBJECT_STORE_BIN_PATHS, GIT_OBJECT_STORE_INDEX_PATHS};
use crate::binary_db::{
    AuthorityId, BinaryDbCommandLockSet, BinaryDbCommandScope, BinaryDbDurableDirectory,
    BinaryDbError, BinaryDbReadLockSet, LocalBinaryDbFs, LocalStateScope, StorePath, StoreResult,
};
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

pub const GIT_INTEROP_BUNDLE_PATH: &str = ".ait/git/interop.tar.zst";
pub const GIT_INTEROP_LEGACY_DIRECTORY: &str = ".ait/git-interop/v1/binary-v0";
pub const GIT_INTEROP_MEMBER_LIMIT: u64 = 4 * 1024 * 1024 * 1024;
pub const GIT_INTEROP_BUNDLE_LIMIT: u64 = 16 * 1024 * 1024 * 1024;

const WIDTHS: &[(&str, u64)] = &[
    ("git_repository.bin", 28),
    ("git_object.bin", 32),
    ("git_generation.bin", 20),
    ("git_identity.bin", 24),
    ("git_commit_mapping.bin", 100),
    ("git_ref_mapping.bin", 80),
    ("git_operation_checkpoint.bin", 44),
    ("git_repository_fingerprint.idx", 16),
    ("git_object.idx", 29),
    ("git_document_plan.idx", 40),
    ("git_document_revision.idx", 50),
    ("git_generation_id.idx", 12),
    ("git_commit_object.idx", 28),
    ("git_commit_snapshot.idx", 8),
    ("git_ref_lookup.idx", 16),
    ("git_operation_id.idx", 12),
];

fn members() -> impl Iterator<Item = &'static str> {
    GIT_OBJECT_STORE_BIN_PATHS
        .iter()
        .chain(GIT_OBJECT_STORE_INDEX_PATHS)
        .copied()
}
fn io<T>(result: std::io::Result<T>) -> StoreResult<T> {
    result.map_err(|e| BinaryDbError::io("Git interop bundle", e))
}
fn corrupt(message: impl Into<String>) -> BinaryDbError {
    BinaryDbError::corruption(format!("Git interop bundle: {}", message.into()))
}
fn direct_directory(path: &Path, create: bool) -> StoreResult<()> {
    if create {
        io(fs::create_dir_all(path))?;
    }
    let m = io(fs::symlink_metadata(path))?;
    if !m.file_type().is_dir() {
        return Err(corrupt("directory alias or non-directory"));
    }
    Ok(())
}
fn open_direct(path: &Path, write: bool, create: bool) -> StoreResult<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(write).create(create);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        if fs::symlink_metadata(path).is_ok_and(|m| !m.file_type().is_file()) {
            return Err(corrupt("file alias"));
        }
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = io(options.open(path))?;
    if !io(file.metadata())?.is_file() {
        return Err(corrupt("non-regular file"));
    }
    Ok(file)
}
fn sync_dir(path: &Path) -> StoreResult<()> {
    io(crate::file_io::sync_filesystem_directory(path))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Stamp {
    length: u64,
    modified: std::time::SystemTime,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}
fn stamp(path: &Path) -> StoreResult<Option<Stamp>> {
    let m = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(BinaryDbError::io("Git bundle authority stamp", e)),
    };
    if !m.file_type().is_file() {
        return Err(corrupt("authority is not a direct file"));
    }
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    Ok(Some(Stamp {
        length: m.len(),
        modified: io(m.modified())?,
        #[cfg(unix)]
        identity: (m.dev(), m.ino(), m.ctime(), m.ctime_nsec()),
    }))
}

#[derive(Debug)]
enum LegacyLock {
    Reader { _lock: BinaryDbReadLockSet },
    Writer { _lock: BinaryDbCommandLockSet },
}

#[derive(Debug)]
struct BundleLock(File);

impl Drop for BundleLock {
    fn drop(&mut self) {
        // Close-on-exec does not release descriptors in a child still between
        // fork and exec. Release the operation's lock before closing our copy.
        let _ = FileExt::unlock(&self.0);
    }
}

/// Owns the decoded files and all outer locks until the last DB clone drops.
#[derive(Debug)]
pub struct GitInteropBundle {
    decoded: TempDir,
    archive: PathBuf,
    legacy: PathBuf,
    writable: bool,
    _legacy_lock: Mutex<Option<LegacyLock>>,
    expected: Mutex<Option<Stamp>>,
    // Fields drop in declaration order: decoded files and inner locks first.
    _lock: BundleLock,
}

impl GitInteropBundle {
    pub fn exists(repo: &Path) -> bool {
        fs::symlink_metadata(repo.join(GIT_INTEROP_BUNDLE_PATH))
            .map_or_else(|e| e.kind() != std::io::ErrorKind::NotFound, |_| true)
            || legacy_inputs(&repo.join(GIT_INTEROP_LEGACY_DIRECTORY))
                .map_or(true, |files| !files.is_empty())
    }

    pub fn open(repo: &Path, writable: bool) -> StoreResult<Arc<Self>> {
        Self::open_mode(repo, writable, true)
    }
    /// Outer integrity still applies; only explicit recovery may repair inner tails.
    pub fn open_for_recovery(repo: &Path, writable: bool) -> StoreResult<Arc<Self>> {
        Self::open_mode(repo, writable, false)
    }
    fn open_mode(repo: &Path, writable: bool, strict: bool) -> StoreResult<Arc<Self>> {
        let parent = repo.join(".ait/git");
        direct_directory(&repo.join(".ait"), true)?;
        direct_directory(&parent, true)?;
        let lock = open_direct(&parent.join("interop.lock"), true, true)?;
        let locked = if writable {
            FileExt::try_lock_exclusive(&lock)
        } else {
            FileExt::try_lock_shared(&lock)
        };
        locked.map_err(|e| {
            BinaryDbError::retryable_busy(format!("Git interop bundle is busy: {e}"))
        })?;
        let lock = BundleLock(lock);
        // Exclusive ownership proves no live decoded workspace can be removed.
        if writable {
            for entry in io(fs::read_dir(&parent))? {
                let entry = io(entry)?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                let kind = io(entry.file_type())?;
                if (name.starts_with(".interop-decode-")
                    || name.starts_with(".interop-check-")
                    || name.starts_with(".interop-recovery-"))
                    && kind.is_dir()
                {
                    io(fs::remove_dir_all(entry.path()))?;
                }
                if name.starts_with(".interop-publish-") && kind.is_file() {
                    io(fs::remove_file(entry.path()))?;
                }
            }
        }
        let archive = repo.join(GIT_INTEROP_BUNDLE_PATH);
        let legacy = repo.join(GIT_INTEROP_LEGACY_DIRECTORY);
        let legacy_files = legacy_inputs(&legacy)?;
        let legacy_lock = if !legacy_files.is_empty() {
            Some(if writable {
                LegacyLock::Writer {
                    _lock: BinaryDbCommandLockSet::try_acquire(
                        &StorePath::from(legacy.clone()),
                        BinaryDbCommandScope::General,
                    )?
                    .ok_or_else(|| BinaryDbError::retryable_busy("legacy Git authority is busy"))?,
                }
            } else {
                LegacyLock::Reader {
                    _lock: BinaryDbReadLockSet::try_acquire(&StorePath::from(legacy.clone()))?,
                }
            })
        } else {
            None
        };
        let expected = stamp(&archive)?;
        let decoded = io(tempfile::Builder::new()
            .prefix(".interop-decode-")
            .tempdir_in(&parent))?;
        if expected.is_some() {
            decode(&archive, decoded.path(), strict)?;
            for (path, name) in &legacy_files {
                if digest_file(path)? != digest_file(&decoded.path().join(name))? {
                    return Err(corrupt("ambiguous old/new native authority"));
                }
            }
        } else {
            let mut total = 0u64;
            for (path, name) in &legacy_files {
                let n = io(fs::symlink_metadata(path))?.len();
                total = total
                    .checked_add(n)
                    .ok_or_else(|| corrupt("legacy size overflow"))?;
                if n > GIT_INTEROP_MEMBER_LIMIT || total > GIT_INTEROP_BUNDLE_LIMIT {
                    return Err(corrupt("legacy size limit"));
                }
                io(fs::copy(path, decoded.path().join(name)))?;
            }
            for name in members() {
                let path = decoded.path().join(name);
                if !path.exists() {
                    io(fs::write(&path, 1u32.to_le_bytes()))?;
                }
                if strict {
                    validate_member(&path, name)?;
                }
            }
        }
        if stamp(&archive)? != expected {
            return Err(corrupt("authority changed while decoding"));
        }
        Ok(Arc::new(Self {
            decoded,
            archive,
            legacy,
            writable,
            _lock: lock,
            _legacy_lock: Mutex::new(legacy_lock),
            expected: Mutex::new(expected),
        }))
    }
    pub fn database(self: &Arc<Self>, repo: &Path, authority: AuthorityId) -> LocalBinaryDbFs {
        LocalBinaryDbFs::new(
            self.decoded.path().to_path_buf(),
            repo.to_path_buf(),
            authority,
            LocalStateScope::Repository,
        )
        .with_declared_bin_paths(GIT_OBJECT_STORE_BIN_PATHS)
        .with_declared_index_paths(GIT_OBJECT_STORE_INDEX_PATHS)
        .with_durable_directory(self.clone())
    }
    pub fn decoded_root(&self) -> &Path {
        self.decoded.path()
    }
    pub fn needs_migration(&self) -> StoreResult<bool> {
        Ok(self
            .expected
            .lock()
            .map_err(|_| corrupt("poisoned publication lock"))?
            .is_none()
            || !legacy_inputs(&self.legacy)?.is_empty())
    }

    /// Caller first admits every native owner and its shared-content closure.
    /// A read never changes storage selection. First admitted write migrates it.
    pub fn publish_admitted(&self) -> StoreResult<()> {
        self.checkpoint()
    }
    /// Explicit recovery may first retire byte-identical remnants of an
    /// already selected, integrity-checked archive. It never selects old data.
    pub fn retire_recovery_duplicates(&self) -> StoreResult<()> {
        self.require_writable()?;
        if self
            .expected
            .lock()
            .map_err(|_| corrupt("poisoned publication lock"))?
            .is_some()
        {
            self.retire_legacy()?;
        }
        Ok(())
    }

    fn retire_legacy(&self) -> StoreResult<()> {
        for (path, name) in legacy_inputs(&self.legacy)? {
            if digest_file(&path)? != digest_file(&self.decoded.path().join(&name))? {
                return Err(corrupt("legacy authority changed before retirement"));
            }
            io(fs::remove_file(path))?;
        }
        if self.legacy.exists() {
            sync_dir(&self.legacy)?;
        }
        Ok(())
    }
}
impl BinaryDbDurableDirectory for GitInteropBundle {
    fn verify_authority(&self) -> StoreResult<()> {
        if stamp(&self.archive)?
            != *self
                .expected
                .lock()
                .map_err(|_| corrupt("poisoned publication lock"))?
        {
            return Err(BinaryDbError::retryable_busy(
                "Git bundle authority changed; reopen before using proofs",
            ));
        }
        Ok(())
    }
    fn require_writable(&self) -> StoreResult<()> {
        if !self.writable {
            return Err(corrupt("read-only decoded authority cannot be written"));
        }
        self.verify_authority()
    }
    fn checkpoint(&self) -> StoreResult<()> {
        self.require_writable()?;
        let mut expected = self
            .expected
            .lock()
            .map_err(|_| corrupt("poisoned publication lock"))?;
        if stamp(&self.archive)? != *expected {
            return Err(corrupt("authority changed before publication"));
        }
        let parent = self
            .archive
            .parent()
            .ok_or_else(|| corrupt("missing archive parent"))?;
        let mut candidate = io(tempfile::Builder::new()
            .prefix(".interop-publish-")
            .tempfile_in(parent))?;
        #[cfg(test)]
        let phase = std::time::Instant::now();
        encode(self.decoded.path(), candidate.as_file_mut())?;
        #[cfg(test)]
        crate::binary_db::read_profile::note("bundle_encode_ns", phase.elapsed().as_nanos() as u64);
        #[cfg(test)]
        let phase = std::time::Instant::now();
        io(candidate.as_file().sync_all())?;
        #[cfg(test)]
        crate::binary_db::read_profile::note(
            "bundle_archive_sync_ns",
            phase.elapsed().as_nanos() as u64,
        );
        #[cfg(test)]
        let phase = std::time::Instant::now();
        decode_to(candidate.path(), self.decoded.path(), true, true)?;
        #[cfg(test)]
        crate::binary_db::read_profile::note(
            "bundle_compare_ns",
            phase.elapsed().as_nanos() as u64,
        );
        if stamp(&self.archive)? != *expected {
            return Err(corrupt("authority changed during publication"));
        }
        publication_pause("before-rename");
        #[cfg(test)]
        let phase = std::time::Instant::now();
        candidate
            .persist(&self.archive)
            .map_err(|e| BinaryDbError::io("publish Git interop bundle", e.error))?;
        *expected = stamp(&self.archive)?;
        sync_dir(parent)?;
        #[cfg(test)]
        crate::binary_db::read_profile::note(
            "bundle_publish_ns",
            phase.elapsed().as_nanos() as u64,
        );
        publication_pause("after-rename");
        drop(expected);
        self.retire_legacy()
    }
}
fn publication_pause(_at: &str) {
    #[cfg(test)]
    tests::publication_pause(_at);
}

/// Only selected native leaves are eligible for retirement; other entries fail.
pub fn legacy_git_interop_inputs(root: &Path) -> StoreResult<Vec<(PathBuf, String)>> {
    legacy_inputs(root)
}
fn legacy_inputs(root: &Path) -> StoreResult<Vec<(PathBuf, String)>> {
    if root.ends_with(GIT_INTEROP_LEGACY_DIRECTORY) {
        for parent in root.ancestors().skip(1).take(3) {
            if fs::symlink_metadata(parent).is_ok_and(|m| !m.file_type().is_dir()) {
                return Err(corrupt("legacy parent directory alias"));
            }
        }
    }
    match fs::symlink_metadata(root) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(BinaryDbError::io("legacy Git authority", e)),
        Ok(m) if !m.file_type().is_dir() => return Err(corrupt("legacy directory alias")),
        _ => {}
    }
    let mut result = Vec::new();
    for entry in io(fs::read_dir(root))? {
        let entry = io(entry)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| corrupt("non UTF-8 legacy leaf"))?;
        let kind = io(entry.file_type())?;
        if name == ".locks" && kind.is_dir() {
            continue;
        }
        if !kind.is_file() || !members().any(|n| n == name) {
            return Err(corrupt(format!("unexpected legacy entry {name}")));
        }
        result.push((entry.path(), name));
    }
    result.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(result)
}
fn digest_file(path: &Path) -> StoreResult<(u64, [u8; 32])> {
    let mut file = open_direct(path, false, false)?;
    let mut hash = Sha256::new();
    let n = io(std::io::copy(&mut file, &mut hash))?;
    Ok((n, hash.finalize().into()))
}
fn validate_member(path: &Path, name: &str) -> StoreResult<u64> {
    let mut file = open_direct(path, false, false)?;
    let n = io(file.metadata())?.len();
    if !(4..=GIT_INTEROP_MEMBER_LIMIT).contains(&n) {
        return Err(corrupt("member size limit/header"));
    }
    let mut header = [0; 4];
    io(file.read_exact(&mut header))?;
    if header != 1u32.to_le_bytes() {
        return Err(corrupt(format!("unsupported inner layout: {name}")));
    }
    if let Some((_, width)) = WIDTHS.iter().find(|(leaf, _)| *leaf == name) {
        if !(n - 4).is_multiple_of(*width) {
            return Err(corrupt(format!("incomplete inner record: {name}")));
        }
    }
    Ok(n)
}
fn octal(bytes: &[u8]) -> StoreResult<u64> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| corrupt("invalid tar number"))?
        .trim_matches(['\0', ' ']);
    if text.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(text, 8).map_err(|_| corrupt("invalid tar number"))
}
fn put_octal(bytes: &mut [u8], value: u64) -> StoreResult<()> {
    let text = format!("{:0width$o}", value, width = bytes.len() - 1);
    if text.len() != bytes.len() - 1 {
        return Err(corrupt("tar number overflow"));
    }
    bytes[..text.len()].copy_from_slice(text.as_bytes());
    Ok(())
}
fn encode(root: &Path, file: &mut File) -> StoreResult<()> {
    // Checkpoints favor bounded fast compression; all framing and byte checks
    // are independent of compression level.
    let mut encoder = io(zstd::stream::write::Encoder::new(file, -3))?;
    io(encoder.include_checksum(true))?;
    let mut total = 0u64;
    for name in members() {
        let path = root.join(name);
        let n = validate_member(&path, name)?;
        total = total
            .checked_add(n)
            .ok_or_else(|| corrupt("size overflow"))?;
        if total > GIT_INTEROP_BUNDLE_LIMIT {
            return Err(corrupt("decoded size limit"));
        }
        let before = stamp(&path)?;
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        put_octal(&mut header[100..108], 0o600)?;
        put_octal(&mut header[108..116], 0)?;
        put_octal(&mut header[116..124], 0)?;
        put_octal(&mut header[124..136], n)?;
        put_octal(&mut header[136..148], 0)?;
        header[148..156].fill(b' ');
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        let sum = header.iter().map(|b| u64::from(*b)).sum();
        put_octal(&mut header[148..155], sum)?;
        header[155] = b' ';
        io(encoder.write_all(&header))?;
        let copied = io(std::io::copy(
            &mut open_direct(&path, false, false)?.take(n),
            &mut encoder,
        ))?;
        if copied != n || stamp(&path)? != before {
            return Err(corrupt("inner authority changed while encoding"));
        }
        io(encoder.write_all(&[0; 512][..((512 - n % 512) % 512) as usize]))?;
    }
    io(encoder.write_all(&[0; 1024]))?;
    io(encoder.finish())?;
    Ok(())
}
fn decode(path: &Path, root: &Path, strict: bool) -> StoreResult<()> {
    decode_to(path, root, strict, false)
}
fn decode_to(path: &Path, root: &Path, strict: bool, compare: bool) -> StoreResult<()> {
    let file = open_direct(path, false, false)?;
    if io(file.metadata())?.len() > GIT_INTEROP_BUNDLE_LIMIT {
        return Err(corrupt("compressed size limit"));
    }
    let mut input = BufReader::new(file);
    let prefix = io(input.fill_buf())?;
    if prefix.len() < 5 || prefix[..4] != [0x28, 0xb5, 0x2f, 0xfd] || prefix[4] & 4 == 0 {
        return Err(corrupt("one checksummed zstd frame is required"));
    }
    let mut decoder = io(zstd::stream::read::Decoder::with_buffer(input))?.single_frame();
    io(decoder.window_log_max(27))?;
    let mut seen = BTreeSet::new();
    let mut total = 0u64;
    loop {
        let mut header = [0u8; 512];
        io(decoder.read_exact(&mut header))?;
        if header == [0; 512] {
            let mut end = [0; 512];
            io(decoder.read_exact(&mut end))?;
            if end != [0; 512] {
                return Err(corrupt("missing second tar end block"));
            }
            let mut extra = [0; 1];
            if io(decoder.read(&mut extra))? != 0 {
                return Err(corrupt("trailing tar data"));
            }
            break;
        }
        let stored = octal(&header[148..156])?;
        let sum = header
            .iter()
            .enumerate()
            .map(|(i, b)| u64::from(if (148..156).contains(&i) { b' ' } else { *b }))
            .sum::<u64>();
        if stored != sum || header[257..263] != *b"ustar\0" || header[263..265] != *b"00" {
            return Err(corrupt("invalid ustar header/checksum"));
        }
        let end = header[..100].iter().position(|b| *b == 0).unwrap_or(100);
        let name =
            std::str::from_utf8(&header[..end]).map_err(|_| corrupt("invalid member name"))?;
        if header[end..100].iter().any(|b| *b != 0)
            || !members().any(|n| n == name)
            || !seen.insert(name.to_string())
            || !matches!(header[156], 0 | b'0')
            || header[157..257].iter().any(|b| *b != 0)
            || header[345..500].iter().any(|b| *b != 0)
        {
            return Err(corrupt("unexpected, duplicate, linked or unsafe member"));
        }
        let n = octal(&header[124..136])?;
        total = total
            .checked_add(n)
            .ok_or_else(|| corrupt("size overflow"))?;
        if !(4..=GIT_INTEROP_MEMBER_LIMIT).contains(&n) || total > GIT_INTEROP_BUNDLE_LIMIT {
            return Err(corrupt("decoded size limit"));
        }
        let target = root.join(name);
        if compare {
            let before = stamp(&target)?;
            if validate_member(&target, name)? != n {
                return Err(corrupt("candidate member size differs"));
            }
            let mut original = open_direct(&target, false, false)?;
            let mut actual = [0; 64 * 1024];
            let mut expected = [0; 64 * 1024];
            let mut remaining = n;
            while remaining > 0 {
                let count = remaining.min(actual.len() as u64) as usize;
                io(decoder.read_exact(&mut actual[..count]))?;
                io(original.read_exact(&mut expected[..count]))?;
                if actual[..count] != expected[..count] {
                    return Err(corrupt("candidate does not preserve inner bytes"));
                }
                remaining -= count as u64;
            }
            if stamp(&target)? != before {
                return Err(corrupt(
                    "inner authority changed during candidate comparison",
                ));
            }
        } else {
            let mut output = io(OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target))?;
            if io(std::io::copy(&mut decoder.by_ref().take(n), &mut output))? != n {
                return Err(corrupt("truncated member"));
            }
        }
        // Decoded files are private and disposable. Durability belongs to the
        // flushed archive, not to a second uncompressed copy on disk.
        let padding = ((512 - n % 512) % 512) as usize;
        let mut pad = [0; 512];
        io(decoder.read_exact(&mut pad[..padding]))?;
        if pad[..padding].iter().any(|b| *b != 0) {
            return Err(corrupt("nonzero tar padding"));
        }
        if strict && !compare {
            validate_member(&target, name)?;
        }
    }
    if seen.len() != 19 {
        return Err(corrupt("incomplete member inventory"));
    }
    if !io(decoder.finish().fill_buf())?.is_empty() {
        return Err(corrupt("trailing zstd frames/bytes"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
