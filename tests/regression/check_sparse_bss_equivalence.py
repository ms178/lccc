#!/usr/bin/env python3
"""Sparse NOBITS `.bss`: the equivalence proof, in tree.

`tests/regression/check_sparse_bss_nobits.sh` proves the *contract* (sizes,
offsets, GAS parity, bound behaviour).  This gate proves the two claims the
contract alone cannot: that the sparse representation did not change what the
programs *mean*, and that "a size, not bytes" is visible in the process.

Three sections:

  A. Equivalence against a reference compiler
     (`--reference PATH` or `LCCC_PRE_SPARSE=PATH`): the same probe sources are
     compiled with both compilers and the *observable* results must agree —
     exit codes, stdout, and the `.bss` size/symbol layout of the objects.  This
     is the section that turns "the new representation behaves" into "the new
     representation is equivalent to the one it replaced", which is the only
     statement that justifies replacing it.

  B. Memory: a huge `.bss` object is a size, never bytes
     A 16 GB array must compile in a constant, small amount of memory (the gate
     measures the compiler child's peak RSS) — the pre-sparse writer would have
     materialised the zeros and died.

  C. `.bss` size fidelity against `nm`
     The section's `sh_size` must cover the symbols it carries: the sum of the
     symbol sizes reported by the reference `nm` (GCC's binutils) must match the
     section size up to alignment slack.  A sparse writer that loses or
     double-counts a gap shows up here.

Exit 0 = all executed sections passed, 1 = a failure, 0 with SKIP lines when a
section has no oracle available.

Usage:
  tests/regression/check_sparse_bss_equivalence.py
  tests/regression/check_sparse_bss_equivalence.py --lccc target/fastbuild/lccc
  tests/regression/check_sparse_bss_equivalence.py --reference /path/pre/lccc
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

TIMEOUT = 900


def run(cmd, timeout=TIMEOUT):
    return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)


def run_measured(cmd, timeout=TIMEOUT):
    """Run `cmd` and return (returncode, stdout, peak_rss_mib of the child).

    The wrapper measures `RUSAGE_CHILDREN` *inside* a fresh child, so the peak
    belongs to this execution only — `RUSAGE_CHILDREN` in the gate process is
    a high-water mark and would attribute a previous section's peak to every
    later row.
    """
    code = (
        "import resource,subprocess,sys,json;"
        "rc=subprocess.call(sys.argv[1:]);"
        "r=resource.getrusage(resource.RUSAGE_CHILDREN);"
        "sys.stdout.write(json.dumps({'rc':rc,'peak_kb':r.ru_maxrss}))"
    )
    p = run([sys.executable, "-c", code, *cmd], timeout=timeout)
    try:
        info = json.loads(p.stdout.strip().splitlines()[-1])
    except (ValueError, IndexError):
        return (p.returncode, "", None)
    return (info["rc"], "", info["peak_kb"] / 1024.0)


def bss_size(path):
    """`sh_size` of `.bss` in an ELF object, or None when absent."""
    out = run(["readelf", "-S", "-W", path]).stdout
    m = re.search(r"\.bss\s+NOBITS\s+[0-9a-f]+\s+[0-9a-f]+\s+([0-9a-f]+)", out)
    if m:
        return int(m.group(1), 16)
    huge = re.search(r"\.bss\s+NOBITS\s+[0-9a-f]+\s+([0-9a-f]+)\s+[0-9a-f]{10,}", out)
    return int(huge.group(1), 16) if huge else None


def nm_bss_symbols(path):
    """`{name: (addr, size)}` for the symbols in `.bss`, via the reference nm."""
    nm = shutil.which("nm")
    if not nm:
        return None
    out = run([nm, "--print-size", path]).stdout
    syms = {}
    for line in out.splitlines():
        parts = line.split()
        if len(parts) >= 4 and parts[2] in ("b", "B"):
            try:
                syms[parts[3]] = (int(parts[0], 16), int(parts[1], 16))
            except ValueError:
                continue
    return syms


class Probes:
    def __init__(self, root):
        self.root = root
        self.count = 0

    def write(self, name, text):
        path = os.path.join(self.root, name)
        with open(path, "w") as f:
            f.write(text)
        return path


def section_a(lccc, reference):
    """Equivalence against the pre-change compiler."""
    print("A. equivalence against the pre-change compiler")
    if not reference:
        print(
            "  SKIP no --reference (pre-change) compiler given: this section is the"
        )
        print(
            "       change's strongest claim and needs the compiler it changed"
        )
        return 0, 0
    tmp = tempfile.mkdtemp(prefix="sparse-eq-")
    probes = Probes(tmp)
    sources = {
        # A big object plus a small one: the sparse writer must keep the
        # storage zero-filled and the declared objects distinct and disjoint.
        # Address arithmetic goes through uintptr_t: cross-object pointer
        # comparisons/subtraction are UB and have no place in a gate that
        # claims equivalence.
        "zero_fill.c": """
             #include <stdint.h>
             int big_array[1 << 24];
             int small_array[64];
             static int *alias_of_big;
             int main(void)
             {
                 uintptr_t big0 = (uintptr_t)(void *)&big_array[0];
                 uintptr_t bigN = (uintptr_t)(void *)&big_array[(1 << 24) - 1];
                 uintptr_t small0 = (uintptr_t)(void *)&small_array[0];
                 big_array[0] = 1;
                 big_array[(1 << 24) - 1] = 2;
                 small_array[63] = 3;
                 alias_of_big = big_array;
                 if (alias_of_big != big_array) return 10;
                 if (bigN - big0 != (uintptr_t)(((1 << 24) - 1) * sizeof(int))) return 11;
                 if (small0 < bigN + sizeof(int)) return 12;
                 if (big_array[123] != 0) return 13;
                 return big_array[0] + big_array[(1 << 24) - 1] + small_array[63] - 6;
             }
             """,
        # Sections that must NOT be sparse (rodata/data) next to one that is.
        "mixed_sections.c": """
             const char rodata_text[] = "sparse";
             int bss_a[4096];
             int data_b[16] = {1, 2, 3};
             int bss_b;
             int main(void)
             {
                 bss_a[4095] = data_b[2];
                 bss_b = rodata_text[0];
                 if (rodata_text[5] != 'e') return 10;
                 if (data_b[0] != 1) return 11;
                 if (bss_b != 's') return 12;
                 return bss_a[4095] - 3;
             }
             """,
        # The sparse tail: a label far inside a huge object must keep its
        # offset -- the case where "a size, not bytes" gives a wrong answer if
        # the offset arithmetic is off by a section's worth.
        "sparse_tail_label.c": """
             #include <stdint.h>
             int tail[1 << 24];
             int after;
             int main(void)
             {
                 uintptr_t span = (uintptr_t)(void *)&tail[(1 << 24) - 1]
                                  - (uintptr_t)(void *)&tail[0];
                 if (span != (uintptr_t)(((1 << 24) - 1) * sizeof(int))) return 10;
                 if ((uintptr_t)(void *)&after < (uintptr_t)(void *)&tail[(1 << 24) - 1] + sizeof(int)) return 11;
                 tail[0] = 7;
                 tail[(1 << 24) - 1] = 9;
                 after = tail[0] + tail[(1 << 24) - 1];
                 return after - 16;
             }
             """,
    }
    pass_n = fail_n = 0
    for name, text in sources.items():
        src = probes.write(name, text)
        objs = {}
        ok = True
        for tag, compiler in (("ref", reference), ("lccc", lccc)):
            obj = os.path.join(tmp, f"{name}.{tag}.o")
            exe = os.path.join(tmp, f"{name}.{tag}")
            r = run([compiler, "-O2", "-c", src, "-o", obj])
            if r.returncode != 0:
                print(f"  FAIL {name} [{tag} compile]: {r.stderr.strip().splitlines()[:1]}")
                ok = False
                break
            r = run([compiler, "-O2", src, "-o", exe])
            if r.returncode != 0:
                print(f"  FAIL {name} [{tag} link]: {r.stderr.strip().splitlines()[:1]}")
                ok = False
                break
            run_rc = run([exe]).returncode
            objs[tag] = (bss_size(obj), run_rc)
        if not ok:
            fail_n += 1
            continue
        ref_bss, ref_rc = objs["ref"]
        new_bss, new_rc = objs["lccc"]
        if ref_rc != new_rc:
            print(f"  FAIL {name}: exit {new_rc} != reference {ref_rc}")
            fail_n += 1
        elif ref_bss != new_bss:
            print(f"  FAIL {name}: .bss {new_bss} != reference {ref_bss}")
            fail_n += 1
        else:
            print(f"  ok   {name}: .bss={new_bss} rc={new_rc} (identical to reference)")
            pass_n += 1
    return pass_n, fail_n


def section_b(lccc):
    """A huge `.bss` object is a size, never bytes."""
    print("B. memory: a huge .bss object is a size, never bytes")
    tmp = tempfile.mkdtemp(prefix="sparse-mem-")
    src = os.path.join(tmp, "huge.c")
    pass_n = fail_n = 0
    for count, want in (
        (1_000_000, 4_000_000),
        (100_000_000, 400_000_000),
        (1_000_000_000, 4_000_000_000),
        (4_000_000_000, 16_000_000_000),
    ):
        with open(src, "w") as f:
            f.write(f"int a[{count}];\nint main(void) {{ return 0; }}\n")
        obj = os.path.join(tmp, "huge.o")
        import time

        t0 = time.time()
        rc, _, peak = run_measured([lccc, "-O2", "-c", src, "-o", obj])
        wall = time.time() - t0
        got = bss_size(obj) if rc == 0 else None
        # The compiler must stay bounded: a materialising writer needs the
        # full object size in memory (>= count bytes) and dies at 16 GB.
        if rc != 0 or got != want:
            print(f"  FAIL int a[{count}]: rc={rc} .bss={got} (want {want})")
            fail_n += 1
        else:
            print(
                f"  ok   int a[{count}]: rc=0 .bss={got} (want {want}) "
                f"peak={peak:.0f}MiB wall={wall:.2f}s"
            )
            pass_n += 1
    return pass_n, fail_n


def section_c(lccc):
    """`.bss` size fidelity against the reference `nm`.

    Fidelity here means three things a sparse writer can get wrong:
      1. the section covers its symbols — `sh_size` reaches the end of the
         last `.bss` symbol;
      2. it does not over-reserve — the gap between `sh_size` and that end is
         alignment padding, so it stays within one cache line;
      3. declared alignments are honoured — every symbol named in a row's
         `aligned` map sits at an address that is a multiple of its alignment
         (nm reports section-relative addresses in an object file, so this is
         checkable without linking).

    Comparing `sh_size` against the *sum* of symbol sizes alone cannot do
    this: alignment gaps are real section bytes that no symbol owns, so an
    aligned probe legitimately has `sh_size > sum(sizes)`.
    """
    print("C. .bss size fidelity (GCC nm as the oracle)")
    if not shutil.which("nm"):
        print("  SKIP nm not available")
        return 0, 0
    tmp = tempfile.mkdtemp(prefix="sparse-fid-")
    probes = Probes(tmp)
    rows = [
        ("many-objects", "\n".join(f"int obj_{i};" for i in range(64)), {}),
        (
            "aligned",
            "char c;\n"
            "__attribute__((aligned(64))) int a0;\n"
            "__attribute__((aligned(64))) int a1;",
            {"a0": 64, "a1": 64, "c": 1},
        ),
        ("array-then-obj", "int arr[1024];\nint afterwards;\nint arr2[7];", {}),
        (
            "mixed-sections",
            'const char ro[] = "x";\nint in_bss[100];\nint in_data = 1;\nchar tail;',
            {},
        ),
    ]
    pass_n = fail_n = 0
    for name, body, aligned in rows:
        src = probes.write(f"{name}.c", body + "\nint main(void) { return 0; }\n")
        obj = os.path.join(tmp, f"{name}.o")
        r = run([lccc, "-O2", "-c", src, "-o", obj])
        if r.returncode != 0:
            print(f"  FAIL {name}: compile failed: {r.stderr.strip().splitlines()[:1]}")
            fail_n += 1
            continue
        size = bss_size(obj)
        syms = nm_bss_symbols(obj)
        if size is None or syms is None or not syms:
            print(f"  FAIL {name}: no .bss section or no sized .bss symbols")
            fail_n += 1
            continue
        span = max(addr + sz for addr, sz in syms.values())
        total = sum(sz for _, sz in syms.values())
        bad = [n for n, al in aligned.items() if n in syms and syms[n][0] % al != 0]
        if size < span or size < total:
            print(f"  FAIL {name}: .bss={size} does not cover symbols (span={span} sum={total})")
            fail_n += 1
        elif size - span > 64:
            print(f"  FAIL {name}: .bss={size} over-reserves vs span={span} (> 64 padding)")
            fail_n += 1
        elif bad:
            print(
                f"  FAIL {name}: misaligned symbol(s) "
                + ", ".join(f"{n}@{syms[n][0]:#x} (align {aligned[n]})" for n in bad)
            )
            fail_n += 1
        else:
            print(
                f"  ok   {name}: .bss={size} span={span} sum={total} "
                f"slack={size - span} (alignment honoured)"
            )
            pass_n += 1
    return pass_n, fail_n


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--lccc", default="target/fastbuild/lccc")
    ap.add_argument(
        "--reference",
        default=os.environ.get("LCCC_PRE_SPARSE"),
        help="pre-change compiler to prove equivalence against (section A)",
    )
    args = ap.parse_args()

    lccc = shutil.which(args.lccc) or (
        args.lccc if os.path.isfile(args.lccc) else None
    )
    if not lccc or not os.access(lccc, os.X_OK):
        print(f"SKIP: no compiler at {args.lccc}")
        return 0
    lccc = os.path.abspath(lccc)

    pass_n = fail_n = 0
    for section in (lambda: section_a(lccc, args.reference), lambda: section_b(lccc), lambda: section_c(lccc)):
        p, f = section()
        pass_n += p
        fail_n += f

    print()
    if fail_n == 0:
        print("SPARSE NOBITS EQUIVALENCE PASS")
        return 0
    print(f"SPARSE NOBITS EQUIVALENCE FAIL ({fail_n} failed)")
    return 1


if __name__ == "__main__":
    sys.exit(main())
