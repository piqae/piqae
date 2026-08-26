use anyhow::{Context as _, Result};
use std::{fs::OpenOptions, io::Write as _, path::Path};

/// Replaces a bounded JSON journal without a delete gap. The staged file and,
/// on Unix, containing directory are synced before success is reported.
pub fn replace_json(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("durable journal has no parent")?;
    std::fs::create_dir_all(parent)?;
    let staged = path.with_extension("json.replacing");
    let _ = std::fs::remove_file(&staged);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(&staged)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    replace_file_atomically(&staged, path)?;
    #[cfg(unix)]
    std::fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .with_context(|| format!("sync {}", parent.display()))?;
    Ok(())
}

#[cfg(not(windows))]
fn replace_file_atomically(staged: &Path, destination: &Path) -> Result<()> {
    std::fs::rename(staged, destination)
        .with_context(|| format!("replace {}", destination.display()))
}

#[cfg(windows)]
#[allow(
    unsafe_code,
    reason = "isolated MoveFileExW call is required for atomic replacement on Windows"
)]
fn replace_file_atomically(staged: &Path, destination: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt as _;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let staged = staged
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both vectors are live, NUL-terminated UTF-16 strings for the
    // duration of the synchronous call.
    if unsafe {
        MoveFileExW(
            staged.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error()).context("replace durable journal");
    }
    Ok(())
}
