#!/usr/bin/env python3
"""Run a build/test command and republish its failure through readable channels.

Why this exists
---------------
A failed `cargo test` step is the most common red in this repository and, until
this file existed, the *least* diagnosable one.  GitHub's own annotation for it
is

    Process completed with exit code 101

and both channels that would answer "which test" are shut to a triager:
`GET /repos/{owner}/{repo}/actions/jobs/{id}/logs` answers HTTP 500 for a failed
job, and the public job page (`.../checks/{id}/logs`) is a client-side shell
that carries no verdict text.  That is not a theory.  On 2026-10-13 a red branch
sat unexplained while ten failing tests were plainly listed in a runner log
nobody could open; the failing names were only recovered because that branch
happened to wrap `cargo test` in a script that re-emitted the tail as a check
annotation.  A red run whose cause is unreadable from the run page costs hours
per occurrence and invites the wrong fix -- the same argument
`Surface regression failures in annotations` makes for the corpus harness,
applied to the suite that fails most often.

What this does
--------------
Two modes over one parser.

  run     ``ci_test_report.py [OPTIONS] -- COMMAND [ARG...]``
          Executes COMMAND with stdout and stderr merged and streamed through
          byte-for-byte unchanged, and on a non-zero exit republishes the
          verdict where it can be read:

          * one ``::error`` check annotation per failing test, carrying the
            panic site as ``file=<path>,line=<n>`` so the annotation lands on
            the failing source line in the Checks UI;
          * a summary annotation naming every failure, emitted FIRST because
            GitHub displays only a bounded number of annotations per step and
            the list is the part that must survive the cut;
          * a Markdown job summary (``$GITHUB_STEP_SUMMARY``) with the complete
            failure list, the panic messages and the log tail -- not subject to
            the annotation cap, and visible on the public run page;
          * a stderr digest for local runs, so ``ci_local.sh`` prints the same
            verdict without a browser.

          The wrapped command's exit status is preserved exactly, including
          death by signal (``128+n``), so the step that uses this wrapper fails
          exactly when it failed before -- only legibly.

  parse   ``ci_test_report.py --parse-log FILE``
          Runs the same parser over a saved log and prints the annotations
          without executing anything (the channel that was missing when the
          only surviving copy of a failing log was an annotation body).

Parser contract
---------------
Recognised in libtest's and rustc's own spellings:

* ``---- <name> stdout ----`` sections, ``test <name> ... FAILED`` marks, the
  trailing ``failures:`` list, and ``panicked at <path>:<line>:<col>`` in both
  the modern (``thread 'name' (id) panicked at file:line:col:``) and the legacy
  (``panicked at 'message', file:line:col``) form;
* rustc diagnostics: ``error[EXXXX]: message`` / ``error: message`` with a
  following ``--> <path>:<line>:<col>`` span, which is also how ``-D warnings``
  promotions arrive.  The ``aborting due to`` / ``could not compile`` roll-ups
  are skipped: they name no source and would bury the real diagnostic.

Rustc diagnostics are only harvested during the build phase (before the first
test section): the harness's own tests invoke compilers, and a diagnostic
printed *inside* a test section is that test's output, not the build's.

Nothing is invented.  When a non-zero exit parsed no test and no diagnostic,
the log tail is published verbatim under a "the command named no test or
diagnostic" annotation -- an exit code alone is precisely the unhelpful signal
this tool exists to replace.

The corpus is itself environment-blind by rule: `--self-test` re-runs the file
as a child process with `GITHUB_ACTIONS` set, with it absent, and with an ASCII
stdout, because the first hosted run of this tool failed its own corpus while
every local run passed -- an assertion was reading the runner's environment
instead of the code.  A suite that only runs where it was written cannot tell a
correct decision from an inherited one.

Exit codes: last command's status for `--parse-log` (0 published something,
3 nothing was parsable, 2 usage error); the wrapped command's status in run
mode (70 if the tool itself crashes, after publishing the traceback); 0/1 for
`--self-test`.
"""
from __future__ import annotations

import argparse
import os
import re
import signal
import subprocess
import sys
from collections import deque
from dataclasses import dataclass, field
from pathlib import Path

# ---------------------------------------------------------------------------
# constants that encode GitHub's own limits and this file's budget
# ---------------------------------------------------------------------------
#
# GitHub keeps a bounded number of annotations per step; a wall of individual
# errors crowds out the one that matters, so the summary goes first and the
# individual annotations stop at `--max-tests`.
DEFAULT_MAX_TESTS = 8
DEFAULT_TAIL_LINES = 60
MESSAGE_LIMIT = 1500  # GitHub truncates longer annotation bodies anyway
PROPERTY_LIMIT = 200

ANSI_ESCAPE = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]")

TEST_SECTION = re.compile(r"^---- (?P<name>.+?) stdout ----$")
TEST_MARK = re.compile(r"^test (?P<name>\S.*?) \.\.\. (?P<verdict>FAILED|ok|ignored)\b")
FAILURES_HEADER = re.compile(r"^failures:$")
FAILURE_LIST_ITEM = re.compile(r"^ {4}(?P<name>\S.*?)$")
# Modern: `thread 'name' (15824) panicked at src/x.rs:12:5:`
PANIC_LOCATION = re.compile(
    r"^(?:thread '(?P<thread>[^']*)'(?: \(\d+\))? )?panicked at "
    r"(?P<file>.+?):(?P<line>\d+):(?P<col>\d+):$"
)
# Legacy: `thread 'name' panicked at 'message', src/x.rs:12:5`
PANIC_LEGACY = re.compile(
    r"^(?:thread '(?P<thread>[^']*)'(?: \(\d+\))? )?panicked at "
    r"'(?P<message>.*)', (?P<file>.+?):(?P<line>\d+):(?P<col>\d+)$"
)
RUSTC_ERROR = re.compile(r"^error(?:\[(?P<code>[A-Z]\d{4})\])?: (?P<message>.+)$")
RUSTC_SPAN = re.compile(r"^\s*--> (?P<file>[^:]+):(?P<line>\d+):(?P<col>\d+)")
ROLLUP = re.compile(r"^(?:could not compile|aborting due to|test failed, to rerun pass)\b")
TEST_RESULT = re.compile(r"^test result: (?P<verdict>\w+)\.")
BACKTRACE_NOTE = re.compile(r"^note: run with `RUST_BACKTRACE=")


