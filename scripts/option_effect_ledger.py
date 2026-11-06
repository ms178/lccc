#!/usr/bin/env python3
"""GP-22 / MB-01 option-effect ledger: make the trust pillar executable.

`src/driver/option_table.rs` classifies each accepted spelling as
``Implemented``, ``Inert`` or ``Refused`` and gives every ``Inert`` row a
``probe_id``.  Before this gate those probe ids were inert strings too: no
consumer existed, and five rows contradicted the compiler's observable
behaviour (three options marked ``Inert`` were in fact hard errors, and
``-Ofast``/``-Og`` marked ``Inert`` changed both the macro environment and
the emitted assembly).  A ledger nobody executes is a claim, not a contract.

This gate makes the ledger a *test*:

* static: no duplicate spellings, every class valid, every rationale
  non-empty, every accepted (``Implemented``/``Inert``) row carries a probe
  id, probe ids are unique, the probe registry has no orphans or holes, the
  GP-22 priority set is fully ledgered, and the blanket ``-f`` catch-all arm
  that silently swallowed unimplemented options is absent from the driver.
* dynamic: every row is executed against the built compiler.  ``Refused``
  must fail and name the option; ``Inert`` must exit 0 with byte-identical
  assembly *and* a byte-identical macro environment and no diagnostic;
  ``Implemented`` must produce the specific observable its probe names.
* ``--self-test`` mutates copies of the ledger to prove each check has teeth.

Observables are chosen so they cannot pass by accident: assembly is compared
by SHA-256 of a compiled fixture (not by exit status), and the macro
environment is compared by exact `-dM -E` stdout.  Determinism is asserted
first with a repeated baseline compile, so a non-reproducible object cannot
masquerade as an option effect.

Usage::

    python3 scripts/option_effect_ledger.py --offline
    python3 scripts/option_effect_ledger.py --lccc target/fastbuild/lccc
    python3 scripts/option_effect_ledger.py --self-test [--lccc PATH]

Exit status 0 means the ledger and the binary agree.  Any disagreement is a
named problem, not a warning: the trust pillar forbids silently accepted
artifact-changing options.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LEDGER = ROOT / "src/driver/option_table.rs"
CLI = ROOT / "src/driver/cli.rs"

# GP-22's priority set.  Every one of these must be ledgered; the accept
# criterion is "zero accepted-and-inert options for the priority set", which
# is only checkable if each spelling has a row.
PRIORITY_SET = (
    "-fprofile-arcs",
    "-ftest-coverage",
    "-Wattributes",
    "-Ofast",
    "-Og",
    "-static-pie",
    "-pedantic",
)

VALID_CLASSES = frozenset({"Implemented", "Inert", "Refused"})

# A fixture whose codegen is sensitive to optimisation mode, ABI, hardening
# and arithmetic flags, without depending on headers or the host libc.
ASM_FIXTURE = (
    "extern int g(int);\n"
    "int f(int x){ int a[8]={1,2,3,4,5,6,7,8}; return g(a[x&7]) + x*3; }\n"
)
MACRO_FIXTURE = ""

# The probe registry.  A probe id in the ledger MUST resolve to exactly one
# entry here, and every entry here MUST be referenced by exactly one row.
# `needle` is checked against `-dM -E` stdout (macro-contains) or `-S` output
# (asm-contains); `no-delta` rows are checked for byte identity instead.
PROBES: dict[str, tuple[str, str]] = {
    "probe-fopenmp-serial": ("macro-contains", "#define _OPENMP 202111"),
    "probe-lto-no-artifact": ("no-delta", ""),
    "probe-sanitize-none-no-artifact": ("no-delta", ""),
    "probe-strict-aliasing-no-effect": ("no-delta", ""),
    "probe-interposition-default": ("no-delta", ""),
    "probe-interposition-conservative": ("no-delta", ""),
    "probe-Wattributes-inert": ("no-delta", ""),
    "probe-Ofast-fast-math": ("macro-contains", "#define __FAST_MATH__ 1"),
    "probe-ffast-math-macro": ("macro-contains", "#define __FAST_MATH__ 1"),
    "probe-finite-math-only-macro": ("macro-contains", "#define __FINITE_MATH_ONLY__ 1"),
    "probe-no-math-errno-macro": ("macro-contains", "#define __NO_MATH_ERRNO__ 1"),
    "probe-Og-debug-opt": ("macro-contains", "#define __OPTIMIZE__ 1"),
    "probe-pedantic-inert": ("no-delta", ""),
    "probe-nullptr-checks-inert": ("no-delta", ""),
}

_BLOCK_RE = re.compile(r"Contract\s*\{(.*?)\n\s*\},", re.S)
_FIELD_RE = {
    "spelling": re.compile(r'spelling:\s*"([^"]+)"'),
    "class": re.compile(r"class:\s*Class::(\w+)"),
    "rationale": re.compile(r'rationale:\s*"([^"]*)"'),
    "probe": re.compile(r'probe_id:\s*(?:Some\("([^"]+)"\)|None)'),
}


@dataclass(frozen=True)
class Row:
    spelling: str
    cls: str
    rationale: str
    probe: str | None


def parse_rows(text: str) -> list[Row]:
    """Parse the declarative `CONTRACTS` array entries only.

    `lookup()` also constructs `Contract` values for generated option
    families; those use the sentinel spelling "option family" and are
    inline (`});`, not a `\n},` array element), so they are skipped.  Parsing
    a whole block rather than a fixed field order tolerates the comments the
    table uses to record measurement provenance between fields.
    """
    rows: list[Row] = []
    for block in _BLOCK_RE.finditer(text):
        body = block.group(1)
        found = {name: (m.group(1) if m and m.lastindex else None)
                 for name, m in ((name, regex.search(body)) for name, regex in _FIELD_RE.items())}
        if not found["spelling"] or found["spelling"] == "option family":
            continue
        rows.append(Row(found["spelling"], found["class"] or "",
                        found["rationale"] or "", found["probe"]))
    return rows


def static_problems(rows: list[Row], cli_text: str) -> list[str]:
    problems: list[str] = []
    if not rows:
        return ["option-effect ledger parsed zero rows (regex or file drift)"]

    seen: set[str] = set()
    probes: list[str] = []
    for row in rows:
        if row.spelling in seen:
            problems.append(f"duplicate ledger spelling {row.spelling!r}")
        seen.add(row.spelling)
        if row.cls not in VALID_CLASSES:
            problems.append(f"{row.spelling}: invalid class {row.cls!r}")
        if not row.rationale.strip():
            problems.append(f"{row.spelling}: empty rationale")
        if row.cls in ("Inert", "Implemented") and not row.probe:
            problems.append(f"{row.spelling}: accepted option has no probe_id")
        if row.cls == "Refused" and row.probe:
            problems.append(f"{row.spelling}: refused option must not carry a probe_id")
        if row.probe:
            probes.append(row.probe)

    for probe, count in sorted({p: probes.count(p) for p in probes}.items()):
        if count > 1:
            problems.append(f"probe_id {probe!r} is used by {count} rows")
        if probe not in PROBES:
            problems.append(f"probe_id {probe!r} has no implementation in the registry")
    for probe in sorted(set(PROBES) - set(probes)):
        problems.append(f"probe registry entry {probe!r} is orphaned (no ledger row)")

    for spelling in PRIORITY_SET:
        if spelling not in seen:
            problems.append(f"GP-22 priority option {spelling!r} has no ledger row")

    # MB-40: the blanket -f catch-all made every unimplemented -f option a
    # silent no-op and shadowed explicit refusal arms below it.  It must not
    # come back.  Strip comments first: cli.rs documents the historical shape
    # inside a NOTE, and that prose must not be mistaken for live code.
    code = "\n".join(line.split("//", 1)[0] for line in cli_text.splitlines())
    if re.search(r'arg\s+if\s+arg\.starts_with\("-f"\)\s*=>\s*\{\s*\}', code):
        problems.append("cli.rs reintroduced the blanket `-f` catch-all no-op arm")
    if "use super::option_table::{Class, lookup};" not in cli_text:
        problems.append("cli.rs no longer consults the option-effect ledger")

    return problems


def _run(lccc: str, args: list[str], env: dict[str, str] | None = None) -> subprocess.CompletedProcess:
    merged = os.environ.copy()
    if env:
        merged.update(env)
    return subprocess.run(
        [lccc, *args], capture_output=True, text=True, timeout=60, env=merged
    )


def _sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def resolve_binary(lccc: str) -> tuple[str, str | None]:
    """Resolve a compiler path to an absolute executable, or a named problem.

    A gate must never die with a Python traceback because a path was wrong:
    CI parses its exit status, not its stack. A relative path is resolved
    against the caller's cwd first (the documented usage), then against the
    repository root, so `--lccc target/fastbuild/lccc` also works when the
    gate is invoked from a temporary directory.
    """
    path = Path(lccc).expanduser()
    if not path.is_absolute():
        if os.sep not in lccc and not path.is_file():
            # A bare command name is a PATH lookup, not a file path: the
            # documented usage is a path, but a gate must not regress a
            # caller that passed `lccc` and previously worked.
            found = shutil.which(lccc)
            if found:
                path = Path(found)
        elif not path.is_file() and (ROOT / path).is_file():
            path = ROOT / path
    path = path.resolve()
    if not path.is_file():
        return str(path), f"--lccc {lccc!r}: no such file ({path})"
    if not os.access(path, os.X_OK):
        return str(path), f"--lccc {lccc!r}: {path} is not executable"
    return str(path), None


def compile_asm(lccc: str, work: Path, src: Path, extra: list[str],
                out: str) -> tuple[subprocess.CompletedProcess, str]:
    """Compile one variant and hash its assembly.

    `opt.s` is reused for every row, so a failed compile must not leave the
    previous row's bytes behind: hashing a stale file would make a broken
    option look like a codegen change (or hide one). Remove it first; a
    missing output hashes as the empty string, which never equals a real
    baseline hash.
    """
    dest = work / out
    dest.unlink(missing_ok=True)
    proc = _run(lccc, ["-O2", *extra, "-S", str(src), "-o", str(dest)])
    return proc, _sha(dest) if dest.is_file() else ""


def dynamic_problems(rows: list[Row], lccc: str) -> list[str]:
    """Execute every row against the built compiler and compare the observed
    class with the declared class."""
    problems: list[str] = []
    lccc, problem = resolve_binary(lccc)
    if problem:
        return [problem]
    with tempfile.TemporaryDirectory(prefix="lccc-option-ledger-") as td:
        work = Path(td)
        src = work / "fixture.c"
        macro_src = work / "macro.c"
        src.write_text(ASM_FIXTURE)
        macro_src.write_text(MACRO_FIXTURE)

        def variant(extra: list[str], out: str) -> tuple[subprocess.CompletedProcess, str]:
            return compile_asm(lccc, work, src, extra, out)

        base1, hash1 = variant([], "base1.s")
        base2, hash2 = variant([], "base2.s")
        if base1.returncode or base2.returncode:
            return [f"baseline compile failed: {base1.stderr.strip()} {base2.stderr.strip()}"]
        if hash1 != hash2:
            return ["baseline assembly is not deterministic; an option effect cannot be measured"]
        base_macros = _run(lccc, ["-dM", "-E", str(macro_src)])
        if base_macros.returncode:
            return [f"baseline -dM -E failed: {base_macros.stderr.strip()}"]

        for row in rows:
            proc, asm_hash = variant([row.spelling], "opt.s")
            macro = _run(lccc, [row.spelling, "-dM", "-E", str(macro_src)])
            diagnostic = "warning" in proc.stderr or "error" in proc.stderr

            if row.cls == "Refused":
                if proc.returncode == 0:
                    problems.append(f"{row.spelling}: declared Refused but the driver accepted it")
                elif row.spelling not in proc.stderr and row.spelling not in macro.stderr:
                    problems.append(f"{row.spelling}: refusal diagnostic does not name the option")
                continue

            if proc.returncode != 0:
                problems.append(f"{row.spelling}: declared {row.cls} but compile failed: {proc.stderr.strip()[:160]}")
                continue

            if row.cls == "Inert":
                if diagnostic:
                    problems.append(f"{row.spelling}: Inert but emitted a diagnostic: {proc.stderr.strip()[:160]}")
                if asm_hash != hash1:
                    problems.append(f"{row.spelling}: Inert but changed the emitted assembly")
                if macro.stdout != base_macros.stdout:
                    problems.append(f"{row.spelling}: Inert but changed the macro environment")
            elif row.cls == "Implemented":
                kind, needle = PROBES.get(row.probe or "", ("", ""))
                if kind == "macro-contains" and needle not in macro.stdout:
                    problems.append(f"{row.spelling}: probe {row.probe} expected {needle!r} in -dM -E")
                elif kind == "asm-contains":
                    asm = (work / "opt.s").read_text() if (work / "opt.s").is_file() else ""
                    if needle not in asm:
                        problems.append(f"{row.spelling}: probe {row.probe} expected {needle!r} in -S output")
                elif kind == "no-delta":
                    problems.append(f"{row.spelling}: Implemented row uses a no-delta probe")
                elif asm_hash == hash1 and macro.stdout == base_macros.stdout:
                    problems.append(f"{row.spelling}: declared Implemented but has no observable effect")

            # Every probe, including no-delta ones, must describe reality.
            kind, needle = PROBES.get(row.probe or "", ("", ""))
            if kind == "macro-contains" and needle not in macro.stdout:
                problems.append(f"{row.spelling}: probe {row.probe} observable is missing")
            if kind == "no-delta" and (asm_hash != hash1 or macro.stdout != base_macros.stdout):
                problems.append(f"{row.spelling}: probe {row.probe} claims no delta but one exists")

    # Two branches can name the same defect (a class check and its probe
    # check). Deduplicate while preserving order so --json is readable.
    return list(dict.fromkeys(problems))


def self_test(lccc: str | None) -> list[str]:
    """Negative controls: each mutation must be caught.  Without these the
    static checks are decoration rather than a gate."""
    source = LEDGER.read_text()
    cli_text = CLI.read_text()
    failures: list[str] = []

    def expect_caught(name: str, mutated: str, cli: str = cli_text, needle: str | None = None) -> None:
        found = static_problems(parse_rows(mutated), cli)
        if not found:
            failures.append(f"negative control {name!r} was NOT caught")
        elif needle and not any(needle in f for f in found):
            failures.append(f"negative control {name!r} was caught for the wrong reason: {found}")

    expect_caught(
        "duplicate spelling",
        source.replace('spelling: "-fno-lto"', 'spelling: "-fsanitize=none"', 1),
        needle="duplicate",
    )
    expect_caught(
        "inert row without probe_id",
        source.replace('probe_id: Some("probe-lto-no-artifact")', "probe_id: None", 1),
        needle="no probe_id",
    )
    # Delete a whole row: the registry entry it used becomes an orphan.
    orphaned = re.sub(
        r'    Contract \{\n        spelling: "-fno-lto".*?\n    \},\n', "", source,
        count=1, flags=re.S,
    )
    expect_caught("orphaned registry entry", orphaned, needle="orphaned")
    expect_caught(
        "unregistered probe_id",
        source.replace('probe_id: Some("probe-lto-no-artifact")',
                       'probe_id: Some("probe-does-not-exist")', 1),
        needle="no implementation",
    )
    expect_caught(
        "priority option dropped",
        source.replace('spelling: "-pedantic"', 'spelling: "-pedantic-REMOVED"', 1),
        needle="priority option",
    )
    expect_caught(
        "blanket -f catch-all restored",
        source,
        cli=cli_text + '\n                arg if arg.starts_with("-f") => {}\n',
        needle="catch-all",
    )
    expect_caught(
        "invalid class",
        source.replace("class: Class::Inert,\n        rationale: \"Objects contain native code",
                       "class: Class::Bogus,\n        rationale: \"Objects contain native code", 1),
        needle="invalid class",
    )

    if lccc:
        # A misclassification is only catchable by executing the binary.
        swapped = source.replace(
            'spelling: "-Ofast",\n        class: Class::Implemented,',
            'spelling: "-Ofast",\n        class: Class::Inert,', 1,
        )
        found = static_problems(parse_rows(swapped), cli_text)
        dynamic = dynamic_problems(parse_rows(swapped), lccc)
        if not dynamic and not found:
            failures.append("negative control 'reclassified -Ofast as Inert' was NOT caught")

    return failures


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--lccc", help="built lccc to verify dynamically")
    parser.add_argument("--offline", action="store_true", help="static checks only")
    parser.add_argument("--self-test", action="store_true", help="run negative controls")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)

    rows = parse_rows(LEDGER.read_text())
    cli_text = CLI.read_text()
    problems = static_problems(rows, cli_text)
    binary: str | None = None
    binary_problem: str | None = None
    if args.lccc and not args.offline:
        binary, binary_problem = resolve_binary(args.lccc)
        if binary_problem:
            binary = None
    if args.self_test:
        problems += [f"self-test: {f}" for f in self_test(binary)]
    if binary_problem:
        problems.append(binary_problem)
    elif binary:
        problems += dynamic_problems(rows, binary)

    report = {
        "rows": len(rows),
        "probes": len(PROBES),
        "priority_options": len(PRIORITY_SET),
        "dynamic": bool(binary),
        "problems": problems,
    }
    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    elif problems:
        for problem in problems:
            print(f"BROKEN  {problem}", file=sys.stderr)
        print(f"option-effect ledger: {len(rows)} rows, {len(problems)} problem(s)")
    else:
        mode = "static + dynamic" if args.lccc and not args.offline else "static only"
        print(f"option-effect ledger: {len(rows)} rows, {len(PROBES)} probes, "
              f"{len(PRIORITY_SET)} priority options, {mode}: PASS")
    return 1 if problems else 0


if __name__ == "__main__":
    raise SystemExit(main())
