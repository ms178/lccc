//! Proper temp file handling with RAII cleanup and $TMPDIR support.
//!
//! Provides:
//! - `temp_dir()`: returns the platform temp directory (respects $TMPDIR)
//! - `TempFile`: RAII guard that deletes the file on drop (even on panic/error)
//!
//! # Why per-file private directories (AUD-TMP-1)
//!
//! Temp paths used to be `$TMPDIR/<prefix>_<pid>_<stem>.<n>.<ext>`: predictable
//! apart from the pid, and created with a plain open that follows symlinks. A
//! local user who pre-placed a symlink at the predicted path (or a real
//! directory) made the compiler write *through* it — overwriting any file the
//! invoking user could write:
//!
//! ```text
//! ln -s ~/victim.o /tmp/ccc_<pid>_big.0.o && TMPDIR=/tmp exec lccc -O1 big.c
//! ```
//!
//! Every temp file now lives alone in `$TMPDIR/lccc-<pid>-<nanos>-<id>/`,
//! created with `mkdir` mode 0700 and never reused: an attacker cannot
//! pre-create it (a pre-placed file/symlink/directory makes `mkdir` fail with
//! `EEXIST`, and we retry with the next candidate), and cannot write into it
//! afterwards, because it is owned by us and mode 0700. The directory dies
//! with its one file (`TempFile::drop`), so a compile leaves no debris behind,
//! and two compiler threads can never race over a shared directory.
//!
//! If no private directory can be created at all (read-only or missing
//! `$TMPDIR`, exotic filesystems), the error propagates to the driver as a
//! normal "Failed to reserve temporary file" diagnostic — compilation fails
//! closed instead of silently reverting to attacker-reachable flat names.

use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Global counter for generating unique temp file names within a process.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Number of `mkdir` candidates tried before giving up on the private dir.
const PRIVATE_DIR_ATTEMPTS: u32 = 128;

/// Returns the temp directory, respecting $TMPDIR (via std::env::temp_dir()).
///
/// On Unix, this checks $TMPDIR first, falling back to /tmp.
/// On other platforms, it uses the OS-appropriate temp directory.
pub fn temp_dir() -> PathBuf {
    std::env::temp_dir()
}

/// Nanosecond stamp used to make the private directory name unpredictable: an
/// attacker cannot create the directory in advance (it does not exist yet) and
/// cannot guess it in time to swap it under us.
fn unique_stamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Create one candidate private directory with mode 0700 and prove we own it.
///
/// `DirBuilder::create` fails with `AlreadyExists` when *anything* occupies
/// the path (including a symlink, or a directory owned by another user), so a
/// pre-placed path can never be adopted. Ownership is then proven without
/// `libc`: an `O_CREAT|O_EXCL` probe file is created inside the directory and
/// the directory's uid must equal the probe file's uid — a directory owned by
/// somebody else (which a world-writable mount could still accept writes
/// into) is rejected.
fn create_private_dir_at(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};

    // 0700: only this process (and root) may look inside. `mkdir` applies the
    // umask, so set the mode explicitly afterwards — we own the directory the
    // moment the create returns.
    std::fs::DirBuilder::new().mode(0o700).create(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;

    let probe = path.join(".probe");
    let created = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true) // O_CREAT|O_EXCL: never follows a symlink
        .mode(0o600)
        .open(&probe);
    match created {
        Ok(_) => {
            let dir_uid = std::fs::metadata(path)?.uid();
            let probe_uid = std::fs::metadata(&probe)?.uid();
            let _ = std::fs::remove_file(&probe);
            if dir_uid != probe_uid {
                let _ = std::fs::remove_dir(path);
                return Err(Error::new(
                    ErrorKind::PermissionDenied,
                    "private temp directory owned by another user",
                ));
            }
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_dir(path);
            Err(e)
        }
    }
}

/// Reserve a fresh per-file private directory and return it.
fn reserve_private_dir() -> std::io::Result<PathBuf> {
    let parent = temp_dir();
    for _ in 0..PRIVATE_DIR_ATTEMPTS {
        let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(
            "lccc-{}-{:x}-{:x}",
            std::process::id(),
            unique_stamp(),
            id
        ));
        match create_private_dir_at(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(Error::new(
        ErrorKind::AlreadyExists,
        "temporary directory collision limit exceeded",
    ))
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
        // All caller-controlled parts must stay within the private directory.
        let filename = format!("{prefix}_{stem}.{extension}");
        if Path::new(&filename).file_name() != Some(std::ffi::OsStr::new(&filename)) {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "invalid temporary filename",
            ));
        }
        // The per-file private directory is created under `parent` (the
        // platform temp dir in the normal case; the unit tests exercise other
        // parents). reserve_private_dir() uses temp_dir(); when parent differs,
        // build the directory here with the same discipline.
        let directory = if parent == temp_dir() {
            reserve_private_dir()?
        } else {
            let mut created = None;
            for _ in 0..PRIVATE_DIR_ATTEMPTS {
                let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
                let candidate = parent.join(format!(
                    "lccc-{}-{:x}-{:x}",
                    std::process::id(),
                    unique_stamp(),
                    id
                ));
                match create_private_dir_at(&candidate) {
                    Ok(()) => {
                        created = Some(candidate);
                        break;
                    }
                    Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
                    Err(e) => return Err(e),
                }
            }
            created.ok_or_else(|| {
                Error::new(
                    ErrorKind::AlreadyExists,
                    "temporary directory collision limit exceeded",
                )
            })?
        };
        Ok(Self {
            path: directory.join(&filename),
            directory: Some(directory),
            keep: false,
        })
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
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn private_temp_rejects_escape_and_missing_parent() {
        assert!(TempFile::new("test", "../../victim", "o").is_err());
        let parent = TempFile::new("test", "parent", "o").unwrap();
        // The file path is not a directory, and cannot be used as a TMPDIR.
        std::fs::write(parent.path(), b"not a directory").unwrap();
        assert!(TempFile::new_in(parent.path(), "test", "child", "o").is_err());
    }

    #[test]
    fn private_dir_is_never_adopted_from_a_preplaced_path() {
        // Pre-place a symlink at the exact name shape the generator uses and
        // verify a later reservation still gets a directory it owns — the
        // `mkdir` EEXIST retry must skip the poisoned candidate (AUD-TMP-1).
        let parent = temp_dir();
        let mut adopted = 0;
        for _ in 0..8 {
            let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let candidate = parent.join(format!(
                "lccc-{}-{:x}-{:x}",
                std::process::id(),
                unique_stamp(),
                id
            ));
            // Occupy the name with a symlink to a directory we control.
            let victim = parent.join(format!("lccc-victim-{}", std::process::id()));
            let _ = std::fs::create_dir(&victim);
            std::os::unix::fs::symlink(&victim, &candidate).unwrap();
            match create_private_dir_at(&candidate) {
                Ok(()) => adopted += 1, // BAD: adopted the pre-placed path
                Err(_) => {}
            }
            let _ = std::fs::remove_file(&candidate);
            let _ = std::fs::remove_dir(&victim);
        }
        assert_eq!(adopted, 0, "pre-placed symlink was adopted as a private dir");
    }
}
