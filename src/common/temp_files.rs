//! Proper temp file handling with RAII cleanup and $TMPDIR support.
//!
//! Provides:
//! - `temp_dir()`: returns the platform temp directory (respects $TMPDIR)
//! - `private_temp_dir()`: a persistent 0700 directory for unmanaged/debug paths
//! - `TempFile`: RAII guard that deletes the file on drop (even on panic/error)
//! - `write_file_exclusive()`: symlink-safe write for fixed-purpose debug dumps
//!
//! # Why a private directory (AUD-TMP-1)
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
//! Managed temporary files live in `$TMPDIR/lccc-<pid>-<nanos>-<n>/`, created with
//! `mkdir` mode 0700 and never reused: an attacker cannot pre-create it (a
//! pre-placed file/symlink/directory makes `mkdir` fail with `EEXIST`, and we
//! retry with the next candidate), and cannot write into it afterwards, because
//! it is owned by us and mode 0700. The directory is removed when the last
//! `TempFile` guard drops, so a compile leaves no debris behind.
//!
//! If no private directory can be created, path-reserving APIs fail closed.
//! A read-only public directory can still contain a symlink to a writable file;
//! falling back to predictable flat names would follow that symlink. Optional
//! debug dumps instead return None. A kept path retains its directory even if
//! the file has not yet been created; the recipient then owns cleanup.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

/// Global counter for generating unique temp file names within a process.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Number of `mkdir` candidates tried before giving up on the private dir.
const PRIVATE_DIR_ATTEMPTS: u32 = 64;

/// Returns the temp directory, respecting $TMPDIR (via std::env::temp_dir()).
///
/// On Unix, this checks $TMPDIR first, falling back to /tmp.
/// On other platforms, it uses the OS-appropriate temp directory.
pub fn temp_dir() -> PathBuf {
    std::env::temp_dir()
}

/// Timestamp used to reduce accidental name collisions, not a security secret.
/// Exclusive mkdir and private permissions provide the ownership guarantee.
fn unique_stamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Create one candidate private directory with mode 0700 and prove we own it.
///
/// `DirBuilder::create` fails with `AlreadyExists` when *anything* occupies the
/// path (including a symlink, or a directory owned by another user), so a
/// pre-placed path can never be adopted. Ownership is then proven without
/// `libc`: a `O_CREAT|O_EXCL` probe file is created inside the directory and
/// the directory's uid must equal the probe file's uid — a directory owned by
/// somebody else (which a world-writable mount could still accept writes into)
/// is rejected, and `mkdir` on top of it would have failed anyway.
#[cfg(unix)]
fn create_private_dir_at(path: &Path) -> bool {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};

    // 0700: only this process (and root) may look inside. `mkdir` applies the
    // umask, so set the mode explicitly afterwards — we own the directory the
    // moment the create returns.
    if fs::DirBuilder::new().mode(0o700).create(path).is_err() {
        return false;
    }
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));

    let probe = path.join(".probe");
    let created = fs::OpenOptions::new()
        .write(true)
        .create_new(true) // O_CREAT|O_EXCL: never follows a symlink
        .mode(0o600)
        .open(&probe);
    let ok = match created {
        Ok(_) => {
            let dir_meta = fs::symlink_metadata(path);
            let file_meta = fs::symlink_metadata(&probe);
            let _ = fs::remove_file(&probe);
            match (dir_meta, file_meta) {
                (Ok(d), Ok(f)) => d.is_dir() && d.uid() == f.uid() && d.mode() & 0o077 == 0,
                _ => false,
            }
        }
        Err(_) => false,
    };
    if !ok {
        // Best effort: only removes a directory we created or already emptied.
        let _ = fs::remove_dir(path);
    }
    ok
}

/// Non-Unix fallback: the platform temp directory has no mode bits to set and
/// POSIX symlink attacks do not apply in the same way; `create_dir` is still
/// non-reusing, so a pre-placed path is rejected.
#[cfg(not(unix))]
fn create_private_dir_at(path: &Path) -> bool {
    fs::create_dir(path).is_ok()
}

/// Create a fresh private directory; callers decide its lifetime.
fn create_private_dir() -> Option<PathBuf> {
    let base = temp_dir();
    let pid = std::process::id();
    let nanos = unique_stamp();
    for attempt in 0..PRIVATE_DIR_ATTEMPTS {
        let name = format!("lccc-{}-{:x}-{}", pid, nanos, attempt);
        let candidate = base.join(name);
        if create_private_dir_at(&candidate) {
            if std::env::var_os("CCC_TEMP_DEBUG").is_some() {
                eprintln!("[ccc] private temp dir: {}", candidate.display());
            }
            return Some(candidate);
        }
        // `create_private_dir_at` is all-or-nothing: it removes its own
        // candidate when verification fails, so the next attempt starts clean.
    }
    // Never substitute an unowned flat path: even an unwritable directory can
    // contain attacker-controlled symlinks to files the compiler can overwrite.
    eprintln!(
        "ccc: warning: no private temp directory could be created under {}; \
         refusing insecure flat temp-file names",
        base.display()
    );
    None
}

