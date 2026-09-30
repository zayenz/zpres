//! Native filesystem operations shared by the publication policies.
//!
//! Callers retain ownership checks, commit points, and user-facing diagnostics.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(crate) enum RenameError {
    InvalidPath(PathBuf),
    Io(io::Error),
}

pub(crate) fn rename_noreplace(source: &Path, destination: &Path) -> Result<(), RenameError> {
    #[cfg(target_os = "macos")]
    let flags = libc::RENAME_EXCL;
    #[cfg(target_os = "linux")]
    let flags = libc::RENAME_NOREPLACE;
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let flags = 0;
    rename_with_flags(source, destination, flags)
}

pub(crate) fn exchange_paths(source: &Path, destination: &Path) -> Result<(), RenameError> {
    #[cfg(target_os = "macos")]
    let flags = libc::RENAME_SWAP;
    #[cfg(target_os = "linux")]
    let flags = libc::RENAME_EXCHANGE;
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let flags = 0;
    rename_with_flags(source, destination, flags)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn rename_with_flags(source: &Path, destination: &Path, flags: u32) -> Result<(), RenameError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let path_string = |path: &Path| {
        CString::new(path.as_os_str().as_bytes())
            .map_err(|_| RenameError::InvalidPath(path.to_path_buf()))
    };
    let source_c = path_string(source)?;
    let destination_c = path_string(destination)?;
    #[cfg(target_os = "macos")]
    // SAFETY: both C strings remain alive and contain no interior NUL.
    let result = unsafe {
        // libc does not expose this macOS flag yet. Preserve the existing
        // refusal to follow symlinks in either path.
        const RENAME_NOFOLLOW_ANY: u32 = 0x0000_0010;
        libc::renameatx_np(
            libc::AT_FDCWD,
            source_c.as_ptr(),
            libc::AT_FDCWD,
            destination_c.as_ptr(),
            flags | RENAME_NOFOLLOW_ANY,
        )
    };
    #[cfg(target_os = "linux")]
    let result = {
        use std::os::raw::{c_char, c_int, c_uint};
        unsafe extern "C" {
            fn renameat2(
                old_dir_fd: c_int,
                old_path: *const c_char,
                new_dir_fd: c_int,
                new_path: *const c_char,
                flags: c_uint,
            ) -> c_int;
        }
        // SAFETY: both C strings remain alive and contain no interior NUL.
        unsafe {
            renameat2(
                libc::AT_FDCWD,
                source_c.as_ptr(),
                libc::AT_FDCWD,
                destination_c.as_ptr(),
                flags,
            )
        }
    };
    syscall_result(result).map_err(RenameError::Io)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn rename_with_flags(_source: &Path, _destination: &Path, _flags: u32) -> Result<(), RenameError> {
    Err(RenameError::Io(io::Error::new(
        io::ErrorKind::Unsupported,
        "native atomic renames are implemented only for macOS and Linux",
    )))
}

pub(crate) fn atomic_operation_unsupported(source: &io::Error) -> bool {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        matches!(
            source.raw_os_error(),
            Some(libc::EINVAL | libc::ENOTSUP | libc::ENOSYS)
        )
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        source.kind() == io::ErrorKind::Unsupported
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn lock_file_nonblocking(file: &File) -> io::Result<()> {
    flock(file, libc::LOCK_EX | libc::LOCK_NB)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn lock_file_blocking(file: &File) -> io::Result<()> {
    loop {
        match flock(file, libc::LOCK_EX) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            result => return result,
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn unlock_file(file: &File) {
    let _ = flock(file, libc::LOCK_UN);
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn unlock_file(_file: &File) {}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn flock(file: &File, operation: libc::c_int) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    // SAFETY: the descriptor remains owned by `file` for the call.
    syscall_result(unsafe { libc::flock(file.as_raw_fd(), operation) })
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn syscall_result(result: libc::c_int) -> io::Result<()> {
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
