#!/usr/bin/env python3
"""Narrow-ring -> pointer-ring linearisation sweep ("the wrap class").

WHY.  PR #777 claims one live miscompile: `buf[i * 2]` with `uint32_t i`, where
the back end's SIB peel folds a product that C evaluates in a 32-bit wrapping
ring into pointer-ring arithmetic that cannot wrap.  A fix aimed at one instance
of a class is how classes survive, so this harness enumerates the CLASS and asks,
for every shape, whether the EXECUTED result matches the reference compiler.

METHOD.  Each case mmaps 2^32 + 2 pages of PROT_NONE and commits exactly four
pages -- byte 0, the 2 GiB point, the top of the 32-bit range, and the 2^32
point.  For an index whose narrow-ring product is `2^32 + d`:

  * the CORRECT (wrapped) access reads offset `d`        -> marker 11
  * the LINEARISED (pointer-ring) access reads `2^32+d`  -> marker 99
  * anything else faults into a guard page, which is also a verdict

Each case is its own translation unit and its own process, so a fault is
attributable to exactly one shape.  `--opaque` additionally routes the index
through argv into a `volatile`, which is the severity question: a constant index
can be folded away, a runtime one cannot.

The reference is gcc at the same -O level.  A case is a MISCOMPILE when lccc's
executed output differs from the reference's.  Always run the reference against
itself first (`wrap_sweep.py gcc`) -- that is what caught three defects in this
harness before any compiler was accused of anything.

WHAT IT MEASURED (2026-10-08, this tree, gcc 14.2 as reference, 39 shapes)
-------------------------------------------------------------------------
                       constant index      index routed through a volatile
  -O0                     39/39 agree                 78/78 agree
  -O1                     18/39 agree   42/78 diverge
  -O2                     39/39 agree   21/39 diverge  <- default optimisation
  -O3                     39/39 agree   21/39 diverge

The divergent set is exactly the gated peel family -- mul 2/4/8, shl 1/2/3,
`i+i`, `i+k`, `i-k`, `(i+1)*2`, `i*2` from the top of the ring, and all three
loop variants -- and it includes one SIGSEGV (`u32_top_mul2` at -O1).  Shapes
that agree even unfixed are the ones the peel never claimed: scale 3 and 16 (no
SIB scale), every masked/modulus bound (no wrap reachable), the `struct[i]`
element control (C scales in ptrdiff_t, so LINEAR is correct there), and the
signed/u64/small-index controls.  Read the constant-index column as the reason
this class was mis-filed as an -O1-only bug: a folded index hides it at -O2.

With the fix in place the same sweep is 78/78 at -O1, -O2 and -O3, and 39/39
with constant indices at -O0.  Evidence:
engineering/evidence/2026-10-08-pr-audit/wrap-sweep-verdicts{,-opaque}.txt

Usage:  python3 scripts/wrap_ring_sweep.py <cc> [<cc> ...] [--opts=-O1,-O2,-O3] [--opaque]
        python3 scripts/wrap_ring_sweep.py gcc     # reference self-check, do this first
"""
import pathlib
import subprocess
import sys

WORK = pathlib.Path("/tmp/wrapsweep")
INC = subprocess.check_output(["gcc", "-print-file-name=include"], text=True).strip()
OPAQUE = "--opaque" in sys.argv

PRELUDE = r"""
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <unistd.h>
#define SPAN ((size_t)0x100000000ull + 0x2000ull)
static unsigned char *g_buf;
static int setup(void) {
    size_t page = (size_t)sysconf(_SC_PAGESIZE);
    char *map = mmap(0, SPAN, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
    if (map == MAP_FAILED) { printf("MMAP-FAIL\n"); return 1; }
    g_buf = (unsigned char *)map;
    /* Commit a page at every offset the case table touches.  Everything else
     * stays PROT_NONE, so a wrong address faults instead of reading zero. */
    static const size_t offs[] = { 0u, 0x80000000ull, 0xFFFFF000ull, 0x100000000ull };
    for (unsigned k = 0; k < sizeof offs / sizeof offs[0]; k++)
        if (mprotect(g_buf + offs[k], page, PROT_READ | PROT_WRITE)) {
            printf("MPROT-FAIL %zx\n", offs[k]); return 1;
        }
    return 0;
}
#define MARK(off, val) g_buf[(size_t)(off)] = (unsigned char)(val)
#define REPORT(name, got) printf("%s %u\n", name, (unsigned)(got))
volatile unsigned g_seed;
"""

