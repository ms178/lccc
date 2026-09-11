#!/usr/bin/env python3
"""Deterministic FP codegen stress: exercises the PR #359 peephole paths.

Shapes targeted (all with real register reuse / repeated pool constants, so
the fp_register_loads overwrite-refinement and the fp_const_hoist pass get
real pressure):

  * scalar double/float accumulation over a loop with a repeated constant
    (addsd/mulsd .LCFP_*  ->  hoist target; gcc materializes once);
  * dot-product kernels (the dot8d shape that motivated #359);
  * branch/join FP diamonds (register reuse across CFG arms);
  * FP values live across a noinline call (caller-saved xmm reuse);
  * mixes of sd/ss widths and of float params / returns in xmm0-xmm7;
  * dead-side pure-overwrite reuse patterns (allocator reuses a loaded xmm
    for an unrelated later value: the fold-refinement accept side).

Each generated program prints a deterministic checksum; LCCC output must
equal gcc output byte-for-byte.
"""
import random, sys

def gen(seed: int) -> str:
    r = random.Random(seed)
    out = []
    w = out.append
    w("#include <stdio.h>")
    w("#include <stdint.h>")
    w("__attribute__((noinline)) double opaque(double x){ volatile int q=(int)x; (void)q; return x*1.0000000001 + 0.25; }")
    w("__attribute__((noinline)) void barrier(void){ volatile int q=0; (void)q; }")
    w("static double acc;")
    w("static unsigned long long mixv;")
    w("__attribute__((noinline)) static double dot(double *a, double *b, int n){")
    w("    double s = 0.0;")
    w("    for (int i = 0; i < n; i++) s += a[i]*b[i];")
    w("    return s;")
    w("}")
    w("int main(void){")
    # runtime values
    for i in range(6):
        w(f"    double v{i} = {r.uniform(-1e3,1e3):.10f};")
    consts = [0.0, 1.0, -1.0, 0.5, r.uniform(-100,100), r.uniform(-100,100), 3.0, 2.0]
    for i,c in enumerate(consts):
        w(f"    const double c{i} = {c:.17g};")

    # body: pick a few shapes
    nshapes = r.randint(4, 9)
    for _ in range(nshapes):
        kind = r.randint(0, 5)
        if kind == 0:  # hoist-friendly: many reads of the same const
            ci = r.randrange(len(consts))
            t = r.choice(["double","float"])
            loop = r.randint(20, 400)
            w(f"    double s_{_} = 0.0;")
            w(f"    for (int k{_} = 0; k{_} < {loop}; k{_}++) {{ s_{_} += v{r.randrange(6)} * c{ci}; if (k{_} & 1) s_{_} -= c{ci} * 0.5; }}")
            w(f"    acc += s_{_};")
        elif kind == 1:  # dot with reuse + constants
            w(f"    double aa{_}[5] = {{v0,v1,v2,v3,v4}}; double bb{_}[5] = {{c0+1,c1+2,c2+3,c3+4,c4+5}};")
            w(f"    acc += dot(aa{_},bb{_},5);")
        elif kind == 2:  # CFG join: both arms use the same const
            ci = r.randrange(len(consts))
            w(f"    double s_{_} = v{r.randrange(6)};")
            w(f"    if ((uintptr_t)&s_{_} & 1) {{ s_{_} += c{ci}; barrier(); }} else {{ s_{_} = c{ci} - v{r.randrange(6)}; }}")
            w(f"    acc += s_{_} * c{ci};")
        elif kind == 3:  # opaque call between FP ops (caller-saved churn)
            ci = r.randrange(len(consts))
            w(f"    double s_{_} = v{r.randrange(6)} + c{ci};")
            w(f"    s_{_} = opaque(s_{_}) + c{ci};")
            w(f"    s_{_} = s_{_} * c{ci} - opaque(v{r.randrange(6)});")
            w(f"    acc += s_{_};")
        elif kind == 4:  # float (ss) width + register reuse of an unrelated load
            ci = r.randrange(len(consts))
            w(f"    float g{_}a = (float)v0 + (float)c{ci};")
            w(f"    float g{_}b = g{_}a; for (int w{_}=0;w{_}<50;w{_}++) g{_}b = g{_}b*(float)0.5 + (float)c{r.randrange(len(consts))};")
            w(f"    acc += (double)g{_}b;")
        else:  # long straight-line const reads -> overwrite-reuse shapes
            ci = r.randrange(len(consts))
            w(f"    double s_{_} = v{r.randrange(6)};")
            for k in range(r.randint(2,6)):
                w(f"    s_{_} = s_{_} * c{ci} + v{r.randrange(6)};")
            w(f"    acc += s_{_};")

    w("    mixv = 0;")
    w("    double u = acc; unsigned long long h = 0x9e3779b97f4a7c15ULL;")
    w("    for (int i = 0; i < 64; i++) { h ^= (unsigned long long)(u * (i+1)); u = u*0.999 + 0.001; h = (h<<7)|(h>>57); }")
    w("    printf(\"fpstress %llu\\n\", (unsigned long long)h);")
    w("    return 0;")
    w("}")
    return "\n".join(out)

if __name__ == "__main__":
    seed = int(sys.argv[1])
    print(gen(seed))
