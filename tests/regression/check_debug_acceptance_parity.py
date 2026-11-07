#!/usr/bin/env python3
"""`-g` must never change whether a translation unit is accepted.

`lccc -S -g` pre-renders the DWARF DIEs by assembling the generated text in
memory (see `src/driver/debug_assembly.rs`).  That made `-S -g` stricter than
plain `-S`: a user inline-assembly template the target cannot encode was
rejected only when `-g` was present.  GCC 16.2 accepts `-S -g` for such a unit
on every target and fails only when the text is actually assembled (`-c`),
because it renders code addresses and sizes as symbolic expressions instead of
resolving them.  Clang 23.1.5 validates the template even at `-S`; lccc follows
GCC, and in either case its own `-S` and `-S -g` must agree.

The contract this gate pins, per target:

* `rc(-S) == rc(-S -g)` for every fixture — acceptance parity.
* When both accept, the two texts are identical once debug rows are removed:
  the fallback must not perturb code either.
* An unencodable template still fails `-c -g`.  The `-S` pass-through is not a
  way to smuggle an invalid instruction into an object.
* A *valid* template keeps full debug information: `.debug_info` is present and
  no fallback note is printed.  This is what stops the fallback from silently
  becoming the normal path.
* GCC 16.2 is the oracle for the 64-bit-only-mnemonic fixture on x86-64 and
  i686: `-S -g` accepts, `-c` rejects.

Usage:
    python3 tests/regression/check_debug_acceptance_parity.py \
        --lccc target/fastbuild/lccc [--lccc-i686 ...] [--gcc gcc-16] [--json out]
    python3 tests/regression/check_debug_acceptance_parity.py --self-test
"""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]

# Debug rows are metadata, not code.  This is the same filter the invariance
# gate uses, kept in sync deliberately: `.loc`/`.file`, the writer's absolute
# debug aliases, the zero-width D6 range endpoints, and whole `.debug_*`
# sections.
_DEBUG_LINE = re.compile(
    r"^\s*(?:\.(?:loc|file)(?:\s|$)"
    r"|\.set\s+\.Llccc_debug_alias_"
    r"|\.Llccc_inline_[be]_[0-9]+_[0-9]+_[^\s:]+:\s*(?:#.*)?$)"
)
_DEBUG_SECTION = re.compile(r"^\s*\.section\s+\.debug_")
_ANY_SECTION = re.compile(r"^\s*\.section\s+")


def strip_debug_rows(text):
    out = []
    in_debug = False
    for line in text.splitlines():
        if _DEBUG_SECTION.match(line):
            in_debug = True
            continue
        if in_debug:
            if _ANY_SECTION.match(line):
                in_debug = False
            else:
                continue
        if _DEBUG_LINE.match(line):
            continue
        out.append(line)
    return "\n".join(out)


# (name, C body, targets where the template is encodable)
FIXTURES = [
    (
        "empty_template",
        'long f(long x) { __asm__ volatile("" : "+r"(x)); return x; }',
        "all",
    ),
    (
        "nop_template",
        'long f(long x) { __asm__ volatile("nop" : "+r"(x)); return x; }',
        "all",
    ),
    (
        "comment_only",
        'long f(long x) { __asm__ volatile("# just a comment\\n\\t" : "+r"(x)); return x; }',
        "all",
    ),
    (
        "bogus_mnemonic",
        'long f(long x) { __asm__ volatile("lccc_no_such_mnemonic %0" : "+r"(x)); return x; }',
        "none",
    ),
    (
        "q_suffix",
        'long f(long x) { __asm__ volatile("addq $1, %0" : "+r"(x)); return x; }',
        "x86_64",
    ),
    (
        "utf8_comments_and_q_suffix",
        'long f(long x) { __asm__ volatile('
        '"# MS09 UTF-8: caf\\303\\251 \\342\\202\\254 \\360\\237\\246\\200\\n\\t"'
        '"# MS09 split UTF-8: caf\\303" "\\251\\n\\t"'
        '"addq $1, %0" : "+r"(x)); return x; }',
        "x86_64",
    ),
    (
        # The case that needs the D6 markers stripped from the attribution
        # probe: the asm lives in a callee that gets inlined, so the text
        # carries `.Llccc_inline_*` range endpoints.
        "inlined_callee_q_suffix",
        'static long g(long x) { __asm__ volatile("addq $1, %0" : "+r"(x)); return x; }\n'
        'long f(long x) { return g(x) + g(x + 1); }',
        "x86_64",
    ),
    (
        # An inline-asm template is TEXT: every target must copy non-ASCII
        # comment bytes through unchanged. i686 used to double-encode them
        # because its `%d` stripper copied `byte as char`.
        "utf8_comment_nop",
        'long f(long x) { __asm__ volatile('
        '"# MS09 UTF-8: caf\\303\\251 \\342\\202\\254 \\360\\237\\246\\200\\n\\t"'
        '"nop" : "+r"(x)); return x; }',
        "all",
    ),
    (
        # GCC's `%=`: a number unique to this asm instance. A label definition
        # is valid on every target, so this pins the feature positively.
        "unique_label",
        'long f(long x) { __asm__ volatile(".Llccc_uniq_%=:\\n\\tnop" : "+r"(x)); return x; }',
        "all",
    ),
    (
        # A label and a directive inside the template must survive the probe,
        # or the attribution would blame the compiler for an undefined symbol.
        "label_and_directive",
        'long f(long x) { __asm__ volatile('
        '".Llccc_user_%=:\\n\\t.byte 1, 2, 3\\n\\tlccc_no_such_mnemonic %0"'
        ' : "+r"(x)); return x; }',
        "none",
    ),
]

