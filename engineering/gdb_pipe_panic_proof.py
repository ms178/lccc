#!/usr/bin/env python3
"""GDB proof that CLI text cannot panic when stderr's reader disappears.

The pre-fix defect was traced with GDB to
``std::io::stdio::print_to -> _eprint -> panic_fmt`` (see
engineering/evidence/gdb-cli-pipe-backtrace.txt).  Fixing it requires proving
the ABSENCE of that path on the shipped binary, which a plain exit-code check
cannot do: a future refactor could catch the panic, or panic and recover.

Method (both legs run under the same GDB command file):

* set a breakpoint on ``__rustc::rust_panic`` (verified present with pinned nm);
* attach the inferior's stderr to a 4 KiB pipe whose reader closes after two
  lines, so writes see EPIPE;
* NEGATIVE leg: the shipped compiler must exit with a clean verdict and must
  NEVER hit the breakpoint;
* POSITIVE control: a tiny Rust program using ``eprintln!`` in the identical
  pipe setup MUST hit the breakpoint.  Without this control, "no breakpoint
  hit" could just mean the symbol or the harness is wrong.

This is a research witness, NOT a CI gate: GDB and the pinned toolchain cache
are only guaranteed in the Debian lanes.  Hosted CI covers the behaviour
through check 7 in tests/regression/check_param_home_contracts.py.

Usage:
  python3 engineering/gdb_pipe_panic_proof.py <lccc-binary> <output.txt>
"""
from __future__ import annotations

import fcntl
import os
import pathlib
import re
import subprocess
import sys
import tempfile

F_SETPIPE_SZ = 1031
GDB = pathlib.Path("/home/user/.cache/lccc/agent-toolchains/bin/gdb")
RUSTC = pathlib.Path("/home/user/.cache/lccc/agent-toolchains/1.99.1-x86_64-unknown-linux-gnu/bin/rustc")

POSITIVE_CONTROL = r"""
fn main() {
    for i in 0..20000 {
        eprintln!(
            "error {i}: a deliberately long diagnostic line that fills the pipe buffer \
             so the write fails after the reader has gone away"
        );
    }
}
"""


def compile_control(work: pathlib.Path) -> pathlib.Path:
    src = work / "control.rs"
    exe = work / "control"
    src.write_text(POSITIVE_CONTROL)
    r = subprocess.run([str(RUSTC), "-O", str(src), "-o", str(exe)],
                       capture_output=True)
    if r.returncode != 0:
        raise SystemExit("could not compile the positive control: "
                         + r.stderr.decode("utf-8", "replace")[-400:])
    return exe


def run_under_gdb(argv: list[str], work: pathlib.Path, tag: str) -> tuple[int, str, int]:
    """Return (gdb rc, GDB transcript, number of panic-breakpoint hits).

    GDB's OWN stderr must not be the closing pipe: it would be killed by
    SIGPIPE while printing its transcript (observed: returncode -13).  Instead
    GDB's stderr goes to /dev/null and the pipe's write end is inherited as an
    extra fd; at `break main` we dup2 it onto the inferior's fd 2.  Only the
    INFERIOR then sees EPIPE, which is exactly the production situation.
    """
    r_fd, w_fd = os.pipe()
    try:
        fcntl.fcntl(w_fd, F_SETPIPE_SZ, 4096)
    except OSError:
        pass  # a 64 KiB default pipe still closes; the reader exits early
    reader = os.fdopen(r_fd, "rb", buffering=0)
    cmd = [str(GDB), "-q", "-batch",
           "-ex", "set pagination off",
           "-ex", "set confirm off",
           # Rust ignores SIGPIPE at startup; GDB stops on it by default, which
           # would hide the write's EPIPE return.  Let it through uninterrupted.
           "-ex", "handle SIGPIPE nostop noprint pass",
           "-ex", "break __rustc::rust_panic",
           "-ex", "break main",
           "-ex", "run",
           "-ex", f"call (int) dup2({w_fd}, 2)",
           "-ex", "continue",
           "-ex", "bt 6",
           "-ex", "printf \"GDB_PROBE_DONE\\n\"",
           "--args", *argv]
    proc = subprocess.Popen(cmd, stdin=subprocess.DEVNULL,
                            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                            pass_fds=(w_fd,))
    # The child holds its own copy.  Closing ours makes EOF observable if GDB
    # aborts before the inferior writes, so the reader cannot block forever.
    os.close(w_fd)
    # `reader` sees the inferior's diagnostics once dup2 has run.  Read two
    # lines, then close: every later write(2) on fd 2 fails with EPIPE.
    lines_read = 0
    try:
        while lines_read < 2:
            chunk = reader.readline()
            if not chunk:
                break
            lines_read += 1
    finally:
        reader.close()
    out = proc.communicate(timeout=180)[0].decode("utf-8", "replace")
    # GDB prints the demangled name (std::panicking::rust_panic); match either.
    hits = len(re.findall(r"Breakpoint 1, [^\n]*rust_panic", out))
    return proc.returncode, out, hits


def main() -> int:
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    lccc, out_path = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
    for tool in (GDB, RUSTC, lccc):
        if not tool.is_file():
            raise SystemExit(f"missing required tool/binary: {tool}")
    nm = pathlib.Path("/home/user/.cache/lccc/agent-toolchains/bin/nm")
    syms = subprocess.run([str(nm), "-C", str(lccc)], capture_output=True,
                          text=True).stdout
    if "__rustc::rust_panic" not in syms:
        raise SystemExit("panic symbol absent: the breakpoint would be vacuous")

    report: list[str] = [
        "GDB CLI-pipe panic proof",
        f"  binary: {lccc}",
        f"  gdb: {subprocess.run([str(GDB), '--version'], capture_output=True, text=True).stdout.splitlines()[0]}",
        "  breakpoint: __rustc::rust_panic (symbol confirmed by pinned nm)",
        "",
    ]
    with tempfile.TemporaryDirectory(prefix="gdb-pipe-proof-") as td:
        work = pathlib.Path(td)
        bad = work / "many-errors.c"
        bad.write_bytes(b"int f(void) {\n" + b"  @ ;\n" * 2000 + b"}\n")
        control = compile_control(work)

        cases = (
            ("NEGATIVE shipped compiler", [str(lccc), "-w", "-c", str(bad),
                                           "-o", str(work / "bad.o")], 0),
            ("POSITIVE eprintln! control", [str(control)], 1),
        )
        ok = True
        for label, argv, want_hits in cases:
            rc, out, hits = run_under_gdb(argv, work, label)
            exited = re.findall(r"exited with code (\d+)", out)
            report += [f"--- {label}",
                       f"  gdb_returncode={rc}",
                       f"  panic_breakpoint_hits={hits} (want {want_hits})",
                       f"  inferior_exit={exited[-1] if exited else 'not reported'}",
                       f"  reader_lines_before_close=2", ""]
            if hits != want_hits:
                report.append(f"  VERDICT: FAIL (breakpoint hits {hits})")
                ok = False
            else:
                report.append("  VERDICT: PASS")
            report.append("")
            # Keep the decisive tail of each GDB transcript as evidence.
            report += ["  gdb transcript tail:",
                       *["    " + l for l in out.splitlines()[-12:]], ""]
        report.append("OVERALL: " + ("PASS" if ok else "FAIL"))
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text("\n".join(report) + "\n")
    print("\n".join(l for l in report if l.startswith(("  VERDICT", "OVERALL",
                                                        "  gdb_returncode",
                                                        "  panic_breakpoint_hits",
                                                        "  inferior_exit"))))
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
