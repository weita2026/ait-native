//! Transient Windows handles only; no execution or cleanup authority is stored.
use std::ffi::{c_void, OsStr};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Component, Path, PathBuf};
use std::ptr::{null, null_mut};
use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows_sys::Wdk::Storage::FileSystem::{
    NtCreateFile, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_REPARSE_POINT,
    FILE_SYNCHRONOUS_IO_NONALERT,
};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
use windows_sys::Win32::Security::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::SystemServices::{
    ACCESS_ALLOWED_ACE_TYPE, ACCESS_DENIED_ACE_TYPE, MAXIMUM_ALLOWED,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

fn refused(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

// DWORD alignment is sufficient for a SID; token/descriptor pointers never outlive
// their owning buffers. Every SID supplied to EqualSid is validated first.
struct Sid(Vec<u32>);
impl Sid {
    fn pointer(&self) -> PSID {
        self.0.as_ptr().cast_mut().cast()
    }
    unsafe fn copy(pointer: PSID) -> io::Result<Self> {
        if pointer.is_null() || unsafe { IsValidSid(pointer) } == 0 {
            return Err(refused("Managed runtime owner SID is unavailable"));
        }
        let length = unsafe { GetLengthSid(pointer) };
        if length > SECURITY_MAX_SID_SIZE {
            return Err(refused("Managed runtime SID exceeds its native limit"));
        }
        let sid = Self(vec![0; (length as usize).div_ceil(size_of::<u32>())]);
        if unsafe { CopySid(length, sid.pointer(), pointer) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(sid)
    }
    fn well_known(kind: WELL_KNOWN_SID_TYPE) -> io::Result<Self> {
        let sid = Self(vec![0; (SECURITY_MAX_SID_SIZE as usize).div_ceil(4)]);
        let mut length = SECURITY_MAX_SID_SIZE;
        // SAFETY: sid is an aligned output buffer of the declared size.
        if unsafe { CreateWellKnownSid(kind, null_mut(), sid.pointer(), &mut length) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(sid)
    }
    fn equals(&self, other: PSID) -> bool {
        // SAFETY: callers pass SIDs validated within a live native buffer.
        unsafe { EqualSid(self.pointer(), other) != 0 }
    }
}

struct Principals {
    user: Sid,
    owner: Sid,
    system: Sid,
    administrators: Sid,
    creator_owner: Sid,
}
impl Principals {
    fn current() -> io::Result<Self> {
        let mut token = null_mut();
        // SAFETY: the process pseudo-handle is valid and token is writable.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: OpenProcessToken returned a new owned handle.
        let token = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(token) };
        fn information(token: HANDLE, class: TOKEN_INFORMATION_CLASS) -> io::Result<Sid> {
            let mut length = 0;
            // SAFETY: a zero-length query only writes the required length.
            unsafe { GetTokenInformation(token, class, null_mut(), 0, &mut length) };
            if length == 0
                || io::Error::last_os_error().raw_os_error()
                    != Some(ERROR_INSUFFICIENT_BUFFER as i32)
            {
                return Err(io::Error::last_os_error());
            }
            let mut buffer = vec![0_usize; (length as usize).div_ceil(size_of::<usize>())];
            // SAFETY: buffer is pointer-aligned and has at least length bytes.
            if unsafe {
                GetTokenInformation(
                    token,
                    class,
                    buffer.as_mut_ptr().cast(),
                    length,
                    &mut length,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            // TOKEN_USER and TOKEN_OWNER both start with a SID pointer.
            let pointer = unsafe { *buffer.as_ptr().cast::<PSID>() };
            // SAFETY: the returned token buffer remains live during the copy.
            unsafe { Sid::copy(pointer) }
        }
        Ok(Self {
            user: information(token.as_raw_handle(), TokenUser)?,
            owner: information(token.as_raw_handle(), TokenOwner)?,
            system: Sid::well_known(WinLocalSystemSid)?,
            administrators: Sid::well_known(WinBuiltinAdministratorsSid)?,
            creator_owner: Sid::well_known(WinCreatorOwnerSid)?,
        })
    }
    fn trusted(&self, sid: PSID) -> bool {
        [&self.user, &self.owner, &self.system, &self.administrators]
            .iter()
            .any(|principal| principal.equals(sid))
    }
}

struct Descriptor(PSECURITY_DESCRIPTOR);
impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: GetSecurityInfo allocated this descriptor with LocalAlloc.
        unsafe { LocalFree(self.0) };
    }
}

fn owned(file: &File, principals: &Principals, expected: Option<&Sid>) -> io::Result<Sid> {
    let mut owner = null_mut();
    let mut acl = null_mut();
    let mut descriptor = null_mut();
    // SAFETY: file owns a live handle; output pointers are valid for this call.
    let result = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut acl,
            null_mut(),
            &mut descriptor,
        )
    };
    if result != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(result as i32));
    }
    let _descriptor = Descriptor(descriptor);
    // SAFETY: owner belongs to the live returned descriptor.
    let owner_copy = unsafe { Sid::copy(owner) }?;
    if !principals.user.equals(owner) && !principals.owner.equals(owner)
        || expected.is_some_and(|expected| !expected.equals(owner))
    {
        return Err(refused(
            "Managed runtime owner differs from the Server token/root",
        ));
    }
    // A null DACL permits everybody. An empty DACL permits nobody.
    if acl.is_null() || unsafe { IsValidAcl(acl) } == 0 {
        return Err(refused("Managed runtime DACL is absent or invalid"));
    }
    let mutable = FILE_WRITE_DATA
        | FILE_APPEND_DATA
        | FILE_WRITE_EA
        | FILE_WRITE_ATTRIBUTES
        | FILE_DELETE_CHILD
        | DELETE
        | WRITE_DAC
        | WRITE_OWNER
        | GENERIC_WRITE
        | GENERIC_ALL
        | MAXIMUM_ALLOWED;
    // SAFETY: IsValidAcl validated the ACL header and native GetAce validates entries.
    for index in 0..unsafe { (*acl).AceCount } {
        let mut ace = null_mut();
        if unsafe { GetAce(acl, u32::from(index), &mut ace) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let header = unsafe { ace.cast::<ACE_HEADER>().read_unaligned() };
        if u32::from(header.AceType) == ACCESS_DENIED_ACE_TYPE {
            continue;
        }
        if u32::from(header.AceType) != ACCESS_ALLOWED_ACE_TYPE || header.AceSize < 16 {
            return Err(refused("Managed runtime has an unverifiable grant ACE"));
        }
        let mask = unsafe { ace.cast::<ACCESS_ALLOWED_ACE>().read_unaligned() }.Mask;
        if mask & mutable == 0 {
            continue;
        }
        let sid = unsafe { ace.cast::<u8>().add(8).cast::<c_void>() };
        let sub_authorities = unsafe { *sid.cast::<u8>().add(1) };
        if sub_authorities > 15
            || 16 + 4 * u16::from(sub_authorities) > header.AceSize
            || unsafe { IsValidSid(sid) } == 0
            || unsafe { GetLengthSid(sid) } + 8 > u32::from(header.AceSize)
        {
            return Err(refused("Managed runtime grant SID is invalid"));
        }
        let inherited_owner = u32::from(header.AceFlags) & INHERIT_ONLY_ACE != 0
            && principals.creator_owner.equals(sid);
        if !principals.trusted(sid) && !inherited_owner {
            return Err(refused(
                "Managed runtime grants mutation to a foreign principal",
            ));
        }
    }
    Ok(owner_copy)
}

