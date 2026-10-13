//! Proper temp file handling with RAII cleanup and $TMPDIR support.
//!
//! Provides:
//! - `temp_dir()`: returns the platform temp directory (respects $TMPDIR)
//! - `TempFile`: RAII guard that deletes the file on drop (even on panic/error)
//! Automatically named files live in atomically created private directories.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Global counter for generating unique temp file names within a process.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Returns the temp directory, respecting $TMPDIR (via std::env::temp_dir()).
///
/// On Unix, this checks $TMPDIR first, falling back to /tmp.
/// On other platforms, it uses the OS-appropriate temp directory.
pub fn temp_dir() -> PathBuf {
    std::env::temp_dir()
}

/// RAII guard for a file inside an exclusively created private directory.
///
/// A unique name alone is not a reservation: an attacker can pre-place a
/// symlink in a shared TMPDIR. Directory creation is atomic, and on Unix the
/// directory is mode 0700 from its creation (not a later chmod). Writers and
/// external assemblers can safely reopen the path inside that directory.
pub struct TempFile {
    path: PathBuf,
    directory: Option<PathBuf>,
    /// If true, the file and its directory are retained for debugging.
    keep: bool,
}

impl TempFile {
    /// Reserve a private directory, propagating I/O failures to the driver.
    pub fn new(prefix: &str, stem: &str, extension: &str) -> Result<Self, String> {
        Self::new_in(&temp_dir(), prefix, stem, extension)
            .map_err(|e| format!("Failed to reserve temporary file: {e}"))
    }

    fn new_in(parent: &Path, prefix: &str, stem: &str, extension: &str) -> std::io::Result<Self> {
        use std::io::{Error, ErrorKind};
        use std::time::{SystemTime, UNIX_EPOCH};
        // All caller-controlled parts must stay within the private directory.
        let filename = format!("{prefix}_{stem}.{extension}");
        if Path::new(&filename).file_name() != Some(std::ffi::OsStr::new(&filename)) {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "invalid temporary filename",
            ));
        }
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for _ in 0..128 {
            let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let directory = parent.join(format!("lccc_{}_{nanos:x}_{id:x}", std::process::id()));
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&directory) {
                Ok(()) => {
                    return Ok(Self {
                        path: directory.join(&filename),
                        directory: Some(directory),
                        keep: false,
                    });
                }
                Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(Error::new(
            ErrorKind::AlreadyExists,
            "temporary directory collision limit exceeded",
        ))
    }

    /// Guard an explicitly requested output path, not an automatically named
    /// temporary file. This is used only for the assembler's kept .s output.
    #[cfg_attr(not(feature = "gcc_assembler"), allow(dead_code))]
    pub fn with_path(path: PathBuf) -> Self {
        Self {
            path,
            directory: None,
            keep: false,
        }
    }

    /// Mark this temp file to be kept (not deleted on drop).
    /// Useful for debugging with CCC_KEEP_ASM etc.
    #[cfg_attr(not(feature = "gcc_assembler"), allow(dead_code))] // Used by gcc_assembler's CCC_KEEP_ASM path
    pub fn set_keep(&mut self, keep: bool) {
        self.keep = keep;
    }

    /// Get the path as a Path reference.
    #[cfg_attr(not(feature = "gcc_assembler"), allow(dead_code))] // Used by gcc_assembler's assemble_with_extra()
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get the path as a string slice (panics if not valid UTF-8).
    pub fn to_str(&self) -> &str {
        self.path
            .to_str()
            .expect("temp file path is not valid UTF-8")
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if !self.keep {
            // Silently ignore errors (file may not have been created).
            let _ = std::fs::remove_file(&self.path);
            if let Some(directory) = &self.directory {
                let _ = std::fs::remove_dir(directory);
            }
        }
    }
}

impl std::fmt::Display for TempFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.path.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_temp_cleanup_and_permissions() {
        let file = TempFile::new("test", "private", "o").unwrap();
        let path = file.path().to_path_buf();
        let directory = path.parent().unwrap().to_path_buf();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        std::fs::write(&path, b"object").unwrap();
        drop(file);
        assert!(!path.exists());
        assert!(!directory.exists());
    }

    #[test]
    fn private_temp_keep_and_uniqueness() {
        let mut a = TempFile::new("test", "same", "o").unwrap();
        let b = TempFile::new("test", "same", "o").unwrap();
        assert_ne!(a.path(), b.path());
        let directory = a.path().parent().unwrap().to_path_buf();
        a.set_keep(true);
        drop(a);
        assert!(directory.is_dir());
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn private_temp_rejects_escape_and_missing_parent() {
        assert!(TempFile::new("test", "../../victim", "o").is_err());
        let parent = TempFile::new("test", "parent", "o").unwrap();
        // The file path is not a directory, and cannot be used as a TMPDIR.
        std::fs::write(parent.path(), b"not a directory").unwrap();
        assert!(TempFile::new_in(parent.path(), "test", "child", "o").is_err());
    }
}
