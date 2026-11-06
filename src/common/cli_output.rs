//! Best-effort CLI text output.
//!
//! Rust's `eprintln!` / `println!` macros panic with
//! `failed printing to stderr: Broken pipe` (process exit 101) when the
//! reader of the stream goes away -- the completely ordinary
//! `lccc bad.c 2>&1 | head` case. A compiler's CLI text is best-effort: a
//! vanished reader must not turn a clean diagnostic verdict (exit 1) into a
//! Rust ICE, and must not change the compilation result at all. Every CLI
//! line in this crate therefore goes through one of the four helpers here,
//! all of which ignore write errors.
//!
//! Ignoring an error is correct for *text* (there is nowhere left to report
//! it), but it is NOT a licence to ignore errors on artifacts we produce:
//! object/assembly/binary writers keep their own strict `?` handling. Those
//! writers use `File`, not these helpers.
//!
//! The helpers are public and re-exported as `lccc::cli_output` so the
//! standalone driver binaries (`lccc-ld`, `lccc-x86`, ...) use exactly the
//! same policy as the library. `extern crate self as lccc;` in `lib.rs`
//! lets library modules and binary modules spell the path identically,
//! including inside `macro_rules!` bodies expanded at a binary call site.

use std::fmt::Arguments;
use std::io::Write;

/// Write one formatted line to `out`, ignoring I/O failure.
///
/// Exposed with an explicit sink so the broken-pipe path is unit-testable
/// without re-executing the compiler.
#[inline]
pub fn write_line_to<W: Write + ?Sized>(out: &mut W, args: Arguments<'_>) {
    let _ = writeln!(out, "{args}");
}

/// Write one formatted fragment without a trailing newline, ignoring I/O
/// failure. This is the exact success-path behaviour of `print!` / `eprint!`.
#[inline]
pub fn write_text_to<W: Write + ?Sized>(out: &mut W, args: Arguments<'_>) {
    let _ = write!(out, "{args}");
}

/// Emit one line to the process's stderr, ignoring I/O failure.
#[inline]
pub fn stderr_line(args: Arguments<'_>) {
    let mut err = std::io::stderr().lock();
    write_line_to(&mut err, args);
}

/// Emit a fragment to the process's stderr without a newline, ignoring I/O
/// failure.
#[inline]
pub fn stderr_text(args: Arguments<'_>) {
    let mut err = std::io::stderr().lock();
    write_text_to(&mut err, args);
}

/// Emit one line to the process's stdout, ignoring I/O failure.
#[inline]
pub fn stdout_line(args: Arguments<'_>) {
    let mut out = std::io::stdout().lock();
    write_line_to(&mut out, args);
}

/// Emit a fragment to the process's stdout without a newline, ignoring I/O
/// failure.
#[inline]
pub fn stdout_text(args: Arguments<'_>) {
    let mut out = std::io::stdout().lock();
    write_text_to(&mut out, args);
}

#[cfg(test)]
mod tests {
    use super::{write_line_to, write_text_to};
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

    /// The exact failure mode `eprintln!` panics on must be a no-op here,
    /// for both the newline and the fragment form.
    #[test]
    fn broken_pipe_is_ignored_instead_of_panicking() {
        let mut sink = BrokenPipe;
        write_line_to(&mut sink, format_args!("error: {}", "expected ';'"));
        write_text_to(&mut sink, format_args!("partial: {}", 7));
        write_line_to(&mut sink, format_args!("{} warning(s) generated", 3));
    }

    /// The success path must still produce byte-identical output to
    /// `eprintln!`/`println!`/`print!`, so this refactor does not change any
    /// CLI text.
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
        write_text_to(&mut buf, format_args!("dep.o: "));
        write_line_to(&mut buf, format_args!("a.c"));
        assert_eq!(
            buf,
            b"a.c:7: error: oops\n  fix-it hint: insert ';'\ndep.o: a.c\n".to_vec()
        );
    }

    /// A newline-only call must emit exactly one byte (the `println!()` form).
    #[test]
    fn empty_line_is_one_newline() {
        let mut buf = Vec::new();
        write_line_to(&mut buf, format_args!(""));
        assert_eq!(buf, b"\n".to_vec());
    }
}
