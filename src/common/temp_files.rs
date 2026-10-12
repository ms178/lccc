//! Proper temp file handling with RAII cleanup and $TMPDIR support.
//!
//! Provides:
//! - `temp_dir()`: returns the platform temp directory (respects $TMPDIR)
//! - `private_temp_dir()`: a per-process 0700 directory owning every temp file
//! - `TempFile`: RAII guard that deletes the file on drop (even on panic/error)
//! - `make_temp_path()`: generates a unique temp file path inside the private dir
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
//! Every temp file now lives in `$TMPDIR/lccc-<pid>-<nanos>-<n>/`, created with
//! `mkdir` mode 0700 and never reused: an attacker cannot pre-create it (a
//! pre-placed file/symlink/directory makes `mkdir` fail with `EEXIST`, and we
//! retry with the next candidate), and cannot write into it afterwards, because
//! it is owned by us and mode 0700. The directory is removed when the last
//! `TempFile` guard drops, so a compile leaves no debris behind.
//!
//! If no private directory can be created at all (read-only or missing
//! `$TMPDIR`, exotic filesystems), we fall back to the historical flat naming
//! so that compilation still works; the fallback is reported once on stderr
//! under `CCC_TEMP_DEBUG=1` and is deliberately *not* silent.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
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

/// Monotonic-ish nanosecond stamp used to make the private directory name
/// unpredictable: an attacker cannot create the directory in advance (it does
/// not exist yet) and cannot guess it in time to swap it under us.
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

/// Create (once per process) the private temp directory.
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
    // Never silent: the fallback re-introduces the predictable-name shape
    // that AUD-TMP-1 hardened, so an operator must see that it happened.
    eprintln!(
        "ccc: warning: no private temp directory could be created under {}; \
         falling back to flat temp-file names in that directory",
        base.display()
    );
    None
}

/// The process-private temp directory, or `None` if it could not be created.
///
/// Created lazily on first use and shared by every temp file of this process.
pub fn private_temp_dir() -> Option<&'static Path> {
    static PRIVATE_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    PRIVATE_DIR
        .get_or_init(create_private_dir)
        .as_deref()
}

/// Remove the private temp directory if it is empty.
///
/// Called from `TempFile::drop` on every temp-file release: the last release of
/// a compile therefore removes the directory, and an interrupted or
/// `CCC_KEEP_*` run leaves it behind for inspection instead of deleting files
/// a user asked to keep. Failures are ignored (a non-empty directory, a file
/// another thread still holds, a transient EPERM) because cleanup must never
/// turn into a compile failure.
fn remove_private_dir_if_empty() {
    if let Some(dir) = private_temp_dir() {
        let _ = fs::remove_dir(dir);
    }
}

/// Generate a unique temp file path with the given prefix and extension.
///
/// The path includes the PID and an atomic counter to avoid collisions
/// in parallel builds and multi-file compilations. Inside the process-private
/// directory the historical file-name shape is kept verbatim
/// (`<prefix>_<pid>_<stem>.<n>.<ext>`) so diagnostics and `CCC_KEEP_*`
/// workflows that read the name keep working; only the *directory* changed.
pub fn make_temp_path(prefix: &str, stem: &str, extension: &str) -> PathBuf {
    let pid = std::process::id();
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let filename = format!("{}_{}_{}.{}.{}", prefix, pid, stem, id, extension);
    match private_temp_dir() {
        Some(dir) => dir.join(filename),
        None => temp_dir().join(filename),
    }
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
pub fn write_debug_dump(prefix: &str, stem: &str, extension: &str, contents: &[u8]) -> Option<PathBuf> {
    let path = make_temp_path(prefix, stem, extension);
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
}

impl TempFile {
    /// Create a new TempFile guard for a unique temp path.
    ///
    /// The file is not created on disk — this just reserves the path and
    /// registers it for cleanup on drop.
    pub fn new(prefix: &str, stem: &str, extension: &str) -> Self {
        Self {
            path: make_temp_path(prefix, stem, extension),
            keep: false,
        }
    }

    /// Create a TempFile with a specific path (for cases where the caller
    /// controls the path but wants RAII cleanup).
    #[cfg_attr(not(feature = "gcc_assembler"), allow(dead_code))] // Used by gcc_assembler's CCC_KEEP_ASM path
    pub fn with_path(path: PathBuf) -> Self {
        Self { path, keep: false }
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
            let _ = fs::remove_file(&self.path);
            // …and reap the shared directory when this was the last file in it.
            remove_private_dir_if_empty();
        }
    }
}

impl std::fmt::Display for TempFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.path.display())
    }
}
