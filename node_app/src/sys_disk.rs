//! Filesystem capacity via `statvfs` (Unix) and directory tree size helpers.

use eld_common::error::EldError;
use std::path::Path;

/// Recursive sum of file sizes under `path` (directories traversed; symlinks not followed specially).
pub fn directory_tree_size_bytes(path: &Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    if path.is_file() {
        return Ok(path.metadata()?.len());
    }
    if !path.exists() {
        return Ok(0);
    }
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let p = entry.path();
        let md = entry.metadata()?;
        if md.is_dir() {
            total = total.saturating_add(directory_tree_size_bytes(&p)?);
        } else {
            total = total.saturating_add(md.len());
        }
    }
    Ok(total)
}

/// Bytes reported by `statvfs` for the filesystem containing `path`.
#[derive(Debug, Clone, Copy)]
pub struct DiskUsage {
    /// `f_blocks * f_frsize`
    pub total_bytes: u64,
    /// Unprivileged free space: `f_bavail * f_frsize`
    pub available_bytes: u64,
    /// `(f_blocks - f_bfree) * f_frsize` — bytes occupied on the volume (kernel accounting).
    pub used_bytes: u64,
}

/// `fsblkcnt_t` is `u32` on macOS and `u64` on Linux. `Into<u64>` covers both
/// without a same-type `u64::from` that Clippy rejects on Linux.
#[cfg(unix)]
fn block_count_to_u64(count: impl Into<u64>) -> u64 {
    count.into()
}

/// Returns total and available bytes on the mount containing `path`.
#[cfg(unix)]
pub fn statvfs_for_path(path: &Path) -> Result<DiskUsage, EldError> {
    use std::ffi::CString;
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStrExt;

    let c_path =
        CString::new(path.as_os_str().as_bytes()).map_err(|_| EldError::FileSystemError {
            operation: "statvfs_path".to_string(),
            path: path.display().to_string(),
            details: "Path contains NUL byte".to_string(),
        })?;

    let mut stat = MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `statvfs` writes to `stat` when it returns 0.
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) };
    if rc != 0 {
        return Err(EldError::FileSystemError {
            operation: "statvfs".to_string(),
            path: path.display().to_string(),
            details: std::io::Error::last_os_error().to_string(),
        });
    }
    let stat = unsafe { stat.assume_init() };

    let frsize = stat.f_frsize;
    let total_bytes = block_count_to_u64(stat.f_blocks).saturating_mul(frsize);
    let available_bytes = block_count_to_u64(stat.f_bavail).saturating_mul(frsize);
    let used_bytes = block_count_to_u64(stat.f_blocks)
        .saturating_sub(block_count_to_u64(stat.f_bfree))
        .saturating_mul(frsize);

    Ok(DiskUsage {
        total_bytes,
        available_bytes,
        used_bytes,
    })
}

#[cfg(not(unix))]
pub fn statvfs_for_path(_path: &Path) -> Result<DiskUsage, EldError> {
    Err(EldError::FileSystemError {
        operation: "statvfs".to_string(),
        path: String::new(),
        details: "Disk usage via statvfs is only implemented on Unix targets".to_string(),
    })
}