SCALAR = r"""
__attribute__((noinline)) static unsigned probe(const unsigned char *buf, {itype} i) {{
    return (unsigned)({expr});
}}
int main(int argc, char **argv) {{
    if (setup()) return 90;
    {marks}
    {init}
    REPORT("{name}", probe(g_buf, i));
    return 0;
}}
"""

LOOP = r"""
__attribute__((noinline)) static unsigned long walk(const unsigned char *buf, {itype} start) {{
    unsigned long s = 0;
    /* A separate counter: `i < start + 4u` would itself wrap for exactly the
     * start values that make the index arithmetic interesting. */
    for (unsigned k = 0; k < 4u; k++) {{
        {itype} i = ({itype})(start + ({itype})k);
        s += (unsigned)({expr});
    }}
    return s;
}}
int main(int argc, char **argv) {{
    if (setup()) return 90;
    {marks}
    {init}
    REPORT("{name}", walk(g_buf, start));
    return 0;
}}
"""

# name | index type | expression | index value | wrapped marker offset | linear marker offset
# A `# CONTROL` case must read the LINEAR marker under correct C: array indexing
# converts the index to ptrdiff_t BEFORE scaling, so it never wraps.
CASES = [
    # ── scale peels: mul.  index chosen so index*scale == 2^32 + d ──────────
    ("mul2_u32", "unsigned", "buf[i * 2u]", "0x80000001u", 2, 0x100000002),
    ("mul4_u32", "unsigned", "buf[i * 4u]", "0x40000001u", 4, 0x100000004),
    ("mul8_u32", "unsigned", "buf[i * 8u]", "0x20000001u", 8, 0x100000008),
    ("mul16_u32", "unsigned", "buf[i * 16u]", "0x10000001u", 16, 0x100000010),
    ("mul3_u32", "unsigned", "buf[i * 3u]", "0x55555556u", 2, 0x100000002),
    # ── scale peels: shl ────────────────────────────────────────────────────
    ("shl1_u32", "unsigned", "buf[i << 1]", "0x80000001u", 2, 0x100000002),
    ("shl2_u32", "unsigned", "buf[i << 2]", "0x40000001u", 4, 0x100000004),
    ("shl3_u32", "unsigned", "buf[i << 3]", "0x20000001u", 8, 0x100000008),
    # ── self-add (the canonicalizer's `i + i` for `i * 2`) ──────────────────
    ("selfadd_u32", "unsigned", "buf[i + i]", "0x80000001u", 2, 0x100000002),
    # ── affine: add(iv, const) peels ────────────────────────────────────────
    ("addconst_u32", "unsigned", "buf[i + 1u]", "0xFFFFFFFFu", 0, 0x100000000),
    ("addconst4_u32", "unsigned", "buf[i + 4u]", "0xFFFFFFFCu", 0, 0x100000000),
    ("affine_mul2_add4", "unsigned", "buf[i * 2u + 4u]", "0x80000000u", 4, 0x100000004),
    ("affine_add1_mul4", "unsigned", "buf[(i + 1u) * 4u]", "0x3FFFFFFFu", 0, 0x100000000),
    ("affine_add1_mul8", "unsigned", "buf[(i + 1u) * 8u]", "0x1FFFFFFFu", 0, 0x100000000),
    # ── row indexing ────────────────────────────────────────────────────────
    ("row2048_u32", "unsigned", "buf + (i * 2048u)", "0x200001u", 2048, 0x100000800),
    ("u32_top_mul2", "unsigned", "buf[i * 2u]", "0xFFFFFFFEu", 0xFFFFFFFC, None),
    ("u32_mid_mul2", "unsigned", "buf[i * 2u]", "0x40000000u", 0x80000000, None),
    # ── provably bounded index: a RANGE gate must still fold these ──────────
    ("masked_ff_mul4", "unsigned", "buf[(i & 0xFFu) * 4u]", "0xDEADu", 0xAD * 4, None),
    ("masked_ff_u32", "unsigned", "buf[i & 0xFFu]", "0x1234u", 0x34, None),
    ("masked_fff_mul2", "unsigned", "buf[((i & 0xFFFu) * 2u) & 0xFFFu]", "0xBEEFu", (0xEEF * 2) & 0xFFF, None),
    ("mod_small_u32", "unsigned", "buf[(i % 100u) * 4u]", "1234u", (1234 % 100) * 4, None),
    # ── controls that MUST keep folding (no pessimization is acceptable) ────
    ("signed_i32_mul4", "int", "buf[i * 4]", "1", 4, None),
    ("u64_mul4", "uint64_t", "buf[i * 4u]", "1u", 4, None),
    ("u32_small_mul4", "unsigned", "buf[i * 4u]", "1u", 4, None),
    ("u32_small_shl3", "unsigned", "buf[i << 3]", "1u", 8, None),
    # ── sub(iv, const) peel: the wrap goes BELOW zero, so the linearised
    #    address leaves the mapping entirely and faults (a louder verdict)
    ("subconst_u32", "unsigned", "buf[i - 1u]", "0u", 0xFFFFFFFF, None),
    ("subconst4_u32", "unsigned", "buf[i - 4u]", "2u", 0xFFFFFFFE, None),
    # ── narrow product explicitly widened afterwards: C wraps FIRST, then
    #    extends, so a transform that widens before multiplying is wrong
    ("widen_after_mul2", "unsigned", "buf[(uint64_t)(i * 2u)]", "0x80000001u", 2, 0x100000002),
    ("widen_after_add1", "unsigned", "buf[(uint64_t)(i + 1u)]", "0xFFFFFFFFu", 0, 0x100000000),
    ("widen_after_selfadd", "unsigned", "buf[(size_t)(i + i)]", "0x80000001u", 2, 0x100000002),
    # ── nested / two-level affine: (i*2)*2 and (i+1)*2+2
    ("nested_mul2_mul2", "unsigned", "buf[(i * 2u) * 2u]", "0x40000001u", 4, 0x100000004),
    ("nested_add_mul_add", "unsigned", "buf[(i + 1u) * 2u + 2u]", "0x7FFFFFFFu", 4, 0x100000004),
    # ── index that wraps twice over (product far past 2^32)
    ("mul2_far_u32", "unsigned", "buf[i * 2u]", "0xFFFFFFFFu", 0xFFFFFFFE, None),
    ("mul4_far_u32", "unsigned", "buf[i * 4u]", "0xFFFFFFFFu", 0xFFFFFFFC, None),
    # ── CONTROLS: C widens the index before scaling, so these read the linear
    #    marker under correct semantics.  A compiler that "fixes" them is wrong.
    ("struct4_u32", "unsigned", "((const unsigned *)buf)[i]", "0x40000001u", 4, 0x100000004),
    ("struct8_u32", "unsigned", "((const uint64_t *)buf)[i]", "0x20000001u", 8, 0x100000008),
]

