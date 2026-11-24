#!/usr/bin/env python3
"""Generate and verify the lccc option inventory (docs/OPTIONS.md).

Why this exists
---------------
The driver implements a tiered option policy (see the header of
``src/driver/cli.rs``): an option is *implemented*, *tolerated* (accepted with a
warning that nothing acts on it) or *refused* (a hard error).  Until now the only
record of which option sits in which tier was the capability probe's manifest,
and "accepted" was never separated from "has the documented effect" -- the exact
gap that let ``-fno-plt`` be accepted for a long time while changing nothing.

This tool keeps one table (``ROWS`` below) and measures every row against the
compiler under test:

``implemented``
    exit status 0, no "not implemented / tolerated / ignored" warning, **and** the
    observable effect the row names (an instruction, a relocation, a symbol
    visibility, an ELF field, a diagnostic) is present;
``tolerated``
    exit status 0 and the warning pattern on stderr;
``refused``
    non-zero exit status and the error pattern on stderr.

``--write`` regenerates ``docs/OPTIONS.md`` from the table; ``--check`` fails when
the committed document differs from the table *or* when any row's measured
behaviour differs from its class -- so a tier change in the driver cannot land
without the document and the table following.

Adding an option: add a row, run ``--check`` (it tells you what it measured), then
``--write``.  Before trusting an ``implemented`` row, make the option a silent
no-op in the driver and watch the row fail; ``--self-test`` does the same against
a fake runner, which is what keeps the checker honest.

``--discover`` is informational: it asks GCC for its option names and reports how
lccc classifies each (accepted / warned / refused) that is not yet a row.
"""
from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from dataclasses import dataclass, field
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
DOC = REPO / "docs" / "OPTIONS.md"
DEFAULT_LCCC = REPO / "target" / "fastbuild" / "lccc"

# Warnings that mean "accepted, but nothing acts on it".
NOT_IMPLEMENTED = re.compile(r"tolerated but not implemented|not supported|ignored|unsupported|not implemented")

SOURCES = {
    "hello": "int puts(const char*);\nint f(void){return puts(\"x\");}\n",
    "plain": "int f(int x){return x+1;}\n",
    "add": "int a(int x,int y){return x+y;}\n",
    "bigframe": "int g(int n){volatile char big[20000]; big[n]=1; return big[0];}\n",
    "guard": ("char *strcpy(char*,const char*); int puts(const char*);\n"
              "int f(const char*s){char b[64]; strcpy(b,s); return puts(b);}\n"),
    "start": "void _start(void){for(;;);}\n",
    "reloc": "int x; int *p = &x;\nvoid _start(void){for(;;);}\n",
    "icf": ("int fa(int x){return x*3+1;} int fb(int x){return x*3+1;}\n"
            "int (*volatile p1)(int)=fa; int (*volatile p2)(int)=fb;\n"
            "void _start(void){for(;;) p1(1)+p2(2);}\n"),
    "diag": ("int printf(const char*, ...);\nint h(unsigned u){int x; int y=3; int i=-1; return (i<u)+printf(\"%d\\n\");}\n"),
    "shadow": "int f(int a){ { int a=2; return a; } }\n",
    "const1": "int f(void){return 1;}\n",
    "structsz": "struct s{char a;int b;}; int n(void){return sizeof(struct s);}\n",
    "charsign": "int u(void){char c=-1; return c<0;}\n",
    "callee": "int g(int);int h(int x){return g(x)+1;}\n",
}

# view -> how the observable text is produced.
#   asm      lccc ARGS -S -o - SRC
#   macros   lccc ARGS -dM -E SRC   (predefined macros)
#   diag     stderr of lccc ARGS -c SRC
#   reloc    objdump -dr of the object
#   syms     readelf -sW of the object
#   sections readelf -S of the object
#   header/dynamic/notes/segments/symbols   readelf -h/-d/-n/-lW/-sW of the linked image
OBJ_VIEWS = {"reloc": ["objdump", "-dr"], "syms": ["readelf", "-sW"], "sections": ["readelf", "-SW"]}
LINK_VIEWS = {"header": ["-h"], "dynamic": ["-d"], "notes": ["-n"], "segments": ["-lW"], "symbols": ["-sW"]}


