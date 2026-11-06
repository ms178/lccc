//! Best-effort CLI output.
//!
//! Rust's `eprintln!` panics with `failed printing to stderr: Broken pipe`
//! (process exit 101) when the reader of stderr goes away -- the completely
//! ordinary `lccc bad.c 2>&1 | head` case. A compiler's CLI text is
//! best-effort: a vanished reader must not turn a clean diagnostic verdict
//! (exit 1) into a Rust ICE, and must not change the compilation result at
//! all. Every CLI line in the driver therefore goes through [`stderr_line`],
//! which ignores write errors.
//!
//! Ignoring an error is correct here (there is nowhere left to report it), but
//! it is NOT a licence to ignore errors on files we produce: object/assembly
//! writers keep their own strict `?` handling.

use std::fmt::Arguments;
use std::io::Write;

/// Write one formatted line to `out`, ignoring I/O failure.
///
/// Exposed with an explicit sink so the broken-pipe path is unit-testable
/// without re-executing the compiler.
#[inline]
pub(crate) fn write_line_to<W: Write>(out: &mut W, args: Arguments<'_>) {
    let _ = writeln!(out, "{args}");
}

/// Emit one line to the process's stderr, ignoring I/O failure.
#[inline]
pub(crate) fn stderr_line(args: Arguments<'_>) {
    let mut err = std::io::stderr().lock();
    write_line_to(&mut err, args);
}

#[cfg(test)]
mod tests {
    use super::write_line_to;
    use std::io::{Error, ErrorKind, Write};

    struct BrokenPipe;

    impl Write for BrokenPipe {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(Error::new(ErrorKind::BrokenPipe, "reader exited"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(Error::new(ErrorKind::BrokenPipe, "reader exited"))
        }
    }

    /// The exact failure mode `eprintln!` panics on must be a no-op here.
    #[test]
    fn broken_pipe_is_ignored_instead_of_panicking() {
        let mut sink = BrokenPipe;
        write_line_to(&mut sink, format_args!("error: {}", "expected ';'"));
        write_line_to(&mut sink, format_args!("{} warning(s) generated", 3));
    }

    /// The success path must still produce byte-identical output to
    /// `eprintln!`, so this refactor does not change any diagnostic text.
    #[test]
    fn normal_output_is_unchanged() {
        let mut buf = Vec::new();
        write_line_to(
            &mut buf,
            format_args!(
                "{file}:{line}: error: {msg}",
                file = "a.c",
                line = 7,
                msg = "oops"
            ),
        );
        write_line_to(&mut buf, format_args!("  fix-it hint: insert ';'"));
        assert_eq!(
            buf,
            b"a.c:7: error: oops\n  fix-it hint: insert ';'\n".to_vec()
        );
    }
}