# ---------------------------------------------------------------------------
# report model
# ---------------------------------------------------------------------------
@dataclass
class Failure:
    """One failing test, merged from every signal libtest emits for it."""

    name: str
    message: str = ""
    file: str = ""
    line: int = 0
    col: int = 0
    marked: bool = False  # saw `test <name> ... FAILED`
    listed: bool = False  # saw the name in the trailing `failures:` list

    def location(self) -> str:
        if not self.file:
            return ""
        return f"{self.file}:{self.line}:{self.col}" if self.line else self.file


@dataclass
class BuildError:
    """One rustc diagnostic from the build phase."""

    code: str
    message: str
    file: str = ""
    line: int = 0
    col: int = 0

    def headline(self) -> str:
        return f"error[{self.code}]" if self.code else "error"


@dataclass
class Report:
    failures: list[Failure] = field(default_factory=list)
    build_errors: list[BuildError] = field(default_factory=list)
    counts: str = ""
    tail: list[str] = field(default_factory=list)

    def verdict(self) -> str:
        if self.failures:
            return "tests"
        if self.build_errors:
            return "build"
        return "unnamed"

    def empty(self) -> bool:
        return not self.failures and not self.build_errors


def _clean_span(path: str, root: str | None = None) -> str:
    """Annotations must point at a repo-relative path, not at ./src/... .

    GitHub only attaches an annotation to the file list when the path is
    relative to the repository root, and a panic inside a test binary reports
    an absolute path, so a path under the working tree is made relative here.
    `root` is injectable so the rule can be tested from anywhere -- and the
    filesystem root is deliberately NOT treated as "the tree": `/src/x.rs` from
    a process whose cwd is `/` is ambiguous, and guessing there would rewrite
    paths that have nothing to do with this repository.
    """
    path = path.strip()
    if os.path.isabs(path):
        base = (os.getcwd() if root is None else root).rstrip("/")
        if base and base != "/" and path.startswith(base + "/"):
            path = path[len(base) + 1:]
    while path.startswith("./"):
        path = path[2:]
    return path


def _short_command(command: str, limit: int = 120) -> str:
    """A command line as it can appear inside an annotation, not as it is run."""
    collapsed = " ".join(command.split())
    if len(collapsed) <= limit:
        return collapsed
    return collapsed[: limit - 1] + "…"


def _exit_phrase(exit_code: int | None) -> str:
    return f"exit {exit_code}" if exit_code is not None else "an unknown exit status"


# ---------------------------------------------------------------------------
# scanner: streaming, so a 40 MB log costs the size of its failures
# ---------------------------------------------------------------------------
class Scanner:
    """Feeds lines in and keeps only what a report needs.

    The scan is incremental rather than whole-log because `--all-targets` on a
    cold cache emits tens of megabytes; buffering that to parse it once would
    trade a readability bug for a memory bug.
    """

    def __init__(self, tail_lines: int = DEFAULT_TAIL_LINES) -> None:
        self._tail: deque[str] = deque(maxlen=max(tail_lines, 0))
        self._failures: dict[str, Failure] = {}
        self._build_errors: list[BuildError] = []
        self._counts = ""
        self._current_test: str | None = None
        self._await_message: Failure | None = None
        self._pending_error: BuildError | None = None
        self._in_failure_list = False
        self._carry = b""

    # -- feeding -----------------------------------------------------------
    def feed_bytes(self, chunk: bytes) -> None:
        """Feed raw bytes, keeping a partial line for the next chunk.

        `read1` returns whatever the pipe has, so a panic location can be split
        across two reads; decoding each chunk on its own would then present two
        half-lines to the state machine and lose the failure.  The carry is
        bytes, not text, so a multi-byte character split across reads also
        stays intact.
        """
        data = self._carry + chunk
        head, newline, tail = data.rpartition(b"\n")
        if not newline:
            self._carry = data
            return
        self._carry = tail
        for line in head.split(b"\n"):
            self.feed_line(line.decode("utf-8", "replace"))

    def feed_line(self, line: str) -> None:
        line = ANSI_ESCAPE.sub("", line.rstrip("\r"))
        self._tail.append(line)
        self._scan(line)

    def _record(self, name: str) -> Failure:
        failure = self._failures.get(name)
        if failure is None:
            failure = Failure(name=name)
            self._failures[name] = failure
        return failure

    # -- the state machine -------------------------------------------------
    def _scan(self, line: str) -> None:
        section = TEST_SECTION.match(line)
        if section is not None:
            self._current_test = section.group("name")
            self._await_message = self._record(self._current_test)
            self._in_failure_list = False
            self._pending_error = None
            return

        mark = TEST_MARK.match(line)
        if mark is not None:
            if mark.group("verdict") == "FAILED":
                failure = self._record(mark.group("name"))
                failure.marked = True
            self._await_message = None
            return

        if FAILURES_HEADER.match(line):
            self._in_failure_list = True
            self._await_message = None
            return

        if self._in_failure_list:
            if not line.strip():
                self._in_failure_list = False
                return
            item = FAILURE_LIST_ITEM.match(line)
            if item is not None:
                failure = self._record(item.group("name"))
                failure.listed = True
                return
            self._in_failure_list = False

        result = TEST_RESULT.match(line)
        if result is not None:
            self._counts = line.strip()
            return

        legacy = PANIC_LEGACY.match(line)
        if legacy is not None:
            failure = self._panic_failure(legacy.group("thread"))
            failure.file = _clean_span(legacy.group("file"))
            failure.line = int(legacy.group("line"))
            failure.col = int(legacy.group("col"))
            if not failure.message:
                failure.message = " ".join(legacy.group("message").split())[:MESSAGE_LIMIT]
            self._await_message = None
            return

        panic = PANIC_LOCATION.match(line)
        if panic is not None:
            failure = self._panic_failure(panic.group("thread"))
            failure.file = _clean_span(panic.group("file"))
            failure.line = int(panic.group("line"))
            failure.col = int(panic.group("col"))
            self._await_message = failure
            return

        # Rustc diagnostics are the build phase's output.  Inside a test
        # section they belong to that test (the harness compiles C code), so
        # only the phase before the first section may raise a build error.
        if self._current_test is None:
            error = RUSTC_ERROR.match(line)
            if error is not None:
                if not ROLLUP.match(error.group("message")):
                    self._pending_error = BuildError(
                        code=error.group("code") or "",
                        message=error.group("message").strip(),
                    )
                    self._build_errors.append(self._pending_error)
                return
            span = RUSTC_SPAN.match(line)
            if span is not None and self._pending_error is not None:
                self._pending_error.file = _clean_span(span.group("file"))
                self._pending_error.line = int(span.group("line"))
                self._pending_error.col = int(span.group("col"))
                self._pending_error = None
                return

        if self._await_message is not None and line.strip():
            if not BACKTRACE_NOTE.match(line.strip()):
                self._await_message.message = " ".join(line.split())[:MESSAGE_LIMIT]
                self._await_message = None

    def _panic_failure(self, thread: str | None) -> Failure:
        name = self._current_test or thread or "(unknown)"
        failure = self._record(name)
        if failure.message.startswith("test failed"):
            failure.message = ""
        return failure

    def report(self) -> Report:
        if self._carry:  # a log whose last line has no newline
            self.feed_line(self._carry.decode("utf-8", "replace"))
            self._carry = b""
        return Report(
            failures=list(self._failures.values()),
            build_errors=list(self._build_errors),
            counts=self._counts,
            tail=list(self._tail),
        )