@dataclass(frozen=True)
class Row:
    opts: tuple[str, ...]
    cls: str                      # implemented | tolerated | refused
    note: str                     # one line for the document
    source: str = "plain"
    view: str = "asm"             # see above; for tolerated/refused rows the diagnostics are checked
    expect: tuple[str, ...] = ()  # regexes that must all match the view text (implemented)
    absent: tuple[str, ...] = ()  # regexes that must NOT match (implemented)
    message: str = ""             # regex on stderr (tolerated / refused)
    base: tuple[str, ...] = ()    # options added to the *control* run (without opts)
    link: bool = False            # compile and link (needed for linker options)

    @property
    def label(self) -> str:
        return " ".join(self.opts)


def R(opts, cls, note, **kw):
    return Row(tuple(opts.split()) if isinstance(opts, str) else tuple(opts), cls, note, **kw)


ROWS: tuple[Row, ...] = (
    # ---- hardening and code generation: implemented, effect observed -------------
    R("-O2 -fno-plt -fPIC", "implemented", "calls through the GOT (`call *f@GOTPCREL`) instead of the PLT",
      source="hello", view="reloc", expect=(r"R_X86_64_GOTPCRELX\s+puts",)),
    R("-fvisibility=hidden", "implemented", "defined symbols get `STV_HIDDEN`",
      view="syms", expect=(r"GLOBAL\s+HIDDEN\s+\d+\s+f$",)),
    R("-O2 -ftrapv", "implemented", "signed overflow checked; the failing path calls `abort`",
      source="add", expect=(r"call\s+abort@PLT",)),
    R("-O2 -fstack-clash-protection", "implemented", "frames over a page are probed page by page (`orl $0, (%rsp)`)",
      source="bigframe", expect=(r"orl\s+\$0,\s+\(%rsp\)",)),
    R("-m32 -O2 -fstack-clash-protection", "implemented", "same probing on i686 (`orl $0, (%esp)`)",
      source="bigframe", expect=(r"orl\s+\$0,\s+\(%esp\)",)),
    R("-O2 -fstack-protector-all", "implemented", "canary load from `%fs:40` and a `__stack_chk_fail` call",
      source="guard", expect=(r"%fs:40", r"__stack_chk_fail")),
    R("-O2 -fstack-protector-all -mstack-protector-guard=tls -mstack-protector-guard-reg=gs "
      "-mstack-protector-guard-offset=64", "implemented", "kernel-style guard location (`%gs:64`)",
      source="guard", expect=(r"%gs:64",), absent=(r"%fs:40",)),
    R("-O2 -fstack-protector-all -mstack-protector-guard=tls -mstack-protector-guard-symbol=__stack_chk_guard",
      "implemented", "guard read from a named TLS symbol", source="guard",
      expect=(r"%fs:__stack_chk_guard",)),
    R("-m32 -O2 -fstack-protector-all", "implemented", "i686 canary from `%gs:20`",
      source="guard", expect=(r"%gs:20",)),
    R("-O2 -fcf-protection=full", "implemented", "every function starts with `endbr64`",
      source="const1", expect=(r"endbr64",)),
    R("-O1 -fpack-struct", "implemented", "struct members are packed (`sizeof` 5, not 8)",
      source="structsz", expect=(r"movl\s+\$5,\s+%eax",)),
    R("-O1 -funsigned-char", "implemented", "plain `char` is unsigned (`c < 0` folds to 0)",
      source="charsign", expect=(r"xorl\s+%eax,\s+%eax",)),
    R("-fshort-wchar", "implemented", "`wchar_t` is 2 bytes", source="const1", view="macros",
      expect=(r"__SIZEOF_WCHAR_T__ 2",)),
    R("-ffast-math", "implemented", "defines `__FAST_MATH__`", source="const1", view="macros",
      expect=(r"__FAST_MATH__ 1",)),
    R("-O2 -pg", "implemented", "function entry calls `mcount` / `__fentry__`",
      source="const1", expect=(r"mcount|__fentry__",)),
    R("-O2 -fno-omit-frame-pointer", "implemented", "keeps the frame pointer (`pushq %rbp`)",
      source="callee", expect=(r"pushq\s+%rbp",)),
    R("-O2 -fno-asynchronous-unwind-tables", "implemented", "no `.cfi_*` unwind directives",
      source="const1", absent=(r"\.cfi_startproc",)),
    R("-O2 -fpatchable-function-entry=2", "implemented", "two `nop`s at entry and a `__patchable_function_entries` record",
      source="const1", expect=(r"__patchable_function_entries", r"^\s*nop$")),
    R("-g -O0", "implemented", "emits DWARF sections (`.debug_line`, `.debug_info`)",
      view="sections", expect=(r"\.debug_line", r"\.debug_info")),
    # ---- diagnostics ------------------------------------------------------------
    R("-Wall -Wextra", "implemented", "`-Wformat`, `-Wunused-variable` and `-Wsign-compare` report",
      source="diag", view="diag",
      expect=(r"\[-Wformat", r"-Wunused-variable", r"-Wsign-compare")),
    # ---- linker options (linked images) -----------------------------------------
    R("-nostdlib -Wl,--build-id", "implemented", "`.note.gnu.build-id` is emitted",
      source="start", view="notes", link=True, expect=(r"Build ID",)),
    R("-nostdlib -Wl,--image-base=0x10000000", "implemented", "entry point moves to the requested base",
      source="start", view="header", link=True, expect=(r"Entry point address:\s+0x1000",)),
    R("-nostdlib -Wl,-z,max-page-size=65536", "implemented", "`PT_LOAD` alignment becomes 0x10000",
      source="start", view="segments", link=True, expect=(r"LOAD.*\s0x10000\s*$",)),
    R("-nostdlib -fPIE -pie -Wl,-z,pack-relative-relocs", "implemented", "relative relocations are packed (`DT_RELR`)",
      source="reloc", view="dynamic", link=True, expect=(r"\(RELR\)",)),
    R("-nostdlib -O1 -ffunction-sections -Wl,--gc-sections -Wl,--icf=all", "implemented",
      "identical functions share one address",
      source="icf", view="symbols", link=True,
      expect=(r":\s+([0-9a-f]{16})\s+\d+ FUNC\s+GLOBAL\s+DEFAULT\s+\d+\s+fa\n.*:\s+\1\s+\d+ FUNC\s+GLOBAL\s+DEFAULT\s+\d+\s+fb",)),
    # ---- tolerated: accepted with a warning, nothing acts on it ------------------
    R("-Wshadow", "tolerated", "no shadowing diagnostics are produced", source="shadow",
      message=r"tolerated but not implemented"),
    R("-Wmaybe-uninitialized", "tolerated", "no maybe-uninitialized diagnostics are produced",
      message=r"tolerated but not implemented"),
    R("-gz", "tolerated", "debug sections are not compressed", message=r"not supported"),
    R("-fxyzzy-lccc-control", "tolerated", "unknown `-f` options are accepted with a warning (negative control)",
      message=r"tolerated but not implemented"),
    R("-Wxyzzy-lccc-control", "tolerated", "unknown `-W` options are accepted with a warning (negative control)",
      message=r"tolerated but not implemented"),
    # ---- refused: hard error -----------------------------------------------------
    R("-static-pie", "refused", "no static PIE output", message=r"-static-pie is not supported"),
    R("-flto", "refused", "no link-time optimization", message=r"LTO IR production and link-time consumption are not implemented"),
    R("-fsanitize=address", "refused", "no sanitizer runtime", message=r"-fsanitize=address is not supported"),
    R("-ftest-coverage", "refused", "no `.gcno`/`.gcda` output", message=r"-ftest-coverage is not implemented"),
    R("-mstackrealign", "refused", "no stack realignment", message=r"unsupported machine option -mstackrealign"),
    R("-m32 -nostdlib -Wl,--emit-relocs", "refused", "the i686 linker cannot retain relocations",
      source="start", link=True, message=r"--emit-relocs is not supported by the elf_i386"),
    R("-mxyzzy-lccc-control", "refused", "unknown `-m` options are errors (negative control)",
      message=r"unsupported machine option"),
)