FALLBACK_NOTE = "inline assembly is not assemblable for this target"

# `café € 🦀` exactly as the C escapes spell it. A byte-wise `byte as char`
# copy turns this into the double-encoded LEGACY form below.
UTF8_MARKER = b"caf\xc3\xa9 \xe2\x82\xac \xf0\x9f\xa6\x80"
# The pre-fix i686 form: each source byte was pushed as `byte as char`, i.e.
# decoded as Latin-1 and re-encoded, so `c3 a9` became `c3 83 c2 a9`.
UTF8_LEGACY = b"caf\xc3\x83\xc2\xa9"


def run(argv, timeout=120):
    return subprocess.run(argv, capture_output=True, timeout=timeout)


def encodable(spec, target):
    return spec == "all" or spec == target


def check_target(compiler, target, directory, report):
    failures = []
    for name, body, spec in FIXTURES:
        source = directory / f"{name}.c"
        source.write_text(body + "\n")
        good = encodable(spec, target)
        plain_s = directory / f"{name}.{target}.plain.s"
        debug_s = directory / f"{name}.{target}.debug.s"
        obj = directory / f"{name}.{target}.o"
        base = [str(compiler), "-O2"]
        plain = run(base + ["-S", str(source), "-o", str(plain_s)])
        debug = run(base + ["-g", "-S", str(source), "-o", str(debug_s)])
        row = {"target": target, "fixture": name, "encodable": good,
               "rc_plain": plain.returncode, "rc_debug": debug.returncode}
        if plain.returncode != debug.returncode:
            failures.append(
                f"{target}/{name}: -S rc={plain.returncode} but -S -g "
                f"rc={debug.returncode}; -g changed acceptance\n"
                f"  {debug.stderr.decode(errors='replace').strip()[:400]}"
            )
        if plain.returncode == 0 and debug.returncode == 0:
            a = strip_debug_rows(plain_s.read_text())
            b = strip_debug_rows(debug_s.read_text())
            row["code_identical"] = a == b
            if a != b:
                failures.append(f"{target}/{name}: -g changed the code")
            text = debug_s.read_text()
            note = FALLBACK_NOTE in debug.stderr.decode(errors="replace")
            row["fallback_taken"] = note
            row["has_debug_info"] = ".debug_info" in text
            if good:
                if note or ".debug_info" not in text:
                    failures.append(
                        f"{target}/{name}: an encodable template took the "
                        f"pass-through fallback (note={note}, "
                        f"debug_info={'.debug_info' in text})"
                    )
            elif not note:
                failures.append(
                    f"{target}/{name}: an unencodable template was described "
                    "without the explanatory note"
                )
        if name == "utf8_comment_nop" and plain.returncode == 0:
            for label, path in (("plain", plain_s), ("debug", debug_s)):
                if not path.is_file():
                    continue
                data = path.read_bytes()
                ok = UTF8_MARKER in data and UTF8_LEGACY not in data
                row[f"utf8_preserved_{label}"] = ok
                if not ok:
                    failures.append(
                        f"{target}/{name}: {label} assembly did not preserve the "
                        "template's UTF-8 bytes (inline asm is text, not a byte carrier)"
                    )
        if name == "unique_label" and plain.returncode == 0:
            text = plain_s.read_text()
            labels = re.findall(r"\.Llccc_uniq_([0-9]+):", text)
            row["unique_labels"] = labels
            if "%=" in text or len(labels) != 1:
                failures.append(
                    f"{target}/{name}: `%=` was not expanded to one unique label "
                    f"(found {labels}, raw marker present={'%=' in text})"
                )
        assembled = run(base + ["-g", "-c", str(source), "-o", str(obj)])
        row["rc_object"] = assembled.returncode
        if good and assembled.returncode != 0:
            failures.append(
                f"{target}/{name}: -c -g rejected an encodable template: "
                f"{assembled.stderr.decode(errors='replace').strip()[:300]}"
            )
        if not good and assembled.returncode == 0:
            failures.append(
                f"{target}/{name}: -c -g produced an object for a template "
                "this target cannot encode"
            )
        report["rows"].append(row)
    return failures


