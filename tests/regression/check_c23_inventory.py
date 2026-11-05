#!/usr/bin/env python3
"""Measure a fresh C23 accept/reject matrix against the pinned GCC oracle.

This is deliberately a probe inventory, not a conformance percentage: each
row is a small translation unit, compiled in both C23 and legacy C17 modes by
GCC 16.2 and LCCC.  C17 outcomes are recorded rather than inferred from the
standard because GCC accepts a number of C23 spellings as GNU extensions.
The default-mode control separately checks the branch's GNU C23 default.

The script exits non-zero only when the GCC oracle is not the requested 16.2
series, a probe's declared GCC expectation is violated, or --fail-on-diff is
requested and LCCC disagrees with GCC in C23 mode.  LCCC differences are
otherwise reported as explicit follow-up gaps instead of being hidden by a
pass/fail total.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import shlex
import subprocess
import sys
import tempfile
import textwrap
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Iterable

ROOT = Path(__file__).resolve().parents[2]
INVENTORY_DATE = "2026-11-04"
MODES: dict[str, tuple[str, ...]] = {
    "c23": ("-std=c23",),
    "gnu23": ("-std=gnu23",),
    "c17": ("-std=c17",),
    "gnu17": ("-std=gnu17",),
    "default": (),
}

# The strict dialect a GNU one extends.  A probe that declares no expectation
# for `gnu17` is expected to behave as it does in `c17`: the GNU dialects add
# extensions, they do not change what the standard dialect rejects unless the
# probe says so.
STRICT_BASE = {"gnu17": "c17", "gnu23": "c23"}

# Probes whose LCCC verdict differs from GCC's on purpose, with the reason.  A
# difference that is not listed here fails the gate; the list is the only place
# a known gap may live, and every entry is printed on every run.
KNOWN_DEVIATIONS: dict[str, str] = {
    "bitint-17": (
        "C23 `_BitInt(N)` needs arbitrary precision through conversions, layout, "
        "ABI classification and debug info; the fixed-width type model cannot "
        "carry it, so LCCC rejects it in every mode (backlog: the `_BitInt` item)"
    ),
}


@dataclass(frozen=True)
class Probe:
    name: str
    category: str
    description: str
    source: str
    expected: dict[str, str] = field(default_factory=lambda: {"c23": "accept"})
    modes: tuple[str, ...] = ("c23", "gnu23", "c17", "gnu17")
    resources: dict[str, bytes] = field(default_factory=dict)


PROBES: tuple[Probe, ...] = (
    Probe(
        "default-gnu23-version",
        "dialect control",
        "default mode defines the GCC 16.2 C23 __STDC_VERSION__ value",
        """
        #if !defined(__STDC_VERSION__) || __STDC_VERSION__ != 202311L
        #error default mode is not C23
        #endif
        int c23_default_control;
        """,
        expected={"default": "accept", "c23": "accept", "c17": "reject"},
        modes=("default", "c23", "c17"),
    ),
    Probe(
        "bool-true-false",
        "core language",
        "C23 bool, true, and false spellings",
        """
        int main(void) {
            bool yes = true;
            bool no = false;
            return (yes && !no) ? 0 : 1;
        }
        """,
    ),
    Probe(
        "nullptr",
        "core language",
        "nullptr converts to object pointers and compares with nullptr",
        """
        int main(void) {
            void *p = nullptr;
            return p != nullptr;
        }
        """,
    ),
    Probe(
        "constexpr-named-constant",
        "declarations and constants",
        "constexpr object usable by a static assertion and array bound",
        """
        constexpr int count = 4;
        static_assert(count == 4);
        int values[count];
        int main(void) { return (int)sizeof(values) == 4 * (int)sizeof(int) ? 0 : 1; }
        """,
    ),
    Probe(
        "auto-type-inference",
        "declarations and constants",
        "C23 auto infers an object type from its initializer",
        """
        int main(void) {
            auto count = 42;
            return count == 42 ? 0 : 1;
        }
        """,
    ),
    Probe(
        "typeof-and-typeof-unqual",
        "type system",
        "typeof preserves qualification and typeof_unqual removes it",
        """
        int main(void) {
            const int source = 3;
            typeof(source) qualified = source;
            typeof_unqual(source) unqualified = 4;
            return qualified + unqualified == 7 ? 0 : 1;
        }
        """,
    ),
    Probe(
        "fixed-enum-underlying-type",
        "type system",
        "C23 fixed underlying enum type and its size",
        """
        enum Color : unsigned char { Red = 1, Blue = 2 };
        _Static_assert(sizeof(enum Color) == sizeof(unsigned char), "enum base");
        int main(void) { enum Color c = Blue; return c == 2 ? 0 : 1; }
        """,
    ),
    Probe(
        "digit-separator",
        "lexical",
        "digit separator in an integer constant",
        """
        static_assert(1'000 == 1000);
        int main(void) { return 1'000 == 1000 ? 0 : 1; }
        """,
    ),
    Probe(
        "empty-initializer",
        "initialization",
        "empty initializer braces zero-initialize an aggregate",
        """
        struct Pair { int first; int second; };
        int main(void) {
            struct Pair pair = {};
            return pair.first == 0 && pair.second == 0 ? 0 : 1;
        }
        """,
    ),
    Probe(
        "static-assert-keyword",
        "declarations and constants",
        "static_assert keyword form",
        """
        static_assert(sizeof(int) >= 2, "int width");
        int main(void) { return 0; }
        """,
    ),
    Probe(
        "alignof-keyword",
        "type system",
        "alignof keyword and aligned declaration",
        """
        alignas(16) int aligned_value;
        static_assert(alignof(int) >= 1, "natural alignment");
        int main(void) { return (unsigned long)&aligned_value % 16 != 0; }
        """,
    ),
    Probe(
        "standard-maybe-unused-attribute",
        "attributes",
        "C23 [[maybe_unused]] attribute on a block-scope object",
        """
        int main(void) {
            [[maybe_unused]] int intentionally_unused = 7;
            return 0;
        }
        """,
    ),
    Probe(
        "unnamed-variadic-function",
        "variadic functions",
        "C23 function definition with an unnamed variadic parameter list",
        """
        static int has_argument(...) { return 1; }
        int main(void) { return has_argument(7) == 1 ? 0 : 1; }
        """,
    ),
    Probe(
        "one-argument-va-start",
        "variadic functions",
        "C23 one-argument va_start form from the implementation stdarg.h",
        """
        #include <stdarg.h>
        static int first_variadic(int named, ...) {
            va_list ap;
            va_start(ap);
            int value = va_arg(ap, int);
            va_end(ap);
            return value;
        }
        int main(void) { return first_variadic(0, 9) == 9 ? 0 : 1; }
        """,
    ),
    Probe(
        "elifdef",
        "preprocessor",
        "#elifdef selects a branch when the macro is defined",
        """
        #define INVENTORY_SET 1
        #if 0
        #elifdef INVENTORY_SET
        int elifdef_selected;
        #else
        #error elifdef failed
        #endif
        """,
        # GCC enables the directive as an extension in its GNU dialects and
        # errors on it in every strict dialect before C23.
        expected={"c23": "accept", "gnu23": "accept", "c17": "reject", "gnu17": "accept"},
    ),
    Probe(
        "elifndef",
        "preprocessor",
        "#elifndef selects a branch when the macro is not defined",
        """
        #if 0
        #elifndef INVENTORY_NOT_SET
        int elifndef_selected;
        #else
        #error elifndef failed
        #endif
        """,
        expected={"c23": "accept", "gnu23": "accept", "c17": "reject", "gnu17": "accept"},
    ),
    Probe(
        "embed-bytes",
        "preprocessor",
        "#embed contributes resource bytes to an initializer",
        """
        static const unsigned char payload[] = {
        #embed "payload.bin"
        };
        _Static_assert(sizeof(payload) == 3, "embedded byte count");
        int main(void) { return payload[0] == 1 && payload[1] == 2 && payload[2] == 255 ? 0 : 1; }
        """,
        resources={"payload.bin": b"\x01\x02\xff"},
    ),
    Probe(
        "has-embed-operator",
        "preprocessor",
        "__has_embed reports a present resource",
        """
        #if __has_embed("payload.bin") != __STDC_EMBED_FOUND__
        #error resource should be found
        #endif
        int has_embed_selected;
        """,
        resources={"payload.bin": b"x"},
    ),
    Probe(
        "va-opt",
        "preprocessor",
        "__VA_OPT__ conditionally emits variadic macro tokens",
        """
        #define INVENTORY_LOG(fmt, ...) call_log(fmt __VA_OPT__(,) __VA_ARGS__)
        int call_log(const char *, ...);
        int main(void) {
            INVENTORY_LOG("empty");
            INVENTORY_LOG("value", 4);
            return 0;
        }
        """,
    ),
    Probe(
        "bitint-17",
        "type system",
        "C23 _BitInt(17) integer type (included as a coverage gap probe)",
        """
        unsigned _BitInt(17) value = 65535;
        static_assert(sizeof(value) >= 3, "bitint width");
        int main(void) { return value == 65535 ? 0 : 1; }
        """,
    ),
    Probe(
        "float16-gcc-extension",
        "GCC extension",
        "GCC _Float16 extension in the C23 dialect",
        """
        _Float16 half_value = 1.5f;
        int main(void) { return half_value > 1 ? 0 : 1; }
        """,
    ),
    Probe(
        "nullptr-arithmetic-rejected",
        "constraint rejection",
        "invalid arithmetic on nullptr is rejected in C23 mode",
        """
        int main(void) { return (nullptr + 1) != nullptr; }
        """,
        expected={"c23": "reject"},
    ),
    Probe(
        "empty-list-prototype-arity",
        "dialect boundary",
        "C23 f() is a zero-parameter prototype; C17 f() remains unspecified",
        """
        int f();
        int caller(void) { return f(1); }
        """,
        expected={"c23": "reject", "c17": "accept"},
    ),
    Probe(
        "empty-list-prototype-valid-call",
        "dialect boundary",
        "an empty-list prototype accepts a zero-argument call in both modes",
        """
        int f();
        int caller(void) { return f(); }
        """,
    ),
    # --- the C17 side of the same boundary ---------------------------------
    # C23 made `static_assert`, `alignof`, `alignas` and the `u8` character
    # constant into features of the *language*.  In C17 the first three are
    # ordinary identifiers that <assert.h>/<stdalign.h> turn into macros, and
    # `u8` is just a name, so a C17 compiler must both reject the C23 spelling
    # and keep accepting the identifiers.  gcc 16.2 and clang 23 agree on every
    # one of these, in `gnu17` as well as `c17`.
    Probe(
        "implicit-int-storage-class",
        "dialect boundary",
        "a declaration with a storage class and no type is implicit int (C89 only)",
        """
        static counter = 5;
        int f(void) { register local = 1; return local + counter; }
        """,
        expected={"c23": "reject", "c17": "reject"},
    ),
    Probe(
        "implicit-int-auto-c17",
        "dialect boundary",
        "`auto x = 3;` is C23 inference, and implicit int before it",
        """
        int f(void) { auto count = 42; return count; }
        """,
        expected={"c23": "accept", "c17": "reject"},
    ),
    Probe(
        "static-assert-identifier-c17",
        "dialect boundary",
        "`static_assert` is an ordinary identifier outside C23",
        """
        void static_assert(void);
        int f(void) { static_assert(); return 0; }
        """,
        expected={"c23": "reject", "c17": "accept"},
    ),
    Probe(
        "alignof-identifier-c17",
        "dialect boundary",
        "`alignof` is an ordinary identifier outside C23",
        """
        int alignof = 3;
        int f(void) { return alignof; }
        """,
        expected={"c23": "reject", "c17": "accept"},
    ),
    Probe(
        "alignas-identifier-c17",
        "dialect boundary",
        "`alignas` is an ordinary identifier outside C23",
        """
        int alignas = 4;
        int f(void) { return alignas; }
        """,
        expected={"c23": "reject", "c17": "accept"},
    ),
    Probe(
        "u8-identifier",
        "dialect boundary",
        "`u8` stays an ordinary identifier in every dialect: it is only a prefix "
        "when a string or character constant follows",
        """
        int u8 = 5;
        int f(void) { return u8; }
        """,
        expected={"c23": "accept", "c17": "accept"},
    ),
    Probe(
        "u8-char-literal",
        "character constants",
        "`u8'x'` is a C23 character constant, not an identifier before it",
        """
        int f(void) { return u8'a'; }
        """,
        expected={"c23": "accept", "c17": "reject"},
    ),
    Probe(
        "stdc-version-c17",
        "dialect control",
        "the C17 dialects define __STDC_VERSION__ 201710L and nothing newer",
        """
        #if !defined(__STDC_VERSION__) || __STDC_VERSION__ != 201710L
        #error not C17
        #endif
        int f(void) { return 0; }
        """,
        expected={"c23": "reject", "c17": "accept"},
    ),
    Probe(
        "stdc-version-c23",
        "dialect control",
        "the C23 dialects define __STDC_VERSION__ 202311L",
        """
        #if !defined(__STDC_VERSION__) || __STDC_VERSION__ != 202311L
        #error not C23
        #endif
        int f(void) { return 0; }
        """,
        expected={"c23": "accept", "c17": "reject"},
    ),
    Probe(
        "strict-ansi-in-iso-modes",
        "dialect control",
        "`-std=c17`/`-std=c23` define __STRICT_ANSI__, and a GNU dialect does not",
        """
        #ifdef __STRICT_ANSI__
        int strict;
        #else
        #error this dialect is not strict
        #endif
        int f(void) { return 0; }
        """,
        expected={"c23": "accept", "c17": "accept", "gnu23": "reject", "gnu17": "reject"},
    ),
    Probe(
        "gnu-dialect-rejects-strict-control",
        "dialect control",
        "`-std=gnu17`/`-std=gnu23` leave __STRICT_ANSI__ undefined",
        """
        #ifdef __STRICT_ANSI__
        #error this dialect is strict
        #endif
        int f(void) { return 0; }
        """,
        expected={"c23": "reject", "c17": "reject", "gnu23": "accept", "gnu17": "accept"},
    ),
    Probe(
        "typeof-unqual-needs-c23",
        "dialect boundary",
        "bare `typeof_unqual` is C23 only; `__typeof_unqual__` is reserved everywhere",
        """
        int f(void) {
            const int source = 3;
            __typeof_unqual__(source) reserved = source;
            return reserved;
        }
        """,
        expected={"c23": "accept", "c17": "accept"},
    ),
    Probe(
        "typeof-unqual-bare-in-gnu17",
        "dialect boundary",
        "bare `typeof_unqual` is not a GNU extension: gcc 16.2 rejects it in gnu17",
        """
        int f(void) {
            const int source = 3;
            typeof_unqual(source) unqualified = source;
            return unqualified;
        }
        """,
        expected={"c23": "accept", "c17": "reject", "gnu17": "reject", "gnu23": "accept"},
    ),
    Probe(
        "macro-arity-too-few",
        "preprocessor constraints",
        "a short invocation is a constraint violation (6.10.3p4)",
        """
        #define ARITY_PAIR(a, b) ((a) + (b))
        int f(void) { return ARITY_PAIR(1); }
        """,
        expected={"c23": "reject", "c17": "reject"},
    ),
    Probe(
        "macro-arity-too-many",
        "preprocessor constraints",
        "a long invocation is a constraint violation (6.10.3p4)",
        """
        #define ARITY_ONE(a) (a)
        int f(void) { return ARITY_ONE(1, 2); }
        """,
        expected={"c23": "reject", "c17": "reject"},
    ),
    Probe(
        "macro-empty-argument-is-one-argument",
        "preprocessor constraints",
        "`ONE()` is a valid one-argument invocation of a one-parameter macro",
        """
        #define ARITY_ONE(a) 0
        int f(void) { return ARITY_ONE(); }
        """,
        expected={"c23": "accept", "c17": "accept"},
    ),
    Probe(
        "macro-variadic-omitted-tail",
        "preprocessor constraints",
        "the variadic part of a macro argument list may be empty",
        """
        #define ARITY_TAIL(a, ...) (a)
        int f(void) { return ARITY_TAIL(1) + ARITY_TAIL(1, 2, 3); }
        """,
        expected={"c23": "accept", "c17": "accept"},
    ),
    Probe(
        "elifdef-in-skipped-group",
        "dialect boundary",
        "before C23 a skipped #elifdef is an unknown directive: the group keeps skipping, "
        "so the #else runs instead of the #elifdef branch",
        """
        #define SKIPPED_SET 1
        #if 0
        #elifdef SKIPPED_SET
        #error the skipped group must not be re-opened
        #else
        int skipped_group_else_branch;
        #endif
        int f(void) { return 0; }
        """,
        # In a GNU dialect the directive is recognised, so the branch is taken
        # and the `#error` inside it fires; only the strict pre-C23 dialects
        # treat the spelling as an unknown directive and keep skipping.
        expected={"c23": "reject", "gnu23": "reject", "c17": "accept", "gnu17": "reject"},
    ),
    # `-std=c17` does not switch gcc's extensions *off*; it only stops the
    # dialect from *defining* them.  `({...})`, `__attribute__`, `[0]` arrays,
    # case ranges and the omitted `?:` operand are all accepted in strict ISO
    # mode by gcc 16.2 and clang 23 until `-pedantic-errors` is added, so a
    # compiler that rejected them in `-std=c17` would be *less* compatible, not
    # more.  Pinning that keeps a future "strict mode" cleanup from silently
    # breaking real-world code.
    Probe(
        "gnu-extension-accepted-without-pedantic",
        "dialect boundary",
        "GNU extensions stay accepted in strict ISO modes unless -pedantic-errors",
        """
        int f(void) { return ({ int t = 2; t + 1; }); }
        """,
        expected={"c23": "accept", "c17": "accept", "gnu23": "accept", "gnu17": "accept"},
    ),
)


# GCC 16.2's measured verdict for every probe under `-std=c17`.  The matrix is
# measured in C17 as much as in C23 -- a C23 feature that leaks into an older
# dialect is exactly the defect class this gate exists for -- so the expected
# C17 outcome is pinned for every probe instead of being merely observed, and a
# probe that does not state its own C17 expectation inherits this one.  The
# table is complete by construction: `self_test` refuses to run if a probe is
# missing, so adding a probe without deciding what GCC does in C17 fails fast.
C17_VERDICTS: dict[str, str] = {
    "default-gnu23-version": "reject",
    "bool-true-false": "reject",
    "nullptr": "reject",
    "constexpr-named-constant": "reject",
    "auto-type-inference": "reject",
    "typeof-and-typeof-unqual": "reject",
    "fixed-enum-underlying-type": "accept",
    "digit-separator": "reject",
    "empty-initializer": "accept",
    "static-assert-keyword": "reject",
    "alignof-keyword": "reject",
    "standard-maybe-unused-attribute": "accept",
    "unnamed-variadic-function": "accept",
    "one-argument-va-start": "reject",
    "elifdef": "reject",
    "elifndef": "reject",
    "embed-bytes": "accept",
    "has-embed-operator": "accept",
    "va-opt": "accept",
    "bitint-17": "reject",
    "float16-gcc-extension": "accept",
    "nullptr-arithmetic-rejected": "reject",
    "empty-list-prototype-arity": "accept",
    "empty-list-prototype-valid-call": "accept",
    "implicit-int-storage-class": "reject",
    "implicit-int-auto-c17": "reject",
    "static-assert-identifier-c17": "accept",
    "alignof-identifier-c17": "accept",
    "alignas-identifier-c17": "accept",
    "u8-identifier": "accept",
    "u8-char-literal": "reject",
    "stdc-version-c17": "accept",
    "stdc-version-c23": "reject",
    "strict-ansi-in-iso-modes": "accept",
    "gnu-dialect-rejects-strict-control": "reject",
    "gnu-extension-accepted-without-pedantic": "accept",
    "typeof-unqual-needs-c23": "accept",
    "typeof-unqual-bare-in-gnu17": "reject",
    "macro-arity-too-few": "reject",
    "macro-arity-too-many": "reject",
    "macro-empty-argument-is-one-argument": "accept",
    "macro-variadic-omitted-tail": "accept",
    "elifdef-in-skipped-group": "accept",
    # `#embed`, `__has_embed`, `__VA_OPT__` and the C23 attribute spelling are
    # preprocessor additions in GCC and stay enabled in every dialect; only
    # `-pedantic-errors` turns them into diagnostics.  Measured, not assumed.
}


def expected_verdict(probe: Probe, mode: str) -> str | None:
    """The GCC verdict a probe must produce in `mode`, or None if unconstrained."""
    declared = probe.expected.get(mode)
    if declared:
        return declared
    base = STRICT_BASE.get(mode)
    if base and probe.expected.get(base):
        return probe.expected[base]
    if mode in ("c17", "gnu17"):
        return C17_VERDICTS.get(probe.name)
    return None


def compiler_version(command: list[str]) -> str:
    proc = subprocess.run(
        [*command, "-dumpfullversion", "-dumpversion"],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
        timeout=30,
    )
    if proc.returncode:
        raise RuntimeError(
            f"cannot query GCC version: {proc.stderr.strip() or proc.returncode}"
        )
    return proc.stdout.strip().splitlines()[0] if proc.stdout.strip() else "unknown"


def run_compiler(
    command: list[str], flags: Iterable[str], source_path: Path, cwd: Path
) -> dict[str, object]:
    argv = [*command, *flags, "-fsyntax-only", "-x", "c", source_path.name]
    try:
        proc = subprocess.run(
            argv,
            cwd=cwd,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
            timeout=45,
        )
        return {
            "verdict": "accept" if proc.returncode == 0 else "reject",
            "exit_code": proc.returncode,
            "stdout": proc.stdout[-2000:],
            "stderr": proc.stderr[-4000:],
            "argv": argv,
        }
    except subprocess.TimeoutExpired as exc:
        return {
            "verdict": "timeout",
            "exit_code": None,
            "stdout": str(exc.stdout or "")[-2000:],
            "stderr": str(exc.stderr or "")[-4000:],
            "argv": argv,
        }


def resolve_command(command: list[str]) -> list[str]:
    """Resolve an explicit relative executable against the repository root.

    Probe subprocesses run with each case directory as cwd, so leaving the
    default `target/fastbuild/lccc` relative would fail only in full runs.
    Bare command names continue to resolve through PATH.
    """
    if command and not Path(command[0]).is_absolute() and "/" in command[0]:
        command[0] = str((ROOT / command[0]).resolve())
    return command


def execute(args: argparse.Namespace) -> tuple[dict[str, object], int]:
    gcc_command = resolve_command(shlex.split(args.gcc))
    lccc_command = resolve_command(shlex.split(args.lccc))
    version = compiler_version(gcc_command)
    if not re.match(r"^16\.2(?:\.|$)", version):
        raise RuntimeError(f"expected GCC 16.2, found {version!r} ({args.gcc})")

    results: list[dict[str, object]] = []
    gcc_expectation_failures: list[str] = []
    differences: dict[str, list[str]] = {mode: [] for mode in MODES}
    allowed_differences: list[str] = []
    source_hashes: dict[str, str] = {}

    with tempfile.TemporaryDirectory(prefix="lccc-c23-inventory-") as temp:
        root = Path(temp)
        for probe in PROBES:
            case_dir = root / probe.name
            case_dir.mkdir()
            source = textwrap.dedent(probe.source).lstrip()
            source_path = case_dir / "probe.c"
            source_path.write_text(source, encoding="utf-8")
            for filename, contents in probe.resources.items():
                resource_path = case_dir / filename
                resource_path.parent.mkdir(parents=True, exist_ok=True)
                resource_path.write_bytes(contents)

            source_hashes[probe.name] = hashlib.sha256(
                source_path.read_bytes()
                + b"\0"
                + b"\0".join(
                    name.encode() + b"=" + data for name, data in sorted(probe.resources.items())
                )
            ).hexdigest()
            mode_results: dict[str, object] = {}
            for mode in probe.modes:
                flags = MODES[mode]
                gcc_result = run_compiler(gcc_command, flags, source_path, case_dir)
                lccc_result = run_compiler(lccc_command, flags, source_path, case_dir)
                expected = expected_verdict(probe, mode)
                if expected and gcc_result["verdict"] != expected:
                    gcc_expectation_failures.append(
                        f"{probe.name}/{mode}: expected GCC {expected}, got {gcc_result['verdict']}"
                    )
                if gcc_result["verdict"] != lccc_result["verdict"]:
                    label = f"{probe.name}/{mode}"
                    if probe.name in KNOWN_DEVIATIONS:
                        allowed_differences.append(
                            f"{label}: GCC {gcc_result['verdict']}, LCCC {lccc_result['verdict']}"
                        )
                    else:
                        differences[mode].append(label)
                mode_results[mode] = {
                    "expected": expected,
                    "gcc": gcc_result,
                    "lccc": lccc_result,
                }
            results.append(
                {
                    "name": probe.name,
                    "category": probe.category,
                    "description": probe.description,
                    "sha256": source_hashes[probe.name],
                    "results": mode_results,
                }
            )

    report: dict[str, object] = {
        "inventory_date": INVENTORY_DATE,
        "purpose": "Measured per-feature accept/reject inventory; not an overall C23 coverage claim.",
        "gcc": {"command": args.gcc, "version": version},
        "lccc": {"command": args.lccc},
        "modes": {name: list(flags) for name, flags in MODES.items()},
        "probe_count": len(PROBES),
        "results": results,
        "gcc_expectation_failures": gcc_expectation_failures,
        "lccc_gcc_differences": {mode: sorted(set(names)) for mode, names in differences.items()},
        "known_deviations": {
            name: reason for name, reason in sorted(KNOWN_DEVIATIONS.items())
        },
        "allowed_differences": sorted(allowed_differences),
    }
    return report, len(gcc_expectation_failures)


def short_verdict(result: object) -> str:
    if not isinstance(result, dict):
        return "—"
    verdict = result.get("verdict", "?")
    return {"accept": "ACCEPT", "reject": "REJECT", "timeout": "TIMEOUT"}.get(
        str(verdict), str(verdict).upper()
    )


def _pair(entry: dict[str, object], mode: str) -> str:
    modes = entry.get("results") or {}
    cell = modes.get(mode) if isinstance(modes, dict) else None
    if not cell:
        return "—"
    gcc = short_verdict(cell.get("gcc"))  # type: ignore[union-attr]
    lccc = short_verdict(cell.get("lccc"))  # type: ignore[union-attr]
    return f"{gcc}/{lccc}"


def markdown(report: dict[str, object]) -> str:
    gcc = report["gcc"]
    lccc = report["lccc"]
    assert isinstance(gcc, dict) and isinstance(lccc, dict)
    lines = [
        "# C17/C23 dialect inventory vs GCC 16.2",
        "",
        f"Measured: {report['inventory_date']}<br>",
        f"Oracle: GCC {gcc['version']} (`{gcc['command']}`)<br>",
        f"Compiler: `{lccc['command']}`<br>",
        f"Probes: {report['probe_count']} translation units. This is not an overall C23 coverage percentage.",
        "",
        "Each probe is compiled with `-fsyntax-only` in every dialect below; a cell reads "
        "`GCC/LCCC` and a pair that disagrees is a failure unless the probe is a declared "
        "deviation. Probes that do not name an expectation for a GNU dialect inherit the "
        "expectation of the strict dialect it extends.",
        "",
        "| Feature / probe | C23 | GNU23 | C17 | GNU17 |",
        "|---|---:|---:|---:|---:|",
    ]
    for entry in report["results"]:  # type: ignore[index]
        label = f"**{entry['name']}** — {entry['description']}"  # type: ignore[index]
        lines.append(
            "| {} | {} | {} | {} | {} |".format(
                label,
                _pair(entry, "c23"),
                _pair(entry, "gnu23"),
                _pair(entry, "c17"),
                _pair(entry, "gnu17"),
            )
        )
    differences = report.get("lccc_gcc_differences") or {}
    lines.extend(["", "## Differences from GCC", ""])
    if isinstance(differences, dict) and any(differences.values()):
        for mode in MODES:
            for name in differences.get(mode, []):
                lines.append(f"- `{name}`")
    else:
        lines.append("- None: LCCC matches GCC in every probe in every dialect.")
    allowed = report.get("allowed_differences") or []
    known = report.get("known_deviations") or {}
    lines.extend(["", "## Known deviations", ""])
    if isinstance(known, dict):
        for name, reason in known.items():
            lines.append(f"- `{name}` — {reason}")
    if allowed:
        lines.extend(["", "Measured known deviations on this host:", ""])
        lines.extend(f"- `{entry}`" for entry in allowed)  # type: ignore[union-attr]
    failures = report.get("gcc_expectation_failures") or []
    if failures:
        lines.extend(["", "## GCC expectation failures", ""])
        lines.extend(f"- {failure}" for failure in failures)  # type: ignore[union-attr]
    return "\n".join(lines) + "\n"


def self_test() -> int:
    names = [probe.name for probe in PROBES]
    assert len(names) == len(set(names)), "probe names must be unique"
    assert all(set(probe.modes) <= set(MODES) for probe in PROBES)
    assert all(
        expected in {"accept", "reject"}
        for probe in PROBES
        for expected in probe.expected.values()
    )
    assert any("__STDC_VERSION__" in p.source and "202311L" in p.source for p in PROBES)
    assert any("#embed" in p.source and p.resources for p in PROBES)
    assert any(p.expected.get("c23") == "reject" for p in PROBES)
    # Every probe must state what GCC is expected to do in at least one strict
    # dialect, so a silent GCC change cannot slip through as "no expectation",
    # and every measured mode must resolve to an expectation directly or by
    # inheriting from the strict dialect it extends.
    assert all(
        probe.expected.get("c23") or probe.expected.get("c17") for probe in PROBES
    ), "every probe needs an expectation for c23 or c17"
    unresolved = [
        f"{probe.name}/{mode}"
        for probe in PROBES
        for mode in probe.modes
        if not expected_verdict(probe, mode)
    ]
    assert not unresolved, f"modes without an expectation: {unresolved}"
    assert set(C17_VERDICTS) == set(names), (
        "C17_VERDICTS must cover exactly the probe set: "
        f"missing={sorted(set(names) - set(C17_VERDICTS))} "
        f"stale={sorted(set(C17_VERDICTS) - set(names))}"
    )
    # The ledger may only name probes that exist, and every listed probe must be
    # measured in a mode where GCC and LCCC can differ.
    assert set(KNOWN_DEVIATIONS) <= set(names), "known deviation names unknown probe"
    # A deviation without a reason is a silent allow-list entry; require enough
    # text that the reason explains the gap rather than naming it again.
    assert all(len(reason) >= 80 for reason in KNOWN_DEVIATIONS.values()), (
        "every known deviation needs a reason that explains it"
    )
    assert set(STRICT_BASE.values()) <= set(MODES)
    print(f"C23 inventory manifest self-test: PASS ({len(PROBES)} probes)")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc", default="target/fastbuild/lccc", help="LCCC compiler command")
    parser.add_argument("--gcc", default="gcc-16", help="pinned GCC 16.2 command")
    parser.add_argument("--json", type=Path, help="write the machine-readable report")
    parser.add_argument("--markdown", type=Path, help="write a Markdown rendering of the report")
    parser.add_argument(
        "--fail-on-diff",
        action="store_true",
        help="accepted for compatibility; differing verdicts fail the gate anyway",
    )
    parser.add_argument(
        "--report-only",
        action="store_true",
        help="print the matrix and exit 0 even when LCCC and GCC disagree "
        "(use only when regenerating docs; CI runs the gate in its default mode)",
    )
    parser.add_argument("--self-test", action="store_true", help="check the probe manifest only")
    args = parser.parse_args()
    if args.self_test:
        return self_test()

    try:
        report, gcc_failures = execute(args)
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as exc:
        print(f"C23 inventory: ERROR: {exc}", file=sys.stderr)
        return 2

    rendered = markdown(report)
    print(rendered)
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if args.markdown:
        args.markdown.parent.mkdir(parents=True, exist_ok=True)
        args.markdown.write_text(rendered, encoding="utf-8")

    diffs = report["lccc_gcc_differences"]
    unexpected = sum(len(names) for names in diffs.values()) if isinstance(diffs, dict) else 0
    if gcc_failures:
        return 2
    if unexpected and not args.report_only:
        print(
            f"C23 inventory: FAIL: {unexpected} verdict(s) differ from GCC 16.2 "
            "and are not declared in KNOWN_DEVIATIONS",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