fn ordinary_name(name: &OsStr) -> io::Result<Vec<u16>> {
    let wide = name.encode_wide().collect::<Vec<_>>();
    if wide.is_empty()
        || wide.len() > usize::from(u16::MAX) / 2
        || wide.iter().any(|c| matches!(*c, 0 | 47 | 58 | 92))
        || matches!(wide.last(), Some(32 | 46))
        || name == "."
        || name == ".."
    {
        return Err(refused("Managed runtime path has an unsafe component"));
    }
    Ok(wide)
}

fn real_handle(file: &File, directory: bool) -> io::Result<()> {
    let mut attributes = FILE_ATTRIBUTE_TAG_INFO::default();
    // SAFETY: attributes is a writable buffer of the declared native type/size.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileAttributeTagInfo,
            (&mut attributes as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if attributes.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || (attributes.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0) != directory
    {
        return Err(refused(
            "Managed runtime handle is linked or has the wrong type",
        ));
    }
    Ok(())
}

fn child(parent: &File, name: &OsStr, directory: bool, delete: bool) -> io::Result<File> {
    let mut wide = ordinary_name(name)?;
    let mut name = UNICODE_STRING {
        Length: (wide.len() * 2) as u16,
        MaximumLength: (wide.len() * 2) as u16,
        Buffer: wide.as_mut_ptr(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.as_raw_handle(),
        ObjectName: &mut name,
        Attributes: OBJ_CASE_INSENSITIVE,
        ..Default::default()
    };
    let mut status = IO_STATUS_BLOCK::default();
    let mut handle = null_mut();
    let access = READ_CONTROL
        | FILE_READ_ATTRIBUTES
        | SYNCHRONIZE
        | if directory {
            FILE_LIST_DIRECTORY
        } else {
            FILE_READ_DATA
        }
        | if delete { DELETE } else { 0 };
    // SAFETY: all buffers/parent handle live through this synchronous call. One
    // relative component plus OPEN_REPARSE_POINT prevents intermediate traversal.
    let result = unsafe {
        NtCreateFile(
            &mut handle,
            access,
            &attributes,
            &mut status,
            null(),
            0,
            if delete {
                0
            } else if directory {
                FILE_SHARE_READ | FILE_SHARE_WRITE
            } else {
                FILE_SHARE_READ
            },
            FILE_OPEN,
            FILE_OPEN_REPARSE_POINT
                | FILE_SYNCHRONOUS_IO_NONALERT
                | if directory {
                    FILE_DIRECTORY_FILE
                } else {
                    FILE_NON_DIRECTORY_FILE
                },
            null(),
            0,
        )
    };
    if result < 0 {
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(result) } as i32,
        ));
    }
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(refused("Native runtime open returned no verifiable handle"));
    }
    // SAFETY: NtCreateFile returned a new owned file handle.
    let file = unsafe { File::from_raw_handle(handle) };
    real_handle(&file, directory)?;
    Ok(file)
}