# Loop-carried variants: where IVSR and the back-end peel interact.
# (name, index type, expr, start, markers, wrapped sum, linear sum)
LOOP_CASES = [
    ("loop_mul2_u32", "unsigned", "buf[i * 2u]", "0x80000000u",
     [(0, 11), (2, 11), (4, 11), (6, 11),
      (0x100000000, 99), (0x100000002, 99), (0x100000004, 99), (0x100000006, 99)], 44, 396),
    ("loop_addconst_u32", "unsigned", "buf[i + 1u]", "0xFFFFFFFFu",
     [(0, 11), (1, 11), (2, 11), (3, 11), (0x100000000, 99)], 44, 132),
    ("loop_mul4_from_zero", "unsigned", "buf[i * 4u]", "0x3FFFFFFEu",
     [(0xFFFFFFF8, 11), (0xFFFFFFFC, 11), (0, 11), (4, 11),
      (0x100000000, 99), (0x100000004, 99)], 44, 220),
]


def gen():
    WORK.mkdir(parents=True, exist_ok=True)
    made = []

    def emit(name, itype, expr, marks, init_const, init_opaque, tmpl, arg):
        src = PRELUDE + tmpl.format(name=name, itype=itype, expr=expr,
                                    marks="\n    ".join(marks), init=init_const)
        (WORK / f"{name}.c").write_text(src)
        made.append((name, WORK / f"{name}.c", arg))
        if OPAQUE:
            oname = name + "_opq"
            osrc = PRELUDE + tmpl.format(name=oname, itype=itype, expr=expr,
                                         marks="\n    ".join(marks), init=init_opaque)
            (WORK / f"{oname}.c").write_text(osrc)
            made.append((oname, WORK / f"{oname}.c", arg))

    for name, itype, expr, ival, woff, loff in CASES:
        if expr.startswith("buf + "):
            expr = f"*({expr})"
        marks = [f"MARK({woff}ull, 11);"]
        if loff is not None:
            marks.append(f"MARK({loff}ull, 99);")
        emit(name, itype, expr, marks,
             f"{itype} i = ({itype}){ival};",
             f'g_seed = (unsigned)strtoul(argv[1], 0, 0);\n    {itype} i = ({itype})g_seed;',
             SCALAR, str(int(ival.rstrip("u"), 0)))
    for name, itype, expr, ival, marks, wsum, lsum in LOOP_CASES:
        mtext = [f"MARK({off}ull, {val});" for off, val in marks]
        emit(name, itype, expr, mtext,
             f"{itype} start = ({itype}){ival};",
             f'g_seed = (unsigned)strtoul(argv[1], 0, 0);\n    {itype} start = ({itype})g_seed;',
             LOOP, str(int(ival.rstrip("u"), 0)))
    return made


