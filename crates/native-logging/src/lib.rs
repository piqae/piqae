use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub const DEFAULT_MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;
pub const DEFAULT_RETAINED_LOGS: usize = 4;

#[derive(Clone, Debug)]
pub struct BoundedLogWriter {
    inner: Arc<Mutex<LogState>>,
}

#[derive(Debug)]
struct LogState {
    path: PathBuf,
    max_bytes: u64,
    retained: usize,
    file: Option<File>,
    length: u64,
}

impl BoundedLogWriter {
    /// Opens an append-only active log with bounded rotated generations.
    ///
    /// # Errors
    ///
    /// Returns an error when the bound is zero, the parent directory cannot be
    /// created, or the active log cannot be opened.
    pub fn open(path: impl Into<PathBuf>, max_bytes: u64, retained: usize) -> io::Result<Self> {
        if max_bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "maximum log size must be greater than zero",
            ));
        }
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let (file, length) = open_active_log(&path)?;
        let mut state = LogState {
            path,
            max_bytes,
            retained,
            file: Some(file),
            length,
        };
        if length > max_bytes {
            state.rotate()?;
        }
        Ok(Self {
            inner: Arc::new(Mutex::new(state)),
        })
    }

    /// Opens a log using Piqae's native-package retention policy.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory or active file cannot be opened.
    pub fn open_with_defaults(path: impl Into<PathBuf>) -> io::Result<Self> {
        Self::open(path, DEFAULT_MAX_LOG_BYTES, DEFAULT_RETAINED_LOGS)
    }
}

impl Write for BoundedLogWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("native log writer lock was poisoned"))?;
        state.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner
            .lock()
            .map_err(|_| io::Error::other("native log writer lock was poisoned"))?
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("native log file is unavailable"))?
            .flush()
    }
}

impl<'writer> tracing_subscriber::fmt::MakeWriter<'writer> for BoundedLogWriter {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

impl LogState {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let incoming = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        if self.length > 0 && self.length.saturating_add(incoming) > self.max_bytes {
            self.rotate()?;
        }
        let writable = usize::try_from(self.max_bytes)
            .unwrap_or(usize::MAX)
            .min(buffer.len());
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("native log file is unavailable"))?;
        file.write_all(&buffer[..writable])?;
        self.length = self
            .length
            .saturating_add(u64::try_from(writable).unwrap_or(self.max_bytes));
        Ok(buffer.len())
    }

    fn rotate(&mut self) -> io::Result<()> {
        if let Some(mut file) = self.file.take() {
            if self.length > self.max_bytes {
                file.set_len(self.max_bytes)?;
            }
            file.flush()?;
            file.sync_data()?;
        }
        if self.retained == 0 {
            remove_if_present(&self.path)?;
        } else {
            remove_if_present(&rotated_path(&self.path, self.retained))?;
            for generation in (1..self.retained).rev() {
                rename_if_present(
                    &rotated_path(&self.path, generation),
                    &rotated_path(&self.path, generation + 1),
                )?;
            }
            rename_if_present(&self.path, &rotated_path(&self.path, 1))?;
        }
        let (file, length) = open_active_log(&self.path)?;
        self.file = Some(file);
        self.length = length;
        Ok(())
    }
}

fn open_active_log(path: &Path) -> io::Result<(File, u64)> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    let length = file.metadata()?.len();
    Ok((file, length))
}

fn rotated_path(path: &Path, generation: usize) -> PathBuf {
    let mut rotated = path.as_os_str().to_owned();
    rotated.push(format!(".{generation}"));
    PathBuf::from(rotated)
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn rename_if_present(source: &Path, destination: &Path) -> io::Result<()> {
    match std::fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn rotates_at_the_bound_and_removes_the_oldest_generation() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("agent.log");
        let mut writer = BoundedLogWriter::open(&path, 5, 2).expect("writer");

        writer.write_all(b"aaaa\n").expect("first record");
        writer.write_all(b"bbbb\n").expect("second record");
        writer.write_all(b"cccc\n").expect("third record");
        writer.write_all(b"dddd\n").expect("fourth record");
        writer.flush().expect("flush");

        assert_eq!(std::fs::read(&path).expect("active"), b"dddd\n");
        assert_eq!(
            std::fs::read(rotated_path(&path, 1)).expect("first generation"),
            b"cccc\n"
        );
        assert_eq!(
            std::fs::read(rotated_path(&path, 2)).expect("second generation"),
            b"bbbb\n"
        );
        assert!(!rotated_path(&path, 3).exists());
    }

    #[test]
    fn preserves_an_existing_log_across_upgrade_startup() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("agent.log");
        std::fs::write(&path, b"before-upgrade\n").expect("seed log");
        let mut writer = BoundedLogWriter::open(&path, 64, 2).expect("writer");
        writer.write_all(b"after-upgrade\n").expect("append");
        writer.flush().expect("flush");

        assert_eq!(
            std::fs::read(&path).expect("active"),
            b"before-upgrade\nafter-upgrade\n"
        );
    }

    #[test]
    fn bounds_an_oversized_legacy_log_during_upgrade_startup() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("agent.log");
        std::fs::write(&path, b"legacy-unbounded-log").expect("seed log");

        let mut writer = BoundedLogWriter::open(&path, 6, 1).expect("writer");
        writer.write_all(b"new\n").expect("append");
        writer.flush().expect("flush");

        assert_eq!(std::fs::metadata(&path).expect("active").len(), 4);
        assert_eq!(
            std::fs::metadata(rotated_path(&path, 1))
                .expect("generation")
                .len(),
            6
        );
    }

    #[test]
    fn oversized_single_record_is_retained_without_an_infinite_rotation_loop() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("shell.log");
        let mut writer = BoundedLogWriter::open(&path, 4, 1).expect("writer");
        writer
            .write_all(b"bounded-write")
            .expect("oversized record");
        writer.write_all(b"next").expect("next record");
        writer.flush().expect("flush");

        assert_eq!(std::fs::read(&path).expect("active"), b"next");
        assert_eq!(
            std::fs::read(rotated_path(&path, 1)).expect("generation"),
            b"boun"
        );
    }
}
