#!/usr/bin/env python3
"""Differential fuzzer for the bool_thread int-phi-cmp threading pass.

Generates randomized merge-diamond programs covering:
  - dead-after-test int phis tested through merge-local compares (threadable)
  - live-after-test phis (rule 3 must reject threading, keep semantics)
  - constant arms (compile-time fold path, signed and unsigned)
  - loop-carried phis (must not thread)
  - critical edges / nested diamonds
  - bool phis with a tolerated dead Cmp in the merge (the Agent Z soundness
    hole this fuzzer explicitly hunts)
Each program is compiled with gcc -O0 (oracle), gcc -O2, and lccc -O2 (plus
lccc -O1/-O0 spot checks); all outputs must match exactly.
"""
import random, subprocess, sys, os, tempfile

LCCC = os.environ.get("LCCC", "/home/user/lccc/target/fastbuild/lccc")

def gen_program(rng, idx):
    """Return C source text for one randomized program."""
    ops = ["<", "<=", ">", ">=", "==", "!="]
    uops = ["<", "<=", ">", ">="]
    lines = []
    body = []

    def cmp_expr(p, ty, rng):
        op = rng.choice(ops if ty == "int" else uops)
        if ty == "unsigned":
            k = rng.choice([0, 1, 7, 8, 0xfffffff0, 0x7fffffff])
            return f"{p} {op} {k}u"
        k = rng.choice([-8, -1, 0, 1, 3, 100])
        return f"{p} {op} ({k})"

    # fn: two diamonds, optionally a loop, phis compared merge-locally.
    fn = f"static unsigned long long f{idx}(int a, int b) {{\n"
    fn += "    unsigned long long acc = 0;\n"
    # diamond 1 -> int phi p (dead or live after test, random)
    live1 = rng.random() < 0.4
    c1op = rng.choice(["a + 1", "a - b", "a ^ 3", "a | b", "(a * 5) % 97"])
    c2op = rng.choice(["b + 2", "b - a", "b ^ 7", "b & 15", "(b * 3) % 89"])
    const_arms = rng.random() < 0.3
    v1, v2 = ("5", "-9") if const_arms else (c1op, c2op)
    fn += f"    int p;\n    if (a > b) p = {v1}; else p = {v2};\n"
    if rng.random() < 0.3:
        ty = "unsigned"
        fn += f"    unsigned q;\n    if (a < b) q = (unsigned)p + 1u; else q = 0xfffffff0u;\n"
        fn += f"    if ({cmp_expr('q', ty, rng)}) acc += 1000; else acc += 7;\n"
        if live1:
            fn += f"    acc += (unsigned long long)(p ^ q);\n"
    else:
        fn += f"    if ({cmp_expr('p', 'int', rng)}) acc += 1000; else acc += 7;\n"
        if live1:
            fn += f"    acc += (unsigned long long)(p < 0 ? -p : p * 2);\n"
    # diamond 2: bool phi with possible dead Cmp in the merge (hole hunt)
    if rng.random() < 0.5:
        fn += "    int r;\n    if (b > 3) r = p; else r = -p;\n"
        style = rng.random()
        if style < 0.4:
            # dead Cmp in merge + branch on the bool phi (Agent Z hole shape)
            fn += "    int t;\n    if (r > 0) t = 1; else t = 0;\n"
            fn += "    if (t) acc += 3; else acc += 4;\n"
        else:
            fn += "    if (r) acc += 3; else acc += 4;\n"
        if rng.random() < 0.5:
            fn += "    acc += (unsigned long long)(r + 40);\n"
    # loop-carried phi tested through a compare (rule 5 rejection shape)
    if rng.random() < 0.4:
        fn += "    for (int i = 0; i < a % 5; i++) {\n"
        fn += "        int s;\n        if (i & 1) s = i; else s = -i;\n"
        fn += "        if (s <= 0) acc += 2; else acc += (unsigned long long)s;\n"
        fn += "        if (s == -3) acc += 11;\n"
        fn += "    }\n"
    # nested / critical-edge shape
    if rng.random() < 0.4:
        fn += "    int u;\n    if (a & 1) { if (b & 1) u = a * b; else u = a + b; } else u = a - b;\n"
        fn += f"    if ({cmp_expr('u', 'int', rng)}) acc += 31; else acc += 13;\n"
        if rng.random() < 0.5:
            fn += "    acc += (unsigned long long)u;\n"
    fn += "    return acc * 2654435761ull + (unsigned long long)a;\n}\n"

    src = "#include <stdio.h>\n/* fuzzer program %d */\n" % idx
    src += "extern void * volatile sink;\n" if False else ""
    src += fn
    src += f"""
int main(void) {{
    unsigned long long h = 0;
    for (int a = -6; a <= 6; a++)
        for (int b = -6; b <= 6; b++)
            h = h * 31 + f{idx}(a, b);
    printf("%llu\\n", h);
    return 0;
}}
"""
    return src

def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, **kw)

def main():
    n = int(sys.argv[1]) if len(sys.argv) > 1 else 150
    seed = int(sys.argv[2]) if len(sys.argv) > 2 else 20260829
    rng = random.Random(seed)
    fails = 0
    with tempfile.TemporaryDirectory() as td:
        for i in range(n):
            src = gen_program(rng, i)
            cfile = os.path.join(td, f"p{i}.c")
            with open(cfile, "w") as f:
                f.write(src)
            outs = {}
            ok = True
            for tag, cc, opt in [("g0", "gcc", "-O0"), ("g2", "gcc", "-O2"),
                                 ("l2", LCCC, "-O2"), ("l1", LCCC, "-O1"),
                                 ("l0", LCCC, "-O0")]:
                exe = os.path.join(td, f"p{i}.{tag}")
                r = run([cc, opt, cfile, "-o", exe])
                if r.returncode != 0:
                    print(f"prog {i}: {tag} COMPILE FAIL\n{r.stderr.decode()[:400]}")
                    ok = False; break
                r = run([exe], timeout=10)
                outs[tag] = (r.returncode, r.stdout)
            if not ok:
                fails += 1; continue
            if len(set(outs.values())) != 1:
                fails += 1
                print(f"prog {i}: MISMATCH " + " ".join(f"{k}={v[0]}:{v[1].strip()[:20]!r}" for k, v in outs.items()))
                keep = f"/home/user/v9chk/fuzzfail_{i}.c"
                with open(keep, "w") as f:
                    f.write(src)
                print(f"  saved {keep}")
    print(f"{'ALL GREEN' if fails == 0 else 'FAILURES'}: {n - fails}/{n} programs agree")
    return 1 if fails else 0

if __name__ == "__main__":
    sys.exit(main())