def build_and_run(cc, src, out, flags, arg=None):
    cmd = [cc, *flags, "-I", INC, str(src), "-o", str(out)]
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=300)
    if r.returncode != 0:
        return "BUILD-FAIL", (r.stderr or "")[:200]
    try:
        r = subprocess.run([str(out)] + ([arg] if arg else []),
                           capture_output=True, text=True, timeout=120)
    except subprocess.TimeoutExpired:
        return "TIMEOUT", ""
    if r.returncode < 0:
        return f"SIGNAL({-r.returncode})", ""
    return (r.stdout.strip() or f"rc={r.returncode}"), ""


def main():
    compilers = [a for a in sys.argv[1:] if not a.startswith("--")]
    opts = ["-O2"]
    for a in sys.argv[1:]:
        if a.startswith("--opts"):
            opts = a.split("=", 1)[1].split(",")
    cases = gen()
    mode = "opaque index (runtime, volatile)" if OPAQUE else "constant index"
    print(f"{len(cases)} cases x {len(opts)} opt levels x {len(compilers)} compilers, {mode}")
    verdicts = {}
    for opt in opts:
        flags = [opt]
        ref = {}
        for name, src, arg in cases:
            out, err = build_and_run("gcc", src, WORK / f"gcc_{name}_{opt.strip('-')}", flags, arg)
            ref[name] = out
            if out == "BUILD-FAIL":
                print(f"  !! REFERENCE BUILD FAILED {name}: {err}")
            elif out.startswith(("SIGNAL", "MMAP", "MPROT", "TIMEOUT")):
                print(f"  !! REFERENCE UNUSABLE {name}: {out}")
        for cc in compilers:
            tag = pathlib.Path(cc).name
            for name, src, arg in cases:
                out, err = build_and_run(cc, src, WORK / f"{tag}_{name}_{opt.strip('-')}", flags, arg)
                verdicts[(opt, tag, name)] = (out, ref[name], err)
                if out != ref[name]:
                    print(f"  DIFF {opt} {name:26s} gcc={ref[name]!r} {tag}={out!r} {err}")
    print("\n=== summary ===")
    for cc in compilers:
        tag = pathlib.Path(cc).name
        for opt in opts:
            keys = [k for k in verdicts if k[0] == opt and k[1] == tag]
            bad = [k[2] for k in keys if verdicts[k][0] != verdicts[k][1]]
            print(f"{tag:16s} {opt}: {len(keys) - len(bad)}/{len(keys)} agree"
                  + (f"   DIVERGENT({len(bad)}): {', '.join(sorted(bad))}" if bad else ""))
    out = pathlib.Path("/home/user/wrap-sweep-verdicts%s.txt" % ("-opaque" if OPAQUE else ""))
    out.write_text("\n".join(
        f"{k[0]}\t{k[1]}\t{k[2]}\tgcc={v[1]}\tgot={v[0]}" for k, v in sorted(verdicts.items())) + "\n")
    print(f"verdicts -> {out}")


if __name__ == "__main__":
    main()