def parse_log(text: str, tail_lines: int = DEFAULT_TAIL_LINES) -> Report:
    scanner = Scanner(tail_lines=tail_lines)
    scanner.feed_bytes(text.encode("utf-8", "replace"))
    return scanner.report()


# ---------------------------------------------------------------------------
# rendering: annotations (bounded, high signal), summary (complete), digest
# ---------------------------------------------------------------------------
def _escape_data(text: str) -> str:
    return text.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")


def _escape_property(text: str) -> str:
    return _escape_data(text).replace(":", "%3A").replace(",", "%2C")


def annotations_enabled(mode: str) -> bool:
    """`always` / `never` are explicit; `auto` follows GitHub's own marker.

    This is a function, not an `if` inside `publish`, because the self-test
    MUST be independent of the ambient environment: the first CI run of this
    tool failed its own corpus because a test asserted the local-digest path
    while `GITHUB_ACTIONS=true` was inherited from the runner.  The behaviour
    was right and the test was wrong, which is exactly the failure a pure
    predicate makes visible without a runner.
    """
    if mode == "always":
        return True
    if mode == "never":
        return False
    return os.environ.get("GITHUB_ACTIONS") == "true"


def workflow_command(level: str, data: str, **props: object) -> str:
    """A GitHub workflow command, escaped by its own two rules.

    The data body escapes only ``%``/CR/LF; property values additionally escape
    ``:`` and ``,`` because those are the property grammar's separators.  A test
    path contains ``::`` and a message contains commas and colons, so getting
    this wrong silently truncates the very text the annotation exists to carry.
    """
    rendered = []
    for key, value in props.items():
        if value in (None, ""):
            continue
        rendered.append(f"{key}={_escape_property(str(value))[:PROPERTY_LIMIT]}")
    prefix = f"::{level} {','.join(rendered)}" if rendered else f"::{level}"
    return f"{prefix}::{_escape_data(data)[:MESSAGE_LIMIT]}"


def annotations(
    report: Report,
    command: str,
    exit_code: int | None,
    max_tests: int = DEFAULT_MAX_TESTS,
) -> list[str]:
    """The bounded annotation set for a failed command, summary first."""
    if exit_code == 0 and report.empty() and not report.counts.endswith("FAILED."):
        return []
    label = _short_command(command)
    phrase = _exit_phrase(exit_code)
    tests = [f for f in report.failures if f.name]
    out: list[str] = []

    if tests:
        names = [f.name for f in tests]
        head = f"{len(names)} failing test(s) in {label} ({phrase}): " + ", ".join(names[:6])
        if len(names) > 6:
            head += f", … and {len(names) - 6} more (full list in the job summary)"
        out.append(workflow_command("error", head, title=f"{label}: {len(names)} test(s) failed"))
        for failure in tests[:max_tests]:
            data = failure.message or "test failed (no panic message captured)"
            if failure.location():
                data = f"{data} [{failure.location()}]"
            out.append(workflow_command("error", data, file=failure.file, line=failure.line or None, title=f"test {failure.name}"))
        return out

    if report.build_errors:
        primary = report.build_errors[0]
        head = f"{len(report.build_errors)} build error(s) in {label} ({phrase}): {primary.message}"
        out.append(workflow_command("error", head, title=f"{label}: the build failed"))
        for error in report.build_errors[:max_tests]:
            out.append(
                workflow_command(
                    "error",
                    error.message,
                    file=error.file,
                    line=error.line or None,
                    title=f"{error.headline()} in the build",
                )
            )
        return out

    tail = "\n".join(report.tail[-12:]) if report.tail else "(the command produced no output)"
    out.append(
        workflow_command(
            "error",
            f"{label} failed with {phrase} and named no test or diagnostic; last output lines:\n{tail}",
            title=f"{label}: {phrase}, no diagnosable failure parsed",
        )
    )
    return out


def _fence(text: str) -> str:
    """A fence longer than any backtick run inside the text, so it cannot break."""
    longest = 0
    for run in re.findall(r"`+", text):
        longest = max(longest, len(run))
    return "`" * max(3, longest + 1)


def summary_markdown(
    report: Report,
    command: str,
    exit_code: int | None,
    max_listed: int = 50,
) -> str:
    """The complete verdict, for `$GITHUB_STEP_SUMMARY` (no annotation cap)."""
    lines = [
        f"## `{_short_command(command, 200)}` failed with {_exit_phrase(exit_code)}",
        "",
    ]
    if report.counts:
        lines += [f"Last test-binary verdict: `{report.counts}`", ""]

    if report.failures:
        lines += [f"### {len(report.failures)} failing test(s)", ""]
        for failure in report.failures[:max_listed]:
            location = f" — `{failure.location()}`" if failure.location() else ""
            lines.append(f"- `{failure.name}`{location}")
            if failure.message:
                lines.append(f"  - {failure.message}")
        if len(report.failures) > max_listed:
            lines.append(f"- … and {len(report.failures) - max_listed} more (see the log tail below)")
        lines.append("")

    if report.build_errors:
        lines += [f"### {len(report.build_errors)} build error(s)", ""]
        for error in report.build_errors[:max_listed]:
            location = f" (`{error.file}:{error.line}:{error.col}`)" if error.file else ""
            lines.append(f"- **{error.headline()}**: {error.message}{location}")
        lines.append("")

    if not report.failures and not report.build_errors:
        lines += [
            "No test and no rustc diagnostic named itself in the output. The tail below is the",
            "only evidence the run produced; if it is empty the command died before writing any",
            "(a signal, an OOM kill or a missing toolchain all look like this).",
            "",
        ]

    if report.tail:
        tail = "\n".join(report.tail)
        lines += ["### Last output lines", "", _fence(tail), tail, _fence(tail), ""]
    return "\n".join(lines)


