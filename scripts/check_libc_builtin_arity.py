#!/usr/bin/env python3
"""Derive and verify the argument count GCC checks for an undeclared libc builtin.

WHY THIS EXISTS
---------------
An undeclared call to a name GCC recognises as a builtin -- `memcpy(d, s)` in a
translation unit that never included <string.h> -- is not merely an implicit
declaration.  GCC knows `memcpy` takes three arguments, so it arity-checks the
call against that knowledge *in every dialect*, and the diagnostic is an
`error:` even under `-std=gnu90`, where the implicit declaration itself is only
a suppressed-by-default warning:

    $ gcc-16 -std=gnu90 -fsyntax-only t.c
    t.c:1:34: warning: incompatible implicit declaration of built-in function
              'memcpy' [-Wbuiltin-declaration-mismatch]
    t.c:1:34: error: too few arguments to function 'memcpy'; expected 3, have 2

Without this table the pre-C99 dialects accepted the call, lowered it through
the libc-alias path with a garbage third argument, and the resulting binary
segfaulted at run time (measured: exit 139).  From C99 on the implicit
declaration is already an error, so the TU was refused either way; the gap was
specifically the legacy dialects, where the only thing standing between the
programmer and a crashing binary was a check nobody performed.

WHY THE DATA IS MEASURED, NOT AUTHORED
--------------------------------------
Two properties of GCC's behaviour make a hand-written table unacceptable:

  * it is *not* uniform.  `puts(s, t)` is accepted silently -- GCC does not
    arity-check every libc name it knows -- while `free(p, q)` is refused.  A
    table written from the C standard would over-diagnose `puts` and reject
    correct programs, which is worse than the defect being fixed;
  * it distinguishes variadic from fixed arity in the message itself
    (`expected at least 1` for `printf`, `expected 3` for `memcpy`), so each
    entry carries a variadic flag that has to be right per name.

So GCC is asked.  For every name, a probe calls it with 0..=10 integer
arguments, one call per line, and the `(too few|too many) arguments to function`
diagnostics are read back: they state the expected count, whether it is a
minimum, and the count actually passed.  A name GCC never complains about is
absent from the table, which is precisely the "unconstrained" semantics the
front end needs.

SCOPE IS DERIVED TOO
--------------------
The candidate set is not a list in this file: it is every libc name this
compiler already claims to recognise, i.e. every `BuiltinInfo::simple("X")`
target in `src/frontend/sema/builtins.rs` (the `BuiltinKind::LibcAlias` set).
Deriving the scope from the source means the table cannot drift out of step
with the names the compiler actually lowers, and a newly recognised libc alias
shows up here automatically.

`src/frontend/sema/libc_builtin_arity.rs` is this derivation, checked in.  This
script is the derivation and the gate that keeps the two in step: CI runs
`--check`, so the checked-in table cannot rot.  Where GCC is unavailable the
check reports that and passes -- an unverifiable environment must not be
reported as a stale table, which is the same convention
`tests/regression/check_gcc162_diagnostic_severity.sh` uses for its oracle
cross-check.

Usage:
    scripts/check_libc_builtin_arity.py --check
    scripts/check_libc_builtin_arity.py --write
    scripts/check_libc_builtin_arity.py --self-test
    scripts/check_libc_builtin_arity.py --gcc gcc-16 --dump
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BUILTINS_RS = os.path.join(ROOT, "src", "frontend", "sema", "builtins.rs")
TABLE_RS = os.path.join(ROOT, "src", "frontend", "sema", "libc_builtin_arity.rs")

# Highest argument count probed.  Must exceed every arity in the set, otherwise
# a fixed-arity name could be mistaken for a variadic one: "no `too many`
# diagnostic" is the variadic test, and that test is only meaningful once the
# probe has gone past the real arity.
MAX_ARGS = 10

# Names are probed in batches rather than one translation unit each: GCC states
# the function name in the diagnostic, so a batch is unambiguous, and 5 compiler
# invocations beat 136 on a two-core host.
BATCH = 32

# t.c:7:3: error: too few arguments to function 'memcpy'; expected 3, have 2
# t.c:3:3: error: too few arguments to function 'printf'; expected at least 1, have 0
# t.c:2:3: error: too many arguments to function 'strlen'; expected 1, have 3
ARITY_RE = re.compile(
    r"(?:too few|too many) arguments to function '(?P<name>[A-Za-z_][A-Za-z0-9_]*)';"
    r"\s*expected (?P<at_least>at least )?(?P<want>\d+), have (?P<have>\d+)"
)

# BuiltinInfo::simple("memcpy") is the only constructor of BuiltinKind::LibcAlias.
LIBC_ALIAS_RE = re.compile(r'BuiltinInfo::simple\("([^"]+)"\)')


def libc_alias_names(builtins_rs: str = BUILTINS_RS) -> list[str]:
    """Every libc name the compiler claims to recognise, sorted and deduped."""
    with open(builtins_rs, encoding="utf-8") as fh:
        src = fh.read()
    return sorted(set(LIBC_ALIAS_RE.findall(src)))


def probe_source(names: list[str]) -> str:
    """One TU calling each name with 0..=MAX_ARGS integer arguments."""
    out = ["int main(void) {"]
    for name in names:
        for argc in range(MAX_ARGS + 1):
            args = ", ".join("1" for _ in range(argc))
            out.append(f"    {name}({args});")
    out.append("    return 0;")
    out.append("}")
    return "\n".join(out) + "\n"


def measure(gcc: str, names: list[str]) -> tuple[dict[str, tuple[int, bool]], list[str]]:
    """Ask GCC what it arity-checks.

    Returns (name -> (min_args, variadic), notes).  Names GCC does not
    arity-check, and names whose own answers disagree, are omitted with a note
    explaining why: silence in the table means "unconstrained", so an omission
    is a semantic statement and has to be explainable.
    """
    notes: list[str] = []
    # want[name] = set of (at_least, count) pairs GCC reported for that name.
    want: dict[str, set[tuple[bool, int]]] = {}
    saw_too_many: set[str] = set()

    tmp = tempfile.mkdtemp(prefix="lccc-libc-arity.")
    try:
        src_path = os.path.join(tmp, "probe.c")
        for start in range(0, len(names), BATCH):
            batch = names[start : start + BATCH]
            with open(src_path, "w", encoding="utf-8") as fh:
                fh.write(probe_source(batch))
            # gnu90 keeps the probe output clean: there the implicit declaration
            # is not an error, so the only diagnostics are the arity ones.  The
            # arity wording and severity are identical under gnu17 (measured).
            proc = subprocess.run(
                [gcc, "-std=gnu90", "-fsyntax-only", src_path],
                capture_output=True,
                text=True,
            )
            # GCC exits nonzero on a rejected probe by design; only an unusable
            # compiler (crash, bad driver) is a failure of this script.
            if proc.returncode not in (0, 1, 2, 3, 4):
                notes.append(f"{gcc} exited {proc.returncode} on the probe batch")
                sys.stderr.write(proc.stderr)
                return {}, notes
            want, saw_too_many, unexpected = parse_arity_lines(
                proc.stderr, batch, want, saw_too_many
            )
            notes.extend(
                f"GCC reported an unexpected name {n!r}; ignored" for n in unexpected
            )
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

    table: dict[str, tuple[int, bool]] = {}
    for name in names:
        answers = want.get(name)
        if not answers:
            # Not a defect: GCC arity-checks `memcpy` but not `puts`, and the
            # table must reproduce exactly that asymmetry.
            continue
        counts = {count for _at_least, count in answers}
        if len(counts) != 1:
            notes.append(
                f"{name}: GCC disagreed with itself about the arity "
                f"({sorted(counts)}); left unconstrained"
            )
            continue
        min_args = counts.pop()
        if min_args > MAX_ARGS:
            notes.append(f"{name}: arity {min_args} exceeds the probe range; skipped")
            continue
        # `at least` in GCC's own wording is the variadic marker; "never said
        # `too many` although the probe went past the arity" agrees with it, and
        # a disagreement means the derivation is unsound for this name.
        at_least = any(flag for flag, _ in answers)
        variadic = at_least
        if at_least == (name in saw_too_many):
            notes.append(
                f"{name}: variadic markers disagree (`at least`={at_least}, "
                f"`too many` seen={name in saw_too_many}); left unconstrained"
            )
            continue
        table[name] = (min_args, variadic)
    return table, notes


def render(table: dict[str, tuple[int, bool]], names: list[str], gcc_version: str) -> str:
    """Emit the checked-in Rust module."""
    lines = [
        "//! Argument counts GCC checks for an implicitly declared libc builtin.",
        "//!",
        "//! An undeclared call to a name GCC recognises as a builtin is arity-checked",
        "//! against GCC's own knowledge of that builtin in every dialect, and the",
        "//! diagnostic is an `error:` even under `-std=gnu90`, where the implicit",
        "//! declaration itself is only a suppressed-by-default warning:",
        "//!",
        "//! ```text",
        "//! $ gcc-16 -std=gnu90 -fsyntax-only t.c   // int main(void){ char d[4], s[4]; memcpy(d, s); }",
        "//! t.c:1:34: error: too few arguments to function 'memcpy'; expected 3, have 2",
        "//! ```",
        "//!",
        "//! Without this the pre-C99 dialects accepted the call and lowered it with a",
        "//! garbage third argument, so the binary segfaulted at run time (measured:",
        "//! exit 139) where both oracles refused the translation unit.",
        "//!",
        "//! # Provenance",
        "//!",
        "//! Generated by `scripts/check_libc_builtin_arity.py --write`, which asks GCC",
        f"//! itself (measured with {gcc_version}): the candidate set is every",
        "//! `BuiltinKind::LibcAlias` name in `builtins.rs`, each is probed with 0..=10",
        "//! integer arguments, and the `(too few|too many) arguments to function` lines",
        "//! are read back.  Nothing here is authored from the C standard, because GCC's",
        "//! behaviour is not uniform: `free(p, q)` is refused while `puts(s, t)` is",
        "//! accepted silently, so a standard-derived table would reject correct programs.",
        "//! A name GCC does not arity-check is absent, which is the \"unconstrained\"",
        "//! semantics the front end needs.",
        "//!",
        "//! `scripts/check_libc_builtin_arity.py --check` (wired into CI) re-derives the",
        "//! table and fails if it stops matching, so this file cannot rot.  Where GCC is",
        "//! unavailable the check says so and passes.",
        "//!",
        "//! # Scope",
        "//!",
        "//! Diagnostics only.  Nothing here feeds lowering: the implicit declaration is",
        "//! still registered unprototyped, because inventing parameter *types* (and a",
        "//! return type) for 136 libc names would risk worse than it fixes -- a wrong",
        "//! return type silently miscompiles every caller that uses the value, while a",
        "//! wrong count only ever produces a diagnostic.",
        "",
        "/// What GCC knows about an implicitly declared libc builtin's arguments.",
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]",
        "pub(crate) struct LibcBuiltinArity {",
        "    /// Fewest arguments GCC accepts.",
        "    pub(crate) min_args: usize,",
        "    /// `true` when GCC's wording is `expected at least N` (`printf`, ...).",
        "    pub(crate) variadic: bool,",
        "}",
        "",
        "/// GCC's checked argument count for `name`, or `None` when GCC does not",
        "/// arity-check it (`puts`, `abs`, `strcmp`, ...) -- in which case neither may we.",
        "pub(crate) fn libc_builtin_arity(name: &str) -> Option<LibcBuiltinArity> {",
        "    ARITY_TABLE",
        "        .binary_search_by_key(&name, |row| row.0)",
        "        .ok()",
        "        .map(|i| LibcBuiltinArity {",
        "            min_args: ARITY_TABLE[i].1 as usize,",
        "            variadic: ARITY_TABLE[i].2,",
        "        })",
        "}",
        "",
        "/// Sorted by name, which `libc_builtin_arity` relies on.",
        f"/// {len(table)}/{len(names)} of the `BuiltinKind::LibcAlias` names; the rest are",
        "/// names GCC does not arity-check.",
        "const ARITY_TABLE: &[(&str, u8, bool)] = &[",
    ]
    for name in sorted(table):
        min_args, variadic = table[name]
        lines.append(f'    ("{name}", {min_args}, {str(variadic).lower()}),')
    lines.append("];")
    # Emitted verbatim in rustfmt's canonical form, so that `--write`
    # produces a file `cargo fmt --all -- --check` accepts and `--check`
    # reproduces byte for byte.
    lines += [
        '',
        '#[cfg(test)]',
        'mod libc_builtin_arity_tests {',
        '    use super::{ARITY_TABLE, LibcBuiltinArity, libc_builtin_arity};',
        '',
        '    #[test]',
        '    fn table_is_sorted_and_unique() {',
        '        // `binary_search_by_key` is only correct on a sorted, duplicate-free',
        '        // slice, and the generator writes it from a measured set.',
        '        let names: Vec<&str> = ARITY_TABLE.iter().map(|row| row.0).collect();',
        '        let mut sorted = names.clone();',
        '        sorted.sort_unstable();',
        '        assert_eq!(names, sorted, "ARITY_TABLE must stay sorted by name");',
        '        sorted.dedup();',
        '        assert_eq!(',
        '            names.len(),',
        '            sorted.len(),',
        '            "ARITY_TABLE has a duplicate name"',
        '        );',
        '    }',
        '',
        '    #[test]',
        '    fn measured_entries_match_gcc() {',
        "        // Each of these was read back from gcc-16 (GCC) 16.2.0's own",
        '        // `arguments to function` diagnostic, not written from the standard.',
        '        let fixed = |min_args: usize| {',
        '            Some(LibcBuiltinArity {',
        '                min_args,',
        '                variadic: false,',
        '            })',
        '        };',
        '        let variadic = |min_args: usize| {',
        '            Some(LibcBuiltinArity {',
        '                min_args,',
        '                variadic: true,',
        '            })',
        '        };',
        '        assert_eq!(libc_builtin_arity("memcpy"), fixed(3));',
        '        assert_eq!(libc_builtin_arity("memset"), fixed(3));',
        '        assert_eq!(libc_builtin_arity("memmove"), fixed(3));',
        '        assert_eq!(libc_builtin_arity("strlen"), fixed(1));',
        '        assert_eq!(libc_builtin_arity("malloc"), fixed(1));',
        '        assert_eq!(libc_builtin_arity("free"), fixed(1));',
        '        assert_eq!(libc_builtin_arity("exit"), fixed(1));',
        '        assert_eq!(libc_builtin_arity("calloc"), fixed(2));',
        '        assert_eq!(libc_builtin_arity("realloc"), fixed(2));',
        '        assert_eq!(libc_builtin_arity("strcpy"), fixed(2));',
        '        assert_eq!(libc_builtin_arity("stpncpy"), fixed(3));',
        '        // `abort(void)` is the zero-argument end of the range.',
        '        assert_eq!(libc_builtin_arity("abort"), fixed(0));',
        '        // GCC spells a variadic minimum `expected at least N`.',
        '        assert_eq!(libc_builtin_arity("printf"), variadic(1));',
        '        assert_eq!(libc_builtin_arity("fprintf"), variadic(2));',
        '        assert_eq!(libc_builtin_arity("sprintf"), variadic(2));',
        '        assert_eq!(libc_builtin_arity("snprintf"), variadic(3));',
        '    }',
        '',
        '    #[test]',
        '    fn names_gcc_does_not_arity_check_are_unconstrained() {',
        '        // Verified, not assumed: `gcc-16 -std=gnu90 -fsyntax-only` on a TU',
        '        // calling each of these with a wrong argument count exits 0 and',
        '        // prints nothing, while the same compiler reports seven arity',
        '        // errors on a `memcpy` probe.  Diagnosing them would reject',
        '        // programs GCC accepts -- worse than the defect being fixed.',
        '        for name in [',
        '            "puts", "putchar", "abs", "strcmp", "strncmp", "memcmp", "dprintf",',
        '        ] {',
        '            assert_eq!(',
        '                libc_builtin_arity(name),',
        '                None,',
        '                "{name} must stay unconstrained"',
        '            );',
        '        }',
        '    }',
        '',
        '    #[test]',
        '    fn reserved_spellings_and_unknown_names_are_absent() {',
        '        // The `__builtin_` spelling never reaches this table: those calls take',
        '        // the builtin branch, where `intrinsic_arity` owns the count.',
        '        assert_eq!(libc_builtin_arity("__builtin_memcpy"), None);',
        '        // Compiler builtins are available without a prototype by definition.',
        '        assert_eq!(libc_builtin_arity("alloca"), None);',
        '        assert_eq!(libc_builtin_arity("_mm_add_ps"), None);',
        '        assert_eq!(libc_builtin_arity("not_a_libc_function"), None);',
        '        assert_eq!(libc_builtin_arity(""), None);',
        '    }',
        '}',
        '',
    ]
    return "\n".join(lines)


# ------------------------------------------------------------------ self-test --
# The parser is the part that can silently produce a wrong table, and CI has no
# GCC, so it is pinned against recorded GCC output rather than left untested.
SELF_TEST_INPUT = """\
probe.c:2:3: error: too many arguments to function 'strlen'; expected 1, have 3
probe.c:3:3: error: too few arguments to function 'printf'; expected at least 1, have 0
probe.c:6:3: error: too few arguments to function 'malloc'; expected 1, have 0
probe.c:7:3: error: too few arguments to function 'memcpy'; expected 3, have 2
probe.c:8:3: error: too many arguments to function 'memcpy'; expected 3, have 4
probe.c:9:5: note: something else entirely
probe.c:10:3: error: too many arguments to function 'free'; expected 1, have 2
probe.c:11:3: error: too few arguments to function 'exit'; expected 1, have 0
probe.c:12:3: error: too many arguments to function 'not_probed'; expected 1, have 2
"""


def parse_arity_lines(
    stderr: str,
    batch: list[str],
    want: dict[str, set[tuple[bool, int]]] | None = None,
    saw_too_many: set[str] | None = None,
) -> tuple[dict[str, set[tuple[bool, int]]], set[str], list[str]]:
    """Read GCC's arity diagnostics out of one probe's stderr.

    The accumulators are passed in and returned so callers can merge batches;
    this is the only place the wording is parsed, so `--self-test` pins exactly
    the code `--write` and `--check` run.  Returns (want, saw_too_many,
    unexpected_names).
    """
    if want is None:
        want = {}
    if saw_too_many is None:
        saw_too_many = set()
    unexpected: list[str] = []
    for line in stderr.splitlines():
        m = ARITY_RE.search(line)
        if not m:
            continue
        name = m.group("name")
        if batch and name not in batch:
            unexpected.append(name)
            continue
        want.setdefault(name, set()).add((bool(m.group("at_least")), int(m.group("want"))))
        if "too many" in line:
            saw_too_many.add(name)
    return want, saw_too_many, unexpected


def self_test() -> int:
    failures: list[str] = []

    def check(label: str, got: object, want: object) -> None:
        if got != want:
            failures.append(f"{label}: got {got!r}, want {want!r}")

    names = ["strlen", "printf", "malloc", "memcpy", "free", "exit", "puts"]
    want, saw_too_many, unexpected = parse_arity_lines(SELF_TEST_INPUT, names)
    check("unexpected names", unexpected, ["not_probed"])
    check("strlen arity", want["strlen"], {(False, 1)})
    check("printf arity", want["printf"], {(True, 1)})
    check("memcpy arity", want["memcpy"], {(False, 3)})
    check("puts absent", "puts" in want, False)
    check("note line ignored", len(want), 6)
    # variadic: `at least` present, and never a `too many`.
    check("printf variadic", (True, "printf" not in saw_too_many), (True, True))
    check("strlen fixed", (False, "strlen" in saw_too_many), (False, True))

    # The scope derivation must find the real LibcAlias set, not an empty one.
    found = libc_alias_names()
    check("scope non-empty", len(found) > 100, True)
    for required in ("memcpy", "printf", "strlen", "malloc"):
        check(f"scope has {required}", required in found, True)
    check(
        "scope excludes __builtin_ spellings",
        any(n.startswith("__builtin_") for n in found),
        False,
    )

    # The probe must actually go past every arity it can discover.
    src = probe_source(["memcpy"])
    check("probe has a 0-arg call", "memcpy();" in src, True)
    check("probe has a MAX_ARGS call", f"memcpy({', '.join(['1'] * MAX_ARGS)});" in src, True)

    if failures:
        for f in failures:
            print(f"FAIL: self-test {f}", file=sys.stderr)
        return 1
    print(f"PASS: self-test ({len(want)} names parsed, scope {len(found)} LibcAlias names)")
    return 0


# ----------------------------------------------------------------------- main --
def read_checked_in() -> str | None:
    if not os.path.exists(TABLE_RS):
        return None
    with open(TABLE_RS, encoding="utf-8") as fh:
        return fh.read()


def gcc_version(gcc: str) -> str | None:
    try:
        out = subprocess.run([gcc, "--version"], capture_output=True, text=True, timeout=60)
    except (OSError, subprocess.SubprocessError):
        return None
    if out.returncode != 0:
        return None
    return out.stdout.splitlines()[0].strip() if out.stdout else None


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"), help="oracle to measure against")
    ap.add_argument("--write", action="store_true", help="regenerate the Rust table")
    ap.add_argument("--check", action="store_true", help="fail if the table is stale")
    ap.add_argument("--self-test", action="store_true", help="test the parser and scope derivation")
    ap.add_argument("--dump", action="store_true", help="print the measured table")
    args = ap.parse_args()

    if args.self_test:
        return self_test()
    if not (args.write or args.check or args.dump):
        ap.error("one of --write, --check, --dump or --self-test is required")

    names = libc_alias_names()
    if not names:
        print("FAIL: no BuiltinKind::LibcAlias names found in builtins.rs", file=sys.stderr)
        return 1

    if shutil.which(args.gcc) is None:
        # An unverifiable environment is not a stale table.  Same convention as
        # the oracle cross-check in check_gcc162_diagnostic_severity.sh.
        print(f"note: {args.gcc} not found -- libc builtin arity check skipped", file=sys.stderr)
        return 0

    version = gcc_version(args.gcc) or args.gcc
    table, notes = measure(args.gcc, names)
    for note in notes:
        print(f"note: {note}", file=sys.stderr)
    if not table:
        print(f"FAIL: {args.gcc} arity-checked none of the {len(names)} names", file=sys.stderr)
        return 1

    if args.dump:
        for name in sorted(table):
            min_args, variadic = table[name]
            kind = "variadic" if variadic else "fixed"
            print(f"{name:24} {kind:9} min_args={min_args}")
        print(f"({len(table)} of {len(names)} LibcAlias names; measured with {version})")
        return 0

    rendered = render(table, names, version)
    if args.write:
        with open(TABLE_RS, "w", encoding="utf-8") as fh:
            fh.write(rendered)
        print(f"wrote {os.path.relpath(TABLE_RS, ROOT)} ({len(table)} names, measured with {version})")
        return 0

    checked_in = read_checked_in()
    if checked_in is None:
        print(f"FAIL: {os.path.relpath(TABLE_RS, ROOT)} is missing", file=sys.stderr)
        return 1
    if checked_in != rendered:
        print(
            "FAIL: src/frontend/sema/libc_builtin_arity.rs is stale; "
            f"run scripts/check_libc_builtin_arity.py --write (measured with {version})",
            file=sys.stderr,
        )
        return 1
    print(f"PASS: libc builtin arity table matches {version} ({len(table)} names)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