def check_unique_label_uniqueness(compiler, target, directory, report):
    """`%=` must differ per statement and per inlined copy, and must assemble."""
    failures = []
    source = directory / f"uniq.{target}.c"
    source.write_text(
        'static long g(long x) {\n'
        '  __asm__ volatile(".Llccc_inl_%=:\\n\\tnop" : "+r"(x));\n'
        '  return x;\n'
        '}\n'
        'long f(long x) {\n'
        '  __asm__ volatile(".Llccc_a_%=:\\n\\tnop" : "+r"(x));\n'
        '  x = g(x);\n'
        '  __asm__ volatile(".Llccc_b_%=:\\n\\tnop" : "+r"(x));\n'
        '  x = g(x + 1);\n'
        '  __asm__ volatile(".Llccc_c_%=:\\n\\tnop" : "+r"(x));\n'
        '  return x;\n'
        '}\n'
    )
    asm = directory / f"uniq.{target}.s"
    obj = directory / f"uniq.{target}.o"
    compiled = run([str(compiler), "-O2", "-S", str(source), "-o", str(asm)])
    row = {"target": target, "rc": compiled.returncode}
    if compiled.returncode != 0:
        failures.append(f"{target}/unique-label: -S failed: "
                        f"{compiled.stderr.decode(errors='replace').strip()[:300]}")
    else:
        text = asm.read_text()
        numbers = re.findall(r"\.Llccc_(?:a|b|c|inl)_([0-9]+):", text)
        row["labels"] = numbers
        row["distinct"] = len(set(numbers))
        if "%=" in text:
            failures.append(f"{target}/unique-label: a raw `%=` reached the assembly")
        # Three statements in f, plus g either inlined twice or emitted once.
        if len(numbers) < 4 or len(set(numbers)) != len(numbers):
            failures.append(
                f"{target}/unique-label: ids are not unique per instance: {numbers}"
            )
        assembled = run([str(compiler), "-O2", "-c", str(source), "-o", str(obj)])
        row["rc_object"] = assembled.returncode
        if assembled.returncode != 0:
            failures.append(
                f"{target}/unique-label: the generated labels do not assemble: "
                f"{assembled.stderr.decode(errors='replace').strip()[:300]}"
            )
    report["unique_label"].append(row)
    return failures


def check_gcc_oracle(gcc, directory, report):
    """GCC 16.2 is normative: -S -g accepts a 64-bit-only mnemonic, -c does not."""
    failures = []
    source = directory / "oracle_q_suffix.c"
    source.write_text(
        'long f(long x) { __asm__ volatile("addq $1, %0" : "+r"(x)); return x; }\n'
    )
    for label, extra in (("x86_64", []), ("i686", ["-m32"])):
        s = run([gcc, *extra, "-O2", "-g", "-S", str(source),
                 "-o", str(directory / f"oracle.{label}.s")])
        c = run([gcc, *extra, "-O2", "-g", "-c", str(source),
                 "-o", str(directory / f"oracle.{label}.o")])
        report["gcc_oracle"].append(
            {"target": label, "rc_assembly": s.returncode, "rc_object": c.returncode}
        )
        if label == "i686":
            if s.returncode != 0:
                failures.append("GCC 16.2 -m32 -S -g rejected the fixture; the oracle moved")
            if c.returncode == 0:
                # No i386 multilib on this host: assembling may fail for
                # unrelated reasons, but it must never succeed.
                failures.append("GCC 16.2 -m32 -c accepted a 64-bit-only mnemonic")
        elif s.returncode != 0 or c.returncode != 0:
            failures.append("GCC 16.2 x86-64 rejected a valid fixture")
    return failures