def digest(report: Report, command: str, exit_code: int | None, max_listed: int = 10) -> str:
    """The local-run equivalent: the same verdict on stderr, no browser."""
    out = [f"{_short_command(command)}: {_exit_phrase(exit_code)}"]
    if report.counts:
        out.append(f"  {report.counts}")
    for failure in report.failures[:max_listed]:
        location = f" [{failure.location()}]" if failure.location() else ""
        out.append(f"  FAIL {failure.name}{location}")
        if failure.message:
            out.append(f"       {failure.message}")
    for error in report.build_errors[:max_listed]:
        location = f" [{error.file}:{error.line}:{error.col}]" if error.file else ""
        out.append(f"  {error.headline()}: {error.message}{location}")
    if report.empty():
        tail = report.tail[-5:] if report.tail else ["(no output)"]
        out.append("  nothing named itself as the failure; last output lines:")
        out.extend(f"    {line}" for line in tail)
    return "\n".join(out)


# ---------------------------------------------------------------------------
# runner
# ---------------------------------------------------------------------------
def run_wrapped(
    argv: list[str],
    *,
    sink=None,
    tail_lines: int = DEFAULT_TAIL_LINES,
    annotate: str = "auto",
    max_tests: int = DEFAULT_MAX_TESTS,
    summary_file: str | None = None,
    anno_stream=None,
    digest_stream=None,
) -> int:
    """Execute argv, stream it through, and publish a readable verdict on failure.

    `sink` is the byte stream the command's merged output is copied to (stdout
    by default, a buffer in the self-test), so the passthrough path in the
    self-test is the same code CI runs.
    """
    if sink is None:
        sink = sys.stdout.buffer
    if anno_stream is None:
        anno_stream = sys.stdout
    if digest_stream is None:
        digest_stream = sys.stderr

    command = " ".join(argv)
    scanner = Scanner(tail_lines=tail_lines)
    try:
        child = subprocess.Popen(  # noqa: S603 - argv comes from the caller's own command line
            argv,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
        )
    except OSError as error:
        print(f"ci_test_report: cannot execute {argv[0]!r}: {error}", file=sys.stderr)
        return 127

    assert child.stdout is not None
    received: dict[str, int] = {}

    def _forward(signum, _frame):
        received["signal"] = signum
        try:
            child.send_signal(signum)
        except OSError:
            pass

    previous = {}
    for sig in (signal.SIGINT, signal.SIGTERM):
        try:
            previous[sig] = signal.signal(sig, _forward)
        except (ValueError, OSError):  # not the main thread
            pass

    try:
        # read1 keeps partial lines moving: a compiler that writes a 1 MB line
        # without a newline must not stall the stream.
        while True:
            chunk = child.stdout.read1(65536) if hasattr(child.stdout, "read1") else child.stdout.read(65536)
            if not chunk:
                break
            sink.write(chunk)
            sink.flush()
            scanner.feed_bytes(chunk)
    finally:
        child.stdout.close()
        for sig, handler in previous.items():
            try:
                signal.signal(sig, handler)
            except (ValueError, OSError):
                pass

    status = child.wait()
    if status < 0:  # died by signal: report it the way a shell would
        status = 128 - status

    publish(
        scanner.report(),
        command,
        status,
        annotate=annotate,
        max_tests=max_tests,
        summary_file=summary_file,
        anno_stream=anno_stream,
        digest_stream=digest_stream,
    )
    return status


def publish(
    report: Report,
    command: str,
    exit_code: int | None,
    *,
    annotate: str = "auto",
    max_tests: int = DEFAULT_MAX_TESTS,
    summary_file: str | None = None,
    anno_stream=None,
    digest_stream=None,
) -> None:
    if anno_stream is None:
        anno_stream = sys.stdout
    if digest_stream is None:
        digest_stream = sys.stderr
    if exit_code == 0 and report.empty() and not report.counts.endswith("FAILED."):
        return

    enabled = annotations_enabled(annotate)
    for line in annotations(report, command, exit_code, max_tests=max_tests):
        if enabled:
            print(line, file=anno_stream)
        else:
            print(digest(report, command, exit_code), file=digest_stream)
            break

    path = summary_file or os.environ.get("GITHUB_STEP_SUMMARY") or ""
    if path:
        try:
            with open(path, "a", encoding="utf-8") as handle:
                handle.write(summary_markdown(report, command, exit_code))
                handle.write("\n")
        except OSError as error:
            print(f"ci_test_report: cannot append the job summary: {error}", file=sys.stderr)