CLASS_BLURB = {
    "implemented": "Accepted **and** observed to do what it says.  The *Effect* column names the evidence.",
    "tolerated": "Accepted with a warning; the compiler does nothing with it.  Builds that pass these options keep working, "
                 "but get none of the behaviour.  `LCCC_STRICT_OPTIONS=1` turns them into errors.",
    "refused": "A hard error, because accepting it silently would change what the artifact is (security hardening, output "
               "format, runtime) without telling the user.",
}


# ---------------------------------------------------------------------------- running
@dataclass
class Result:
    rc: int
    stderr: str
    text: str = ""


class Runner:
    """Runs the compiler under test.  Replaced by a fake in ``--self-test``."""

    def __init__(self, lccc: str):
        self.lccc = lccc
        self.tmp = tempfile.mkdtemp(prefix="lccc-options-")

    def close(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def _src(self, key: str) -> str:
        path = os.path.join(self.tmp, f"{key}.c")
        if not os.path.exists(path):
            Path(path).write_text(SOURCES[key])
        return path

    def _run(self, argv, **kw):
        env = dict(os.environ)
        env.pop("LCCC_STRICT_OPTIONS", None)
        return subprocess.run(argv, capture_output=True, text=True, timeout=120, env=env, **kw)

    def run(self, row: Row, control: bool = False) -> Result:
        opts = list(row.base) if control else list(row.opts) + list(row.base)
        src = self._src(row.source)
        out = os.path.join(self.tmp, "out")
        if row.link:
            p = self._run([self.lccc, *opts, src, "-o", out])
            res = Result(p.returncode, p.stderr)
            if p.returncode == 0 and row.view in LINK_VIEWS:
                res.text = self._run(["readelf", *LINK_VIEWS[row.view], out]).stdout
            return res
        if row.view == "asm":
            p = self._run([self.lccc, *opts, "-S", "-o", "-", src])
            return Result(p.returncode, p.stderr, p.stdout)
        if row.view == "macros":
            p = self._run([self.lccc, *opts, "-dM", "-E", src])
            return Result(p.returncode, p.stderr, p.stdout)
        p = self._run([self.lccc, *opts, "-c", src, "-o", out + ".o"])
        res = Result(p.returncode, p.stderr)
        if row.view == "diag":
            res.text = p.stderr
        elif p.returncode == 0 and row.view in OBJ_VIEWS:
            res.text = self._run([*OBJ_VIEWS[row.view], out + ".o"]).stdout
        return res


def evaluate(row: Row, runner) -> list[str]:
    """Return a list of problems (empty = the row's measured behaviour matches its class)."""
    got = runner.run(row)
    problems: list[str] = []
    if row.cls == "refused":
        if got.rc == 0:
            problems.append("accepted (exit 0) but the row says refused")
        elif not re.search(row.message, got.stderr):
            problems.append(f"refused with an unexpected message: {got.stderr.strip()[:120]!r}")
    elif row.cls == "tolerated":
        if got.rc != 0:
            problems.append(f"exit {got.rc}; a tolerated option must be accepted")
        elif not re.search(row.message, got.stderr):
            problems.append("accepted silently (no warning); a tolerated option must say nothing acts on it")
    elif row.cls == "implemented":
        if got.rc != 0:
            problems.append(f"exit {got.rc}: {got.stderr.strip()[:120]!r}")
        else:
            warned = [l for l in got.stderr.splitlines() if NOT_IMPLEMENTED.search(l)]
            if warned:
                problems.append(f"warns that it is not implemented: {warned[0][:100]!r}")
            for pat in row.expect:
                if not re.search(pat, got.text, re.M):
                    problems.append(f"effect missing: /{pat}/ not found in the {row.view} output")
            for pat in row.absent:
                if re.search(pat, got.text, re.M):
                    problems.append(f"unexpected: /{pat}/ found in the {row.view} output")
    else:
        problems.append(f"unknown class {row.cls!r}")
    return problems


# ---------------------------------------------------------------------------- document
def render_document() -> str:
    seen: set[tuple[str, ...]] = set()
    out = [
        "<!-- Generated by scripts/lccc_options_inventory.py --write; do not edit by hand. -->",
        "# lccc option inventory",
        "",
        "Which options the compiler driver implements, tolerates or refuses, and the evidence for each.",
        "Every row is re-measured against a built `lccc` by `python3 scripts/lccc_options_inventory.py --check`,",
        "so this table cannot drift from the binary: a row whose measured behaviour differs from its class,",
        "or a document that differs from the table, fails the check.  The driver's policy is described at the",
        "top of `src/driver/cli.rs`; an option that is not listed here is governed by that policy's default",
        "(an unknown `-f` or `-W` option is accepted with a warning, an unknown `-m` option is an error).",
        "",
        "Regenerate with `python3 scripts/lccc_options_inventory.py --write` after changing the table.",
        "",
    ]
    for cls in ("implemented", "tolerated", "refused"):
        rows = [r for r in ROWS if r.cls == cls]
        out += [f"## {cls.capitalize()}", "", CLASS_BLURB[cls], ""]
        if cls == "implemented":
            out += ["| Option | Effect | Evidence |", "|---|---|---|"]
        else:
            out += ["| Option | Behaviour | Message |", "|---|---|---|"]
        for r in rows:
            assert r.opts not in seen, f"duplicate row {r.label}"
            seen.add(r.opts)
            opt = "`" + r.label.replace("|", "\\|") + "`"
            if cls == "implemented":
                ev = "; ".join(f"`{p}`".replace("|", "\\|") for p in r.expect) or "diagnostics"
                out.append(f"| {opt} | {r.note} | {r.view}: {ev} |")
            else:
                out.append(f"| {opt} | {r.note} | `{r.message}` |".replace("|` |", "` |"))
        out.append("")
    return "\n".join(out).rstrip() + "\n"


# ---------------------------------------------------------------------------- discovery
def discover(lccc: str, gcc: str) -> int:
    """Classify GCC's option names by how lccc treats them (informational)."""
    helps = "".join(
        subprocess.run([gcc, "-Q", f"--help={group}"], capture_output=True, text=True).stdout
        for group in ("common", "target", "warnings", "optimizers"))
    names = sorted({m.group(1) for m in re.finditer(r"^\s+(-[A-Za-z][^\s=\[<]*)", helps, re.M)})
    known = {o for r in ROWS for o in r.opts}
    runner = Runner(lccc)
    counts = {"accepted": 0, "warned": 0, "refused": 0}
    refused: list[str] = []
    try:
        src = runner._src("plain")
        for name in names:
            if name in known or name.startswith("-fdump") or name.startswith("--help"):
                continue
            p = runner._run([lccc, name, "-fsyntax-only", src])
            if p.returncode != 0:
                counts["refused"] += 1
                refused.append(name)
            elif p.stderr.strip():
                counts["warned"] += 1
            else:
                counts["accepted"] += 1
    finally:
        runner.close()
    print(f"{len(names)} GCC option names; not yet rows: {counts}")
    print("refused (first 40):", " ".join(refused[:40]))
    return 0


# ---------------------------------------------------------------------------- self-test
class _Fake:
    """Canned results keyed by option label; ``control`` runs return the plain result."""

    def __init__(self, table):
        self.table = table

    def run(self, row, control=False):
        return self.table[row.label]


class SelfTest(unittest.TestCase):
    impl = Row(("-fx",), "implemented", "n", expect=(r"effect",), absent=(r"bad",))

    def test_implemented_requires_the_effect(self):
        self.assertEqual(evaluate(self.impl, _Fake({"-fx": Result(0, "", "has the effect")})), [])
        # The mutation that matters: the option becomes a silent no-op.
        problems = evaluate(self.impl, _Fake({"-fx": Result(0, "", "nothing here")}))
        self.assertTrue(any("effect missing" in p for p in problems), problems)

    def test_implemented_must_not_warn_that_it_is_unimplemented(self):
        problems = evaluate(self.impl, _Fake({"-fx": Result(0, "warning: -fx tolerated but not implemented", "effect")}))
        self.assertTrue(any("not implemented" in p for p in problems), problems)

    def test_implemented_absent_pattern(self):
        problems = evaluate(self.impl, _Fake({"-fx": Result(0, "", "effect bad")}))
        self.assertTrue(any("unexpected" in p for p in problems), problems)

    def test_tolerated_needs_a_warning(self):
        row = Row(("-Wy",), "tolerated", "n", message="tolerated")
        self.assertEqual(evaluate(row, _Fake({"-Wy": Result(0, "warning: tolerated")})), [])
        self.assertTrue(evaluate(row, _Fake({"-Wy": Result(0, "")})))          # silent accept
        self.assertTrue(evaluate(row, _Fake({"-Wy": Result(1, "tolerated")}))) # refused

    def test_refused_needs_a_failure_with_the_message(self):
        row = Row(("-fz",), "refused", "n", message="not supported")
        self.assertEqual(evaluate(row, _Fake({"-fz": Result(1, "error: not supported")})), [])
        self.assertTrue(evaluate(row, _Fake({"-fz": Result(0, "")})))          # silently accepted
        self.assertTrue(evaluate(row, _Fake({"-fz": Result(1, "other")})))     # wrong message

    def test_table_is_well_formed(self):
        seen = set()
        for r in ROWS:
            self.assertIn(r.cls, CLASS_BLURB, r.label)
            self.assertNotIn(r.opts, seen, f"duplicate {r.label}")
            seen.add(r.opts)
            self.assertIn(r.source, SOURCES, r.label)
            if r.cls == "implemented":
                self.assertTrue(r.expect or r.absent, f"{r.label}: an implemented row needs an observable effect")
            else:
                self.assertTrue(r.message, f"{r.label}: needs a message pattern")
            for pat in (*r.expect, *r.absent, r.message):
                if pat:
                    re.compile(pat)

    def test_document_is_deterministic_and_complete(self):
        doc = render_document()
        self.assertEqual(doc, render_document())
        for r in ROWS:
            self.assertIn(r.label, doc)

    def test_stale_document_is_detected(self):
        self.assertNotEqual(render_document(), render_document().replace("`-flto`", "`-fltoX`"))


# ---------------------------------------------------------------------------- main
def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--lccc", default=os.environ.get("CCC", str(DEFAULT_LCCC)), help="compiler under test")
    mode = ap.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true", help="measure every row and compare the document")
    mode.add_argument("--write", action="store_true", help="measure every row, then write docs/OPTIONS.md")
    mode.add_argument("--discover", action="store_true", help="classify GCC's option names (informational)")
    mode.add_argument("--self-test", action="store_true")
    ap.add_argument("--gcc", default="gcc")
    args = ap.parse_args(argv)

    if args.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(SelfTest)
        return 0 if unittest.TextTestRunner(verbosity=1).run(suite).wasSuccessful() else 1
    if args.discover:
        return discover(args.lccc, args.gcc)

    if not (os.path.isfile(args.lccc) and os.access(args.lccc, os.X_OK)):
        print(f"error: compiler not found: {args.lccc}", file=sys.stderr)
        return 2
    runner = Runner(args.lccc)
    failures = 0
    try:
        for row in ROWS:
            problems = evaluate(row, runner)
            status = "ok  " if not problems else "FAIL"
            print(f"{status} [{row.cls:11}] {row.label}")
            for p in problems:
                print(f"       {p}")
            failures += bool(problems)
    finally:
        runner.close()
    doc = render_document()
    if args.write:
        if failures:
            print(f"refusing to write: {failures} row(s) do not behave as classified", file=sys.stderr)
            return 1
        DOC.write_text(doc)
        print(f"wrote {DOC.relative_to(REPO)} ({len(ROWS)} rows)")
        return 0
    current = DOC.read_text() if DOC.exists() else ""
    if current != doc:
        print("FAIL docs/OPTIONS.md is stale: run `python3 scripts/lccc_options_inventory.py --write`", file=sys.stderr)
        failures += 1
    print(f"options inventory: {len(ROWS) - failures} of {len(ROWS)} rows ok")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