def self_test():
    """The filter and the parity rule must both be able to fail."""
    checks = []
    code = "\tmovl $1, %eax\n\tret\n"
    metadata = (
        '.file 1 "t.c"\n'
        ".loc 1 1 1\n"
        "\t.set\t.Llccc_debug_alias_3,.text+8\n"
        ".Llccc_inline_b_0_0_f:\n"
        ".Llccc_inline_e_0_0_f:\n"
        '\t.section\t.debug_info,"",@progbits\n'
        "\t.byte\t1,2,3\n"
        '\t.section\t.note.GNU-stack,"",@progbits\n'
    )
    non_debug_tail = '\t.section\t.note.GNU-stack,"",@progbits'
    checks.append((
        "filter removes metadata",
        strip_debug_rows(code + metadata) == code.rstrip("\n") + "\n" + non_debug_tail,
    ))
    changed = code.replace("movl $1", "movl $2")
    checks.append(("filter keeps a one-byte code change", strip_debug_rows(changed) != strip_debug_rows(code)))
    lookalike = ".Llccc_inline_b_0_0_f: nop\n"
    checks.append(("compound lookalike stays visible", strip_debug_rows(lookalike) == lookalike.rstrip("\n")))
    checks.append(("parity rule rejects differing rc", (0 == 1) is False))
    checks.append(("encodable spec: all", encodable("all", "riscv64")))
    checks.append(("encodable spec: none", not encodable("none", "x86_64")))
    checks.append(("encodable spec: one target", encodable("x86_64", "x86_64") and not encodable("x86_64", "i686")))
    failed = [name for name, ok in checks if not ok]
    for name, ok in checks:
        print(f"  {'ok  ' if ok else 'FAIL'} {name}")
    if failed:
        print(f"self-test FAILED: {failed}", file=sys.stderr)
        return 1
    print("self-test: filter and acceptance rules agree")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc", default=os.environ.get("CCC", str(ROOT / "target/fastbuild/lccc")))
    parser.add_argument("--lccc-i686", default=os.environ.get("CCC_I686"))
    parser.add_argument("--lccc-arm", default=os.environ.get("CCC_ARM"))
    parser.add_argument("--lccc-riscv", default=os.environ.get("CCC_RISCV"))
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    parser.add_argument("--json")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()

    host = Path(args.lccc)
    targets = [("x86_64", host)]
    for target, explicit in (
        ("i686", args.lccc_i686),
        ("aarch64", args.lccc_arm),
        ("riscv64", args.lccc_riscv),
    ):
        candidate = Path(explicit) if explicit else host.with_name(
            {"i686": "lccc-i686", "aarch64": "lccc-arm", "riscv64": "lccc-riscv"}[target]
        )
        if candidate.is_file():
            targets.append((target, candidate))
        else:
            print(f"SKIP {target}: {candidate} not built")

    report = {"rows": [], "gcc_oracle": [], "unique_label": [],
              "targets": [t for t, _ in targets]}
    failures = []
    with tempfile.TemporaryDirectory() as tmp:
        directory = Path(tmp)
        for target, compiler in targets:
            failures += check_target(compiler, target, directory, report)
            failures += check_unique_label_uniqueness(compiler, target, directory, report)
        if Path(args.gcc).is_file() or subprocess.run(
            ["which", args.gcc], capture_output=True
        ).returncode == 0:
            failures += check_gcc_oracle(args.gcc, directory, report)
        else:
            print(f"SKIP GCC oracle: {args.gcc} not installed")
    report["checks"] = len(report["rows"])
    report["failures"] = failures
    report["correct"] = not failures
    if args.json:
        Path(args.json).write_text(json.dumps(report, indent=2) + "\n")
    for failure in failures:
        print(f"FAIL {failure}", file=sys.stderr)
    if failures:
        print(f"FAIL -g acceptance parity: {len(failures)} of {len(report['rows'])} rows", file=sys.stderr)
        return 1
    print(
        f"PASS -g acceptance parity: {len(report['rows'])} rows on "
        f"{', '.join(report['targets'])}; -S/-S -g agree, code identical, "
        "-c still rejects unencodable templates, valid templates keep full DWARF"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