# ---------------------------------------------------------------------------
# self-test
# ---------------------------------------------------------------------------
# The first fixture is the real shape recovered from the 2026-10-13 red branch
# (annotation body of run 38480896785): a modern panic header carrying the
# thread id in parentheses, a source location, a message that is a Rust debug
# dump of emitted assembly, and libtest's two `failures:` blocks.
FIXTURE_TESTS = """\
   Compiling lccc v0.1.0 (/home/runner/work/lccc/lccc)
    Finished `fastbuild` profile [optimized + debuginfo] target(s) in 41.02s
     Running unittests src/lib.rs (target/fastbuild/deps/lccc-44c104259653c947)
running 4625 tests
test backend::x86::codegen::peephole::passes::load_op_fuse::tests::commuting_adjacent_fuse_rehomes_a_single_family_rmw_product_update ... FAILED
test passes::vectorize::map_fp_select_tests::fold_max_from_slt_swapped_arms ... FAILED
test backend::x86::codegen::peephole::passes::load_op_fuse::tests::commuting_adjacent_fuse_keeps_full_width_self_move_reads_live ... FAILED

failures:

---- backend::x86::codegen::peephole::passes::load_op_fuse::tests::commuting_adjacent_fuse_rehomes_a_single_family_rmw_product_update stdout ----
thread 'backend::x86::codegen::peephole::passes::load_op_fuse::tests::commuting_adjacent_fuse_rehomes_a_single_family_rmw_product_update' (15824) panicked at src/backend/x86/codegen/peephole/passes/load_op_fuse.rs:2088:9:
["movq (%rsi), %rax", "imulq %rax, %rdx", "addq %rdx, %rcx"]
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

---- passes::vectorize::map_fp_select_tests::fold_max_from_slt_swapped_arms stdout ----
thread 'passes::vectorize::map_fp_select_tests::fold_max_from_slt_swapped_arms' (15830) panicked at src/passes/vectorize.rs:9001:5:
assertion `left == right` failed
  left: 1
 right: 2

---- backend::x86::codegen::peephole::passes::load_op_fuse::tests::commuting_adjacent_fuse_keeps_full_width_self_move_reads_live stdout ----
thread 'backend::x86::codegen::peephole::passes::load_op_fuse::tests::commuting_adjacent_fuse_keeps_full_width_self_move_reads_live' (15826) panicked at src/backend/x86/codegen/peephole/passes/load_op_fuse.rs:2071:9:
missing the self-move read

failures:
    backend::x86::codegen::peephole::passes::load_op_fuse::tests::commuting_adjacent_fuse_rehomes_a_single_family_rmw_product_update
    passes::vectorize::map_fp_select_tests::fold_max_from_slt_swapped_arms
    backend::x86::codegen::peephole::passes::load_op_fuse::tests::commuting_adjacent_fuse_keeps_full_width_self_move_reads_live

test result: FAILED. 4615 passed; 3 failed; 7 ignored; 0 measured; 0 filtered out; finished in 4.02s

error: test failed, to rerun pass `--lib`
"""

FIXTURE_BUILD = """\
   Compiling lccc v0.1.0 (/home/runner/work/lccc/lccc)
warning: unused variable: `unused`
 --> src/frontend/sema/analysis.rs:118:9
error[E0308]: mismatched types
 --> src/frontend/sema/analysis.rs:120:9
error: use of unstable library feature `div_floor`
  --> src/passes/vectorize.rs:44:12
error: could not compile `lccc` (lib) due to 2 previous errors
"""

FIXTURE_PASSING = """\
running 4556 tests
test passes::vectorize::map_fp_select_tests::fold_max_from_sgt ... ok
test result: ok. 4556 passed; 0 failed; 7 ignored; 0 measured; 0 filtered out; finished in 8.86s
"""

FIXTURE_LEGACY = """\
---- doc_tests::legacy_panic stdout ----
thread 'doc_tests::legacy_panic' panicked at 'explicit panic', tests/doc.rs:41:9
   0: backtrace frame
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
"""

FIXTURE_UNNAMED = "Killed\nsignal: 9 (SIGKILL)\nmake: *** [Makefile:12: all] Error 137\n"

RMW = "backend::x86::codegen::peephole::passes::load_op_fuse::tests::commuting_adjacent_fuse_rehomes_a_single_family_rmw_product_update"
SELF_MOVE = "backend::x86::codegen::peephole::passes::load_op_fuse::tests::commuting_adjacent_fuse_keeps_full_width_self_move_reads_live"
SWAPPED = "passes::vectorize::map_fp_select_tests::fold_max_from_slt_swapped_arms"


def _check(condition: bool, message: str, problems: list[str]) -> None:
    if not condition:
        problems.append(message)


def _failure(report: Report, name: str) -> Failure | None:
    return next((f for f in report.failures if f.name == name), None)


def publish_self_test_problems(problems: list[str], annotate: str, stream=None) -> None:
    """Announce the tool's own failures on the channel that a triager can read.

    The reason this file exists is that a red step naming nothing is
    unreadable; its own red has no exemption.  The step still fails -- an
    annotation is not a pass.
    """
    if not problems or not annotations_enabled(annotate):
        return
    if stream is None:
        stream = sys.stdout
    for problem in problems[:DEFAULT_MAX_TESTS]:
        print(workflow_command("error", problem, title="scripts/ci_test_report.py --self-test"), file=stream)


