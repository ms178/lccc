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
INVENTORY_DATE = "2026-10-27"
MODES: dict[str, tuple[str, ...]] = {
    "c23": ("-std=c23",),
    "c17": ("-std=c17",),
    "default": (),
}


@dataclass(frozen=True)
class Probe:
    name: str
    category: str
    description: str
    source: str
    expected: dict[str, str] = field(default_factory=lambda: {"c23": "accept"})
    modes: tuple[str, ...] = ("c23", "c17")
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
)


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
    c23_differences: list[str] = []
    c17_differences: list[str] = []
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
                expected = probe.expected.get(mode)
                if expected and gcc_result["verdict"] != expected:
                    gcc_expectation_failures.append(
                        f"{probe.name}/{mode}: expected GCC {expected}, got {gcc_result['verdict']}"
                    )
                if mode == "c23" and gcc_result["verdict"] != lccc_result["verdict"]:
                    c23_differences.append(probe.name)
                if mode == "c17" and gcc_result["verdict"] != lccc_result["verdict"]:
                    c17_differences.append(probe.name)
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
        "lccc_gcc_c23_differences": sorted(set(c23_differences)),
        "lccc_gcc_c17_differences": sorted(set(c17_differences)),
    }
    return report, len(gcc_expectation_failures)


def short_verdict(result: object) -> str:
    if not isinstance(result, dict):
        return "—"
    verdict = result.get("verdict", "?")
    return {"accept": "ACCEPT", "reject": "REJECT", "timeout": "TIMEOUT"}.get(
        str(verdict), str(verdict).upper()
    )


def markdown(report: dict[str, object]) -> str:
    gcc = report["gcc"]
    lccc = report["lccc"]
    assert isinstance(gcc, dict) and isinstance(lccc, dict)
    lines = [
        "# C23 accept/reject inventory vs GCC 16.2",
        "",
        f"Measured: {report['inventory_date']}<br>",
        f"Oracle: GCC {gcc['version']} (`{gcc['command']}`)<br>",
        f"Compiler: `{lccc['command']}`<br>",
        f"Probes: {report['probe_count']} translation units. This is not an overall C23 coverage percentage.",
        "",
        "Every probe is compiled with `-fsyntax-only` in the listed mode. C17 outcomes are measured and recorded; a C17 acceptance may be a GNU extension, so it is not inferred from the standard.",
        "",
        "| Feature / probe | GCC C23 | LCCC C23 | GCC C17 | LCCC C17 | Expected GCC C23 |",
        "|---|---:|---:|---:|---:|---:|",
    ]
    for entry in report["results"]:  # type: ignore[index]
        modes = entry["results"]  # type: ignore[index]
        c23 = modes.get("c23", {})
        c17 = modes.get("c17", {})
        default = modes.get("default", {})
        expected = (c23.get("expected") or "accept") if c23 else "—"
        label = f"**{entry['name']}** — {entry['description']}"  # type: ignore[index]
        lines.append(
            "| {} | {} | {} | {} | {} | {} |".format(
                label,
                short_verdict(c23.get("gcc")),
                short_verdict(c23.get("lccc")),
                short_verdict(c17.get("gcc")),
                short_verdict(c17.get("lccc")),
                expected.upper(),
            )
        )
        if default:
            lines.append(
                f"| ↳ default-mode control | {short_verdict(default.get('gcc'))} | {short_verdict(default.get('lccc'))} | — | — | ACCEPT |"
            )
    diffs = report.get("lccc_gcc_c23_differences", [])
    lines.extend(
        [
            "",
            "## Measured C23 differences",
            "",
        ]
    )
    if diffs:
        lines.extend(f"- `{name}`" for name in diffs)  # type: ignore[union-attr]
    else:
        lines.append("- None in this probe set.")
    c17_diffs = report.get("lccc_gcc_c17_differences", [])
    lines.extend(["", "## Measured C17 mode differences", ""])
    if c17_diffs:
        lines.extend(f"- `{name}`" for name in c17_diffs)  # type: ignore[union-attr]
    else:
        lines.append("- None in this probe set.")
    failures = report.get("gcc_expectation_failures", [])
    if failures:
        lines.extend(["", "## GCC expectation failures", ""])
        lines.extend(f"- {failure}" for failure in failures)  # type: ignore[union-attr]
    lines.extend(
        [
            "",
            "Source hashes, complete diagnostics, and argv are preserved when the same run is requested with `--json`.",
            "",
        ]
    )
    return "\n".join(lines)


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
        help="fail if any measured C23 verdict differs between LCCC and GCC",
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

    diffs = report["lccc_gcc_c23_differences"]
    if gcc_failures:
        return 2
    if args.fail_on_diff and diffs:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