struct Scope {
    handles: Vec<File>,
    principals: Principals,
    owner: Sid,
}
impl Scope {
    fn open(root: &Path) -> io::Result<Self> {
        let mut components = root.components();
        let Some(Component::Prefix(prefix)) = components.next() else {
            return Err(refused("Managed runtime root is not absolute"));
        };
        if components.next() != Some(Component::RootDir) {
            return Err(refused("Managed runtime root has no volume anchor"));
        }
        let mut anchor = PathBuf::from(prefix.as_os_str());
        anchor.push(std::path::MAIN_SEPARATOR_STR);
        let file = OpenOptions::new()
            .access_mode(READ_CONTROL | FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY | SYNCHRONIZE)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(anchor)?;
        real_handle(&file, true)?;
        let mut handles = vec![file];
        // Pin all absolute ancestors against rename, but enforce Server ownership
        // only from the configured managed root (not C:\ or the user profile).
        for component in components {
            let Component::Normal(name) = component else {
                return Err(refused("Managed runtime root contains traversal"));
            };
            handles.push(child(handles.last().unwrap(), name, true, false)?);
        }
        let principals = Principals::current()?;
        let owner = owned(handles.last().unwrap(), &principals, None)?;
        Ok(Self {
            handles,
            principals,
            owner,
        })
    }
    fn push(&mut self, name: &OsStr, directory: bool, delete: bool) -> io::Result<()> {
        let file = child(self.handles.last().unwrap(), name, directory, delete)?;
        owned(&file, &self.principals, Some(&self.owner))?;
        self.handles.push(file);
        Ok(())
    }
}