def self_test(annotate: str = "auto") -> int:
    import io

    problems: list[str] = []
    # -- escaping: the two rules GitHub documents, applied where they apply ----
    _check(
        workflow_command("error", "50% x\ny") == "::error::50%25 x%0Ay",
        "data escaping must only rewrite %/CR/LF",
        problems,
    )
    _check(
        workflow_command("error", "body", title="a:b,c") == "::error title=a%3Ab%2Cc::body",
        "property escaping must also rewrite : and ,",
        problems,
    )

    # -- annotation paths must be repo-relative, commands must stay short -----
    # `root` is injected and NOT taken from this process: a test whose verdict
    # depends on where it runs is the defect this corpus exists to prevent.
    _check(_clean_span("./src/x.rs") == "src/x.rs", "_clean_span must strip a leading ./", problems)
    _check(
        _clean_span("/w/lccc/src/x.rs", root="/w/lccc") == "src/x.rs",
        "a path inside the tree must become repo-relative",
        problems,
    )
    _check(
        _clean_span("/opt/rust/x.rs", root="/w/lccc") == "/opt/rust/x.rs",
        "a path outside the tree must stay absolute",
        problems,
    )
    _check(_clean_span("/src/x.rs", root="/") == "/src/x.rs", "the filesystem root is not a tree and must not rewrite paths", problems)
    _check(_clean_span("src/x.rs", root="/w/lccc") == "src/x.rs", "a relative path must be left alone", problems)

    # -- the annotations/local-digest decision is a pure predicate ------------
    saved_actions = os.environ.pop("GITHUB_ACTIONS", None)
    _check(annotations_enabled("auto") is False, "auto without GITHUB_ACTIONS must stay local", problems)
    os.environ["GITHUB_ACTIONS"] = "true"
    _check(annotations_enabled("auto") is True, "auto with GITHUB_ACTIONS=true must annotate", problems)
    _check(annotations_enabled("never") is False, "never is never, even where annotations exist", problems)
    os.environ["GITHUB_ACTIONS"] = "false"
    _check(annotations_enabled("auto") is False, "GITHUB_ACTIONS=false is not an annotation channel", problems)
    if saved_actions is None:
        os.environ.pop("GITHUB_ACTIONS", None)
    else:
        os.environ["GITHUB_ACTIONS"] = saved_actions
    _check(annotations_enabled("always") is True, "always must not depend on the environment", problems)
    long_command = "cargo test " + " ".join(f"--flag{i}" for i in range(40))
    _check(_short_command(long_command).endswith("…") and len(_short_command(long_command)) <= 120, "a long command must be shortened for annotations", problems)
    _check(_exit_phrase(None) == "an unknown exit status", "an unknown exit status must not be invented", problems)

    # -- the real-world shape -------------------------------------------------
    report = parse_log(FIXTURE_TESTS, tail_lines=500)
    names = [f.name for f in report.failures]
    _check(len(names) == 3, f"expected 3 failures, parsed {names}", problems)
    update = _failure(report, RMW)
    _check(update is not None, "the rmw_product_update failure must be parsed", problems)
    if update is not None:
        _check(
            update.location() == "src/backend/x86/codegen/peephole/passes/load_op_fuse.rs:2088:9",
            f"panic location wrong: {update.location()}",
            problems,
        )
        _check("imulq" in update.message, f"panic message not captured: {update.message!r}", problems)
        _check(update.marked, "the `test ... FAILED` mark must be recorded", problems)
        _check(update.listed, "the trailing failures list must be recorded", problems)
    second = _failure(report, SWAPPED)
    _check(second is not None, "the swapped-arms failure must be parsed", problems)
    if second is not None:
        _check(
            second.location() == "src/passes/vectorize.rs:9001:5",
            f"second panic location wrong: {second.location()}",
            problems,
        )
        _check(
            second.message == "assertion `left == right` failed",
            f"assertion message not captured: {second.message!r}",
            problems,
        )
    self_move = _failure(report, SELF_MOVE)
    _check(self_move is not None and self_move.marked and self_move.listed, "the third section must merge all three signals", problems)
    if self_move is not None:
        _check(self_move.message == "missing the self-move read", f"third message: {self_move.message!r}", problems)
    _check(
        report.counts == "test result: FAILED. 4615 passed; 3 failed; 7 ignored; 0 measured; 0 filtered out; finished in 4.02s",
        f"test-result line not captured: {report.counts!r}",
        problems,
    )
    _check(report.verdict() == "tests", "a failing suite must classify as tests", problems)

    rendered = annotations(report, "cargo test", 101, max_tests=8)
    _check(len(rendered) == 4, f"summary + one annotation per test = 4, got {len(rendered)}", problems)
    _check(rendered and rendered[0].startswith("::error title=cargo test%3A 3 test(s) failed::"), f"summary must come first: {rendered[0] if rendered else None}", problems)
    _check(all(name in rendered[0] for name in names), "the summary annotation must name every failure", problems)
    _check(
        any("file=src/backend/x86/codegen/peephole/passes/load_op_fuse.rs,line=2088," in annotation for annotation in rendered),
        f"an annotation must land on the panic line: {rendered}",
        problems,
    )
    _check("\n" not in "".join(rendered), "annotations must be single escaped lines", problems)

    capped = annotations(report, "cargo test", 101, max_tests=1)
    _check(len(capped) == 2, f"max_tests=1 must yield summary + 1, got {len(capped)}", problems)
    _check(all(name in capped[0] for name in names), "the summary must name every failure even when max_tests cuts the rest", problems)

    markdown = summary_markdown(report, "cargo test", 101)
    _check("3 failing test(s)" in markdown, "the summary must count the failures", problems)
    _check("src/backend/x86/codegen/peephole/passes/load_op_fuse.rs:2088:9" in markdown, "the summary must carry locations", problems)
    _check("test result: FAILED" in markdown, "the summary must quote libtest's verdict", problems)
    _check(markdown.count("```") % 2 == 0, "the fenced tail must be balanced", problems)

    ugly = Report(failures=[Failure(name="x", message="back```tick\nand ``` more")], tail=["a ``` b"])
    ugly_markdown = summary_markdown(ugly, "cargo test", 1)
    _check("````" in ugly_markdown, "the tail fence must outgrow backticks in the text", problems)
    _check(ugly_markdown.count("````") % 2 == 0, "the longer fence must be balanced too", problems)

    # -- build failures, including -D warnings promotions ---------------------
    build = parse_log(FIXTURE_BUILD, tail_lines=50)
    _check(build.verdict() == "build", f"a rustc failure must classify as build, got {build.verdict()}", problems)
    codes = [e.code for e in build.build_errors]
    _check(codes == ["E0308", ""], f"roll-ups must be skipped and both codes kept: {codes}", problems)
    _check(build.build_errors[0].file.endswith("analysis.rs") and build.build_errors[0].line == 120, f"span not attached: {build.build_errors[0]}", problems)
    _check(build.build_errors[1].file.endswith("vectorize.rs"), "the second diagnostic's span must not stick to the first", problems)
    build_annotations = annotations(build, "cargo test", 101)
    _check("the build failed" in build_annotations[0], f"build summary missing: {build_annotations[0]}", problems)

    # -- a compiler diagnostic inside a test section is that test's output ----
    nested = parse_log(
        "---- harness::compiles_and_runs stdout ----\n"
        "error[E0308]: mismatched types\n"
        " --> /tmp/probe.rs:3:5\n"
        "thread 'harness::compiles_and_runs' panicked at src/harness.rs:9:5:\n"
        "the probe did not build\n",
        tail_lines=20,
    )
    _check(nested.build_errors == [], "a diagnostic inside a test section is not a build error", problems)
    _check([f.name for f in nested.failures] == ["harness::compiles_and_runs"], f"the nested panic must attach to the test: {[f.name for f in nested.failures]}", problems)

    # -- passing logs publish nothing ----------------------------------------
    passing = parse_log(FIXTURE_PASSING)
    _check(passing.empty(), "a passing log must produce an empty report", problems)
    _check(annotations(passing, "cargo test", 0) == [], "a passing run must produce no annotation", problems)

    # -- legacy panic spelling ----------------------------------------------
    legacy = parse_log(FIXTURE_LEGACY)
    _check([f.location() for f in legacy.failures] == ["tests/doc.rs:41:9"], f"legacy panic location: {[f.location() for f in legacy.failures]}", problems)
    _check(legacy.failures and legacy.failures[0].message == "explicit panic", f"legacy message: {legacy.failures[0].message if legacy.failures else None}", problems)

    # -- nothing named itself: the tail is the only evidence ------------------
    unnamed = parse_log(FIXTURE_UNNAMED, tail_lines=10)
    _check(unnamed.empty() and unnamed.verdict() == "unnamed", "an unnamed failure must stay unnamed", problems)
    unnamed_annotations = annotations(unnamed, "make all", 137)
    _check(len(unnamed_annotations) == 1 and "SIGKILL" in _escape_data("\n".join(unnamed.tail)), "the tail must be published verbatim", problems)
    _check("::error" in unnamed_annotations[0] and "\n" not in unnamed_annotations[0], f"the fallback must be one escaped annotation: {unnamed_annotations[0]!r}", problems)
    _check("exit 137" in unnamed_annotations[0], "the fallback must carry the exit status", problems)

    # -- CRLF, ANSI colour and invalid UTF-8 must not break the scan ---------
    messy = Scanner(tail_lines=5)
    messy.feed_bytes(b"\x1b[31merror: bad\x1b[0m\r\n \x1b[1m--> src/a.rs:7:2\x1b[0m\r\n---\xff\xfe---\r\n")
    messy_report = messy.report()
    _check([e.message for e in messy_report.build_errors] == ["bad"], f"ANSI/CRLF handling: {[e.message for e in messy_report.build_errors]}", problems)
    _check(bool(messy_report.build_errors) and messy_report.build_errors[0].line == 7, "the span must survive ANSI and CRLF", problems)

    # -- a line split across two reads must not be lost ----------------------
    split = Scanner(tail_lines=10)
    split.feed_bytes(b"---- boom::split stdout ----\nthread 'boom::split' (9) panicked at src/boo")
    split.feed_bytes(b"m.rs:3:1:\nsplit across read boundaries\n")
    split_report = split.report()
    _check(
        [f.location() for f in split_report.failures] == ["src/boom.rs:3:1"],
        f"a panic split across reads must still parse: {[f.location() for f in split_report.failures]}",
        problems,
    )
    _check(
        [f.message for f in split_report.failures] == ["split across read boundaries"],
        f"the message after a split line must still attach: {[f.message for f in split_report.failures]}",
        problems,
    )
    tail_only = Scanner(tail_lines=5)
    tail_only.feed_bytes(b"make: *** [Makefile:12: all] Error 137")  # no trailing newline
    _check(tail_only.report().tail == ["make: *** [Makefile:12: all] Error 137"], "an unterminated last line must be kept", problems)

    # -- the tool's own failure is published on the readable channel ----------
    fake_stream = io.StringIO()
    publish_self_test_problems(["assertion X failed"], "always", fake_stream)
    _check(
        fake_stream.getvalue().startswith("::error title=scripts/ci_test_report.py --self-test::"),
        f"a self-test failure must become an annotation: {fake_stream.getvalue()!r}",
        problems,
    )
    quiet_stream = io.StringIO()
    publish_self_test_problems(["assertion X failed"], "never", quiet_stream)
    _check(quiet_stream.getvalue() == "", "never must not annotate a self-test failure", problems)

    # -- a stderr-only failure still reaches the digest -----------------------
    digest_text = digest(Report(tail=["boom"]), "make all", 2)
    _check("boom" in digest_text and "make all" in digest_text, f"the digest must carry the tail: {digest_text!r}", problems)

    # -- run mode end to end --------------------------------------------------
    problems += _self_test_run_mode()
    problems += self_test_under_foreign_environments()

    if problems:
        print("ci-test-report self-test: FAIL", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        publish_self_test_problems(problems, annotate)
        return 1
    print("ci-test-report self-test: ok")
    return 0


# The child marker makes the re-exec probe terminate: this corpus runs a
# subprocess of itself, and a subprocess that repeats the probe recurses.
SELF_TEST_CHILD = "LCCC_CI_TEST_REPORT_SELF_TEST_CHILD"


def self_test_under_foreign_environments() -> list[str]:
    """Re-run this corpus under the environments the CI runner actually has.

    Written because the first hosted run of this tool failed *its own corpus*
    while every local run passed: an assertion exercised the local-digest path
    while the runner's exported `GITHUB_ACTIONS=true` turned the very same call
    into the annotation path.  The behaviour was right; the test was reading
    the ambient environment.  A suite that only ever runs in the developer's
    environment cannot tell those two apart, so the probe runs the file again
    as a child process with the runner's variables set, with them absent, and
    with an ASCII stdout -- and the local suite is green only if all of them
    are.
    """
    if os.environ.get(SELF_TEST_CHILD) == "1":
        return []  # this *is* the child
    problems: list[str] = []
    environments = (
        {"GITHUB_ACTIONS": "true"},
        {"GITHUB_ACTIONS": "true", "PYTHONIOENCODING": "ascii", "LC_ALL": "C", "LANG": "C"},
        {},
    )
    for extra in environments:
        env = {key: value for key, value in os.environ.items() if key != "GITHUB_ACTIONS"}
        env.update(extra)
        env[SELF_TEST_CHILD] = "1"
        label = ", ".join(f"{k}={v}" for k, v in extra.items()) or "a scrubbed environment"
        try:
            result = subprocess.run(  # noqa: S603 - this file, with its own interpreter
                [sys.executable, __file__, "--self-test", "--annotate", "never"],
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                timeout=300,
                check=False,
            )
        except (OSError, subprocess.SubprocessError) as error:
            problems.append(f"--self-test under {label}: could not be re-run: {error}")
            continue
        if result.returncode != 0:
            tail = result.stdout.decode("utf-8", "replace").strip().splitlines()[-8:]
            problems.append(f"--self-test under {label}: exit {result.returncode}: " + " | ".join(tail))
    return problems


def _self_test_run_mode() -> list[str]:
    """End-to-end: stream passthrough, published verdict, exit status."""
    import contextlib
    import io
    import tempfile

    problems: list[str] = []
    with tempfile.TemporaryDirectory() as tmp:
        summary = Path(tmp) / "summary.md"
        payload = (
            "running 2 tests\n"
            "test boom::one ... FAILED\n"
            "---- boom::one stdout ----\n"
            "thread 'boom::one' (7) panicked at src/boom.rs:12:5:\n"
            "the probe failed on purpose\n"
            "failures:\n"
            "    boom::one\n"
            "\ntest result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n"
        )
        failing = [sys.executable, "-c", f"import sys;sys.stdout.write({payload!r});sys.exit(101)"]
        sink = io.BytesIO()
        annotated = io.StringIO()
        status = run_wrapped(
            failing,
            sink=sink,
            annotate="always",
            summary_file=str(summary),
            anno_stream=annotated,
            digest_stream=io.StringIO(),
        )
        _check(status == 101, f"run mode must preserve exit 101, got {status}", problems)
        _check(sink.getvalue().decode() == payload, "run mode must stream the output byte-for-byte", problems)
        _check(
            "file=src/boom.rs,line=12,title=test boom%3A%3Aone::" in annotated.getvalue(),
            f"the failing test must be annotated with its location: {annotated.getvalue()!r}",
            problems,
        )
        _check("the probe failed on purpose" in annotated.getvalue(), "the panic message must reach the annotation", problems)
        if summary.is_file():
            text = summary.read_text()
            _check("boom::one" in text and "failed with exit 101" in text, "the job summary must record the verdict", problems)
        else:
            problems.append("run mode did not write the job summary")

        # A pass publishes nothing and keeps exit 0.
        sink = io.BytesIO()
        annotated = io.StringIO()
        status = run_wrapped(
            [sys.executable, "-c", "print('all good')"],
            sink=sink,
            annotate="always",
            anno_stream=annotated,
            digest_stream=io.StringIO(),
        )
        _check(status == 0 and annotated.getvalue() == "", "a passing command must stay silent", problems)
        _check(sink.getvalue() == b"all good\n", "a passing command's output must pass through", problems)

        # A local run (no GITHUB_ACTIONS) prints the digest instead.
        digest_out = io.StringIO()
        status = run_wrapped(
            failing,
            sink=io.BytesIO(),
            annotate="never",
            anno_stream=io.StringIO(),
            digest_stream=digest_out,
        )
        _check(status == 101 and "FAIL boom::one" in digest_out.getvalue(), f"local digest missing: {digest_out.getvalue()!r}", problems)

        # Death by signal must be reported the way a shell reports it.
        status = run_wrapped(
            [sys.executable, "-c", "import os,signal;os.kill(os.getpid(), signal.SIGTERM)"],
            sink=io.BytesIO(),
            annotate="never",
            anno_stream=io.StringIO(),
            digest_stream=io.StringIO(),
        )
        _check(status == 143, f"a SIGTERM'd child must report 143, got {status}", problems)

        # --parse-log mode over a saved log.
        log = Path(tmp) / "saved.log"
        log.write_text(FIXTURE_TESTS, encoding="utf-8")
        parsed = io.StringIO()
        status = parse_log_command(str(log), annotate="always", anno_stream=parsed, summary_file=str(summary))
        _check(status == 0 and "load_op_fuse" in parsed.getvalue(), "--parse-log must annotate a saved log", problems)
        nothing = Path(tmp) / "empty.log"
        nothing.write_text(FIXTURE_PASSING, encoding="utf-8")
        # A passing log is the "nothing named itself" case: exit 3, and the
        # notice belongs on stderr rather than in the annotation stream.
        with contextlib.redirect_stderr(io.StringIO()):
            status = parse_log_command(str(nothing), annotate="always", anno_stream=io.StringIO())
        _check(status == 3, f"--parse-log must report 3 when nothing named itself, got {status}", problems)
    return problems


def parse_log_command(path: str, *, annotate: str = "auto", anno_stream=None, summary_file: str | None = None) -> int:
    try:
        text = Path(path).read_text(encoding="utf-8", errors="replace")
    except OSError as error:
        print(f"ci_test_report: cannot read {path}: {error}", file=sys.stderr)
        return 2
    report = parse_log(text)
    if report.empty() and not report.counts.endswith("FAILED."):
        print(f"ci_test_report: {path} names no failing test and no rustc diagnostic", file=sys.stderr)
        return 3
    publish(
        report,
        f"<{path}>",
        None,
        annotate=annotate,
        summary_file=summary_file,
        anno_stream=anno_stream,
        digest_stream=sys.stderr,
    )
    return 0


# ---------------------------------------------------------------------------
# entry point
# ---------------------------------------------------------------------------
def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(
        prog="ci_test_report.py",
        description="Run a build/test command and republish its failure as annotations and a job summary.",
    )
    parser.add_argument("--self-test", action="store_true", help="run the parser/runner test corpus and exit")
    parser.add_argument("--parse-log", metavar="FILE", help="annotate a saved log instead of running a command")
    parser.add_argument("--annotate", choices=("auto", "always", "never"), default="auto", help="auto = only when GITHUB_ACTIONS=true")
    parser.add_argument("--max-tests", type=int, default=DEFAULT_MAX_TESTS, help="individual test annotations per failure (summary is always first)")
    parser.add_argument("--tail-lines", type=int, default=DEFAULT_TAIL_LINES, help="log lines kept for the digest and the job summary")
    parser.add_argument("--summary-file", default=None, help="append the Markdown summary here (default: $GITHUB_STEP_SUMMARY)")
    parser.add_argument("command", nargs=argparse.REMAINDER, help="command to run, after `--`")
    args = parser.parse_args(argv)

    try:
        if args.self_test:
            return self_test(annotate=args.annotate)
        if args.parse_log:
            return parse_log_command(args.parse_log, annotate=args.annotate, summary_file=args.summary_file)

        command = [token for token in args.command if token != "--"]
        if not command:
            parser.print_usage(sys.stderr)
            print("ci_test_report: no command given (put it after `--`)", file=sys.stderr)
            return 2
        return run_wrapped(
            command,
            tail_lines=args.tail_lines,
            annotate=args.annotate,
            max_tests=args.max_tests,
            summary_file=args.summary_file,
        )
    except Exception:  # noqa: BLE001 - a silent reporter is the bug this closes
        import traceback

        trace = traceback.format_exc()
        print(trace, file=sys.stderr)
        if annotations_enabled(args.annotate):
            print(
                workflow_command(
                    "error",
                    f"scripts/ci_test_report.py crashed and published nothing; traceback tail:\n{trace[-1200:]}",
                    title="ci_test_report.py internal error",
                ),
                file=sys.stdout,
            )
        return 70  # EX_SOFTWARE


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