/// The process-private temp directory, or `None` if it could not be created.
///
/// Created lazily for unmanaged/debug paths. Never reaped by a TempFile guard;
/// its caller may write the returned path later. Managed files use leases below.
pub fn private_temp_dir() -> Option<&'static Path> {
    static PRIVATE_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    PRIVATE_DIR.get_or_init(create_private_dir).as_deref()
}

/// A generation is owned by live guards, including paths not yet materialized.
/// Removing an empty directory on *every* file drop races those pending paths;
/// caching the removed path in OnceLock also breaks sequential generations.
struct ManagedTempDirectory {
    path: PathBuf,
    retained: AtomicBool,
}

impl Drop for ManagedTempDirectory {
    fn drop(&mut self) {
        // A kept pending path transfers directory cleanup to its recipient.
        // Never recursively delete unowned files or a retained directory.
        if !self.retained.load(Ordering::Acquire) {
            let _ = fs::remove_dir(&self.path);
        }
    }
}

#[derive(Default)]
struct TempDirectoryPool {
    current: Mutex<Weak<ManagedTempDirectory>>,
}

impl TempDirectoryPool {
    fn reserve(&self, prefix: &str, stem: &str, extension: &str) -> io::Result<TempFile> {
        if [prefix, stem, extension]
            .iter()
            .any(|part| part.contains('\0') || part.chars().any(std::path::is_separator))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "temporary-file name components must not contain separators or NUL",
            ));
        }
        let mut current = self
            .current
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let directory = current.upgrade().or_else(|| {
            let owned = Arc::new(ManagedTempDirectory {
                path: create_private_dir()?,
                retained: AtomicBool::new(false),
            });
            *current = Arc::downgrade(&owned);
            Some(owned)
        });
        let directory = directory
            .ok_or_else(|| io::Error::other("cannot reserve a secure temporary-file directory"))?;
        let filename = unique_filename(prefix, stem, extension);
        let path = directory.path.join(filename);
        Ok(TempFile {
            path,
            keep: false,
            _directory: Some(directory),
        })
    }
}

fn unique_filename(prefix: &str, stem: &str, extension: &str) -> String {
    let pid = std::process::id();
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}_{}_{}.{}.{}", prefix, pid, stem, id, extension)
}

/// Write `contents` to a fresh file at `path` without following symlinks.
///
/// `create_new(true)` maps to `O_CREAT|O_EXCL`, which fails on an existing
/// path — including a symlink — instead of writing through it. Used for dumps
/// whose purpose is fixed (so they cannot simply move into the private
/// directory without losing their discoverability).
pub fn write_file_exclusive(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // 0600: the dump is for the invoking user only.
        opts.mode(0o600);
    }
    let mut file = opts.open(path)?;
    file.write_all(contents)?;
    file.flush()
}

/// Debug dump helper: write `contents` to a uniquely named file in the private
/// temp directory and return its path.
///
/// Unlike `TempFile`, the file is *not* removed on drop: a debug dump exists to
/// be inspected after the compiler exits. The name is unique per process, so
/// concurrent compilations cannot collide (the old fixed
/// `/tmp/asm_debug_<stem>.s` name could be pre-created as a symlink by another
/// local user and every parallel run overwrote the same file).
pub fn write_debug_dump(
    prefix: &str,
    stem: &str,
    extension: &str,
    contents: &[u8],
) -> Option<PathBuf> {
    let path = private_temp_dir()?.join(unique_filename(prefix, stem, extension));
    match write_file_exclusive(&path, contents) {
        Ok(()) => Some(path),
        Err(_) => None,
    }
}

/// RAII guard for a temporary file. Deletes the file when dropped.
///
/// This ensures cleanup happens even on early returns, panics, or errors.
/// The file path is accessible via `path()` and `to_str()`.
pub struct TempFile {
    path: PathBuf,
    /// If true, the file will NOT be deleted on drop (for debugging).
    keep: bool,
    // The lease outlives file removal in Drop, including never-created paths.
    _directory: Option<Arc<ManagedTempDirectory>>,
}

impl TempFile {
    /// Fallible reservation for production callers. Never returns an unowned
    /// flat path when private-directory creation fails.
    pub fn try_new(prefix: &str, stem: &str, extension: &str) -> io::Result<Self> {
        static POOL: OnceLock<TempDirectoryPool> = OnceLock::new();
        POOL.get_or_init(TempDirectoryPool::default)
            .reserve(prefix, stem, extension)
    }