fn relative<'a>(root: &Path, path: &'a Path) -> io::Result<Vec<&'a OsStr>> {
    let path = path
        .strip_prefix(root)
        .map_err(|_| refused("Runtime path escaped managed root"))?;
    let components = path
        .components()
        .map(|component| match component {
            Component::Normal(name) => Ok(name),
            _ => Err(refused("Runtime path contains traversal")),
        })
        .collect::<io::Result<Vec<_>>>()?;
    if components.is_empty() {
        return Err(refused("Runtime root is not a dedicated child"));
    }
    Ok(components)
}

/// Keeps verified directories unrenamable and the archive unwritable while read.
pub struct WindowsManagedSource {
    scope: Scope,
    parent_count: usize,
    archive: File,
}
pub struct WindowsRuntimeParents {
    _scope: Scope,
}
impl WindowsManagedSource {
    pub fn read_package(&self) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        (&self.archive).read_to_end(&mut bytes)?;
        Ok(bytes)
    }
    /// Existing owned recursive cleanup needs DELETE on its Job. Keep every
    /// ancestor pinned; std::fs::remove_dir_all uses Windows no-follow deletion.
    pub fn retain_parents(mut self) -> WindowsRuntimeParents {
        self.scope.handles.truncate(self.parent_count);
        WindowsRuntimeParents { _scope: self.scope }
    }
}

pub fn verify_managed_runtime_source(root: &Path, path: &Path) -> io::Result<WindowsManagedSource> {
    let components = relative(root, path)?;
    let mut scope = Scope::open(root)?;
    let parent_count = scope.handles.len() + components.len() - 1;
    for component in components {
        scope.push(component, true, false)?;
    }
    let directory = scope.handles.last().unwrap().try_clone()?;
    let source = child(&directory, OsStr::new("source"), true, false)?;
    owned(&source, &scope.principals, Some(&scope.owner))?;
    let archive = child(&directory, OsStr::new("package.zst"), false, false)?;
    owned(&archive, &scope.principals, Some(&scope.owner))?;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        match entry.file_name().to_str() {
            Some("package.zst" | "source") => {}
            Some("inputs" | "publication-inputs") => {
                let input = child(&directory, &entry.file_name(), true, false)?;
                owned(&input, &scope.principals, Some(&scope.owner))?;
                // Keep each direct child pinned through source verification.
                scope.handles.push(input);
            }
            _ => {
                return Err(refused(
                    "Retained source contains foreign runtime resources",
                ))
            }
        }
    }
    scope.handles.push(source);
    Ok(WindowsManagedSource {
        scope,
        parent_count,
        archive,
    })
}

pub(super) fn remove_empty(
    root: &Path,
    path: &Path,
    before_remove: impl FnOnce() -> io::Result<()>,
) -> io::Result<bool> {
    let components = relative(root, path)?;
    let mut scope = Scope::open(root)?;
    for component in &components {
        if let Err(error) = scope.push(component, true, false) {
            if error.kind() == io::ErrorKind::NotFound {
                return Ok(true);
            }
            return Err(error);
        }
    }
    if fs::read_dir(path)?.next().transpose()?.is_some() {
        return Ok(false);
    }
    // Reopen the exact leaf with DELETE after closing our read-only leaf. All
    // ancestors remain pinned. Any replacement is verified again, not adopted
    // from a prior identity, and disposition still checks emptiness atomically.
    scope.handles.pop();
    scope.push(components.last().unwrap(), true, true)?;
    before_remove()?;
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: exact no-follow directory handle with DELETE access. Windows
    // atomically refuses a nonempty directory; no file/recursive fallback exists.
    if unsafe {
        SetFileInformationByHandle(
            scope.handles.last().unwrap().as_raw_handle(),
            FileDispositionInfo,
            (&disposition as *const FILE_DISPOSITION_INFO).cast(),
            size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    } == 0
    {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_DIR_NOT_EMPTY as i32) {
            return Ok(false);
        }
        return Err(error);
    }
    // Close the delete-pending child before reporting physical removal. This
    // exclusive leaf open also refuses existing data/list readers or writers,
    // including handles that would otherwise permit delete sharing.
    scope.handles.pop();
    Ok(true)
}

#[cfg(test)]
#[path = "windows_fs/tests.rs"]
mod tests;