    /// Create a TempFile with a specific path (for cases where the caller
    /// controls the path but wants RAII cleanup).
    #[cfg_attr(not(feature = "gcc_assembler"), allow(dead_code))] // Used by gcc_assembler's CCC_KEEP_ASM path
    pub fn with_path(path: PathBuf) -> Self {
        Self {
            path,
            keep: false,
            _directory: None,
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

    /// Borrow a UTF-8 path for internal string-based driver APIs.
    /// A valid Unix TMPDIR may contain non-UTF-8 bytes; report that limitation
    /// as an ordinary error, never an internal compiler panic or a lossy path.
    pub fn to_str(&self) -> io::Result<&str> {
        self.path.to_str().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "temp file path is not valid UTF-8",
            )
        })
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if self.keep {
            if let Some(directory) = &self._directory {
                directory.retained.store(true, Ordering::Release);
            }
        } else {
            // Silently ignore errors (file may not have been created).
            let _ = fs::remove_file(&self.path);
            // The directory lease is released after this destructor returns.
        }
    }
}

impl std::fmt::Display for TempFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.path.display())
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    #[test]
    fn name_components_cannot_escape_the_private_directory() {
        let pool = TempDirectoryPool::default();
        for (prefix, stem, extension) in [
            ("/tmp/escape", "stem", "o"),
            ("prefix", "../../escape", "o"),
            ("prefix", "stem", "../o"),
            #[cfg(windows)]
            ("prefix", "..\\escape", "o"),
            ("prefix", "nul\0byte", "o"),
        ] {
            match pool.reserve(prefix, stem, extension) {
                Err(error) => assert_eq!(error.kind(), io::ErrorKind::InvalidInput),
                Ok(_) => panic!("untrusted name escaped component validation"),
            }
        }
        assert!(pool.current.lock().unwrap().upgrade().is_none());
    }

    #[test]
    #[cfg(unix)]
    fn ordinary_unix_filename_characters_are_preserved() {
        let pool = TempDirectoryPool::default();
        let file = pool.reserve("prefix", "unix\\name: space_ä", "o").unwrap();
        let name = file.path().file_name().unwrap().to_str().unwrap();
        assert!(name.contains("unix\\name: space_ä"));
        assert!(file.path().parent().unwrap().is_dir());
    }

    #[test]
    fn keeping_a_pending_path_retains_its_directory() {
        let pool = TempDirectoryPool::default();
        let mut guard = pool.reserve("test", "pending-kept", "txt").unwrap();
        let path = guard.path().to_owned();
        let parent = path.parent().unwrap().to_owned();
        guard.set_keep(true);
        drop(guard);
        fs::write(&path, b"written after handoff").unwrap();
        fs::remove_file(path).unwrap();
        fs::remove_dir(parent).unwrap();
    }

    #[test]
    fn sequential_generations_recreate_secure_directories() {
        let pool = TempDirectoryPool::default();
        let mut previous = None;
        for _ in 0..32 {
            let file = pool.reserve("lease", "sequential", "tmp").unwrap();
            let parent = file.path().parent().unwrap().to_path_buf();
            assert!(parent.is_dir());
            assert_ne!(previous.as_ref(), Some(&parent));
            fs::write(file.path(), b"generation").unwrap();
            drop(file);
            assert!(!parent.exists());
            previous = Some(parent);
        }
    }

    #[test]
    fn uncreated_guard_keeps_the_directory_alive() {
        let pool = TempDirectoryPool::default();
        let first = pool.reserve("lease", "first", "tmp").unwrap();
        let pending = pool.reserve("lease", "pending", "tmp").unwrap();
        let parent = first.path().parent().unwrap().to_path_buf();
        assert_eq!(pending.path().parent(), Some(parent.as_path()));
        fs::write(first.path(), b"first").unwrap();
        drop(first);
        assert!(parent.is_dir());
        fs::write(pending.path(), b"second").unwrap();
        drop(pending);
        assert!(!parent.exists());
    }

    #[test]
    fn parallel_pending_paths_survive_other_guard_drops() {
        let pool = TempDirectoryPool::default();
        let files: Vec<_> = (0..16)
            .map(|_| pool.reserve("lease", "parallel", "tmp").unwrap())
            .collect();
        let parent = files[0].path().parent().unwrap().to_path_buf();
        let handles: Vec<_> = files
            .into_iter()
            .map(|file| {
                std::thread::spawn(move || {
                    fs::write(file.path(), b"parallel").unwrap();
                    assert_eq!(fs::read(file.path()).unwrap(), b"parallel");
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        assert!(!parent.exists());
    }

    #[test]
    fn keeping_a_file_keeps_its_directory_without_reusing_that_generation() {
        let pool = TempDirectoryPool::default();
        let mut file = pool.reserve("lease", "kept", "tmp").unwrap();
        let path = file.path().to_path_buf();
        let parent = path.parent().unwrap().to_path_buf();
        file.set_keep(true);
        fs::write(&path, b"kept").unwrap();
        drop(file);
        assert_eq!(fs::read(&path).unwrap(), b"kept");
        let next = pool.reserve("lease", "next", "tmp").unwrap();
        assert_ne!(next.path().parent(), Some(parent.as_path()));
        drop(next);
        fs::remove_file(path).unwrap();
        fs::remove_dir(parent).unwrap();
    }

    #[test]
    fn externally_owned_parent_is_not_reaped() {
        let parent = create_private_dir().unwrap();
        let file = TempFile::with_path(parent.join("external"));
        fs::write(file.path(), b"external").unwrap();
        drop(file);
        assert!(parent.is_dir());
        fs::remove_dir(parent).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn non_utf8_path_conversion_is_an_error_not_a_panic() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        let owner = TempFile::try_new("utf8", "owner", "tmp").unwrap();
        assert!(owner.to_str().is_ok());
        let path = owner
            .path()
            .parent()
            .unwrap()
            .join(OsString::from_vec(vec![0xff]));
        let file = TempFile::with_path(path);
        assert_eq!(
            file.to_str().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn directory_lease_reaps_empty_but_preserves_kept_files() {
        // A private lease, not the global weak cache, makes this assertion
        // independent of unrelated tests holding their own managed guards.
        let directory = Arc::new(ManagedTempDirectory {
            path: create_private_dir().unwrap(),
            retained: AtomicBool::new(false),
        });
        let parent = directory.path.clone();
        let path = parent.join("owned");
        fs::write(&path, b"owned").unwrap();
        let file = TempFile {
            path,
            keep: false,
            _directory: Some(directory),
        };
        drop(file);
        assert!(!parent.exists());

        let directory = Arc::new(ManagedTempDirectory {
            path: create_private_dir().unwrap(),
            retained: AtomicBool::new(false),
        });
        let parent = directory.path.clone();
        let path = parent.join("kept");
        fs::write(&path, b"kept").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&parent).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        let mut file = TempFile {
            path: path.clone(),
            keep: false,
            _directory: Some(directory),
        };
        file.set_keep(true);
        drop(file);
        assert_eq!(fs::read(&path).unwrap(), b"kept");
        fs::remove_file(path).unwrap();
        fs::remove_dir(parent).unwrap();
    }

    #[test]
    fn managed_cleanup_does_not_invalidate_unmanaged_paths() {
        let path = private_temp_dir()
            .unwrap()
            .join(unique_filename("lifecycle", "unmanaged", "tmp"));
        drop(TempFile::try_new("lifecycle", "managed", "tmp").unwrap());
        write_file_exclusive(&path, b"unmanaged lifetime").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"unmanaged lifetime");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn pending_reservation_survives_sibling_drop() {
        let first = TempFile::try_new("lifecycle", "first", "tmp").unwrap();
        let pending = TempFile::try_new("lifecycle", "pending", "tmp").unwrap();
        drop(first);
        fs::write(pending.path(), b"still reserved").unwrap();
        assert_eq!(fs::read(pending.path()).unwrap(), b"still reserved");
    }

    #[test]
    fn sequential_lifetimes_create_writable_directories() {
        for _ in 0..3 {
            let file = TempFile::try_new("lifecycle", "sequential", "tmp").unwrap();
            fs::write(file.path(), b"next generation").unwrap();
        }
    }

    #[test]
    fn unrelated_explicit_path_cannot_reap_reserved_directory() {
        let pending = TempFile::try_new("lifecycle", "pending", "tmp").unwrap();
        let explicit = TempFile::with_path(temp_dir().join(format!(
            "lccc-unrelated-{}-{}",
            std::process::id(),
            unique_stamp()
        )));
        drop(explicit);
        fs::write(pending.path(), b"not owned by the explicit guard").unwrap();
    }

    #[test]
    fn parallel_pending_writers_survive_an_empty_sibling() {
        let pending: Vec<_> = (0..8)
            .map(|_| TempFile::try_new("lifecycle", "parallel", "tmp").unwrap())
            .collect();
        drop(TempFile::try_new("lifecycle", "empty", "tmp").unwrap());
        std::thread::scope(|scope| {
            for file in pending {
                scope.spawn(move || fs::write(file.path(), b"parallel").unwrap());
            }
        });
    }
}
